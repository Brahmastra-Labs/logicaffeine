use super::*;
use std::mem::size_of;

#[test]
fn the_memory_report_counts_every_store_of_a_session() {
    let mut st = State::default();
    for e in 0..200u32 {
        let set: std::rc::Rc<[u32]> = (0..e % 70).collect::<Vec<u32>>().into();
        st.loose_sets.insert(ExprId(e), set);
        st.eval_cache.insert((e, e), e);
        st.proven.insert((e, e + 1), 1);
    }
    let State {
        arena,
        budget: _,
        epoch: _,
        const_type,
        const_body,
        rule_rhs,
        eval_cache,
        wide_prune_cache,
        loose_sets,
        infer_cache,
        check_cache,
        safe_checked,
        scoped_checked,
        scope: _,
        proven,
        neq,
        failure,
        depth: _,
        level: _,
        eager: _,
        probe: _,
        scratch_seen: _,
        counters: _,
        atoms,
    } = &st;
    let loose = loose_sets.capacity() * (size_of::<(ExprId, std::rc::Rc<[u32]>)>() + 1)
        + loose_sets
            .values()
            .map(|s| 2 * size_of::<usize>() + s.len() * 4)
            .sum::<usize>();
    let held = arena.bytes()
        + const_type.bytes()
        + const_body.bytes()
        + rule_rhs.bytes()
        + eval_cache.bytes()
        + wide_prune_cache.bytes()
        + loose
        + infer_cache.bytes()
        + check_cache.bytes()
        + safe_checked.bytes()
        + scoped_checked.bytes()
        + proven.bytes()
        + neq.bytes()
        + failure.bytes()
        + atoms.capacity() * size_of::<(u32, u32, u32)>();
    assert!(loose > 200 * 4 * 30, "the sets' own words are counted");
    assert_eq!(st.bytes(), held);
}
