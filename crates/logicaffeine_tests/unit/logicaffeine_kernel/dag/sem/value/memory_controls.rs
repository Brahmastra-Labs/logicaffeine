use super::*;
use std::mem::size_of;

/// An arena with something in every store: values of every shape, extensions, narrow and wide
/// frames, contexts, and the bases of two substitutions.
fn filled() -> Arena {
    let mut a = Arena::new();
    let c = |a: &mut Arena, n: u32| {
        a.head_value(
            Kind::Opaque,
            Head::Const {
                name: NameId(n),
                levels: LevelsId::EMPTY,
            },
        )
    };
    let ty = c(&mut a, 1);
    let mut env = a.lsub_base(SubstId(3));
    let mut ctx = CtxId::EMPTY;
    for i in 0..300u32 {
        let x = a.head_value(Kind::Local, Head::Local { level: i, ty });
        let f = c(&mut a, 10 + i % 7);
        let n = a.neutral_apply(f, &[x, ty]);
        env = a.extend(env, n);
        ctx = a.ctx_extend(ctx, n);
        let lam = a.lam(ExprId(i), env, ExprId(i + 1));
        let pi = a.pi(lam, env, ExprId(i + 2));
        a.pi_lam(pi, env, ctx, ExprId(i + 3));
        a.sort(LevelId(i % 5));
        a.nat(NatId(i));
        a.str(StrId(i));
        if i % 3 == 0 && i < 60 {
            a.prune(env, i + 1, 1 | 1 << i);
        }
    }
    let vals: Vec<V> = (0..70).map(|i| c(&mut a, 100 + i)).collect();
    let indices: Vec<u32> = (0..70).map(|i| i * 2).collect();
    a.wide_frame(140, &indices, &vals, SubstId(3));
    a.lsub_base(SubstId(4));
    a
}

#[test]
fn the_memory_report_counts_every_store_of_the_arena() {
    let a = filled();
    let Arena {
        recs,
        heads,
        spine_slots,
        envs,
        frame_slots,
        frame_index,
        wide_idx,
        base_index,
        ctxs,
        neutral_index,
        head_index,
        env_index,
        ctx_index,
        atom_index,
        lam_index,
        pi_index,
        pilam_index,
    } = &a;
    let held = recs.capacity() * size_of::<Rec>()
        + heads.capacity() * size_of::<Head>()
        + spine_slots.capacity() * size_of::<V>()
        + envs.capacity() * size_of::<EnvNode>()
        + frame_slots.capacity() * size_of::<V>()
        + frame_index.bytes()
        + wide_idx.capacity() * size_of::<u32>()
        + base_index.bytes()
        + ctxs.capacity() * size_of::<CtxNode>()
        + neutral_index.bytes()
        + head_index.bytes()
        + env_index.bytes()
        + ctx_index.bytes()
        + atom_index.bytes()
        + lam_index.bytes()
        + pi_index.bytes()
        + pilam_index.bytes();
    for (what, store) in [
        ("frame slots", frame_slots.len()),
        ("wide indices", wide_idx.len()),
        ("contexts", ctxs.len()),
        ("substitution bases", base_index.len()),
    ] {
        assert!(store > 1, "the filled arena has {what}");
    }
    assert_eq!(a.bytes(), held);
}

#[test]
fn the_memory_report_names_each_part_once() {
    let a = filled();
    let report = a.memory_report();
    let mut names: Vec<&str> = report.iter().map(|(n, _)| *n).collect();
    let all = names.len();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), all, "{report:?}");
    for part in ["value engine: frame index", "value engine: contexts"] {
        assert!(
            report.iter().any(|(n, b)| *n == part && *b > 0),
            "{part} is reported: {report:?}"
        );
    }
}
