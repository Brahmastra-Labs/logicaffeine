//! Universe instantiation in the value engine: a universe-polymorphic constant's type and body are
//! evaluated under its universe substitution, and every result must be what instantiating the
//! term first would give — at two instantiations in one declaration, through δ, and through the
//! closures such an unfolding produces — while the parts that mention no universe parameter stay
//! one value across instantiations. Every verdict is checked against the substitution engine.

use logicaffeine_kernel::dag::export::Reader;
use logicaffeine_kernel::dag::tc::Engine;
use logicaffeine_kernel::dag::{check_export_with, Kernel, Verdict};
use serde_json::{json, Value};
use std::io::BufReader;

/// A tiny lean4export writer: names, levels and expressions numbered as they are added.
struct Export {
    rows: Vec<Value>,
    names: usize,
    levels: usize,
    exprs: usize,
}

impl Export {
    fn new() -> Export {
        Export {
            rows: vec![json!({"meta":{"format":{"version":"3.1.0"}}})],
            names: 0,
            levels: 0,
            exprs: 0,
        }
    }
    fn name(&mut self, s: &str) -> usize {
        self.names += 1;
        self.rows
            .push(json!({"in": self.names, "str": {"pre": 0, "str": s}}));
        self.names
    }
    fn level(&mut self, key: &str, v: Value) -> usize {
        self.levels += 1;
        self.rows.push(json!({"il": self.levels, key: v}));
        self.levels
    }
    fn expr(&mut self, key: &str, v: Value) -> usize {
        let id = self.exprs;
        self.exprs += 1;
        self.rows.push(json!({"ie": id, key: v}));
        id
    }
    fn sort(&mut self, l: usize) -> usize {
        self.expr("sort", json!(l))
    }
    fn cnst(&mut self, n: usize, us: &[usize]) -> usize {
        self.expr("const", json!({"name": n, "us": us}))
    }
    fn pi(&mut self, ty: usize, body: usize) -> usize {
        self.expr(
            "forallE",
            json!({"name": 0, "type": ty, "body": body, "binderInfo": "default"}),
        )
    }
    fn lam(&mut self, ty: usize, body: usize) -> usize {
        self.expr(
            "lam",
            json!({"name": 0, "type": ty, "body": body, "binderInfo": "default"}),
        )
    }
    fn bvar(&mut self, i: usize) -> usize {
        self.expr("bvar", json!(i))
    }
    fn def(&mut self, name: usize, params: &[usize], ty: usize, value: usize) {
        self.rows.push(
            json!({"def": {"name": name, "levelParams": params, "type": ty, "value": value,
            "all": [name], "safety": "safe", "hints": {"regular": 1}}}),
        );
    }
    fn text(&self) -> String {
        self.rows.iter().map(|r| r.to_string() + "\n").collect()
    }
}

/// `S.{u} : Sort (u+1) := Sort u`, and `T.{u} : Sort (u+2) := Prop → Sort (u+1)`.
struct Universe {
    e: Export,
    u: usize,
    s: usize,
    t: usize,
    one: usize,
    two: usize,
    three: usize,
}

fn universe() -> Universe {
    let mut e = Export::new();
    let s = e.name("S");
    let u = e.name("u");
    let t = e.name("T");
    let one = e.level("succ", json!(0));
    let two = e.level("succ", json!(one));
    let three = e.level("succ", json!(two));
    let pu = e.level("param", json!(u));
    let pu1 = e.level("succ", json!(pu));
    let pu2 = e.level("succ", json!(pu1));
    let (sort_u, sort_u1, sort_u2) = (e.sort(pu), e.sort(pu1), e.sort(pu2));
    e.def(s, &[u], sort_u1, sort_u);
    let prop = e.sort(0);
    let arrow = e.pi(prop, sort_u1);
    e.def(t, &[u], sort_u2, arrow);
    Universe {
        e,
        u,
        s,
        t,
        one,
        two,
        three,
    }
}

fn verdict(text: &str, engine: Engine) -> i32 {
    let text = text.to_string();
    std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(move || {
            check_export_with(BufReader::new(text.as_bytes()), |k| k.engine = engine)
                .0
                .exit_code()
        })
        .unwrap()
        .join()
        .unwrap()
}

fn expect(text: &str, exit: i32, what: &str) {
    assert_eq!(
        verdict(text, Engine::Classic),
        exit,
        "substitution engine on {what}"
    );
    assert_eq!(verdict(text, Engine::Sem), exit, "value engine on {what}");
}

#[test]
fn a_definition_unfolds_to_its_instantiation() {
    let mut w = universe();
    let ok = w.e.name("ok");
    let s1 = w.e.cnst(w.s, &[w.one]);
    let prop = w.e.sort(0);
    w.e.def(ok, &[], s1, prop);
    expect(&w.e.text(), 0, "Prop : S.{1}");
}

#[test]
fn a_definition_does_not_unfold_to_another_instantiation() {
    let mut w = universe();
    let bad = w.e.name("bad");
    let s0 = w.e.cnst(w.s, &[0]);
    let prop = w.e.sort(0);
    w.e.def(bad, &[], s0, prop);
    expect(&w.e.text(), 1, "Prop : S.{0}");
}

#[test]
fn closures_from_two_instantiations_are_not_confused() {
    let mut w = universe();
    let bad = w.e.name("bad");
    let t0 = w.e.cnst(w.t, &[0]);
    let t1 = w.e.cnst(w.t, &[w.one]);
    let ty = w.e.pi(t0, t1);
    let x = w.e.bvar(0);
    let id = w.e.lam(t0, x);
    w.e.def(bad, &[], ty, id);
    expect(&w.e.text(), 1, "fun x => x : T.{0} → T.{1}");
}

#[test]
fn closures_from_one_instantiation_agree() {
    let mut w = universe();
    let good = w.e.name("good");
    let t1 = w.e.cnst(w.t, &[w.one]);
    let t1b = w.e.cnst(w.t, &[w.one]);
    let ty = w.e.pi(t1, t1b);
    let x = w.e.bvar(0);
    let id = w.e.lam(t1, x);
    w.e.def(good, &[], ty, id);
    expect(&w.e.text(), 0, "fun x => x : T.{1} → T.{1}");
}

#[test]
fn two_instantiations_in_one_declaration_keep_their_own_levels() {
    let mut w = universe();
    let both = w.e.name("both");
    let s1 = w.e.cnst(w.s, &[w.one]);
    let s2 = w.e.cnst(w.s, &[w.two]);
    let ty = w.e.pi(s2, s1);
    let prop = w.e.sort(0);
    let body = w.e.lam(s2, prop);
    w.e.def(both, &[], ty, body);
    expect(&w.e.text(), 0, "fun (_ : S.{2}) => Prop : S.{2} → S.{1}");
    let mut w = universe();
    let both = w.e.name("both");
    let s1 = w.e.cnst(w.s, &[w.one]);
    let s2 = w.e.cnst(w.s, &[w.two]);
    let ty = w.e.pi(s1, s2);
    let prop = w.e.sort(0);
    let body = w.e.lam(s1, prop);
    w.e.def(both, &[], ty, body);
    expect(&w.e.text(), 1, "fun (_ : S.{1}) => Prop : S.{1} → S.{2}");
    let _ = w.three;
}

#[test]
fn a_universe_polymorphic_declaration_checks_under_its_own_parameters() {
    let mut w = universe();
    let v = w.e.name("v");
    let own = w.e.name("own");
    let pv = w.e.level("param", json!(v));
    let pv1 = w.e.level("succ", json!(pv));
    let sv = w.e.cnst(w.s, &[pv1]);
    let sort_v = w.e.sort(pv);
    w.e.def(own, &[v], sv, sort_v);
    expect(&w.e.text(), 0, "own.{v} : S.{v+1} := Sort v");
    let _ = w.u;
}

/// `G.{u} : Prop → Prop → Sort (u+1) := fun a p => (fun (y : Prop) (x : Sort (u+1)) => x) p (Sort u)`:
/// the inner application mentions `p` but not `a`, so evaluating it prunes an environment that
/// carries G's substitution into a frame of `p` alone — two instantiations with the same entries
/// must stay two frames.
fn open_under_substitution(src_level: usize, dst_level: usize) -> String {
    let mut w = universe();
    let g = w.e.name("G");
    let mix = w.e.name("mix");
    let pu = w.e.level("param", json!(w.u));
    let pu1 = w.e.level("succ", json!(pu));
    let prop = w.e.sort(0);
    let sort_u = w.e.sort(pu);
    let sort_u1 = w.e.sort(pu1);
    let g_ty_inner = w.e.pi(prop, sort_u1);
    let g_ty = w.e.pi(prop, g_ty_inner);
    let x = w.e.bvar(0);
    let pick_x = w.e.lam(sort_u1, x);
    let pick = w.e.lam(prop, pick_x);
    let p = w.e.bvar(0);
    let picked_p = w.e.expr("app", json!({"fn": pick, "arg": p}));
    let applied = w.e.expr("app", json!({"fn": picked_p, "arg": sort_u}));
    let over_p = w.e.lam(prop, applied);
    let g_val = w.e.lam(prop, over_p);
    w.e.def(g, &[w.u], g_ty, g_val);
    let q0 = w.e.bvar(0);
    let q = w.e.pi(prop, q0);
    let r = w.e.pi(q, q);
    let at = |w: &mut Universe, level: usize| {
        let c = w.e.cnst(g, &[level]);
        let ca = w.e.expr("app", json!({"fn": c, "arg": r}));
        w.e.expr("app", json!({"fn": ca, "arg": q}))
    };
    let src = at(&mut w, src_level);
    let dst = at(&mut w, dst_level);
    let ty = w.e.pi(src, dst);
    let y = w.e.bvar(0);
    let id = w.e.lam(src, y);
    w.e.def(mix, &[], ty, id);
    w.e.text()
}

#[test]
fn open_terms_under_two_instantiations_are_not_confused() {
    let w = universe();
    let (one, zero) = (w.one, 0);
    expect(
        &open_under_substitution(one, zero),
        1,
        "fun x => x : G.{1} R Q → G.{0} R Q",
    );
    expect(
        &open_under_substitution(zero, one),
        1,
        "fun x => x : G.{0} R Q → G.{1} R Q",
    );
    expect(
        &open_under_substitution(one, one),
        0,
        "fun x => x : G.{1} R Q → G.{1} R Q",
    );
}

#[test]
fn a_level_free_body_is_one_value_across_instantiations() {
    let mut e = Export::new();
    let d = e.name("D");
    let u = e.name("u");
    let prop = e.sort(0);
    let arrow = e.pi(prop, prop);
    let x = e.bvar(0);
    let body = e.lam(prop, x);
    e.def(d, &[u], arrow, body);
    let text = e.text();
    std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(move || {
            let mut k = Kernel::new();
            k.engine = Engine::Sem;
            Reader::new(&mut k)
                .run(BufReader::new(text.as_bytes()))
                .unwrap();
            let name =
                k.s.names
                    .str(logicaffeine_kernel::dag::name::NameId::ANON, "D");
            let zero = k.s.levels.of_nat(0);
            let one = k.s.levels.of_nat(1);
            let d0 = k.s.cnst_levels(name, &[zero]);
            let d1 = k.s.cnst_levels(name, &[one]);
            let (a, b) = k
                .with_sem(|sem| {
                    let env = logicaffeine_kernel::dag::sem::value::EnvId::EMPTY;
                    let v0 = sem.eval(env, d0)?;
                    let v1 = sem.eval(env, d1)?;
                    Ok((sem.unfold(v0)?.unwrap(), sem.unfold(v1)?.unwrap()))
                })
                .unwrap();
            assert_eq!(
                a, b,
                "a body that mentions no universe parameter is one value at every instantiation"
            );
        })
        .unwrap()
        .join()
        .unwrap();
}

/// The universe level of the codomain of the type of `L.{1}`'s unfolding, where
/// `L.{u} : Prop → Sort (u+1) := fun (x : Prop) => body` — the lambda's type is inferred from its
/// body, read under L's substitution.
fn unfolded_codomain_level(body_of: impl FnOnce(&mut Universe) -> (usize, usize)) -> (u32, u32) {
    let mut w = universe();
    let l = w.e.name("L");
    let (body, body_level) = body_of(&mut w);
    let prop = w.e.sort(0);
    let ty_cod = w.e.sort(body_level);
    let ty = w.e.pi(prop, ty_cod);
    let value = w.e.lam(prop, body);
    w.e.def(l, &[w.u], ty, value);
    let text = w.e.text();
    std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(move || {
            use logicaffeine_kernel::dag::sem::value::{EnvId, Val};
            let mut k = Kernel::new();
            k.engine = Engine::Sem;
            Reader::new(&mut k)
                .run(BufReader::new(text.as_bytes()))
                .unwrap();
            let name =
                k.s.names
                    .str(logicaffeine_kernel::dag::name::NameId::ANON, "L");
            let one = k.s.levels.of_nat(1);
            let two = k.s.levels.of_nat(2);
            let at_one = k.s.cnst_levels(name, &[one]);
            let prop = k.s.sort(logicaffeine_kernel::dag::level::LevelId::ZERO);
            let got = k
                .with_sem(|sem| {
                    let v = sem.eval(EnvId::EMPTY, at_one)?;
                    let lam = sem.unfold(v)?.expect("L unfolds");
                    let ty = sem.type_of(lam)?;
                    let p = sem.eval(EnvId::EMPTY, prop)?;
                    let cod = sem.under(p, |sem, x| sem.instantiate(ty, x))?;
                    let cod = sem.whnf(cod)?;
                    Ok(match sem.st.arena.get(cod) {
                        Val::Sort(l) => l,
                        other => {
                            panic!("the codomain of a lambda's type is a sort here, got {other:?}")
                        }
                    })
                })
                .unwrap();
            (got.0, two.0)
        })
        .unwrap()
        .join()
        .unwrap()
}

#[test]
fn inference_inside_an_unfolded_body_reads_its_instantiation() {
    let (sort, two) = unfolded_codomain_level(|w| {
        let pu = w.e.level("param", json!(w.u));
        let pu1 = w.e.level("succ", json!(pu));
        (w.e.sort(pu), pu1)
    });
    assert_eq!(sort, two, "fun x => Sort u at u := 1 has codomain Sort 2");
    let (constant, two) = unfolded_codomain_level(|w| {
        let pu = w.e.level("param", json!(w.u));
        let pu1 = w.e.level("succ", json!(pu));
        (w.e.cnst(w.s, &[pu]), pu1)
    });
    assert_eq!(
        constant, two,
        "fun x => S.{{u}} at u := 1 has codomain Sort 2 (S.{{1}} : Sort 2)"
    );
}

#[test]
fn verdicts_stay_those_of_a_plain_check() {
    let mut w = universe();
    let ok = w.e.name("ok");
    let s1 = w.e.cnst(w.s, &[w.one]);
    let prop = w.e.sort(0);
    w.e.def(ok, &[], s1, prop);
    assert!(matches!(
        std::thread::Builder::new()
            .stack_size(1 << 30)
            .spawn({
                let t = w.e.text();
                move || {
                    check_export_with(BufReader::new(t.as_bytes()), |k| k.engine = Engine::Sem).0
                }
            })
            .unwrap()
            .join()
            .unwrap(),
        Verdict::Accept
    ));
}
