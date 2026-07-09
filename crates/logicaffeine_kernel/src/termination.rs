//! Termination checking for fixpoints.
//!
//! This module implements the syntactic guard condition that ensures
//! all recursive functions terminate. Without this check, the type system
//! would be unsound - we could "prove" False by writing `fix f. f`.
//!
//! The algorithm (following Coq):
//! 1. Identify the "structural parameter" - the first inductive-typed argument
//! 2. Track variables that are "structurally smaller" than the structural parameter
//! 3. Verify all recursive calls use a smaller argument
//!
//! A variable `k` is smaller than `n` if it was bound by matching on `n`:
//! `match n with Succ k => ...` means k < n structurally.

use std::collections::{HashMap, HashSet};

use crate::context::Context;
use crate::error::{KernelError, KernelResult};
use crate::term::Term;

/// Context for termination checking.
struct GuardContext {
    /// Each fixpoint name in the (possibly mutual) block → the ARGUMENT POSITION of its
    /// structural (decreasing) argument in a call. For a plain recursor the position is
    /// `0` (the scrutinee is the first argument); for an INDEXED family the scrutinee
    /// follows its index binders, so a call `rec i… smaller` must decrease the argument
    /// at that position. A single fixpoint is the ONE-ENTRY case; a mutual block lists
    /// every member, since a call to ANY member must decrease.
    fix_positions: HashMap<String, usize>,
    /// The structural parameter of the CURRENT body — every recursive call, to SELF or a
    /// sibling, must pass an argument structurally smaller than THIS one.
    struct_param: String,
    /// The type of the structural parameter (inductive name)
    struct_type: String,
    /// Variables known to be structurally smaller than struct_param
    smaller_than: HashSet<String>,
    /// False once an inner binder has shadowed `struct_param`. A `match` on that
    /// name then refers to the shadowing binding, not the structural argument, so
    /// it must NOT be treated as the guarding match.
    struct_param_live: bool,
    /// Fixpoint names shadowed by an inner binder — a call to such a name refers to the
    /// shadowing binding, not our block, so it is not a recursive call.
    shadowed_fix: HashSet<String>,
}

/// Check that a Fix term satisfies the syntactic guard condition.
///
/// This is the main entry point for termination checking.
pub fn check_termination(ctx: &Context, fix_name: &str, body: &Term) -> KernelResult<()> {
    // Extract the structural parameter (the scrutinee the body matches on) and its position.
    let (struct_param, struct_type, struct_pos, inner) = extract_structural_param(ctx, body)?;

    // A single fixpoint is a mutual block of one.
    let mut fix_positions = HashMap::new();
    fix_positions.insert(fix_name.to_string(), struct_pos);
    let guard_ctx = GuardContext {
        fix_positions,
        struct_param,
        struct_type,
        smaller_than: HashSet::new(),
        struct_param_live: true,
        shadowed_fix: HashSet::new(),
    };

    // Check all recursive calls are guarded
    check_guarded(ctx, &guard_ctx, inner)
}

/// Check that a MUTUAL block of fixpoints terminates — the mutual Giménez guard.
///
/// Every member's structural (decreasing) argument position is computed first; then
/// each body is checked so that a call to ANY member — itself OR a sibling — passes, at
/// that member's structural position, an argument structurally SMALLER than the CURRENT
/// body's structural parameter. Any infinite call chain would then demand an infinite
/// strictly-descending sequence of subterms, which cannot exist — so the whole block
/// terminates. `Even.rec`'s fixpoint calling `Odd.rec` on the sub-proof, and vice
/// versa, is exactly what this guards. It is the single-fix guard with the recursive
/// name generalized from one to a set of positions.
pub fn check_termination_mutual(ctx: &Context, defs: &[(String, Term)]) -> KernelResult<()> {
    if defs.is_empty() {
        return Err(KernelError::TerminationViolation {
            fix_name: String::new(),
            reason: "empty mutual fixpoint block".to_string(),
        });
    }
    // 1. Structural parameter (name + type) and position of every member.
    let mut fix_positions = HashMap::new();
    let mut params: Vec<(String, String)> = Vec::with_capacity(defs.len());
    for (name, body) in defs {
        let (struct_param, struct_type, struct_pos, _) = extract_structural_param(ctx, body)?;
        fix_positions.insert(name.clone(), struct_pos);
        params.push((struct_param, struct_type));
    }
    // 2. Check every body against the whole block.
    for (i, (_, body)) in defs.iter().enumerate() {
        let (_, _, _, inner) = extract_structural_param(ctx, body)?;
        let (struct_param, struct_type) = params[i].clone();
        let guard_ctx = GuardContext {
            fix_positions: fix_positions.clone(),
            struct_param,
            struct_type,
            smaller_than: HashSet::new(),
            struct_param_live: true,
            shadowed_fix: HashSet::new(),
        };
        check_guarded(ctx, &guard_ctx, inner)?;
    }
    Ok(())
}

/// Identify the structural parameter — the argument recursion decreases — as the binder the
/// fixpoint body's `match` actually discriminates on (the scrutinee), returning its name,
/// inductive type, argument POSITION in the λ-telescope, and the body just past it.
///
/// This is what lets an INDEXED family recurse: `le.rec`'s fixpoint is
/// `fix rec. λm:Nat. λh:le n m. match h …`, and the structural argument is the proof `h`
/// (position 1) — NOT the `Nat` index `m` (position 0), even though `Nat` is inductive too.
/// When the body does not match on a bound variable (a non-recursor fixpoint), we fall back
/// to the first inductive-typed binder, preserving the original behavior.
fn extract_structural_param<'a>(
    ctx: &Context,
    body: &'a Term,
) -> KernelResult<(String, String, usize, &'a Term)> {
    // Peel the λ-telescope, recording each binder and its type.
    let mut chain: Vec<(&'a str, &'a Term)> = Vec::new();
    let mut cur = body;
    while let Term::Lambda { param, param_type, body: inner } = cur {
        chain.push((param.as_str(), param_type.as_ref()));
        cur = inner;
    }

    // The scrutinee: the binder the innermost `match` discriminates on, if any.
    let scrutinee = match cur {
        Term::Match { discriminant, .. } => match discriminant.as_ref() {
            Term::Var(d) => chain.iter().position(|(n, _)| n == d),
            _ => None,
        },
        _ => None,
    };

    // Choose the scrutinee when it is inductive-typed; otherwise the first inductive-typed
    // binder (the legacy shape for non-indexed recursors and plain fixpoints).
    let pos = scrutinee
        .filter(|&p| extract_inductive_name(ctx, chain[p].1).is_some())
        .or_else(|| chain.iter().position(|(_, t)| extract_inductive_name(ctx, t).is_some()))
        .ok_or_else(|| KernelError::TerminationViolation {
            fix_name: String::new(),
            reason: "No inductive parameter found for structural recursion".to_string(),
        })?;

    let name = chain[pos].0.to_string();
    let ind = extract_inductive_name(ctx, chain[pos].1).unwrap();

    // The body just past the chosen structural λ, so `check_guarded` neither re-descends
    // into that binder nor marks it shadowed.
    let mut inner = body;
    for _ in 0..=pos {
        if let Term::Lambda { body: b, .. } = inner {
            inner = b;
        }
    }
    Ok((name, ind, pos, inner))
}

/// Extract the inductive type name from a type.
///
/// Handles both simple inductives like `Nat` and polymorphic ones like `List A`.
fn extract_inductive_name(ctx: &Context, ty: &Term) -> Option<String> {
    match ty {
        Term::Global(name) if ctx.is_inductive(name) => Some(name.clone()),
        Term::App(func, _) => extract_inductive_name(ctx, func),
        _ => None,
    }
}

/// Build the guard context for the scope under a binder that introduces `param`.
///
/// The binder shadows any prior meaning of `param`: within its body `param` no
/// longer refers to the structural parameter, a structurally-smaller variable,
/// or the fixpoint itself. Failing to honor this lets an inner `match` on a
/// shadowed name be mistaken for the guarding match (admitting a non-decreasing
/// recursion), or an inner binding of a smaller-marked name keep its "smaller"
/// status after being rebound to an arbitrary value.
fn enter_binder(guard_ctx: &GuardContext, param: &str) -> GuardContext {
    let mut child = GuardContext {
        fix_positions: guard_ctx.fix_positions.clone(),
        struct_param: guard_ctx.struct_param.clone(),
        struct_type: guard_ctx.struct_type.clone(),
        smaller_than: guard_ctx.smaller_than.clone(),
        struct_param_live: guard_ctx.struct_param_live,
        shadowed_fix: guard_ctx.shadowed_fix.clone(),
    };
    child.smaller_than.remove(param);
    if param == guard_ctx.struct_param {
        child.struct_param_live = false;
    }
    if guard_ctx.fix_positions.contains_key(param) {
        child.shadowed_fix.insert(param.to_string());
    }
    child
}

/// Check that all recursive calls in `term` are guarded (use smaller arguments).
fn check_guarded(ctx: &Context, guard_ctx: &GuardContext, term: &Term) -> KernelResult<()> {
    match term {
        // Application. Peel the spine into (head, args-in-order). The recursive name is allowed to
        // occur ONLY here, as the head of a call whose structural argument is smaller; in that position
        // the head is *consumed* by the application and must NOT be re-descended as a bare value. Every
        // argument is still checked, so a recursive name smuggled into an argument (`g f`) is caught by
        // the `Var` leaf below.
        Term::App(func, arg) => {
            let mut args: Vec<&Term> = vec![arg.as_ref()];
            let mut head = func.as_ref();
            while let Term::App(inner_func, inner_arg) = head {
                args.push(inner_arg.as_ref());
                head = inner_func.as_ref();
            }
            args.reverse();

            if let Term::Var(name) = head {
                if !guard_ctx.shadowed_fix.contains(name) {
                    if let Some(&pos) = guard_ctx.fix_positions.get(name) {
                        // A recursive call — to this member or a sibling. The argument at
                        // THAT member's structural position must be structurally smaller
                        // than the CURRENT body's structural parameter.
                        match args.get(pos) {
                            Some(sarg) => verify_structural_arg_smaller(guard_ctx, sarg)?,
                            None => {
                                return Err(KernelError::TerminationViolation {
                                    fix_name: name.clone(),
                                    reason: format!(
                                        "recursive call to '{}' is missing its structural argument (position {})",
                                        name, pos
                                    ),
                                })
                            }
                        }
                        for a in &args {
                            check_guarded(ctx, guard_ctx, a)?;
                        }
                        return Ok(());
                    }
                }
            }

            // Not a recursive call at the head: check the head and every argument normally.
            check_guarded(ctx, guard_ctx, head)?;
            for a in &args {
                check_guarded(ctx, guard_ctx, a)?;
            }
            Ok(())
        }

        // Match on structural parameter introduces smaller variables
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            // The return motive is an ordinary subterm (in the current scope) and MUST be
            // guarded too — a recursive occurrence hidden in the return predicate would
            // otherwise evade the check, diverging from the standard CIC guard.
            check_guarded(ctx, guard_ctx, motive)?;
            // Check if we're matching on the structural parameter (and that it
            // has not been shadowed by an inner binder).
            if let Term::Var(disc_name) = discriminant.as_ref() {
                if guard_ctx.struct_param_live && disc_name == &guard_ctx.struct_param {
                    // This match guards recursive calls - check cases with
                    // constructor-bound variables marked as smaller
                    return check_match_cases_guarded(ctx, guard_ctx, cases);
                }
            }

            // Not matching on structural param - just recurse normally
            check_guarded(ctx, guard_ctx, discriminant)?;
            for case in cases {
                check_guarded(ctx, guard_ctx, case)?;
            }
            Ok(())
        }

        // Lambda: guard the DOMAIN annotation (current scope) as well as the body — a
        // recursive occurrence in a binder's type must not evade the check. The binder may
        // shadow the structural parameter, a smaller variable, or the fixpoint name.
        Term::Lambda { param, param_type, body } => {
            check_guarded(ctx, guard_ctx, param_type)?;
            let child = enter_binder(guard_ctx, param);
            check_guarded(ctx, &child, body)
        }

        // Pi: same domain + binder-shadowing handling as Lambda.
        Term::Pi { param, param_type, body_type } => {
            check_guarded(ctx, guard_ctx, param_type)?;
            let child = enter_binder(guard_ctx, param);
            check_guarded(ctx, &child, body_type)
        }

        // Nested fixpoint: its own name shadows ours; its body gets its own
        // termination check when type-checked.
        Term::Fix { name, body } => {
            let child = enter_binder(guard_ctx, name);
            check_guarded(ctx, &child, body)
        }

        // Nested MUTUAL fixpoint: all of its names shadow ours; each body gets its own
        // mutual termination check when type-checked.
        Term::MutualFix { defs, .. } => {
            let mut child = enter_binder(guard_ctx, &defs[0].0);
            for (n, _) in &defs[1..] {
                child = enter_binder(&child, n);
            }
            for (_, b) in defs {
                check_guarded(ctx, &child, b)?;
            }
            Ok(())
        }

        // Let: the type and value are in the outer scope; the body is under the
        // let-binder, which may shadow the tracked names. The value is NOT
        // registered as smaller (the conservative v1 guard — a `fix` whose
        // recursive call passes a let-alias of a smaller variable is rejected).
        Term::Let { name, ty, value, body, .. } => {
            check_guarded(ctx, guard_ctx, ty)?;
            check_guarded(ctx, guard_ctx, value)?;
            let child = enter_binder(guard_ctx, name);
            check_guarded(ctx, &child, body)
        }

        // A bare occurrence of a block member's name is the higher-order escape: `f` used
        // as a first-class value (returned, or passed as an argument) rather than applied
        // to a structurally-smaller argument. Only fully-applied decreasing calls are
        // guarded (Giménez 1995; the Coq guard), so reject it. Valid recursive calls never
        // reach here — their head is consumed by the `App` arm above.
        Term::Var(name)
            if guard_ctx.fix_positions.contains_key(name)
                && !guard_ctx.shadowed_fix.contains(name) =>
        {
            Err(KernelError::TerminationViolation {
                fix_name: name.clone(),
                reason: format!(
                    "recursive name '{}' occurs as a first-class value, not applied to a structurally-smaller argument",
                    name
                ),
            })
        }

        // Other leaves: no recursive calls possible.
        Term::Sort(_) | Term::Var(_) | Term::Global(_) | Term::Lit(_) | Term::Hole
        | Term::Const { .. } => Ok(()),
    }
}

/// Verify the structural (first) argument of a recursive call is structurally smaller
/// than the decreasing parameter. Two admissible shapes:
///
/// - a bare variable bound by matching on the structural parameter (`x` in
///   `match n with Succ x => … rec x …`); or
/// - an APPLICATION `h a₁ … aₙ` whose HEAD `h` is such a smaller variable
///   (Giménez's rule). This is what makes recursion over an accessibility proof
///   terminate: `Acc_intro`'s field `h : Π(y). R y x → Acc A R y` is bound by the
///   match and marked smaller, and `h y hr` is a sub-`Acc`-proof it contains.
///
/// The applied form is SOUND precisely because strict positivity (`positivity.rs`)
/// guarantees every functional constructor field places the inductive only in a
/// codomain-result position — so applying such a field can only yield a proper
/// SUBTERM, never a larger one. A field placing the inductive in a domain (the
/// `(Bad → …) → Bad` paradox) is rejected at inductive-registration, so it never
/// reaches this guard.
impl GuardContext {
    /// A stable name for the fixpoint block, for error messages.
    fn block_name(&self) -> String {
        let mut names: Vec<&String> = self.fix_positions.keys().collect();
        names.sort();
        names.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("/")
    }
}

fn verify_structural_arg_smaller(guard_ctx: &GuardContext, first_arg: &Term) -> KernelResult<()> {
    // Peel the application spine to its head.
    let mut head = first_arg;
    while let Term::App(f, _) = head {
        head = f;
    }
    match head {
        Term::Var(arg_name) if guard_ctx.smaller_than.contains(arg_name) => Ok(()),
        Term::Var(arg_name) => Err(KernelError::TerminationViolation {
            fix_name: guard_ctx.block_name(),
            reason: format!(
                "Recursive call with '{}' which is not structurally smaller than '{}'",
                arg_name, guard_ctx.struct_param
            ),
        }),
        _ => Err(KernelError::TerminationViolation {
            fix_name: guard_ctx.block_name(),
            reason: "Recursive call whose structural argument is not headed by a \
                     structurally-smaller variable"
                .to_string(),
        }),
    }
}

/// Check match cases with structural variables marked as smaller.
fn check_match_cases_guarded(
    ctx: &Context,
    guard_ctx: &GuardContext,
    cases: &[Term],
) -> KernelResult<()> {
    // Get constructors for the inductive type
    let constructors = ctx.get_constructors(&guard_ctx.struct_type);

    for (case, (ctor_name, ctor_type)) in cases.iter().zip(constructors.iter()) {
        // Count constructor parameters
        let param_count = count_pi_params(ctor_type);

        // Extract the smaller variables from this case
        // The case is typically: λx1. λx2. ... λxn. body
        // where x1..xn are the constructor parameters
        let (smaller_vars, case_body) = extract_lambda_params(case, param_count);

        // Create extended guard context with these variables as smaller
        let mut extended_ctx = GuardContext {
            fix_positions: guard_ctx.fix_positions.clone(),
            struct_param: guard_ctx.struct_param.clone(),
            struct_type: guard_ctx.struct_type.clone(),
            smaller_than: guard_ctx.smaller_than.clone(),
            struct_param_live: guard_ctx.struct_param_live,
            shadowed_fix: guard_ctx.shadowed_fix.clone(),
        };

        // Mark every constructor parameter as structurally smaller.
        //
        // Soundness: we only reach here when the discriminant is the structural
        // parameter itself (see `check_guarded`'s `Match` arm), so the matched
        // value is `ctor x1 … xn` and each `xi` is a *direct argument* of that
        // constructor — hence a genuine structural subterm of the parameter.
        // Recursing on any `xi` therefore decreases. Parameters of a foreign
        // type are still marked, but a recursive call on one cannot type-check
        // (the fixpoint expects the structural type), and complex (non-variable)
        // recursive arguments are rejected outright — so the over-approximation
        // never admits a non-terminating fixpoint. `ctor_name`/`ctor_type` are
        // bound for the constructor-arity computation above.
        let _ = ctor_name;
        for var in &smaller_vars {
            extended_ctx.smaller_than.insert(var.clone());
            // A constructor binder may reuse the name of the structural parameter
            // or the fixpoint; inside this case that name now refers to the
            // (smaller) bound variable, so the original meaning is shadowed.
            if var == &guard_ctx.struct_param {
                extended_ctx.struct_param_live = false;
            }
            if guard_ctx.fix_positions.contains_key(var) {
                extended_ctx.shadowed_fix.insert(var.clone());
            }
        }

        // Check the case body with the extended context
        check_guarded(ctx, &extended_ctx, case_body)?;
    }

    Ok(())
}

/// Count the number of Pi parameters in a type.
fn count_pi_params(ty: &Term) -> usize {
    match ty {
        Term::Pi { body_type, .. } => 1 + count_pi_params(body_type),
        _ => 0,
    }
}

/// Extract lambda parameters and return (param_names, body).
fn extract_lambda_params(term: &Term, count: usize) -> (Vec<String>, &Term) {
    if count == 0 {
        return (Vec::new(), term);
    }

    match term {
        Term::Lambda { param, body, .. } => {
            let (mut params, final_body) = extract_lambda_params(body, count - 1);
            params.insert(0, param.clone());
            (params, final_body)
        }
        _ => (Vec::new(), term),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::term::Universe;

    /// A context with `Nat` (with `Zero`/`Succ`) and `False : Prop` — enough to exercise the guard.
    fn nat_context() -> Context {
        let mut ctx = Context::new();
        let nat = Term::Global("Nat".to_string());
        ctx.add_inductive("Nat", Term::Sort(Universe::Type(0)));
        ctx.add_constructor("Zero", "Nat", nat.clone());
        ctx.add_constructor(
            "Succ",
            "Nat",
            Term::Pi { param: "_".to_string(), param_type: Box::new(nat.clone()), body_type: Box::new(nat) },
        );
        ctx.add_inductive("False", Term::Sort(Universe::Prop));
        ctx
    }

    fn nat() -> Term {
        Term::Global("Nat".to_string())
    }

    fn app(f: Term, x: Term) -> Term {
        Term::App(Box::new(f), Box::new(x))
    }

    fn var(n: &str) -> Term {
        Term::Var(n.to_string())
    }

    /// THE SOUNDNESS RED TEST (CRITIQUE finding #1). The structural-recursion guard must reject a
    /// fixpoint that smuggles its own recursive name `f` as a FIRST-CLASS ARGUMENT — `(λg:Nat→False. g
    /// Zero) f` — instead of applying it to a structurally-smaller value. With the higher-order escape,
    /// `f` is visited only as an inert `Var` leaf, the guard passes, and `boom Zero : False` inhabits
    /// `False` with zero axioms. The guard MUST reject this body.
    #[test]
    fn recursive_name_smuggled_as_a_first_class_argument_is_rejected() {
        let ctx = nat_context();
        let nat_to_false = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(nat()),
            body_type: Box::new(Term::Global("False".to_string())),
        };

        // Zero case: (λg:Nat→False. g Zero) f  — `f` (the fixpoint) passed as an argument.
        let zero_case = app(
            Term::Lambda {
                param: "g".to_string(),
                param_type: Box::new(nat_to_false),
                body: Box::new(app(var("g"), Term::Global("Zero".to_string()))),
            },
            var("f"),
        );
        // Succ case: λk:Nat. f k  — a genuinely-guarded recursive call (only the Zero case escapes).
        let succ_case = Term::Lambda {
            param: "k".to_string(),
            param_type: Box::new(nat()),
            body: Box::new(app(var("f"), var("k"))),
        };
        let body = Term::Lambda {
            param: "n".to_string(),
            param_type: Box::new(nat()),
            body: Box::new(Term::Match {
                discriminant: Box::new(var("n")),
                motive: Box::new(Term::Lambda {
                    param: "_".to_string(),
                    param_type: Box::new(nat()),
                    body: Box::new(Term::Global("False".to_string())),
                }),
                cases: vec![zero_case, succ_case],
            }),
        };

        let result = check_termination(&ctx, "f", &body);
        assert!(
            result.is_err(),
            "kernel soundness: a fixpoint that passes its recursive name as a first-class value inhabits \
             False and MUST be rejected by the termination guard, but it was accepted"
        );
    }

    /// Regression guard: genuine structural recursion `fix f. λn. match n with Zero => Zero | Succ k => f k`
    /// must still pass. The fix for the escape must not reject honest fully-applied decreasing calls.
    #[test]
    fn genuine_structural_recursion_still_passes() {
        let ctx = nat_context();
        let succ_case = Term::Lambda {
            param: "k".to_string(),
            param_type: Box::new(nat()),
            body: Box::new(app(var("f"), var("k"))), // f k — k is structurally smaller
        };
        let body = Term::Lambda {
            param: "n".to_string(),
            param_type: Box::new(nat()),
            body: Box::new(Term::Match {
                discriminant: Box::new(var("n")),
                motive: Box::new(Term::Lambda {
                    param: "_".to_string(),
                    param_type: Box::new(nat()),
                    body: Box::new(nat()),
                }),
                cases: vec![Term::Global("Zero".to_string()), succ_case],
            }),
        };
        assert!(check_termination(&ctx, "f", &body).is_ok(), "honest structural recursion must still pass");
    }

    /// The recursive name returned bare from a branch (`match n with Zero => f | …`) is the same escape
    /// in a different costume — `f` as a value, not a guarded call. Must be rejected.
    #[test]
    fn recursive_name_returned_bare_from_a_branch_is_rejected() {
        let ctx = nat_context();
        let succ_case = Term::Lambda {
            param: "k".to_string(),
            param_type: Box::new(nat()),
            body: Box::new(app(var("f"), var("k"))),
        };
        let body = Term::Lambda {
            param: "n".to_string(),
            param_type: Box::new(nat()),
            body: Box::new(Term::Match {
                discriminant: Box::new(var("n")),
                motive: Box::new(Term::Lambda {
                    param: "_".to_string(),
                    param_type: Box::new(nat()),
                    body: Box::new(nat()),
                }),
                cases: vec![var("f"), succ_case], // Zero => f  (bare recursive name)
            }),
        };
        assert!(check_termination(&ctx, "f", &body).is_err(), "a branch returning the bare fixpoint must be rejected");
    }

    /// APPLIED-SMALLER FENCE #1 (the `Acc` extension's over-admission risk). The
    /// applied-smaller rule admits `rec (h a…)` ONLY when the HEAD `h` is a
    /// structurally-smaller variable. A CONSTRUCTOR head is not smaller: `f (Succ k)`
    /// passes `Succ k`, which is strictly LARGER than the matched `k`. Accepting it
    /// would make `fix f. λn. match n with Zero => … | Succ k => f (Succ k)` loop
    /// forever (it recurses on a value bigger than the one it destructed). The guard
    /// MUST reject the constructor-headed structural argument.
    #[test]
    fn recursive_call_on_a_constructor_applied_to_a_smaller_var_is_rejected() {
        let ctx = nat_context();
        let succ_case = Term::Lambda {
            param: "k".to_string(),
            param_type: Box::new(nat()),
            // f (Succ k) — structural argument headed by the constructor `Succ`, not a smaller var.
            body: Box::new(app(var("f"), app(Term::Global("Succ".to_string()), var("k")))),
        };
        let body = Term::Lambda {
            param: "n".to_string(),
            param_type: Box::new(nat()),
            body: Box::new(Term::Match {
                discriminant: Box::new(var("n")),
                motive: Box::new(Term::Lambda {
                    param: "_".to_string(),
                    param_type: Box::new(nat()),
                    body: Box::new(nat()),
                }),
                cases: vec![Term::Global("Zero".to_string()), succ_case],
            }),
        };
        assert!(
            check_termination(&ctx, "f", &body).is_err(),
            "kernel soundness: a recursive call on `Succ k` (larger than the matched `k`) must be rejected"
        );
    }

    /// APPLIED-SMALLER FENCE #2. The applied head must be a SMALLER variable, not
    /// merely *some* variable. Here `h` is an ordinary function parameter — never
    /// bound by the guarding match, so not marked smaller — and `f h (h k)` recurses
    /// on `h k`. Since `h` could be `Succ`, `h k` is not a subterm of `n`; the guard
    /// must reject applying a non-smaller variable in the structural position.
    #[test]
    fn recursive_call_on_a_non_smaller_variable_applied_is_rejected() {
        let ctx = nat_context();
        let nat_to_nat = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(nat()),
            body_type: Box::new(nat()),
        };
        let succ_case = Term::Lambda {
            param: "k".to_string(),
            param_type: Box::new(nat()),
            // f h (h k) — structural argument (position 1) is `h k`, headed by the non-smaller `h`.
            body: Box::new(app(app(var("f"), var("h")), app(var("h"), var("k")))),
        };
        let body = Term::Lambda {
            param: "h".to_string(),
            param_type: Box::new(nat_to_nat),
            body: Box::new(Term::Lambda {
                param: "n".to_string(),
                param_type: Box::new(nat()),
                body: Box::new(Term::Match {
                    discriminant: Box::new(var("n")),
                    motive: Box::new(Term::Lambda {
                        param: "_".to_string(),
                        param_type: Box::new(nat()),
                        body: Box::new(nat()),
                    }),
                    cases: vec![Term::Global("Zero".to_string()), succ_case],
                }),
            }),
        };
        assert!(
            check_termination(&ctx, "f", &body).is_err(),
            "kernel soundness: recursing on `h k` where `h` is not structurally smaller must be rejected"
        );
    }

    /// AUDIT FIX: a non-decreasing recursive call hidden in the match RETURN MOTIVE must be
    /// caught. The guard traverses the motive (as the standard CIC guard does), not only the
    /// discriminant and cases — otherwise `f n` here would evade the check.
    #[test]
    fn recursive_call_hidden_in_the_match_motive_is_rejected() {
        let ctx = nat_context();
        let succ_case = Term::Lambda {
            param: "k".to_string(),
            param_type: Box::new(nat()),
            body: Box::new(app(var("f"), var("k"))),
        };
        let body = Term::Lambda {
            param: "n".to_string(),
            param_type: Box::new(nat()),
            body: Box::new(Term::Match {
                discriminant: Box::new(var("n")),
                // motive λ_:Nat. (f n) — a non-decreasing recursive occurrence.
                motive: Box::new(Term::Lambda {
                    param: "_".to_string(),
                    param_type: Box::new(nat()),
                    body: Box::new(app(var("f"), var("n"))),
                }),
                cases: vec![Term::Global("Zero".to_string()), succ_case],
            }),
        };
        assert!(
            check_termination(&ctx, "f", &body).is_err(),
            "a recursive call in the match motive must be rejected"
        );
    }

    /// AUDIT FIX: a non-decreasing recursive call hidden in a binder's DOMAIN annotation
    /// must be caught — the guard traverses `λ`/`Π` parameter types too.
    #[test]
    fn recursive_call_hidden_in_a_binder_domain_is_rejected() {
        let ctx = nat_context();
        // Succ case: λk. λ(_ : f n). f k  — `f n` sits in the inner λ's domain.
        let succ_case = Term::Lambda {
            param: "k".to_string(),
            param_type: Box::new(nat()),
            body: Box::new(Term::Lambda {
                param: "_".to_string(),
                param_type: Box::new(app(var("f"), var("n"))),
                body: Box::new(app(var("f"), var("k"))),
            }),
        };
        let body = Term::Lambda {
            param: "n".to_string(),
            param_type: Box::new(nat()),
            body: Box::new(Term::Match {
                discriminant: Box::new(var("n")),
                motive: Box::new(Term::Lambda {
                    param: "_".to_string(),
                    param_type: Box::new(nat()),
                    body: Box::new(nat()),
                }),
                cases: vec![Term::Global("Zero".to_string()), succ_case],
            }),
        };
        assert!(
            check_termination(&ctx, "f", &body).is_err(),
            "a recursive call in a binder domain must be rejected"
        );
    }

    /// The classic non-terminating shape `fix f. λn. f n` — a recursive call on the structural
    /// parameter itself, which does not decrease. Must be rejected.
    #[test]
    fn non_decreasing_recursive_call_is_rejected() {
        let ctx = nat_context();
        let body = Term::Lambda {
            param: "n".to_string(),
            param_type: Box::new(nat()),
            body: Box::new(app(var("f"), var("n"))), // f n — n is the parameter, not smaller
        };
        assert!(check_termination(&ctx, "f", &body).is_err(), "a non-decreasing self-call must be rejected");
    }
}

