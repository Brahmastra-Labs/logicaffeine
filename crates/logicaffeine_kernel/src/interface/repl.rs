//! REPL for the Vernacular interface.
//!
//! Orchestrates command parsing and kernel execution.

use super::command::Command;
use super::command_parser::parse_command;
use super::error::InterfaceError;
use crate::prelude::StandardLibrary;
use crate::{
    auto_bind_implicits, bind_self_recursion, derive_recursor, fill_match_motives, infer_type,
    normalize, surface_elaborate, surface_elaborate_against, Context, Term,
};

/// The Vernacular REPL.
///
/// Maintains a kernel context and executes commands against it.
pub struct Repl {
    ctx: Context,
}

impl Repl {
    /// Create a new REPL with the standard library loaded.
    pub fn new() -> Self {
        let mut ctx = Context::new();
        StandardLibrary::register(&mut ctx);
        Self { ctx }
    }

    /// Execute a command string.
    ///
    /// Returns the output string (for Check/Eval) or empty string (for Definition/Inductive).
    pub fn execute(&mut self, input: &str) -> Result<String, InterfaceError> {
        let cmd = parse_command(input)?;

        match cmd {
            Command::Definition { name, ty, body, is_hint, implicit_count } => {
                // Auto-bind free type variables (`id : A -> A`) as leading implicits, then
                // elaborate the body against its declared type — implicit arguments are
                // filled from the body's own arguments AND from the expected type.
                let (ty, body, implicit_count) = match ty {
                    Some(t) => {
                        let (t, b, k) = auto_bind_implicits(&self.ctx, &t, &body, implicit_count);
                        (Some(t), b, k)
                    }
                    None => (None, body, implicit_count),
                };
                // Recursive-definition sugar: a body that calls the definition by name is
                // bound with a `fix` (the kernel's termination guard certifies it).
                let body = bind_self_recursion(&name, &body);
                let body = fill_match_motives(&self.ctx, &body, ty.as_ref())?;
                let body = surface_elaborate_against(&self.ctx, &body, ty.as_ref())?;
                let inferred_ty = infer_type(&self.ctx, &body)?;

                // Use provided type or inferred type
                let ty = ty.unwrap_or(inferred_ty);

                // Add definition to context
                self.ctx.add_definition(name.clone(), ty, body);

                // Record how many leading arguments are implicit, so later uses of this
                // global (`name 0`) get them inserted and inferred.
                if implicit_count > 0 {
                    self.ctx.set_implicit_args(&name, implicit_count);
                }

                // Register as hint if marked
                if is_hint {
                    self.ctx.add_hint(&name);
                }

                Ok(String::new()) // Silent success
            }

            Command::Check(term) => {
                // Infer `match` motives, insert implicit arguments, then infer the type.
                let term = fill_match_motives(&self.ctx, &term, None)?;
                let term = surface_elaborate(&self.ctx, &term)?;
                let ty = infer_type(&self.ctx, &term)?;
                Ok(format!("{} : {}", term, ty))
            }

            Command::Eval(term) => {
                // Infer `match` motives, insert implicits, type check, then normalize.
                let term = fill_match_motives(&self.ctx, &term, None)?;
                let term = surface_elaborate(&self.ctx, &term)?;
                let _ = infer_type(&self.ctx, &term)?;
                let result = normalize(&self.ctx, &term);
                Ok(format!("{}", result))
            }

            Command::Inductive {
                name,
                params,
                sort,
                constructors,
            } => {
                // Build polymorphic sort: Π(p1:T1). Π(p2:T2). ... Type
                let poly_sort = build_polymorphic_sort(&params, sort);

                // Register the inductive type with its polymorphic sort
                self.ctx.add_inductive(&name, poly_sort);

                // Register constructors with prepended parameters
                for (ctor_name, ctor_ty) in constructors {
                    // Prepend params to constructor type:
                    // If ctor_ty = A -> List A -> List A
                    // And params = [(A, Type)]
                    // Result = Π(A:Type). A -> List A -> List A
                    let poly_ctor_ty = build_polymorphic_constructor(&params, ctor_ty);
                    // Strict positivity is enforced on the trusted, user-facing
                    // registration path: a negative-recursive constructor (e.g.
                    // `Cons : (Bad -> False) -> Bad`) would let one inhabit `False`.
                    self.ctx
                        .add_constructor_checked(&ctor_name, &name, poly_ctor_ty)?;
                }

                // Auto-derive the recursor (the dependent eliminator) — declaring an
                // inductive gives you induction/recursion for free, as `{Name}_rec`. If
                // derivation is not yet supported for this shape, the inductive is still
                // usable; we simply skip the eliminator.
                if let Ok((rec_ty, rec_term)) = derive_recursor(&self.ctx, &name) {
                    self.ctx.add_definition(format!("{}_rec", name), rec_ty, rec_term);
                }

                Ok(String::new()) // Silent success
            }
        }
    }

    /// Execute a batch of Vernacular commands, recovering mutual-inductive grouping.
    ///
    /// [`execute`](Self::execute) registers each `Inductive` independently, so a forward
    /// reference between two separately-declared inductives — `Tree` whose `Node`
    /// constructor mentions `Forest`, declared next — fails: `Forest` is not yet in scope
    /// when `Tree`'s constructor is universe-checked. True mutual inductives must be
    /// registered together, through the trusted [`Context::add_mutual_inductives`], which
    /// runs whole-block positivity and a header-first universe check.
    ///
    /// This entry point recovers that grouping from a flat statement sequence: a maximal
    /// run of consecutive `Inductive` commands is split into strongly-connected components
    /// of the "a constructor mentions sibling `b`" graph. A genuine cycle (`Tree`↔`Forest`)
    /// is registered as one mutual block; an acyclic reference (`Leafy` → `Bare`) becomes
    /// two singletons registered dependency-first (`Bare`, then `Leafy`) through the
    /// ordinary single-inductive path. Every non-inductive command — and every inductive
    /// with no forward reference — runs exactly as `execute` would, in source order, so a
    /// batch without forward references is byte-identical to running the statements singly.
    pub fn execute_batch(&mut self, inputs: &[String]) -> Vec<Result<String, InterfaceError>> {
        let parsed: Vec<Option<Command>> =
            inputs.iter().map(|s| parse_command(s).ok()).collect();
        let mut out: Vec<Option<Result<String, InterfaceError>>> =
            (0..inputs.len()).map(|_| None).collect();
        let mut i = 0;
        while i < inputs.len() {
            if !matches!(parsed[i], Some(Command::Inductive { .. })) {
                out[i] = Some(self.execute(&inputs[i]));
                i += 1;
                continue;
            }
            // A maximal run of consecutive inductive commands, registered as a group.
            let start = i;
            while i < inputs.len() && matches!(parsed[i], Some(Command::Inductive { .. })) {
                i += 1;
            }
            let run: Vec<(&str, &Command)> = (start..i)
                .map(|k| (inputs[k].as_str(), parsed[k].as_ref().unwrap()))
                .collect();
            for (k, r) in (start..i).zip(self.register_inductive_run(&run)) {
                out[k] = Some(r);
            }
        }
        out.into_iter().map(|o| o.expect("every position is filled")).collect()
    }

    /// Register one maximal run of `Inductive` commands, grouping mutual cycles into
    /// blocks and registering acyclic references dependency-first. See [`execute_batch`].
    fn register_inductive_run(
        &mut self,
        run: &[(&str, &Command)],
    ) -> Vec<Result<String, InterfaceError>> {
        use std::collections::{BTreeSet, HashMap};
        let n = run.len();
        let names: Vec<&str> = run
            .iter()
            .map(|(_, c)| match c {
                Command::Inductive { name, .. } => name.as_str(),
                _ => unreachable!("run holds only Inductive commands"),
            })
            .collect();
        let name_idx: HashMap<&str, usize> =
            names.iter().enumerate().map(|(k, &nm)| (nm, k)).collect();

        // Dependency edges: `a → b` iff a constructor of member `a` mentions sibling `b`
        // (b ≠ a; a self-reference is handled inside single/mutual registration).
        let mut deps: Vec<BTreeSet<usize>> = (0..n).map(|_| BTreeSet::new()).collect();
        for (a, (_, cmd)) in run.iter().enumerate() {
            if let Command::Inductive { constructors, .. } = cmd {
                let mut mentioned = BTreeSet::new();
                for (_, cty) in constructors.iter() {
                    collect_global_names(cty, &mut mentioned);
                }
                for name in &mentioned {
                    if let Some(&b) = name_idx.get(name.as_str()) {
                        if b != a {
                            deps[a].insert(b);
                        }
                    }
                }
            }
        }

        let comps = tarjan_scc(&deps);
        let mut comp_of = vec![0usize; n];
        for (cid, comp) in comps.iter().enumerate() {
            for &node in comp {
                comp_of[node] = cid;
            }
        }
        // Condensation edges: which components each component references.
        let mut comp_deps: Vec<BTreeSet<usize>> =
            (0..comps.len()).map(|_| BTreeSet::new()).collect();
        for (a, edges) in deps.iter().enumerate() {
            for &b in edges {
                if comp_of[a] != comp_of[b] {
                    comp_deps[comp_of[a]].insert(comp_of[b]);
                }
            }
        }
        // Emit components dependency-first (a referenced component before its referrer),
        // tie-breaking by earliest source position so independent inductives keep order.
        let mut emitted = vec![false; comps.len()];
        let mut order: Vec<usize> = Vec::with_capacity(comps.len());
        while order.len() < comps.len() {
            let mut pick: Option<usize> = None;
            for c in 0..comps.len() {
                if emitted[c] || !comp_deps[c].iter().all(|d| emitted[*d]) {
                    continue;
                }
                let earliest = *comps[c].iter().min().unwrap();
                match pick {
                    Some(p) if earliest >= *comps[p].iter().min().unwrap() => {}
                    _ => pick = Some(c),
                }
            }
            let c = pick.expect("an SCC condensation is a DAG — a ready component exists");
            emitted[c] = true;
            order.push(c);
        }

        let mut results: Vec<Result<String, InterfaceError>> =
            (0..n).map(|_| Ok(String::new())).collect();
        for &cid in &order {
            let comp = &comps[cid];
            if comp.len() == 1 {
                // Singleton (including a self-recursive inductive): the ordinary path, which
                // also derives the recursor. Its dependencies are already registered.
                let node = comp[0];
                results[node] = self.execute(run[node].0);
            } else {
                // A genuine mutual cycle → one block through the trusted mutual machinery.
                let block: Vec<crate::context::MutualInductive> = comp
                    .iter()
                    .map(|&node| match run[node].1 {
                        Command::Inductive { name, params, sort, constructors } => {
                            crate::context::MutualInductive {
                                name: name.clone(),
                                sort: build_polymorphic_sort(params, sort.clone()),
                                num_params: params.len(),
                                constructors: constructors
                                    .iter()
                                    .map(|(cn, cty)| {
                                        (cn.clone(), build_polymorphic_constructor(params, cty.clone()))
                                    })
                                    .collect(),
                            }
                        }
                        _ => unreachable!(),
                    })
                    .collect();
                match self.ctx.add_mutual_inductives(&block) {
                    Ok(()) => {
                        // Best-effort recursor per member, mirroring the single path.
                        for &node in comp {
                            if let Command::Inductive { name, .. } = run[node].1 {
                                if let Ok((rec_ty, rec_term)) = derive_recursor(&self.ctx, name) {
                                    self.ctx.add_definition(
                                        format!("{}_rec", name),
                                        rec_ty,
                                        rec_term,
                                    );
                                }
                            }
                        }
                    }
                    Err(e) => {
                        let msg =
                            format!("mutual inductive block failed to register: {e}");
                        for &node in comp {
                            results[node] = Err(InterfaceError::Kernel(
                                crate::KernelError::CertificationError(msg.clone()),
                            ));
                        }
                    }
                }
            }
        }
        results
    }

    /// Get a reference to the underlying context.
    pub fn context(&self) -> &Context {
        &self.ctx
    }

    /// Get a mutable reference to the underlying context.
    pub fn context_mut(&mut self) -> &mut Context {
        &mut self.ctx
    }
}

impl Default for Repl {
    fn default() -> Self {
        Self::new()
    }
}

/// Build a polymorphic sort from type parameters.
///
/// For params = [(A, Type), (B, Type)] and base_sort = Type,
/// produces: Π(A:Type). Π(B:Type). Type
fn build_polymorphic_sort(params: &[(String, Term)], base_sort: Term) -> Term {
    // Fold right to build nested Pi types
    params.iter().rev().fold(base_sort, |body, (name, ty)| {
        Term::Pi {
            param: name.clone(),
            param_type: Box::new(ty.clone()),
            body_type: Box::new(body),
        }
    })
}

/// Build a polymorphic constructor type by prepending parameters.
///
/// For params = [(A, Type)] and ctor_ty = A -> List A -> List A,
/// produces: Π(A:Type). A -> List A -> List A
///
/// The kernel uses named variables (Var(String)), so we convert
/// Global("A") to Var("A") for parameters.
fn build_polymorphic_constructor(params: &[(String, Term)], ctor_ty: Term) -> Term {
    if params.is_empty() {
        return ctor_ty;
    }

    // Convert Global(param_name) to Var(param_name) for all parameters
    let param_names: Vec<&str> = params.iter().map(|(n, _)| n.as_str()).collect();
    let body = substitute_globals_with_vars(&ctor_ty, &param_names);

    // Wrap with Pi bindings (fold right to build nested Pi)
    params.iter().rev().fold(body, |body, (name, ty)| {
        Term::Pi {
            param: name.clone(),
            param_type: Box::new(ty.clone()),
            body_type: Box::new(body),
        }
    })
}

/// Convert Global(name) to Var(name) for names in the param list.
/// This makes parameter references in constructor types into bound variables.
fn substitute_globals_with_vars(term: &Term, param_names: &[&str]) -> Term {
    match term {
        Term::Global(n) if param_names.contains(&n.as_str()) => Term::Var(n.clone()),
        Term::Global(n) => Term::Global(n.clone()),
        Term::Const { .. } => term.clone(),
        Term::Var(n) => Term::Var(n.clone()),
        Term::Sort(u) => Term::Sort(u.clone()),
        Term::Lit(l) => Term::Lit(l.clone()),
        Term::App(f, a) => Term::App(
            Box::new(substitute_globals_with_vars(f, param_names)),
            Box::new(substitute_globals_with_vars(a, param_names)),
        ),
        Term::Lambda { param, param_type, body } => Term::Lambda {
            param: param.clone(),
            param_type: Box::new(substitute_globals_with_vars(param_type, param_names)),
            body: Box::new(substitute_globals_with_vars(body, param_names)),
        },
        Term::Pi { param, param_type, body_type } => Term::Pi {
            param: param.clone(),
            param_type: Box::new(substitute_globals_with_vars(param_type, param_names)),
            body_type: Box::new(substitute_globals_with_vars(body_type, param_names)),
        },
        Term::Fix { name, body } => Term::Fix {
            name: name.clone(),
            body: Box::new(substitute_globals_with_vars(body, param_names)),
        },
        Term::MutualFix { defs, index } => Term::MutualFix {
            defs: defs
                .iter()
                .map(|(n, b)| (n.clone(), substitute_globals_with_vars(b, param_names)))
                .collect(),
            index: *index,
        },
        Term::Match { discriminant, motive, cases } => Term::Match {
            discriminant: Box::new(substitute_globals_with_vars(discriminant, param_names)),
            motive: Box::new(substitute_globals_with_vars(motive, param_names)),
            cases: cases
                .iter()
                .map(|c| substitute_globals_with_vars(c, param_names))
                .collect(),
        },
        Term::Let { name, ty, value, body } => Term::Let {
            name: name.clone(),
            ty: Box::new(substitute_globals_with_vars(ty, param_names)),
            value: Box::new(substitute_globals_with_vars(value, param_names)),
            body: Box::new(substitute_globals_with_vars(body, param_names)),
        },
        Term::Hole => Term::Hole, // Holes are unchanged
    }
}

/// Collect the names of every `Global`/`Const` referenced anywhere in a term — used to
/// find which sibling inductives a constructor mentions.
fn collect_global_names(term: &Term, out: &mut std::collections::BTreeSet<String>) {
    match term {
        Term::Global(n) => {
            out.insert(n.clone());
        }
        Term::Const { name, .. } => {
            out.insert(name.clone());
        }
        Term::App(f, a) => {
            collect_global_names(f, out);
            collect_global_names(a, out);
        }
        Term::Pi { param_type, body_type, .. } => {
            collect_global_names(param_type, out);
            collect_global_names(body_type, out);
        }
        Term::Lambda { param_type, body, .. } => {
            collect_global_names(param_type, out);
            collect_global_names(body, out);
        }
        Term::Match { discriminant, motive, cases } => {
            collect_global_names(discriminant, out);
            collect_global_names(motive, out);
            for c in cases {
                collect_global_names(c, out);
            }
        }
        Term::Fix { body, .. } => collect_global_names(body, out),
        Term::MutualFix { defs, .. } => {
            for (_, b) in defs {
                collect_global_names(b, out);
            }
        }
        Term::Let { ty, value, body, .. } => {
            collect_global_names(ty, out);
            collect_global_names(value, out);
            collect_global_names(body, out);
        }
        Term::Var(_) | Term::Sort(_) | Term::Lit(_) | Term::Hole => {}
    }
}

/// Tarjan's strongly-connected components over a small adjacency list. Each returned
/// vector is one SCC (node indices, ascending); a self-referential singleton is its own
/// SCC of size one. Runs are tiny (a handful of adjacent inductives), so recursion is safe.
fn tarjan_scc(adj: &[std::collections::BTreeSet<usize>]) -> Vec<Vec<usize>> {
    struct State<'a> {
        adj: &'a [std::collections::BTreeSet<usize>],
        index: Vec<usize>,
        low: Vec<usize>,
        on_stack: Vec<bool>,
        stack: Vec<usize>,
        counter: usize,
        sccs: Vec<Vec<usize>>,
    }
    impl State<'_> {
        fn connect(&mut self, v: usize) {
            self.index[v] = self.counter;
            self.low[v] = self.counter;
            self.counter += 1;
            self.stack.push(v);
            self.on_stack[v] = true;
            let neighbors: Vec<usize> = self.adj[v].iter().copied().collect();
            for w in neighbors {
                if self.index[w] == usize::MAX {
                    self.connect(w);
                    self.low[v] = self.low[v].min(self.low[w]);
                } else if self.on_stack[w] {
                    self.low[v] = self.low[v].min(self.index[w]);
                }
            }
            if self.low[v] == self.index[v] {
                let mut comp = Vec::new();
                loop {
                    let w = self.stack.pop().unwrap();
                    self.on_stack[w] = false;
                    comp.push(w);
                    if w == v {
                        break;
                    }
                }
                comp.sort_unstable();
                self.sccs.push(comp);
            }
        }
    }
    let n = adj.len();
    let mut st = State {
        adj,
        index: vec![usize::MAX; n],
        low: vec![0; n],
        on_stack: vec![false; n],
        stack: Vec::new(),
        counter: 0,
        sccs: Vec::new(),
    };
    for v in 0..n {
        if st.index[v] == usize::MAX {
            st.connect(v);
        }
    }
    st.sccs
}

#[cfg(test)]
mod mutual_batch_tests {
    use super::*;

    fn ctor_names(repl: &Repl, ind: &str) -> Vec<String> {
        repl.context()
            .get_constructors(ind)
            .iter()
            .map(|(n, _)| n.to_string())
            .collect()
    }

    #[test]
    fn mutual_forward_reference_registers_both_constructor_sets() {
        // `Tree.Node : Forest -> Tree` forward-references `Forest`, declared next; the two
        // form a cycle and must register together as a mutual block.
        let mut repl = Repl::new();
        let stmts = vec![
            "Inductive Tree := Leaf : Tree | Node : Forest -> Tree.".to_string(),
            "Inductive Forest := Nil2 : Forest | Grow : Tree -> Forest -> Forest.".to_string(),
        ];
        let results = repl.execute_batch(&stmts);
        assert!(
            results.iter().all(|r| r.is_ok()),
            "batch must register cleanly: {results:?}"
        );
        assert!(
            ctor_names(&repl, "Tree").contains(&"Node".to_string()),
            "Tree.Node (forward ref to Forest) must register: {:?}",
            ctor_names(&repl, "Tree")
        );
        assert!(
            ctor_names(&repl, "Forest").contains(&"Grow".to_string()),
            "Forest.Grow must register: {:?}",
            ctor_names(&repl, "Forest")
        );
        assert!(
            repl.context().mutual_block_of("Tree").is_some(),
            "a genuine cycle must be recorded as a mutual block"
        );
    }

    #[test]
    fn acyclic_forward_reference_registers_without_a_mutual_block() {
        // `Leafy.MkLeafy : Bare -> Leafy` references `Bare`, declared next, but `Bare` does
        // NOT reference back — no cycle, so two singletons registered `Bare`-first.
        let mut repl = Repl::new();
        let stmts = vec![
            "Inductive Leafy := MkLeafy : Bare -> Leafy.".to_string(),
            "Inductive Bare := MkBare : Bare.".to_string(),
        ];
        let results = repl.execute_batch(&stmts);
        assert!(
            results.iter().all(|r| r.is_ok()),
            "batch must register cleanly: {results:?}"
        );
        assert!(
            ctor_names(&repl, "Leafy").contains(&"MkLeafy".to_string()),
            "Leafy.MkLeafy (ref to Bare, declared later) must register"
        );
        assert!(ctor_names(&repl, "Bare").contains(&"MkBare".to_string()));
        assert!(
            repl.context().mutual_block_of("Leafy").is_none(),
            "an acyclic reference must NOT be recorded as mutual"
        );
    }

    #[test]
    fn plain_inductive_batch_matches_single_execution() {
        // No forward references: the batch path is byte-identical to running the statement
        // on its own — same constructors registered, in the same order.
        let stmts =
            vec!["Inductive Color := Red : Color | Green : Color | Blue : Color.".to_string()];
        let mut batched = Repl::new();
        assert!(batched.execute_batch(&stmts).iter().all(|r| r.is_ok()));
        let mut single = Repl::new();
        assert!(single.execute(&stmts[0]).is_ok());
        assert_eq!(ctor_names(&batched, "Color"), ctor_names(&single, "Color"));
    }
}
