//! Observation elision for the compiled tiers (Observability Constitution §0.7).
//!
//! "Elision is erasure, not a branch. A statement elided by the active profile
//! does not exist in the compiled artifact — no flag check, no dead argument
//! evaluation, nothing. **The profile applies at lowering, before any tier's
//! codegen.**" This pass is that doctrine made real for the AOT-to-Rust tier: it
//! strips `Note`/`Warn`/`Record` (`Stmt::Observe`) and unwraps `During` spans
//! (scope-transparent — the body is spliced in place) from the statement tree
//! BEFORE the optimizer and codegen run, so observation can never perturb a
//! single instruction — not even indirectly through overflow/range analysis.
//! Locked by `obs_zero_cost_codegen_lock` (emitted-Rust byte-identity).
//!
//! The register VM and tree-walker keep their own observed lowering (they emit
//! events); this erasure is the compiled/release path only. For a program with
//! no observation the pass is a no-op — the tree is returned unchanged (`None`),
//! so the benchmark corpus and every ordinary program compile byte-for-byte as
//! before.

use logicaffeine_base::Arena;

use crate::ast::stmt::{MatchArm, Stmt};

/// Erase every observation statement from `stmts` (recursively, through every
/// nested block). Returns the rewritten tree, or `None` when nothing was
/// observed (the caller then keeps the original — zero allocation, zero change).
pub(crate) fn elide_observation<'a>(
    stmts: &[Stmt<'a>],
    stmt_arena: &'a Arena<Stmt<'a>>,
) -> Option<&'a [Stmt<'a>]> {
    Cx { stmt_arena }.block(stmts)
}

struct Cx<'a> {
    stmt_arena: &'a Arena<Stmt<'a>>,
}

impl<'a> Cx<'a> {
    /// Rewrite a statement list: drop `Observe`, splice `During` bodies, recurse
    /// into every other statement's nested blocks. `None` ⇒ unchanged.
    fn block(&mut self, stmts: &[Stmt<'a>]) -> Option<&'a [Stmt<'a>]> {
        let mut out: Option<Vec<Stmt<'a>>> = None;
        for (i, s) in stmts.iter().enumerate() {
            match s {
                // A `Note`/`Warn`/`Record`: gone. Materialize the prefix and skip.
                Stmt::Observe { .. } => {
                    out.get_or_insert_with(|| stmts[..i].to_vec());
                }
                // A `During` span is scope-transparent: replace it with its own
                // (recursively elided) body, spliced inline.
                Stmt::During { body, .. } => {
                    let elided = self.block(body).unwrap_or(body);
                    let v = out.get_or_insert_with(|| stmts[..i].to_vec());
                    v.extend_from_slice(elided);
                }
                other => match self.stmt(other) {
                    Some(ns) => match &mut out {
                        Some(v) => v.push(ns),
                        None => {
                            let mut v = stmts[..i].to_vec();
                            v.push(ns);
                            out = Some(v);
                        }
                    },
                    None => {
                        if let Some(v) = &mut out {
                            v.push(other.clone());
                        }
                    }
                },
            }
        }
        out.map(|v| self.stmt_arena.alloc_slice(v))
    }

    /// Recurse into a single statement's nested block(s). `None` ⇒ this statement
    /// (and everything under it) held no observation, so it is kept as-is.
    fn stmt(&mut self, s: &Stmt<'a>) -> Option<Stmt<'a>> {
        match s {
            // Two-block: If.
            Stmt::If {
                cond,
                then_block,
                else_block,
            } => {
                let nt = self.block(then_block);
                let ne = else_block.and_then(|e| self.block(e));
                if nt.is_none() && ne.is_none() {
                    return None;
                }
                Some(Stmt::If {
                    cond: *cond,
                    then_block: nt.unwrap_or(then_block),
                    else_block: ne.or(*else_block),
                })
            }

            // Arms: Inspect (each `MatchArm` carries a body block).
            Stmt::Inspect { arms, .. } => {
                let mut any = false;
                let mut new_arms: Vec<MatchArm<'a>> = Vec::with_capacity(arms.len());
                for arm in arms {
                    match self.block(arm.body) {
                        Some(nb) => {
                            any = true;
                            let mut a = arm.clone();
                            a.body = nb;
                            new_arms.push(a);
                        }
                        None => new_arms.push(arm.clone()),
                    }
                }
                if !any {
                    return None;
                }
                let mut c = s.clone();
                if let Stmt::Inspect { arms: a, .. } = &mut c {
                    *a = new_arms;
                }
                Some(c)
            }

            // Single-block variants — clone-and-patch the one body/tasks field.
            // (Robust to field churn: we never enumerate the other fields.)
            Stmt::While { body, .. }
            | Stmt::Repeat { body, .. }
            | Stmt::FunctionDef { body, .. }
            | Stmt::TestDef { body, .. }
            | Stmt::ExpectFail { body, .. }
            | Stmt::ForAll { body, .. }
            | Stmt::Splice { body, .. }
            | Stmt::Reversibly { body, .. }
            | Stmt::Zone { body, .. } => {
                let nb = self.block(body)?;
                let mut c = s.clone();
                match &mut c {
                    Stmt::While { body: b, .. }
                    | Stmt::Repeat { body: b, .. }
                    | Stmt::FunctionDef { body: b, .. }
                    | Stmt::TestDef { body: b, .. }
                    | Stmt::ExpectFail { body: b, .. }
                    | Stmt::ForAll { body: b, .. }
                    | Stmt::Splice { body: b, .. }
                    | Stmt::Reversibly { body: b, .. }
                    | Stmt::Zone { body: b, .. } => *b = nb,
                    _ => unreachable!("clone preserves the variant we just matched"),
                }
                Some(c)
            }

            // Task-block variants (concurrency): `tasks` is the nested block.
            Stmt::Concurrent { tasks, .. } | Stmt::Parallel { tasks, .. } => {
                let nb = self.block(tasks)?;
                let mut c = s.clone();
                match &mut c {
                    Stmt::Concurrent { tasks: t, .. } | Stmt::Parallel { tasks: t, .. } => *t = nb,
                    _ => unreachable!("clone preserves the variant we just matched"),
                }
                Some(c)
            }

            // No nested statements ⇒ nothing to elide here.
            _ => None,
        }
    }
}
