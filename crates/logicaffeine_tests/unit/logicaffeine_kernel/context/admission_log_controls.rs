use super::*;
use crate::prelude::StandardLibrary;
use std::collections::BTreeSet;

fn prop() -> Term {
    Term::Sort(Universe::Prop)
}

fn g(name: &str) -> Term {
    Term::Global(name.into())
}

fn small() -> Context {
    let mut c = Context::new();
    c.set_auto_auxiliaries(false);
    c.add_inductive("P", prop());
    c.add_constructor("p", "P", g("P"));
    c
}

/// The log, oldest entry first: each name, and whether the entry removes it.
fn entries(ctx: &Context) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    let mut at = ctx.admissions();
    while let Some(entry) = at {
        out.push((entry.name.clone(), entry.removed));
        at = entry.prev.as_ref();
    }
    out.reverse();
    out
}

fn same_head(a: &Context, b: &Context) -> bool {
    match (a.admissions(), b.admissions()) {
        (Some(x), Some(y)) => Arc::ptr_eq(x, y),
        (None, None) => true,
        _ => false,
    }
}

/// Every key of every registry a declaration lands in.
fn registered(ctx: &Context) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    names.extend(ctx.inductives.keys().cloned());
    names.extend(ctx.constructors.keys().cloned());
    names.extend(ctx.declarations.keys().cloned());
    names.extend(ctx.definitions.keys().cloned());
    names.extend(ctx.universe_polys.keys().cloned());
    names.extend(ctx.poly_types.keys().cloned());
    names.extend(ctx.inductive_params.keys().cloned());
    names.extend(ctx.inductive_inferred_params.keys().cloned());
    names.extend(ctx.quotient_primitives.keys().cloned());
    names
}

#[test]
fn every_name_the_standard_library_registers_is_logged() {
    let ctx = StandardLibrary::context();
    let added: BTreeSet<String> = entries(ctx)
        .into_iter()
        .filter(|(_, removed)| !removed)
        .map(|(n, _)| n)
        .collect();
    let registered = registered(ctx);
    let unlogged: Vec<&String> = registered.iter().filter(|n| !added.contains(*n)).collect();
    assert!(
        unlogged.is_empty(),
        "registered without a log entry: {unlogged:?}"
    );
}

#[test]
fn every_registration_of_a_small_context_is_logged_in_order() {
    let mut c = small();
    c.add_definition("x".into(), g("P"), g("p"));
    c.add_declaration("ax", g("P"));
    let names: Vec<String> = entries(&c)
        .into_iter()
        .map(|(n, removed)| {
            assert!(!removed, "nothing was removed");
            n
        })
        .collect();
    let first = |n: &str| {
        names
            .iter()
            .position(|m| m == n)
            .unwrap_or_else(|| panic!("`{n}` is logged: {names:?}"))
    };
    assert!(
        first("P") < first("p") && first("p") < first("x") && first("x") < first("ax"),
        "{names:?}"
    );
}

#[test]
fn a_redeclaration_logs_one_removal_between_its_registrations() {
    let mut c = small();
    c.add_definition("x".into(), g("P"), g("p"));
    c.add_definition("x".into(), g("P"), g("p"));
    let of_x: Vec<bool> = entries(&c)
        .into_iter()
        .filter(|(n, _)| n == "x")
        .map(|(_, removed)| removed)
        .collect();
    let removal = of_x
        .iter()
        .position(|r| *r)
        .expect("the redeclaration removed `x`");
    assert_eq!(
        of_x.iter().filter(|r| **r).count(),
        1,
        "one removal: {of_x:?}"
    );
    assert!(
        removal > 0 && removal + 1 < of_x.len(),
        "the removal sits between the two registrations: {of_x:?}"
    );
}

#[test]
fn displacing_an_absent_name_logs_nothing() {
    let mut c = small();
    let before = c.clone();
    c.displace("never_registered");
    assert!(same_head(&c, &before), "the log is untouched");
}

#[test]
fn a_clone_shares_its_log_and_diverges_on_write() {
    let c = small();
    let mut d = c.clone();
    assert!(same_head(&c, &d), "a clone shares the log");
    d.add_declaration("ax", g("P"));
    assert!(!same_head(&c, &d), "a write moves only the writer's head");
    let prev = d.admissions().and_then(|e| e.prev.as_ref());
    assert!(
        matches!((prev, c.admissions()), (Some(x), Some(y)) if Arc::ptr_eq(x, y)),
        "the writer's log extends the original's"
    );
    assert_eq!(entries(&c).len() + 1, entries(&d).len());
}

#[test]
fn revoking_the_standard_quotient_logs_each_primitive_removed() {
    let mut ctx = StandardLibrary::context().clone();
    let certificate = ctx
        .prelude_quotient
        .clone()
        .expect("the standard library certifies its quotient");
    let primitives: BTreeSet<String> = certificate.names().map(str::to_string).collect();
    let dependency = certificate
        .dependencies
        .iter()
        .next()
        .expect("the certificate has dependencies")
        .clone();
    let before = entries(&ctx).len();
    ctx.invalidate_certificates(&dependency);
    assert!(ctx.prelude_quotient.is_none(), "revoked");
    let removed: BTreeSet<String> = entries(&ctx)[before..]
        .iter()
        .filter(|(_, removed)| *removed)
        .map(|(n, _)| n.clone())
        .collect();
    assert_eq!(
        removed, primitives,
        "each revoked primitive is logged removed"
    );
}

#[test]
fn the_standard_quotient_is_logged_when_certified_and_not_again_when_recertified() {
    let ctx = StandardLibrary::context();
    let primitives: Vec<String> = ctx
        .prelude_quotient
        .as_ref()
        .expect("certified")
        .names()
        .map(str::to_string)
        .collect();
    let log = entries(ctx);
    let first_add = |n: &str| {
        log.iter()
            .position(|(m, r)| m == n && !r)
            .unwrap_or_else(|| panic!("`{n}` is logged"))
    };
    let last_add = |n: &str| log.iter().rposition(|(m, r)| m == n && !r).expect("logged");
    let declared = primitives
        .iter()
        .map(|n| first_add(n))
        .max()
        .expect("four primitives");
    for n in &primitives {
        assert!(
            last_add(n) > declared,
            "`{n}` is logged again once all four are declared, when certified"
        );
    }
    let rec = first_add("Quot_rec");
    for n in &primitives {
        assert!(
            last_add(n) < rec,
            "`{n}` is not logged again by the re-certification after `Quot_rec`"
        );
    }
}

#[test]
fn the_observational_constants_are_logged_when_certified_and_not_again_when_recertified() {
    let mut ctx = StandardLibrary::context().clone();
    let before = entries(&ctx).len();
    crate::observational::register(&mut ctx).expect("registers");
    let certificate = ctx.observational.clone().expect("certified");
    let log = entries(&ctx)[before..].to_vec();
    let first_add = |n: &str| {
        log.iter()
            .position(|(m, r)| m == n && !r)
            .unwrap_or_else(|| panic!("`{n}` is logged"))
    };
    let last_add = |n: &str| log.iter().rposition(|(m, r)| m == n && !r).expect("logged");
    let declared = certificate
        .names()
        .map(first_add)
        .max()
        .expect("four constants");
    for n in certificate.names() {
        assert!(
            last_add(n) > declared,
            "`{n}` is logged again once all four are declared, when certified"
        );
    }
    let length = entries(&ctx).len();
    ctx.certify_observational(crate::observational::Certificate {
        constants: certificate.constants.clone(),
        dependencies: certificate.dependencies.clone(),
    });
    assert_eq!(
        entries(&ctx).len(),
        length,
        "recertifying the same constants logs nothing"
    );
}

#[test]
fn revoking_the_observational_certificate_logs_each_constant_removed() {
    let mut ctx = StandardLibrary::context().clone();
    crate::observational::register(&mut ctx).expect("registers");
    let certificate = ctx.observational.clone().expect("certified");
    let constants: BTreeSet<String> = certificate.names().map(str::to_string).collect();
    let dependency = certificate
        .dependencies
        .iter()
        .find(|d| !constants.contains(*d))
        .expect("a connective")
        .clone();
    let before = entries(&ctx).len();
    ctx.invalidate_certificates(&dependency);
    assert!(ctx.observational.is_none(), "revoked");
    let removed: BTreeSet<String> = entries(&ctx)[before..]
        .iter()
        .filter(|(_, removed)| *removed)
        .map(|(n, _)| n.clone())
        .collect();
    assert_eq!(
        removed, constants,
        "each revoked constant is logged removed"
    );
}

#[test]
fn a_long_log_drops_without_recursion() {
    std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(|| {
            let mut c = Context::new();
            for i in 0..200_000 {
                c.log(format!("n{i}"), false);
            }
            drop(c);
        })
        .expect("spawns")
        .join()
        .expect("a log drops in constant stack");
}
