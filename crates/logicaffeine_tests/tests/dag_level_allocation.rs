//! The core's universe levels (`dag::level`) answer the questions the kernel asks most often
//! without allocating: deciding `≤` builds its trees in one arena whose storage the next decision
//! reuses, a substitution already known is found without building a list, and substituting into a
//! list builds the result on the stack. Only remembering an answer may allocate, as a memo grows.
//!
//! The allocations counted are this thread's own (a counting global allocator with a thread-local
//! count), so tests running beside one another do not disturb each other's counts.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use logicaffeine_kernel::dag::level::{LevelId, Levels};
use logicaffeine_kernel::dag::name::{NameId, Names};

struct Counting;

thread_local! {
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

fn count() {
    let _ = ALLOCATIONS.try_with(|c| c.set(c.get() + 1));
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count();
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count();
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count();
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static COUNTING: Counting = Counting;

/// `f`'s result and the allocations it made on this thread.
fn allocations<R>(f: impl FnOnce() -> R) -> (R, usize) {
    let before = ALLOCATIONS.with(|c| c.get());
    let r = f();
    (r, ALLOCATIONS.with(|c| c.get()) - before)
}

/// Levels over `ps` of every shape up to depth two: successors, `max` and `imax` of each other.
fn levels(ls: &mut Levels, ps: &[NameId]) -> Vec<LevelId> {
    let mut out = vec![LevelId::ZERO];
    out.extend(ps.iter().map(|p| ls.param(*p)));
    for _ in 0..2 {
        let prev = out.clone();
        for &a in &prev {
            out.push(ls.succ(a));
            for &b in &prev {
                out.push(ls.max_raw(a, b));
                out.push(ls.imax_raw(a, b));
            }
        }
        out.sort();
        out.dedup();
    }
    out
}

fn pairs(levels: &[LevelId], seed: u64, n: usize) -> Vec<(LevelId, LevelId)> {
    let mut x = seed;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let k = levels.len() as u64;
            (levels[(x % k) as usize], levels[((x >> 32) % k) as usize])
        })
        .collect()
}

#[test]
fn deciding_an_order_of_levels_allocates_only_to_remember_it() {
    let mut names = Names::new();
    let (u, v) = (names.dotted("u"), names.dotted("v"));
    let mut ls = Levels::new();
    let all = levels(&mut ls, &[u, v]);
    for (a, b) in pairs(&all, 0xa11c, 2_000) {
        ls.leq(a, b);
    }
    let fresh = pairs(&all, 0xdec1de, 4_000);
    let (decided, made) = allocations(|| fresh.iter().filter(|(a, b)| ls.leq(*a, *b)).count());
    assert!(decided > 0 && decided < fresh.len(), "{decided} of {} hold", fresh.len());
    assert!(
        made <= 16,
        "{made} allocations deciding {} orders: only the memo's growth may allocate",
        fresh.len()
    );
}

#[test]
fn a_known_substitution_is_found_without_allocating() {
    let mut names = Names::new();
    let ps: Vec<NameId> = ["u", "v", "w"].iter().map(|n| names.dotted(n)).collect();
    let mut ls = Levels::new();
    let one = ls.succ(LevelId::ZERO);
    let two = ls.succ(one);
    let us = ls.list(&[one, two, LevelId::ZERO]);
    let s = ls.substitution(&ps, us);
    let (again, made) = allocations(|| ls.substitution(&ps, us));
    assert_eq!(again, s);
    assert_eq!(made, 0, "finding a known substitution allocated");
}

#[test]
fn substituting_into_a_list_allocates_only_to_remember_it() {
    let mut names = Names::new();
    let ps: Vec<NameId> = ["u", "v"].iter().map(|n| names.dotted(n)).collect();
    let mut ls = Levels::new();
    let all = levels(&mut ls, &ps);
    let one = ls.succ(LevelId::ZERO);
    let us = ls.list(&[one, LevelId::ZERO]);
    let s = ls.substitution(&ps, us);
    let lists: Vec<_> = pairs(&all, 0x115, 3_000)
        .into_iter()
        .map(|(a, b)| ls.list(&[a, b]))
        .collect();
    // Every substituted level and every result list exists already: what is left to a
    // substitution into a list is building its result and remembering it.
    let mut expected = Vec::with_capacity(lists.len());
    for &l in &lists {
        let (a, b) = (ls.get_list(l)[0], ls.get_list(l)[1]);
        let (x, y) = (ls.subst(a, s), ls.subst(b, s));
        expected.push(ls.list(&[x, y]));
    }
    let mut results = Vec::with_capacity(lists.len());
    let ((), made) = allocations(|| {
        for &l in &lists {
            results.push(ls.subst_list(l, s));
        }
    });
    assert_eq!(results, expected);
    assert!(
        made <= 16,
        "{made} allocations substituting into {} lists: only the memo's growth may allocate",
        lists.len()
    );
    let ((), made) = allocations(|| {
        for &l in &lists {
            ls.subst_list(l, s);
        }
    });
    assert_eq!(made, 0, "substituting into lists already substituted allocated");
}
