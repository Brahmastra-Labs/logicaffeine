//! The value engine keeps one value per meaning, however deeply terms nest — the property its
//! memory and time bounds rest on.
//!
//! A closure captures its environment pruned to the entries its body can see. Were entries it
//! cannot see kept, two closures with the same meaning built under different surrounding binders
//! would be different values: no pointer equality, no cache hit, and a conversion between them
//! descends one binder deeper at every step (the Arena's con-leche, where this grew a checker
//! past 46 GB). Terms whose loose variables reach index 64 and beyond are described by an exact
//! set rather than the store's 64-bit mask, and must be pruned all the same.

use logicaffeine_kernel::dag::env::Safety;
use logicaffeine_kernel::dag::expr::ExprId;
use logicaffeine_kernel::dag::sem::value::{CtxId, EnvId, V};
use logicaffeine_kernel::dag::sem::Sem;
use logicaffeine_kernel::dag::Kernel;
use std::rc::Rc;

/// An environment of `len` entries: position `i` holds `pick(i)`'s sort value.
fn env_of(sem: &mut Sem, len: u32, pick: impl Fn(u32) -> u64) -> EnvId {
    let mut env = EnvId::EMPTY;
    for i in (0..len).rev() {
        let l = sem.k.s.levels.of_nat(pick(i));
        let v = sem.st.arena.sort(l);
        env = sem.st.arena.extend(env, v);
    }
    env
}

/// `fun (x : Prop) => #(near + 1) #(far + 1)`: under its own binder it sees entries `near` and
/// `far` of the environment it is evaluated in.
fn closure_seeing(k: &mut Kernel, near: u32, far: u32) -> ExprId {
    let prop = k.s.prop();
    let a = k.s.bvar(near + 1);
    let b = k.s.bvar(far + 1);
    let body = k.s.app(a, b);
    k.s.lam(prop, body)
}

fn closures_in_two_contexts(near: u32, far: u32, len: u32) -> (V, V) {
    let mut k = Kernel::new();
    let e = closure_seeing(&mut k, near, far);
    k.with_decl(Rc::from(Vec::new()), Safety::Safe, |k| {
        k.with_sem(|sem| {
            let seen = |i: u32| if i == near || i == far { 1 } else { 0 };
            let one = env_of(sem, len, |i| {
                if seen(i) == 1 {
                    7 + i as u64
                } else {
                    100 + i as u64
                }
            });
            let two = env_of(sem, len, |i| {
                if seen(i) == 1 {
                    7 + i as u64
                } else {
                    300 + i as u64
                }
            });
            assert_ne!(
                one, two,
                "the two environments differ in entries the closure cannot see"
            );
            Ok((sem.eval(one, e)?, sem.eval(two, e)?))
        })
    })
    .unwrap()
}

#[test]
fn a_closure_is_one_value_whatever_its_unseen_entries() {
    let (a, b) = closures_in_two_contexts(0, 5, 12);
    assert_eq!(a, b, "a narrow closure captures only what it sees");
}

#[test]
fn a_closure_reaching_past_64_binders_is_one_value_whatever_its_unseen_entries() {
    let (a, b) = closures_in_two_contexts(0, 66, 70);
    assert_eq!(
        a, b,
        "a closure seeing entry 66 is pruned exactly, not captured whole"
    );
}

#[test]
fn a_closure_seeing_only_entries_past_64_is_one_value_whatever_its_unseen_entries() {
    let (a, b) = closures_in_two_contexts(64, 90, 100);
    assert_eq!(
        a, b,
        "a closure whose every loose variable is past 64 is pruned exactly"
    );
}

/// `(λx. (λx. … (f x₁ … xₙ) … ) a) a` with `n` redexes, closed but for `f` (bound past the
/// ladder): every level's body sees every variable bound so far.
fn dense_ladder(k: &mut Kernel, n: u32) -> ExprId {
    let prop = k.s.prop();
    let a = k.s.prop();
    let mut body = k.s.bvar(n);
    for i in (0..n).rev() {
        let x = k.s.bvar(i);
        body = k.s.app(body, x);
    }
    for _ in 0..n {
        let lam = k.s.lam(prop, body);
        body = k.s.app(lam, a);
    }
    body
}

#[test]
fn an_environment_a_wide_term_sees_entirely_is_not_copied() {
    let n = 200;
    let mut k = Kernel::new();
    let e = dense_ladder(&mut k, n);
    let slots = k
        .with_decl(Rc::from(Vec::new()), Safety::Safe, |k| {
            k.with_sem(|sem| {
                let ty = sem
                    .st
                    .arena
                    .sort(logicaffeine_kernel::dag::level::LevelId::ZERO);
                sem.under(ty, |sem, f| {
                    let env = sem.st.arena.extend(EnvId::EMPTY, f);
                    let before = sem.st.arena.frame_slot_count();
                    let v = sem.eval(env, e)?;
                    let logicaffeine_kernel::dag::sem::value::Val::Neu { spine, .. } =
                        sem.st.arena.get(v)
                    else {
                        panic!("the ladder reduces to a neutral")
                    };
                    assert_eq!(
                        sem.st.arena.spine_len(spine),
                        n as usize,
                        "f applied to all {n} variables"
                    );
                    Ok(sem.st.arena.frame_slot_count() - before)
                })
            })
        })
        .unwrap();
    assert!(
        slots < 4 * n as usize,
        "each level sees every entry of its environment, so none is pruned into a copy ({slots} frame slots for {n} levels)"
    );
}

/// `(λx : Type. (λx : Type. … (f x₁ … xₙ) … ) Prop) Prop` with `n` redexes, `f : Type → … → Type`
/// bound past the ladder: well typed, every level's body seeing every variable bound so far.
fn typed_ladder(k: &mut Kernel, n: u32) -> (ExprId, ExprId) {
    let one = k.s.levels.of_nat(1);
    let ty = k.s.sort(one);
    let prop = k.s.prop();
    let mut body = k.s.bvar(n);
    for i in (0..n).rev() {
        let x = k.s.bvar(i);
        body = k.s.app(body, x);
    }
    for _ in 0..n {
        let lam = k.s.lam(ty, body);
        body = k.s.app(lam, prop);
    }
    let f_ty = (0..n).fold(ty, |b, _| k.s.pi(ty, b));
    (body, f_ty)
}

/// Values the value engine makes inferring, with checking, a `typed_ladder` of `n` redexes.
fn values_checking_ladder(n: u32) -> usize {
    let mut k = Kernel::new();
    let (e, f_ty) = typed_ladder(&mut k, n);
    k.with_decl(Rc::from(Vec::new()), Safety::Safe, |k| {
        k.with_sem(|sem| {
            let fv = sem.eval(EnvId::EMPTY, f_ty)?;
            sem.under(fv, |sem, f| {
                let env = sem.st.arena.extend(EnvId::EMPTY, f);
                let ctx = sem.st.arena.ctx_extend(CtxId::EMPTY, fv);
                let before = sem.st.arena.len();
                sem.infer(env, ctx, e, true)?;
                Ok(sem.st.arena.len() - before)
            })
        })
    })
    .unwrap()
}

/// A checked `λ` whose body's type mentions no local has the plain type `Π (_ : A). T`, which the
/// application around it instantiates without inferring the body a second time. Were each level
/// inferred again at its argument, a ladder of redexes would cost the square of its depth (the
/// Arena's beta-ladder, 4 G instructions at depth 2000): doubling the depth doubles the work.
#[test]
fn checking_a_ladder_of_redexes_infers_each_body_once() {
    let (half, full) = (values_checking_ladder(200), values_checking_ladder(400));
    assert!(
        full < 3 * half,
        "{full} values at depth 400 against {half} at depth 200: the work is not linear"
    );
}
