use super::*;
use crate::crdt::PNCounter;

type Stock = ORMap<String, LatestWrite<i64>>;

fn joined(a: &Stock, b: &Stock) -> Stock {
    let mut out = a.clone();
    out.merge(b);
    out
}

fn view(m: &Stock) -> Vec<(String, i64)> {
    m.sorted_entries()
        .into_iter()
        .map(|(k, v)| (k.clone(), *v.get()))
        .collect()
}

#[test]
fn put_overwrites_and_reads_back() {
    let mut m = Stock::new(1);
    m.put("wood".into(), 50);
    m.put("wood".into(), 55);
    m.put("clay".into(), 7);
    assert_eq!(m.value(&"wood".into()), Some(&55));
    assert_eq!(view(&m), vec![("clay".into(), 7), ("wood".into(), 55)]);
}

#[test]
fn a_write_that_has_seen_another_beats_it() {
    let mut a = Stock::new(1);
    let mut b = Stock::new(2);
    a.put("wood".into(), 50);
    b.put("wood".into(), 99);
    a.merge(&b);
    a.put("wood".into(), 1);
    b.merge(&a);
    assert_eq!(b.value(&"wood".into()), Some(&1));
}

/// Random replicas with random write/remove/sync histories converge, whatever the merge order.
#[test]
fn replicas_converge_under_any_merge_order() {
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = move |n: u64| {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (seed >> 33) % n
    };
    for _ in 0..300 {
        let mut reps: Vec<Stock> = (1..=3).map(Stock::new).collect();
        for _ in 0..12 {
            let r = next(3) as usize;
            let key = format!("k{}", next(4));
            match next(5) {
                0 => reps[r].remove(&key),
                1 => {
                    let other = reps[next(3) as usize].clone();
                    reps[r].merge(&other);
                }
                _ => reps[r].put(key, next(10) as i64),
            }
        }
        let orders = [
            [0, 1, 2],
            [0, 2, 1],
            [1, 0, 2],
            [1, 2, 0],
            [2, 0, 1],
            [2, 1, 0],
        ];
        let results: Vec<Vec<(String, i64)>> = orders
            .iter()
            .map(|o| view(&joined(&joined(&reps[o[0]], &reps[o[1]]), &reps[o[2]])))
            .collect();
        assert!(
            results.windows(2).all(|w| w[0] == w[1]),
            "merge order changed the map: {results:?}"
        );
        let all = joined(&joined(&reps[0], &reps[1]), &reps[2]);
        assert_eq!(view(&joined(&all, &all)), view(&all), "idempotent");
    }
}

#[test]
fn test_ormap_get_or_insert() {
    let mut map: ORMap<String, PNCounter> = ORMap::new(1);
    map.get_or_insert("score".to_string()).increment(10);
    assert_eq!(map.get(&"score".to_string()).unwrap().value(), 10);
}

#[test]
fn test_ormap_remove() {
    let mut map: ORMap<String, PNCounter> = ORMap::new(1);
    map.get_or_insert("key".to_string()).increment(5);
    map.remove(&"key".to_string());
    assert!(map.get(&"key".to_string()).is_none());
}

#[test]
fn test_ormap_concurrent_update() {
    let mut a: ORMap<String, PNCounter> = ORMap::new(1);
    let mut b: ORMap<String, PNCounter> = ORMap::new(2);

    a.get_or_insert("score".to_string()).increment(10);
    b.get_or_insert("score".to_string()).increment(5);

    a.merge(&b);
    assert_eq!(a.get(&"score".to_string()).unwrap().value(), 15);
}
