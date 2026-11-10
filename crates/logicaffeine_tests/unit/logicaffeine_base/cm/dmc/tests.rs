use super::*;
use crate::entropy::squash;

/// Feed `bytes` most significant bit first, returning what coding them with the machine's own prediction costs.
fn feed(m: &mut Dmc, bytes: &[u8]) -> f64 {
    let mut bits = 0.0;
    for &b in bytes {
        for i in (0..8).rev() {
            let bit = (b >> i) & 1 == 1;
            let p = f64::from(squash(m.input())) / 4096.0;
            bits -= if bit { p.log2() } else { (1.0 - p).log2() };
            m.update(bit);
        }
    }
    bits
}

fn noise(n: usize, seed: u64) -> Vec<u8> {
    let mut s = seed;
    (0..n)
        .map(|_| {
            s = s.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            (s >> 56) as u8
        })
        .collect()
}

#[test]
fn a_fresh_machine_is_the_byte_trees_and_predicts_one_half() {
    let m = Dmc::new(0, 2);
    assert_eq!(m.nodes.len(), TREES, "the capacity is at least the trees");
    assert_eq!(m.input(), 0);
    for t in 0..256u32 {
        for k in 1..256u32 {
            let n = m.nodes[(t * 255 + k - 1) as usize];
            for bit in 0..2 {
                let partial = 2 * k + bit;
                let expected = if partial < 256 { t * 255 + partial - 1 } else { (partial & 255) * 255 };
                assert_eq!(n.next(bit as usize), expected, "tree {t}, partial byte {k}, bit {bit}");
            }
        }
    }
}

#[test]
fn a_state_is_twelve_bytes_and_its_links_and_history_never_disturb_each_other() {
    assert_eq!(std::mem::size_of::<Node>(), 12);
    let mut n = Node::new([INDEX, 0], [7, 9], 0xA5);
    assert_eq!((n.next(0), n.next(1), n.history(), n.count), (INDEX, 0, 0xA5, [7, 9]));
    n.set_history(0xFF);
    n.set_next(1, 12_345);
    assert_eq!((n.next(0), n.next(1), n.history()), (INDEX, 12_345, 0xFF));
    n.set_next(0, 0);
    n.set_history(0);
    assert_eq!((n.next(0), n.next(1), n.history()), (0, 12_345, 0));
}

#[test]
fn the_eighth_bit_leads_to_the_tree_of_the_byte_just_completed() {
    let mut m = Dmc::new(TREES, 1 << 20);
    for byte in [0u8, 0x41, 0xFF, 0x80, 0x7F] {
        for i in (0..8).rev() {
            m.update((byte >> i) & 1 == 1);
        }
        assert_eq!(m.at, u32::from(byte) * 255, "after {byte:#04x}");
    }
}

#[test]
fn a_repeating_phrase_becomes_almost_free_and_grows_states() {
    let phrase = b"the machine clones the states that carry the traffic; ".repeat(300);
    let mut m = Dmc::new(1 << 20, 2);
    let warm = 20 * 54;
    let first = feed(&mut m, &phrase[..warm]) / warm as f64;
    let rest = feed(&mut m, &phrase[warm..]) / (phrase.len() - warm) as f64;
    assert!(m.nodes.len() > TREES + 500, "cloning grew {} states", m.nodes.len() - TREES);
    assert!(rest < 0.25, "{rest:.3} bits a byte once the phrase has repeated");
    assert!(rest < first / 8.0, "{rest:.3} bits a byte later against {first:.3} early");
}

#[test]
fn deeper_contexts_beat_the_order_one_trees_on_text() {
    let text = b"she sells sea shells by the sea shore, the shells she sells are sea shells for sure. ".repeat(60);
    let mut cloning = Dmc::new(1 << 20, 2);
    let mut fixed = Dmc::new(TREES, 2);
    let (a, b) = (feed(&mut cloning, &text), feed(&mut fixed, &text));
    assert_eq!(fixed.nodes.len(), TREES, "a machine with no room never clones");
    assert!(a < b * 0.5, "cloning {a:.0} bits, order-1 trees alone {b:.0}");
}

#[test]
fn a_machine_never_grows_past_its_capacity_and_goes_stale_once_full() {
    let mut m = Dmc::new(TREES + 5000, 2);
    let data = noise(200_000, 7);
    let mut stale_at = None;
    for (i, chunk) in data.chunks(1000).enumerate() {
        feed(&mut m, chunk);
        assert!(m.nodes.len() <= TREES + 5000);
        if stale_at.is_none() && m.stale() {
            stale_at = Some(i);
        }
    }
    assert_eq!(m.nodes.len(), TREES + 5000, "noise fills the machine");
    assert!(stale_at.is_some(), "and it goes stale once full");
    m.reset(0x41);
    assert_eq!((m.nodes.len(), m.at, m.stale()), (TREES, 0x41 * 255, false), "a reset is back at the trees, at the last byte's root");
}

#[test]
fn the_forest_gives_its_inputs_restarts_only_its_fast_machines_and_stays_in_step() {
    assert_eq!(Forest::INPUTS, 6);
    // At its smallest a fast machine is the trees plus a few thousand states, stale after ~58 KB of noise.
    let mut f = Forest::new(1024);
    let data = noise(120_000, 9);
    let mut restarts = [0usize; MACHINES.len()];
    for &b in &data {
        for i in (0..8).rev() {
            for k in 0..Forest::INPUTS {
                assert!(f.input(k).abs() <= 2047);
            }
            f.update((b >> i) & 1 == 1);
        }
        let before: Vec<usize> = f.machines.iter().map(|m| m.nodes.len()).collect();
        f.end_of_byte(b);
        for (k, m) in f.machines.iter().enumerate() {
            if m.nodes.len() < before[k] {
                restarts[k] += 1;
                assert_eq!(m.at, u32::from(b) * 255, "machine {k} restarts at the last byte's root");
            }
        }
    }
    assert!(restarts[..RESETTING].iter().any(|&n| n > 0), "a fast machine filled and restarted: {restarts:?}");
    assert_eq!(restarts[RESETTING..], [0, 0], "the slow machines never restart");
    // Every machine is still at a byte's first bit: a constant byte is learnt to near nothing by all of them.
    let mut cost = 0.0;
    for round in 0..3000 {
        for i in (0..8).rev() {
            let bit = (0x41u8 >> i) & 1 == 1;
            if round >= 2000 {
                let p = f64::from(squash((0..Forest::INPUTS).map(|k| f.input(k)).sum::<i32>() / Forest::INPUTS as i32)) / 4096.0;
                cost -= if bit { p.log2() } else { (1.0 - p).log2() };
            }
            f.update(bit);
        }
        f.end_of_byte(0x41);
    }
    assert!(cost < 200.0, "1000 constant bytes cost {cost:.0} bits");
}
