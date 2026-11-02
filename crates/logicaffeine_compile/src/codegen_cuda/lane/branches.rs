//! A form's launches as a graph over several streams. A launch runs after every earlier launch that writes memory it
//! reads or writes, or reads memory it writes — memory by where it lives: a step-local array or a launch's scratch by
//! its bytes in the arena (which several tensors share in turn), anything else by its buffer, an alias — a view — where
//! its array lives (`arena::home`). Each launch takes a stream: the stream whose last launch it follows, or one whose work it already
//! follows, or the least recently used; it waits on another stream's launch by an event only where nothing it already
//! follows orders the two. The launches compute what they compute in order, every dependency kept.

use super::arena::{self, Arena};
use super::plan::{Schedule, Touch};

/// The streams a form's launches spread over, the step's own among them.
pub const STREAMS: usize = 4;

/// A form's launches over its streams.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Branched {
    /// Each launch's stream (0 the step's own).
    pub stream: Vec<usize>,
    /// The events each launch's stream waits on before it.
    pub waits: Vec<Vec<usize>>,
    /// The event recorded on each launch's stream after it, where a launch on another stream waits on it.
    pub records: Vec<Option<usize>>,
    /// The streams the launches take besides the step's own.
    pub sides: Vec<usize>,
    /// The events the step's own stream waits on at its end: each side stream's last launch's.
    pub joins: Vec<usize>,
    /// The events of the form.
    pub events: usize,
}

/// Memory a launch touches: bytes of the arena, or a buffer of its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Region {
    Arena(usize, usize),
    Named(String),
}

impl Region {
    pub fn overlaps(&self, other: &Region) -> bool {
        match (self, other) {
            (Region::Arena(a0, a1), Region::Arena(b0, b1)) => a0 < b1 && b0 < a1,
            (Region::Named(a), Region::Named(b)) => a == b,
            _ => false,
        }
    }
}

/// Where tensor `t`'s array `touch` lives: in the arena, or a buffer of its own — an alias's in its home's.
fn region(s: &Schedule, arena: &Arena, t: &str, touch: Touch) -> Region {
    let name = arena::home(s, t, touch);
    match arena.at(name, touch) {
        Some(p) => Region::Arena(p.offset, p.offset + p.bytes),
        None => Region::Named(format!("{name}/{touch:?}")),
    }
}

/// The memory launch `i` reads and writes: its tensors' arrays and its scratch (written and read, as a launch's own).
pub fn regions(s: &Schedule, arena: &Arena, i: usize) -> (Vec<Region>, Vec<Region>) {
    let l = &s.launches[i];
    let (r, w) = l.touches(s.numerics);
    let reads: Vec<Region> = r.into_iter().map(|(t, touch)| region(s, arena, t, touch)).collect();
    let mut writes: Vec<Region> = w.into_iter().map(|(t, touch)| region(s, arena, t, touch)).collect();
    let owner = l.writes().first().map_or(String::new(), |t| t.to_string());
    for sc in arena::scratch(l, s.numerics) {
        writes.push(match arena.scratch_at(i, sc.tag) {
            Some(p) if !sc.counter => Region::Arena(p.offset, p.offset + p.bytes),
            _ => Region::Named(format!("{}#{owner}", sc.tag)),
        });
    }
    (reads, writes)
}

/// Whether launch `j` must follow launch `i`: one writes memory the other reads or writes.
fn conflict(a: &(Vec<Region>, Vec<Region>), b: &(Vec<Region>, Vec<Region>)) -> bool {
    let hits = |x: &[Region], y: &[Region]| x.iter().any(|p| y.iter().any(|q| p.overlaps(q)));
    hits(&a.1, &b.0) || hits(&a.1, &b.1) || hits(&a.0, &b.1)
}

/// The form's launches spread over `STREAMS` streams.
pub fn branch(s: &Schedule, arena: &Arena) -> Branched {
    let n = s.launches.len();
    let mem: Vec<(Vec<Region>, Vec<Region>)> = (0..n).map(|i| regions(s, arena, i)).collect();
    // `clock[i][t]`: the last launch on stream `t` that launch `i` follows (by order on a stream or by events), plus one.
    let mut clock: Vec<[usize; STREAMS]> = Vec::with_capacity(n);
    let mut last: [Option<usize>; STREAMS] = [None; STREAMS];
    let mut stream_clock: [[usize; STREAMS]; STREAMS] = [[0; STREAMS]; STREAMS];
    let mut out = Branched { stream: vec![0; n], waits: vec![Vec::new(); n], records: vec![None; n], ..Default::default() };
    for j in 0..n {
        let preds: Vec<usize> = (0..j).filter(|&i| conflict(&mem[i], &mem[j])).collect();
        // Whether stream `t` has nothing running that `j` does not follow: unused, its last launch a predecessor, or
        // ordered before one.
        let follows = |t: usize| match last[t] {
            None => true,
            Some(l) => preds.iter().any(|&p| p == l || clock[p][t] > l),
        };
        let chained = preds.iter().copied().max().map(|p| out.stream[p]).filter(|&t| last[t] == preds.iter().copied().max());
        let t = chained
            .or_else(|| (0..STREAMS).filter(|&t| follows(t)).max_by_key(|&t| last[t].map_or(0, |l| l + 1)))
            .unwrap_or_else(|| (0..STREAMS).min_by_key(|&t| last[t].map_or(0, |l| l + 1)).expect("a stream"));
        let mut c = stream_clock[t];
        // The latest first: waiting on a stream's later launch orders its earlier ones too.
        for &p in preds.iter().rev() {
            let ps = out.stream[p];
            if ps != t && c[ps] <= p {
                let e = *out.records[p].get_or_insert_with(|| {
                    out.events += 1;
                    out.events - 1
                });
                if !out.waits[j].contains(&e) {
                    out.waits[j].push(e);
                }
                for (u, v) in c.iter_mut().zip(clock[p]) {
                    *u = (*u).max(v);
                }
                c[ps] = c[ps].max(p + 1);
            }
        }
        c[t] = j + 1;
        out.stream[j] = t;
        stream_clock[t] = c;
        last[t] = Some(j);
        clock.push(c);
        if t != 0 && !out.sides.contains(&t) {
            out.sides.push(t);
        }
    }
    out.sides.sort_unstable();
    for &t in &out.sides {
        let l = last[t].expect("a side stream's launch");
        let e = *out.records[l].get_or_insert_with(|| {
            out.events += 1;
            out.events - 1
        });
        out.joins.push(e);
    }
    out
}
