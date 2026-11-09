use super::*;
use std::cell::Cell;

/// A deterministic stream of 64-bit hashes (xorshift), led by hashes whose top half is all zeros
/// or all ones: the two ends a slot's kept half can take.
fn hashes(seed: u64) -> impl Iterator<Item = u64> {
    let mut x = seed;
    [0u64, 1, u32::MAX as u64, (u32::MAX as u64) << 32, u64::MAX, u64::MAX - 1]
        .into_iter()
        .chain(std::iter::from_fn(move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            Some(x)
        }))
}

#[test]
fn a_slot_is_half_a_hash_and_an_id() {
    assert_eq!(std::mem::size_of_val(&ContentTable::VACANT), 8);
    let mut t = ContentTable::new();
    for (e, h) in hashes(0x51ab).take(5_000).enumerate() {
        t.insert(h, e as u32);
    }
    assert_eq!(t.bytes(), t.slots.len() * 8);
}

#[test]
fn every_entry_is_found_by_its_hash_after_every_growth() {
    let hs: Vec<u64> = hashes(0xc0ffee).take(40_000).collect();
    let mut t = ContentTable::new();
    let mut growths = 0;
    for (e, &h) in hs.iter().enumerate() {
        let cap = t.slots.len();
        t.insert(h, e as u32);
        if t.slots.len() != cap {
            growths += 1;
            for (k, &hk) in hs[..=e].iter().enumerate() {
                assert_eq!(
                    t.find(hk, |x| x == k as u32),
                    Some(k as u32),
                    "entry {k} after growing to {} slots",
                    t.slots.len()
                );
            }
        }
    }
    assert!(growths >= 6, "{growths} growths");
    assert_eq!(t.len, hs.len());
}

#[test]
fn entries_whose_hashes_share_their_top_half_are_told_apart_by_content() {
    let top = 0x9E37_79B9u64 << 32;
    let mut t = ContentTable::new();
    for e in 0..40u32 {
        t.insert(top | e as u64, e);
    }
    for e in 40..80u32 {
        t.insert(top | 7, e);
    }
    for e in 0..80u32 {
        let h = if e < 40 { top | e as u64 } else { top | 7 };
        assert_eq!(t.find(h, |x| x == e), Some(e), "entry {e}");
    }
    assert_eq!(t.find(top | 12_345, |x| x == 999), None);
    assert_eq!(t.find(top | 7, |x| x == 3), Some(3), "a top-half match is decided by content");
}

#[test]
fn content_is_compared_only_where_the_kept_half_of_the_hash_agrees() {
    let hs: Vec<u64> = hashes(0xfeed).take(20_000).collect();
    let mut t = ContentTable::new();
    for (e, &h) in hs.iter().enumerate() {
        t.insert(h, e as u32);
    }
    let tops: std::collections::HashSet<u32> = hs.iter().map(|h| (h >> 32) as u32).collect();
    let compared = Cell::new(0u32);
    let mut probed = 0;
    for h in hashes(0xbad5eed).skip(6).take(20_000) {
        if tops.contains(&((h >> 32) as u32)) {
            continue;
        }
        probed += 1;
        assert_eq!(
            t.find(h, |_| {
                compared.set(compared.get() + 1);
                true
            }),
            None
        );
    }
    assert!(probed > 19_000, "{probed} absent hashes probed");
    assert_eq!(compared.get(), 0, "an entry whose top half differs is never compared");
}

#[test]
fn an_emptied_table_finds_nothing_and_keeps_its_slots() {
    let mut t = ContentTable::new();
    let hs: Vec<u64> = hashes(0xabc).take(3_000).collect();
    for (e, &h) in hs.iter().enumerate() {
        t.insert(h, e as u32);
    }
    let slots = t.slots.len();
    t.clear();
    assert_eq!(t.slots.len(), slots);
    for (e, &h) in hs.iter().enumerate() {
        assert_eq!(t.find(h, |x| x == e as u32), None);
    }
    for (e, &h) in hs.iter().enumerate().take(100) {
        t.insert(h, e as u32 + 7);
        assert_eq!(t.find(h, |x| x == e as u32 + 7), Some(e as u32 + 7));
    }
}
