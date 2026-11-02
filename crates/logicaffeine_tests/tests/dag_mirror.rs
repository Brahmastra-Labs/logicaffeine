//! The core's mirror of a context (`dag::mirror`): it advances along a context's admission log,
//! restarts from the standard library's snapshot on a branch from it, replays anything else, and
//! every route ends where a fresh replay of the context ends — the same constants with the same
//! types, kinds and values, and the same verdict on every declaration.

use logicaffeine_kernel::dag::env::Kind;
use logicaffeine_kernel::dag::logos::lift;
use logicaffeine_kernel::dag::logos_env::replay;
use logicaffeine_kernel::dag::mirror::{sync, with, Route, Synced};
use logicaffeine_kernel::dag::name::NameId;
use logicaffeine_kernel::dag::tc::Kernel;
use logicaffeine_kernel::interface::{Repl, TermParser};
use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{Context, Term, Universe};
use std::collections::BTreeMap;

fn on_a_large_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(f)
        .expect("spawns")
        .join()
        .expect("joins")
}

fn term(text: &str) -> Term {
    TermParser::parse(text).unwrap_or_else(|e| panic!("`{text}` parses: {e:?}"))
}

fn prelude() -> Context {
    StandardLibrary::context().clone()
}

fn define(ctx: &mut Context, name: &str, ty: &str, body: &str) {
    ctx.declare(name, term(ty), term(body))
        .unwrap_or_else(|e| panic!("`{name}` is accepted: {e}"));
}

/// A definition's door registers the definition and its equation lemma, both admitted.
fn with_equation(name: &str) -> Vec<(String, bool)> {
    vec![(name.to_string(), true), (format!("{name}_eq_def"), true)]
}

/// The names one sync decided, each with whether the core admitted it.
fn decided(synced: &Synced) -> Vec<(String, bool)> {
    synced
        .verdicts
        .iter()
        .map(|v| (v.name.clone(), v.verdict.is_ok()))
        .collect()
}

/// A kernel's environment, by name: each constant's universe parameters, type, kind and value,
/// read back as LOGOS terms so two kernels' stores compare.
fn environment(k: &Kernel) -> BTreeMap<String, String> {
    let n = |id: NameId| k.s.names.display(id);
    k.env
        .constants()
        .map(|c| {
            let kind = match &c.kind {
                Kind::Axiom { is_unsafe } => format!("axiom unsafe={is_unsafe}"),
                Kind::Defn { hint, safety, .. } => format!("definition {hint:?} {safety:?}"),
                Kind::Thm { .. } => "theorem".to_string(),
                Kind::Opaque { is_unsafe, .. } => format!("opaque unsafe={is_unsafe}"),
                Kind::Quot(q) => format!("quotient {q:?}"),
                Kind::Induct(i) => format!(
                    "inductive params={} indices={} all={:?} ctors={:?} nested={} rec={} reflexive={} unsafe={}",
                    i.num_params,
                    i.num_indices,
                    i.all.iter().map(|m| n(*m)).collect::<Vec<_>>(),
                    i.ctors.iter().map(|m| n(*m)).collect::<Vec<_>>(),
                    i.num_nested,
                    i.is_rec,
                    i.is_reflexive,
                    i.is_unsafe
                ),
                Kind::Ctor(ci) => format!(
                    "constructor of {} #{} params={} fields={} unsafe={}",
                    n(ci.induct),
                    ci.cidx,
                    ci.num_params,
                    ci.num_fields,
                    ci.is_unsafe
                ),
                Kind::Rec(_) => "recursor".to_string(),
            };
            let levels: Vec<String> = c.lparams.iter().map(|l| n(*l)).collect();
            let value = c.delta_value().map(|v| lift(&k.s, v));
            (n(c.name), format!("{levels:?} : {:?} | {kind} | {value:?}", lift(&k.s, c.ty)))
        })
        .collect()
}

/// Everything the syncs on one thread have reported, by name; a later report replaces an earlier.
#[derive(Default)]
struct Reported(BTreeMap<String, bool>);

impl Reported {
    fn note(&mut self, synced: &Synced) {
        for (name, ok) in decided(synced) {
            self.0.insert(name, ok);
        }
    }

    fn sync(&mut self, ctx: &Context) -> Synced {
        let synced = sync(ctx);
        self.note(&synced);
        synced
    }

    /// The mirror of `ctx` holds what a fresh replay of `ctx` holds, and the verdict last reported
    /// for each of `ctx`'s declarations is the replay's.
    fn agree_with_replay(&mut self, ctx: &Context) {
        let (fresh, verdicts) = replay(ctx);
        let (synced, mirrored) = with(ctx, |k| environment(k));
        self.note(&synced);
        let replayed = environment(&fresh);
        let only_mirror: Vec<&String> = mirrored
            .keys()
            .filter(|k| !replayed.contains_key(*k))
            .collect();
        let only_replay: Vec<&String> = replayed
            .keys()
            .filter(|k| !mirrored.contains_key(*k))
            .collect();
        assert!(
            only_mirror.is_empty() && only_replay.is_empty(),
            "only the mirror: {only_mirror:?}; only the replay: {only_replay:?}"
        );
        for (name, constant) in &replayed {
            assert_eq!(
                &mirrored[name], constant,
                "`{name}` is the same constant in the mirror and the replay"
            );
        }
        for v in &verdicts {
            assert_eq!(
                self.0.get(&v.name),
                Some(&v.verdict.is_ok()),
                "`{}`: the mirror's verdict is the replay's ({:?})",
                v.name,
                v.verdict
            );
        }
    }
}

fn holds(ctx: &Context, name: &'static str) -> bool {
    with(ctx, |k| {
        let id = k.s.names.str(NameId::ANON, name);
        k.env.contains(id)
    })
    .1
}

/// The core runs on a thread of its own with the core's stack: a sync from a thread whose stack holds
/// little more than the call, of a context the core replays whole, completes on the core thread.
#[test]
fn a_sync_from_a_small_stack_runs_on_the_core_thread() {
    let ctx = on_a_large_stack(|| {
        let mut ctx = prelude();
        define(&mut ctx, "two", "Nat", "Succ (Succ Zero)");
        ctx
    });
    let route = std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(move || sync(&ctx).route)
        .expect("spawns")
        .join()
        .expect("the sync completes on a small stack");
    assert_ne!(
        route,
        Route::Current,
        "a fresh thread's mirror is built, not found"
    );
}

#[test]
fn a_context_synced_again_is_current() {
    on_a_large_stack(|| {
        let mut ctx = prelude();
        define(&mut ctx, "two", "Nat", "Succ (Succ Zero)");
        sync(&ctx);
        let again = sync(&ctx);
        assert_eq!(again.route, Route::Current);
        assert!(
            again.verdicts.is_empty(),
            "nothing new: {:?}",
            decided(&again)
        );
        let clone = ctx.clone();
        assert_eq!(
            sync(&clone).route,
            Route::Current,
            "a clone is the same context to the mirror"
        );
    });
}

#[test]
fn the_first_sync_of_a_thread_replays_the_standard_library_once() {
    on_a_large_stack(|| {
        let ctx = prelude();
        let first = sync(&ctx);
        assert_eq!(first.route, Route::Replayed);
        let (_, all) = replay(&ctx);
        assert_eq!(
            first.verdicts.len(),
            all.len(),
            "the standard library's every declaration is decided"
        );
        assert!(
            first.verdicts.iter().all(|v| v.verdict.is_ok()),
            "the core admits the standard library"
        );
    });
}

#[test]
fn extending_the_mirrored_context_admits_only_its_new_declarations() {
    on_a_large_stack(|| {
        let mut reported = Reported::default();
        let mut ctx = prelude();
        reported.sync(&ctx);
        define(&mut ctx, "two", "Nat", "Succ (Succ Zero)");
        let synced = reported.sync(&ctx);
        assert_eq!(synced.route, Route::Advanced);
        assert_eq!(
            decided(&synced),
            with_equation("two"),
            "the definition and its equation lemma"
        );
        define(&mut ctx, "three", "Nat", "Succ two");
        let synced = reported.sync(&ctx);
        assert_eq!(synced.route, Route::Advanced);
        assert_eq!(decided(&synced), with_equation("three"));
        reported.agree_with_replay(&ctx);
    });
}

#[test]
fn a_sibling_of_the_mirrored_context_restarts_from_the_standard_library() {
    on_a_large_stack(|| {
        let mut reported = Reported::default();
        let mut a = prelude();
        define(&mut a, "from_a", "Nat", "Zero");
        let mut b = prelude();
        define(&mut b, "from_b", "Nat", "Succ Zero");
        let mut c = prelude();
        define(&mut c, "from_c", "Bool", "true");
        reported.sync(&a);
        let synced = reported.sync(&b);
        assert_eq!(synced.route, Route::Rebased);
        assert_eq!(decided(&synced), with_equation("from_b"));
        assert!(
            !holds(&b, "from_a"),
            "a sibling's declarations are not visible"
        );
        assert_eq!(reported.sync(&c).route, Route::Rebased);
        assert!(
            !holds(&c, "from_b") && !holds(&c, "from_a"),
            "the snapshot never advances"
        );
        reported.agree_with_replay(&c);
        reported.agree_with_replay(&a);
    });
}

#[test]
fn returning_to_an_earlier_context_restarts_from_the_standard_library() {
    on_a_large_stack(|| {
        let mut reported = Reported::default();
        let mut earlier = prelude();
        define(&mut earlier, "x", "Nat", "Zero");
        let mut later = earlier.clone();
        define(&mut later, "y", "Nat", "x");
        reported.sync(&later);
        let synced = reported.sync(&earlier);
        assert_eq!(synced.route, Route::Rebased);
        assert_eq!(decided(&synced), with_equation("x"));
        assert!(!holds(&earlier, "y"), "the later declaration is gone");
        reported.agree_with_replay(&earlier);
    });
}

#[test]
fn certifying_one_theorem_per_context_never_replays_after_the_first() {
    on_a_large_stack(|| {
        for i in 0..12 {
            let mut ctx = prelude();
            define(
                &mut ctx,
                &format!("t{i}"),
                "Eq Nat Zero Zero",
                "refl Nat Zero",
            );
            let synced = sync(&ctx);
            let expected = if i == 0 {
                Route::Replayed
            } else {
                Route::Rebased
            };
            assert_eq!(synced.route, expected, "theorem {i}");
        }
    });
}

#[test]
fn a_redeclared_name_restarts_and_admits_the_new_declaration() {
    on_a_large_stack(|| {
        let mut reported = Reported::default();
        let mut ctx = prelude();
        define(&mut ctx, "x", "Nat", "Zero");
        reported.sync(&ctx);
        ctx.add_definition("x".to_string(), term("Nat"), term("Succ Zero"));
        let synced = reported.sync(&ctx);
        assert_eq!(synced.route, Route::Rebased, "the mirror held the old `x`");
        assert_eq!(decided(&synced), with_equation("x"));
        let value = with(&ctx, |k| {
            let id = k.s.names.str(NameId::ANON, "x");
            k.env
                .get(id)
                .and_then(|c| c.delta_value())
                .map(|v| lift(&k.s, v))
        })
        .1;
        assert_eq!(
            value,
            Some(term("Succ Zero")),
            "the core holds the new body"
        );
        reported.agree_with_replay(&ctx);
    });
}

#[test]
fn a_constructor_added_to_an_admitted_family_restarts_with_the_whole_family() {
    on_a_large_stack(|| {
        let mut reported = Reported::default();
        let mut ctx = prelude();
        ctx.add_inductive("Color", Term::Sort(Universe::Type(0)));
        ctx.add_constructor("Red", "Color", term("Color"));
        reported.sync(&ctx);
        assert!(holds(&ctx, "Color") && holds(&ctx, "Red"));
        ctx.add_constructor("Green", "Color", term("Color"));
        let synced = reported.sync(&ctx);
        assert_eq!(
            synced.route,
            Route::Rebased,
            "the mirror held `Color` with one constructor"
        );
        assert!(holds(&ctx, "Green"), "the family is admitted with both");
        reported.agree_with_replay(&ctx);
    });
}

#[test]
fn a_registration_withdrawn_between_syncs_lets_the_mirror_advance() {
    on_a_large_stack(|| {
        let mut reported = Reported::default();
        let mut ctx = prelude();
        reported.sync(&ctx);
        ctx.add_definition("scratch".to_string(), term("Nat"), term("Zero"));
        ctx.add_definition("scratch".to_string(), term("Nat"), term("Succ Zero"));
        let synced = reported.sync(&ctx);
        assert_eq!(
            synced.route,
            Route::Advanced,
            "nothing the mirror held changed"
        );
        assert_eq!(decided(&synced), with_equation("scratch"));
        reported.agree_with_replay(&ctx);
    });
}

#[test]
fn a_declaration_waiting_for_a_name_is_decided_again_when_the_name_arrives() {
    on_a_large_stack(|| {
        let mut reported = Reported::default();
        let mut ctx = prelude();
        ctx.add_declaration("waiting", term("Later"));
        let synced = reported.sync(&ctx);
        let waiting = synced
            .verdicts
            .iter()
            .find(|v| v.name == "waiting")
            .expect("decided");
        assert!(waiting.verdict.is_err(), "`Later` is not declared yet");
        ctx.add_inductive("Later", Term::Sort(Universe::Prop));
        let synced = reported.sync(&ctx);
        assert_ne!(
            synced.route,
            Route::Advanced,
            "a replay would decide `waiting` again"
        );
        assert_eq!(
            reported.0.get("waiting"),
            Some(&true),
            "decided again, and admitted"
        );
        reported.agree_with_replay(&ctx);
    });
}

#[test]
fn a_context_that_comes_to_read_int_literals_as_data_restarts() {
    // `Int` arrives canonical after the kernel was built, so the kernel must read `5 : Int` as
    // `Int_ofNat 5` from now on, which it fixed when it was built: `T 5` computes to `Nat` only so.
    on_a_large_stack(|| {
        let mut reported = Reported::default();
        let mut ctx = Context::new();
        ctx.add_inductive("Nat", Term::Sort(Universe::Type(0)));
        ctx.add_constructor("Zero", "Nat", term("Nat"));
        ctx.add_constructor("Succ", "Nat", term("Nat -> Nat"));
        reported.sync(&ctx);
        ctx.add_inductive("Int", Term::Sort(Universe::Type(0)));
        ctx.add_constructor("Int_ofNat", "Int", term("Nat -> Int"));
        ctx.add_constructor("Int_negSucc", "Int", term("Nat -> Int"));
        define(
            &mut ctx,
            "T",
            "Int -> Type",
            "fun (i : Int) => match i return (fun (j : Int) => Type) with fun (n : Nat) => Nat | fun (n : Nat) => Nat end",
        );
        define(&mut ctx, "z", "T 5", "Zero");
        let synced = reported.sync(&ctx);
        assert_ne!(
            synced.route,
            Route::Advanced,
            "a kernel built before `Int` read its literals otherwise"
        );
        reported.agree_with_replay(&ctx);
    });
}

#[test]
fn a_context_outside_the_standard_library_is_replayed_then_advanced() {
    on_a_large_stack(|| {
        let mut reported = Reported::default();
        let mut ctx = Context::new();
        ctx.add_inductive("P", Term::Sort(Universe::Prop));
        ctx.add_constructor("p", "P", term("P"));
        assert_eq!(reported.sync(&ctx).route, Route::Replayed);
        ctx.add_definition("q".to_string(), term("P"), term("p"));
        let synced = reported.sync(&ctx);
        assert_eq!(synced.route, Route::Advanced);
        assert_eq!(decided(&synced), vec![("q".to_string(), true)]);
        reported.agree_with_replay(&ctx);
    });
}

#[test]
fn a_session_through_the_vernacular_ends_where_a_replay_ends() {
    on_a_large_stack(|| {
        let mut reported = Reported::default();
        let mut repl = Repl::new();
        let script = [
            "Definition double : Nat -> Nat := fix double => fun n : Nat => match n with | Zero => Zero | Succ k => Succ (Succ (double k)) end.",
            "Inductive Tree := Leaf : Tree | Node : Tree -> Tree -> Tree.",
            "Definition size : Tree -> Nat := fix size => fun t : Tree => match t with | Leaf => Succ Zero | Node l r => Succ (size l) end.",
            "Definition refl_any : forall (A : Type), forall (x : A), Eq A x x := fun A : Type => fun x : A => refl A x.",
            "Definition four : Nat := double (double (Succ Zero)).",
        ];
        for (i, command) in script.iter().enumerate() {
            repl.execute(command)
                .unwrap_or_else(|e| panic!("`{command}` is accepted: {e}"));
            let synced = reported.sync(repl.context());
            if i > 0 {
                assert_eq!(
                    synced.route,
                    Route::Advanced,
                    "`{command}` extends the session"
                );
            }
            assert!(
                decided(&synced).iter().all(|(_, ok)| *ok),
                "`{command}`: {:?}",
                synced.verdicts
            );
        }
        reported.agree_with_replay(repl.context());
    });
}

/// A name the mirror holds, registered again as a constructor of a family it does not: only the
/// name itself says the mirror's declaration of it is gone, since its owner is new.
#[test]
fn a_held_name_registered_again_as_a_constructor_of_a_new_family_restarts() {
    on_a_large_stack(|| {
        let mut reported = Reported::default();
        let mut ctx = prelude();
        ctx.add_declaration("Red", term("Nat"));
        reported.sync(&ctx);
        ctx.add_inductive("Paint", Term::Sort(Universe::Type(0)));
        ctx.add_constructor("Red", "Paint", term("Paint"));
        let synced = reported.sync(&ctx);
        assert_eq!(
            synced.route,
            Route::Rebased,
            "the mirror held `Red` as an axiom of `Nat`"
        );
        let constructor = with(&ctx, |k| {
            let id = k.s.names.str(NameId::ANON, "Red");
            k.env.get(id).map(|c| matches!(c.kind, Kind::Ctor(_)))
        })
        .1;
        assert_eq!(
            constructor,
            Some(true),
            "the core holds `Red` as `Paint`'s constructor"
        );
        reported.agree_with_replay(&ctx);
    });
}

/// A constructor added to a held family whose parameters were stated, with no auxiliaries to
/// regenerate: nothing re-registers the family, so only the constructor's owner says it changed.
#[test]
fn a_constructor_added_to_a_held_indexed_family_restarts_with_the_whole_family() {
    on_a_large_stack(|| {
        let mut reported = Reported::default();
        let mut ctx = prelude();
        ctx.set_auto_auxiliaries(false);
        ctx.add_indexed_inductive("Tag", term("Nat -> Type"), 0);
        ctx.add_constructor("tag0", "Tag", term("Tag Zero"));
        reported.sync(&ctx);
        assert!(holds(&ctx, "Tag") && holds(&ctx, "tag0"));
        ctx.add_constructor("tag1", "Tag", term("Tag (Succ Zero)"));
        let synced = reported.sync(&ctx);
        assert_eq!(
            synced.route,
            Route::Rebased,
            "the mirror held `Tag` with one constructor"
        );
        assert!(holds(&ctx, "tag1"), "the family is admitted with both");
        reported.agree_with_replay(&ctx);
    });
}
