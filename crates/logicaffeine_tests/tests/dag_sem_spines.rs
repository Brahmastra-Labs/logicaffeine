//! The value engine's neutrals and environments (`dag::sem::value::Arena`) against a model.
//!
//! A neutral's arguments lie contiguously and a neutral is interned by its head and argument
//! list, so however an application is built — one argument at a time, all at once, or a prefix and
//! then the rest — it is one value, its arguments read back in order, and it is open exactly when
//! its head or an argument is; finding one stores nothing, and a new one stores its arguments
//! once. An environment is interned by parent and value, with its last extension remembered on
//! the parent: extending by a value, by another, and by the first again gives the first
//! environment back, and a session reset forgets every remembered extension. A pruning keeps
//! exactly the entries its mask marks, is one frame per content (finding it stores nothing, a new
//! one stores its live entries once), and an environment remembers its two latest prunings.

use std::collections::HashMap;

use logicaffeine_kernel::dag::level::LevelsId;
use logicaffeine_kernel::dag::name::NameId;
use logicaffeine_kernel::dag::sem::value::{Arena, EnvId, Head, Kind, Sp, Val, DEAD, V};

/// A deterministic stream of pseudo-random numbers (xorshift).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn constant(a: &mut Arena, n: u32) -> V {
    a.head_value(
        Kind::Opaque,
        Head::Const {
            name: NameId(n),
            levels: LevelsId::EMPTY,
        },
    )
}

/// A bound variable: open, so whatever holds it is open.
fn local(a: &mut Arena, level: u32) -> V {
    let ty = constant(a, 1);
    a.head_value(Kind::Local, Head::Local { level, ty })
}

/// A value for an argument: a closed constant, an open variable, or a neutral built of them.
fn value(a: &mut Arena, r: &mut Rng, depth: u32) -> V {
    match r.below(if depth == 0 { 2 } else { 3 }) {
        0 => constant(a, 2 + r.below(40) as u32),
        1 => local(a, r.below(8) as u32),
        _ => {
            let f = constant(a, 50 + r.below(10) as u32);
            let n = 1 + r.below(4) as usize;
            let args: Vec<V> = (0..n).map(|_| value(a, r, depth - 1)).collect();
            a.neutral_apply(f, &args)
        }
    }
}

fn spine_of(a: &Arena, v: V) -> Sp {
    match a.get(v) {
        Val::Neu { spine, .. } => spine,
        _ => panic!("{v:?} is not a neutral"),
    }
}

/// Builds `head args` one argument at a time.
fn one_at_a_time(a: &mut Arena, head: V, args: &[V]) -> V {
    args.iter().fold(head, |f, &x| a.neutral_app(f, x))
}

#[test]
fn an_application_is_one_value_however_it_is_built() {
    let mut a = Arena::new();
    let mut r = Rng(0x5eed_5_9a5e);
    for round in 0..2_000 {
        let head = if round % 7 == 0 {
            local(&mut a, 3)
        } else {
            constant(&mut a, 100 + r.below(30) as u32)
        };
        let len = 1 + r.below(if round % 50 == 0 { 120 } else { 20 }) as usize;
        let args: Vec<V> = (0..len).map(|_| value(&mut a, &mut r, 2)).collect();
        let whole = a.neutral_apply(head, &args);
        assert_eq!(one_at_a_time(&mut a, head, &args), whole, "round {round}");
        let cut = r.below(len as u64 + 1) as usize;
        let prefix = a.neutral_apply(head, &args[..cut]);
        assert_eq!(
            a.neutral_apply(prefix, &args[cut..]),
            whole,
            "round {round}, cut {cut}"
        );
        assert_eq!(
            a.neutral_apply(whole, &[]),
            whole,
            "applying nothing is the identity"
        );
    }
}

#[test]
fn a_neutrals_arguments_read_back_in_order() {
    let mut a = Arena::new();
    let mut r = Rng(0xa5_6075);
    for _ in 0..2_000 {
        let head = constant(&mut a, 7);
        let len = 1 + r.below(40) as usize;
        let args: Vec<V> = (0..len).map(|_| value(&mut a, &mut r, 1)).collect();
        let v = a.neutral_apply(head, &args);
        let sp = spine_of(&a, v);
        assert_eq!(a.spine_len(sp), len);
        assert_eq!(a.spine(sp), &args[..]);
        assert_eq!(a.args(sp).as_slice(), &args[..]);
        for (i, &x) in args.iter().enumerate() {
            assert_eq!(a.spine_arg(sp, i), x);
            assert_eq!(a.arg(sp, i), Some(x));
        }
        assert_eq!(a.arg(sp, len), None);
    }
}

#[test]
fn a_bare_head_has_the_empty_spine() {
    let mut a = Arena::new();
    let c = constant(&mut a, 9);
    let x = local(&mut a, 0);
    assert_eq!(spine_of(&a, c), Sp::NIL);
    assert_eq!(spine_of(&a, x), Sp::NIL);
    assert!(a.spine(Sp::NIL).is_empty());
    let applied = a.neutral_app(c, x);
    assert_ne!(spine_of(&a, applied), Sp::NIL);
}

#[test]
fn distinct_heads_or_arguments_are_distinct_values() {
    let mut a = Arena::new();
    let (f, g) = (constant(&mut a, 11), constant(&mut a, 12));
    let (x, y) = (constant(&mut a, 13), constant(&mut a, 14));
    let fxy = a.neutral_apply(f, &[x, y]);
    let fyx = a.neutral_apply(f, &[y, x]);
    let gxy = a.neutral_apply(g, &[x, y]);
    let fx = a.neutral_apply(f, &[x]);
    let fxyx = a.neutral_apply(f, &[x, y, x]);
    let all = [fxy, fyx, gxy, fx, fxyx, f];
    for (i, p) in all.iter().enumerate() {
        for q in &all[i + 1..] {
            assert_ne!(p, q);
        }
    }
}

#[test]
fn a_neutral_is_open_exactly_when_its_head_or_an_argument_is() {
    let mut a = Arena::new();
    let mut r = Rng(0x0be_7);
    for _ in 0..2_000 {
        let open_head = r.below(4) == 0;
        let head = if open_head {
            local(&mut a, 5)
        } else {
            constant(&mut a, 20)
        };
        let len = 1 + r.below(10) as usize;
        let args: Vec<V> = (0..len).map(|_| value(&mut a, &mut r, 2)).collect();
        let expected = open_head || args.iter().any(|&x| a.is_open(x));
        let v = a.neutral_apply(head, &args);
        assert_eq!(a.is_open(v), expected);
        let cut = r.below(len as u64) as usize;
        let prefix = a.neutral_apply(head, &args[..cut]);
        let again = a.neutral_apply(prefix, &args[cut..]);
        assert_eq!(again, v);
        assert_eq!(a.is_open(again), expected);
    }
}

#[test]
fn an_environment_extended_by_one_value_is_one_environment() {
    let mut a = Arena::new();
    let mut r = Rng(0xe_57e_d);
    let mut envs = vec![EnvId::EMPTY];
    for _ in 0..5_000 {
        let parent = envs[r.below(envs.len() as u64) as usize];
        let x = value(&mut a, &mut r, 1);
        let e = a.extend(parent, x);
        let y = value(&mut a, &mut r, 1);
        let other = a.extend(parent, y);
        assert_eq!(a.extend(parent, x), e, "back to the first extension");
        assert_eq!(a.extend(parent, y), other, "and to the second");
        assert_eq!(x == y, e == other);
        assert_eq!(a.lookup(e, 0), Some(x));
        assert_eq!(a.lookup(other, 0), Some(y));
        envs.push(e);
    }
}

#[test]
fn a_reset_session_remembers_no_extension() {
    let mut a = Arena::new();
    for round in 0..20u32 {
        let x = constant(&mut a, 1000 + round);
        let e = a.extend(EnvId::EMPTY, x);
        assert_eq!(a.lookup(e, 0), Some(x), "round {round}");
        let y = a.neutral_apply(x, &[x]);
        let f = a.extend(e, y);
        assert_eq!(a.lookup(f, 0), Some(y));
        assert_eq!(a.lookup(f, 1), Some(x));
        a.reset();
        let z = constant(&mut a, 2000 + round);
        let fresh = a.extend(EnvId::EMPTY, z);
        assert_eq!(
            a.lookup(fresh, 0),
            Some(z),
            "round {round}: an extension made before the reset is not reused"
        );
    }
}

#[test]
fn finding_a_neutral_stores_nothing() {
    let mut a = Arena::new();
    let mut r = Rng(0x5_7a1e);
    for round in 0..2_000 {
        let head = constant(&mut a, 300 + r.below(5) as u32);
        let len = 1 + r.below(12) as usize;
        let args: Vec<V> = (0..len).map(|_| value(&mut a, &mut r, 1)).collect();
        let cut = r.below(len as u64) as usize;
        let whole = one_at_a_time(&mut a, head, &args);
        let prefix = a.neutral_apply(head, &args[..cut]);
        let before = a.spine_slot_count();
        assert_eq!(
            a.neutral_apply(prefix, &args[cut..]),
            whole,
            "round {round}"
        );
        assert_eq!(a.neutral_apply(head, &args), whole, "round {round}");
        assert_eq!(one_at_a_time(&mut a, head, &args), whole, "round {round}");
        assert_eq!(
            a.spine_slot_count(),
            before,
            "round {round}: every application was found"
        );
    }
}

#[test]
fn a_new_neutral_stores_its_arguments_once() {
    let mut a = Arena::new();
    let mut r = Rng(0x0_ce5);
    for round in 0..2_000u32 {
        let len = 1 + r.below(20) as usize;
        let args: Vec<V> = (0..len).map(|_| value(&mut a, &mut r, 1)).collect();
        let head = constant(&mut a, 10_000 + round);
        let cut = 1 + r.below(len as u64) as usize;
        let before = a.spine_slot_count();
        let prefix = a.neutral_apply(head, &args[..cut]);
        assert_eq!(a.spine_slot_count() - before, cut, "round {round}");
        let whole = a.neutral_apply(prefix, &args[cut..]);
        let stored = if cut < len { len } else { 0 };
        assert_eq!(
            a.spine_slot_count() - before,
            cut + stored,
            "round {round}: the whole list once more, or nothing when it is the prefix"
        );
        assert_eq!(a.spine(spine_of(&a, whole)), &args[..]);
    }
}

#[test]
fn a_pruning_keeps_exactly_the_entries_its_mask_marks() {
    let mut a = Arena::new();
    let mut r = Rng(0x9_e7a5);
    // Every environment built, with its entries innermost first ([`DEAD`] where pruned away).
    let mut envs: Vec<(EnvId, Vec<V>)> = vec![(EnvId::EMPTY, Vec::new())];
    let mut frames: HashMap<Vec<V>, EnvId> = HashMap::new();
    let mut prunings = 0;
    for round in 0..20_000 {
        let (e, entries) = envs[r.below(envs.len() as u64) as usize].clone();
        let live = entries
            .iter()
            .take(64)
            .enumerate()
            .filter(|(_, v)| **v != DEAD)
            .fold(0u64, |m, (i, _)| m | 1 << i);
        let mask = live & r.next();
        if r.below(3) != 0 || mask == 0 {
            let v = value(&mut a, &mut r, 1);
            let mut longer = vec![v];
            longer.extend(&entries);
            envs.push((a.extend(e, v), longer));
            continue;
        }
        prunings += 1;
        let len = 64 - mask.leading_zeros();
        let want: Vec<V> = (0..len as usize)
            .map(|i| if mask >> i & 1 == 1 { entries[i] } else { DEAD })
            .collect();
        let before = a.frame_slot_count();
        let p = a.prune(e, len, mask);
        let node = a.env(p);
        assert!(node.frame, "round {round}: a pruning is a frame");
        assert_eq!((node.len, node.live), (len, mask), "round {round}");
        for i in 0..len {
            assert_eq!(
                a.lookup(p, i),
                Some(want[i as usize]),
                "round {round}, entry {i}"
            );
        }
        assert_eq!(a.lookup(p, len), None, "round {round}");
        match frames.get(&want) {
            Some(&f) => {
                assert_eq!(p, f, "round {round}: one frame per content");
                assert_eq!(
                    a.frame_slot_count(),
                    before,
                    "round {round}: finding a frame stores nothing"
                );
            }
            None => {
                assert_eq!(
                    a.frame_slot_count() - before,
                    mask.count_ones() as usize,
                    "round {round}: a new frame stores its live entries once"
                );
                frames.insert(want.clone(), p);
            }
        }
        envs.push((p, want));
    }
    assert!(prunings > 3_000, "{prunings} prunings");
    assert!(frames.len() < prunings, "some pruning found its frame");
}

#[test]
fn an_environment_remembers_its_two_latest_prunings() {
    let mut a = Arena::new();
    let vals: Vec<V> = (0..6).map(|i| constant(&mut a, 400 + i)).collect();
    let e = vals.iter().fold(EnvId::EMPTY, |e, &v| a.extend(e, v));
    let len = |m: u64| 64 - m.leading_zeros();
    let (m1, m2, m3) = (0b10_1101u64, 0b11_0011u64, 0b111u64);
    assert_eq!(a.last_prune(e, m1), None, "nothing pruned yet");
    let r1 = a.prune(e, len(m1), m1);
    assert_eq!(a.last_prune(e, m1), Some(r1));
    assert_eq!(a.last_prune(e, m2), None);
    let r2 = a.prune(e, len(m2), m2);
    assert_eq!(a.last_prune(e, m1), Some(r1), "two are remembered");
    assert_eq!(a.last_prune(e, m2), Some(r2));
    let r3 = a.prune(e, len(m3), m3);
    assert_eq!(a.last_prune(e, m1), None, "the third forgets the first");
    assert_eq!(a.last_prune(e, m2), Some(r2));
    assert_eq!(a.last_prune(e, m3), Some(r3));
    assert_eq!(a.prune(e, len(m1), m1), r1, "the same frame again");
    assert_eq!(a.last_prune(e, m1), Some(r1));
    assert_eq!(a.last_prune(e, m3), Some(r3));
    assert_eq!(a.last_prune(e, m2), None, "and now the oldest is forgotten");
    let other = a.extend(e, vals[0]);
    assert_eq!(
        a.last_prune(other, m1),
        None,
        "each environment has its own"
    );
    assert_eq!(a.last_prune(r1, m1 & 0b1), None, "a frame is pruned afresh");
}
