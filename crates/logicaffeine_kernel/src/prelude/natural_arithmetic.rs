//! Natural arithmetic as ordinary checked terms. No name-based reduction rules.
use super::{apps_, gl, lam_, pi_, v_};
use crate::{Context, Term, Universe};

fn nat() -> Term {
    gl("Nat")
}
fn pair_ty() -> Term {
    apps_(gl("Prod"), &[nat(), nat()])
}
fn pair(q: Term, r: Term) -> Term {
    apps_(gl("Prod_mk"), &[nat(), nat(), q, r])
}
fn succ(n: Term) -> Term {
    apps_(gl("Succ"), &[n])
}
fn bin_ty(result: Term) -> Term {
    pi_("m", nat(), pi_("n", nat(), result))
}
fn match_nat(scrutinee: Term, result: Term, zero: Term, successor: Term) -> Term {
    Term::Match {
        discriminant: Box::new(scrutinee),
        motive: Box::new(lam_("ignored", nat(), result)),
        cases: vec![zero, successor],
    }
}
fn fix(name: &str, body: Term) -> Term {
    Term::Fix {
        name: name.into(),
        body: Box::new(body),
    }
}
fn checked(ctx: &mut Context, name: &str, ty: Term, body: Term) {
    ctx.declare(name, ty, body)
        .unwrap_or_else(|e| panic!("natural arithmetic definition {name}: {e}"));
}

pub(crate) fn division_definitions() -> &'static [(&'static str, Term, Term)] {
    static DEFINITIONS: std::sync::OnceLock<Vec<(&'static str, Term, Term)>> =
        std::sync::OnceLock::new();
    DEFINITIONS.get_or_init(|| {
        let mut definitions = Vec::new();
        // Induction on the dividend maintains q * (k+1) + r = n with r < k+1.
        // Keeping q and r in one product avoids two independent division traversals.
        let advance = Term::Match {
            discriminant: Box::new(apps_(gl("natLt"), &[v_("r"), v_("k")])),
            motive: Box::new(lam_("ignored", gl("Bool"), pair_ty())),
            cases: vec![
                pair(v_("q"), succ(v_("r"))),
                pair(succ(v_("q")), gl("Zero")),
            ],
        };
        let step = Term::Match {
            discriminant: Box::new(apps_(v_("divmod"), &[v_("j"), v_("k")])),
            motive: Box::new(lam_("ignored", pair_ty(), pair_ty())),
            cases: vec![lam_("q", nat(), lam_("r", nat(), advance))],
        };
        definitions.push((
            "Nat_divModSucc",
            bin_ty(pair_ty()),
            fix(
                "divmod",
                lam_(
                    "n",
                    nat(),
                    lam_(
                        "k",
                        nat(),
                        match_nat(
                            v_("n"),
                            pair_ty(),
                            pair(gl("Zero"), gl("Zero")),
                            lam_("j", nat(), step),
                        ),
                    ),
                ),
            ),
        ));
        definitions.push((
            "Nat_divMod",
            bin_ty(pair_ty()),
            lam_(
                "n",
                nat(),
                lam_(
                    "d",
                    nat(),
                    match_nat(
                        v_("d"),
                        pair_ty(),
                        pair(gl("Zero"), v_("n")),
                        lam_("k", nat(), apps_(gl("Nat_divModSucc"), &[v_("n"), v_("k")])),
                    ),
                ),
            ),
        ));
        definitions
    })
}

pub(super) fn register(ctx: &mut Context) {
    crate::int_inductive::register_nat_arithmetic(ctx);
    checked(
        ctx,
        "Nat_pow",
        bin_ty(nat()),
        fix(
            "pow",
            lam_(
                "m",
                nat(),
                lam_(
                    "n",
                    nat(),
                    match_nat(
                        v_("n"),
                        nat(),
                        succ(gl("Zero")),
                        lam_(
                            "k",
                            nat(),
                            apps_(
                                gl("Nat_mul"),
                                &[apps_(v_("pow"), &[v_("m"), v_("k")]), v_("m")],
                            ),
                        ),
                    ),
                ),
            ),
        ),
    );

    for (name, ty, body) in division_definitions() {
        checked(ctx, name, ty.clone(), body.clone());
    }
    for (name, field) in [("Nat_div", "Prod_fst"), ("Nat_mod", "Prod_snd")] {
        checked(
            ctx,
            name,
            bin_ty(nat()),
            lam_(
                "n",
                nat(),
                lam_(
                    "d",
                    nat(),
                    apps_(
                        gl(field),
                        &[nat(), nat(), apps_(gl("Nat_divMod"), &[v_("n"), v_("d")])],
                    ),
                ),
            ),
        );
    }
    checked(
        ctx,
        "BitVec",
        pi_("width", nat(), Term::Sort(Universe::Type(0))),
        lam_(
            "width",
            nat(),
            apps_(
                gl("Fin"),
                &[apps_(gl("Nat_pow"), &[succ(succ(gl("Zero"))), v_("width")])],
            ),
        ),
    );
}
