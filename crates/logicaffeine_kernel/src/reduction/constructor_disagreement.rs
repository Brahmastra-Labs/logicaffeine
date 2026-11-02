//! Closed data refuted by observation.
//!
//! Lean's kernel refutes `c₁ ā ≡ c₂ b̄` for two different constructors by comparing heads after
//! weak-head reduction; it never normalizes either side. This is that rule for closed data, on the
//! same checked lazy evaluator (and the same certified accelerations) that proves closed data
//! equal: both sides are forced to weak-head values, a pair of equal constructors descends into its
//! fields, and a pair of DIFFERENT constructors — or two different natural numbers — anywhere along
//! such a path means the two terms are not definitionally equal. A position the evaluator cannot
//! observe (a proof field, a computation it cannot finish) is passed over: it is evidence of
//! nothing, and the fields beside it still are.
//!
//! Every conclusion needs the values at the disagreeing position to be data, never proofs: the
//! constructor's family must have a sort that is `Type`-sized at every universe instance, so no
//! proposition (or `SProp`) family is ever refuted — two different proofs of one proposition are
//! equal by proof irrelevance. Open terms, holes, simp contexts and any unfinished observation give
//! no answer, and the caller keeps its ordinary conversion path.
use super::*;

/// Whether `a` and `b` provably differ as closed data. `false` means only "not refuted here".
pub(crate) fn constructors_disagree(ctx: &Context, a: &Term, b: &Term, fuel: u64) -> bool {
    if fuel == 0 || !ctx.simp_rules().is_empty() {
        return false;
    }
    let mut r = HeadReducer::new(ctx, fuel);
    let a = r.store.intern(a);
    let b = r.store.intern(b);
    if [a, b].into_iter().any(|t| {
        !r.store.free_names_are_empty(t)
            || r.store.shapes(t).contains(NodeShape::Hole)
            || !r.store.eq_faithful(t)
    }) {
        return false;
    }
    disagreement(&mut r, a, b) == Some(true)
}

/// A level that is not `Prop` (nor `SProp`) at any instantiation of its variables.
fn never_propositional(level: &crate::Universe) -> bool {
    use crate::Universe as U;
    match level {
        U::Type(_) | U::Succ(_) => true,
        U::Max(a, b) => never_propositional(a) || never_propositional(b),
        U::IMax(_, b) => never_propositional(b),
        U::SProp | U::Prop | U::Var(_) => false,
    }
}

/// The family of a constructor head whose values are data at every universe instance: its
/// declared type ends in a sort that is never `Prop`. The constructor's own declared type must end
/// in that family: the registry's claim alone is not the constructor's type (the legacy raw API
/// can file a constructor of `ActualUnit` under another inductive, and two inhabitants of a unit
/// type are equal by unit η however they are spelled).
fn data_family(r: &HeadReducer<'_>, head: TermId) -> Option<String> {
    let name = match r.store.node(head) {
        Node::Global(name) | Node::Const { name, .. } => r.store.name(*name),
        _ => return None,
    };
    let family = r.ctx.constructor_inductive(name)?;
    let constructor_type = match r.ctx.poly_declaration(name) {
        Some((_, ty)) => ty,
        None => r.ctx.get_global(name)?,
    };
    let mut result = constructor_type;
    while let Term::Pi { body_type, .. } = result {
        result = body_type;
    }
    while let Term::App(function, _) = result {
        result = function;
    }
    if !matches!(result, Term::Global(n) | Term::Const { name: n, .. } if n == family) {
        return None;
    }
    let declared = match r.ctx.poly_declaration(family) {
        Some((_, ty)) => ty,
        None => r.ctx.get_global(family)?,
    };
    let mut result = declared;
    while let Term::Pi { body_type, .. } = result {
        result = body_type;
    }
    match result {
        Term::Sort(level) if never_propositional(level) => Some(family.to_owned()),
        _ => None,
    }
}

/// `Some(true)`: a disagreement was observed. `Some(false)`: none was found (the terms may be
/// equal, or some observation was inconclusive). `None`: the allowance or the store bound ran out.
fn disagreement(r: &mut HeadReducer<'_>, a: TermId, b: TermId) -> Option<bool> {
    let a = r.delay(a, &None)?;
    let b = r.delay(b, &None)?;
    let mut work = vec![(a, b)];
    let mut visited = HashSet::new();
    while let Some((a, b)) = work.pop() {
        r.fuel = r.fuel.checked_sub(1)?;
        if !visited.insert((a.id, b.id)) {
            continue;
        }
        // A pair the evaluator cannot observe — a proof field, which it never forces, or a
        // computation it cannot finish — holds no evidence either way and is passed over. Only a
        // spent allowance ends the search.
        let (Some(left), Some(right)) = (r.force(a), r.force(b)) else {
            if r.spent() {
                return None;
            }
            continue;
        };
        if r.canonical_nat {
            if let (Some(left), Some(right)) =
                (r.nat_cursor(left.clone()), r.nat_cursor(right.clone()))
            {
                if natural_disagreement(r, left, right)? {
                    return Some(true);
                }
                continue;
            }
        }
        let (
            Value::Constructor {
                head: left_head,
                args: left_args,
            },
            Value::Constructor {
                head: right_head,
                args: right_args,
            },
        ) = (left, right)
        else {
            continue;
        };
        let (Some(left_family), Some(right_family)) =
            (data_family(r, left_head), data_family(r, right_head))
        else {
            continue;
        };
        let (Some(left_info), Some(right_info)) =
            (r.constructor(left_head), r.constructor(right_head))
        else {
            continue;
        };
        if left_family != right_family {
            continue;
        }
        if left_info.index != right_info.index {
            return Some(true);
        }
        if left_args.len() != left_info.arity || right_args.len() != right_info.arity {
            continue;
        }
        // Equal constructors are equal exactly when their fields are: descend into the fields.
        work.extend(
            left_args
                .into_iter()
                .zip(right_args)
                .skip(left_info.parameters),
        );
    }
    Some(false)
}

/// Two naturals observed as `count + tail` prefixes. `Some(true)` only when both are known far
/// enough to differ; an unknown tail is never assumed to be zero.
fn natural_disagreement(
    r: &mut HeadReducer<'_>,
    mut left: NatCursor,
    mut right: NatCursor,
) -> Option<bool> {
    loop {
        r.fuel = r.fuel.checked_sub(1)?;
        left = match left {
            NatCursor::Pending(tail) => {
                let value = r.force(tail)?;
                match r.nat_cursor(value) {
                    Some(cursor) => cursor,
                    None => return Some(false),
                }
            }
            known => known,
        };
        right = match right {
            NatCursor::Pending(tail) => {
                let value = r.force(tail)?;
                match r.nat_cursor(value) {
                    Some(cursor) => cursor,
                    None => return Some(false),
                }
            }
            known => known,
        };
        let NatCursor::Prefix { count: a, tail: at } = left else {
            return Some(false);
        };
        let NatCursor::Prefix { count: b, tail: bt } = right else {
            return Some(false);
        };
        match a.cmp(&b) {
            std::cmp::Ordering::Equal => match (at, bt) {
                (None, None) => return Some(false),
                (Some(at), Some(bt)) => {
                    left = NatCursor::Pending(at);
                    right = NatCursor::Pending(bt);
                }
                (Some(at), None) => {
                    left = NatCursor::Pending(at);
                    right = NatCursor::Prefix {
                        count: crate::BigInt::zero(),
                        tail: None,
                    };
                }
                (None, Some(bt)) => {
                    left = NatCursor::Prefix {
                        count: crate::BigInt::zero(),
                        tail: None,
                    };
                    right = NatCursor::Pending(bt);
                }
            },
            std::cmp::Ordering::Greater => {
                let Some(bt) = bt else { return Some(true) };
                left = NatCursor::Prefix {
                    count: a.sub(&b),
                    tail: at,
                };
                right = NatCursor::Pending(bt);
            }
            std::cmp::Ordering::Less => {
                let Some(at) = at else { return Some(true) };
                left = NatCursor::Pending(at);
                right = NatCursor::Prefix {
                    count: b.sub(&a),
                    tail: bt,
                };
            }
        }
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_kernel/reduction/shared_head/constructor_disagreement/controls.rs"]
mod controls;
