//! Per-check reuse of independently lowered, closed definition syntax.
//!
//! Entries contain no normalization result or typing verdict. The enclosing
//! PrefixScope borrows one immutable Context, so an original context-owned Term
//! address is a stable identity only within that context and generation.
use super::*;

const SLOTS: usize = 16;
// Each admission/copy helper already has an independent bounded-work contract.
// Extra fixed work covers bounded slot scans, replacement, and headroom checks.
const ADMISSION_WORK: usize = 2 * PREFIX_ADMISSION_WORK + 2 * PREFIX_COPY_WORK + 4 * SLOTS * SLOTS;

#[derive(Clone, Copy, Default)]
struct KeyMetrics {
    nodes: usize,
    payload: usize,
    bytes: usize,
}
impl KeyMetrics {
    fn comparison_work(self) -> usize {
        2 * (self.nodes + self.payload) + 1
    }
}
struct Entry {
    source: usize,
    levels: Option<Vec<Universe>>,
    body: Db,
    metrics: PrefixMetrics,
    key_metrics: KeyMetrics,
    touched: u64,
    // Syntax is destroyed before its retained-storage reservations.
    _key_memory: PrefixTicket,
    _body_memory: PrefixTicket,
}
pub(super) struct Cache {
    entries: [Option<Entry>; SLOTS],
    clock: u64,
}
impl Default for Cache {
    fn default() -> Self {
        Self {
            entries: std::array::from_fn(|_| None),
            clock: 0,
        }
    }
}
impl Cache {
    fn touch(&mut self) -> u64 {
        // Saturation can only make eviction less selective; it cannot authorize
        // a hit, extend a lifetime, or create additional storage or work budget.
        self.clock = self.clock.saturating_add(1);
        self.clock
    }
    fn oldest(&self) -> Option<usize> {
        self.entries
            .iter()
            .enumerate()
            .filter_map(|(i, e)| e.as_ref().map(|e| (i, e.touched)))
            .min_by_key(|(_, age)| *age)
            .map(|(i, _)| i)
    }
}
enum Lookup {
    Hit(Db),
    Miss,
    Bypass,
}

fn active(table: &PrefixTable, ctx: &Context) -> bool {
    recheck_budget_active()
        && table.scope != 0
        && table.context == ctx as *const Context as usize
        && PREFIX_MEMORY.with(|m| m.get().scope == table.scope)
}
fn copy_work(metrics: PrefixMetrics) -> usize {
    3 * (metrics.nodes + metrics.levels) + metrics.payload + 2
}

// A comparison only descends where the admitted stored key has the same shape.
// Thus its depth and payload bound the work even for an unadmitted query key.
fn same_levels(stored: Option<&[Universe]>, query: Option<&[Universe]>) -> Option<bool> {
    fn level(a: &Universe, b: &Universe, depth: usize) -> Option<bool> {
        if depth > PREFIX_DEPTH || !nat_order_spend(2) {
            return None;
        }
        Some(match (a, b) {
            (Universe::SProp, Universe::SProp) | (Universe::Prop, Universe::Prop) => true,
            (Universe::Type(a), Universe::Type(b)) => a == b,
            (Universe::Var(a), Universe::Var(b)) => {
                if a.len() != b.len() {
                    false
                } else {
                    if !nat_order_spend(2 * a.len()) {
                        return None;
                    }
                    a == b
                }
            }
            (Universe::Succ(a), Universe::Succ(b)) => level(a, b, depth + 1)?,
            (Universe::Max(a, b), Universe::Max(c, d))
            | (Universe::IMax(a, b), Universe::IMax(c, d)) => {
                level(a, c, depth + 1)? && level(b, d, depth + 1)?
            }
            _ => false,
        })
    }
    if !nat_order_tick() {
        return None;
    }
    match (stored, query) {
        (None, None) => Some(true),
        (Some(a), Some(b)) if a.len() == b.len() => {
            for (a, b) in a.iter().zip(b) {
                if !level(a, b, 1)? {
                    return Some(false);
                }
            }
            Some(true)
        }
        _ => Some(false),
    }
}

fn lookup(ctx: &Context, source: usize, levels: Option<&[Universe]>) -> Lookup {
    PREFIX_TABLE.with(|state| {
        let mut table = state.borrow_mut();
        if !active(&table, ctx) || !prefix_work_available(SLOTS) {
            return Lookup::Bypass;
        }
        for i in 0..SLOTS {
            if !nat_order_tick() {
                return Lookup::Bypass;
            }
            let Some(entry) = table.definitions.entries[i].as_ref() else {
                continue;
            };
            if entry.source != source {
                continue;
            }
            let work = entry.key_metrics.comparison_work() + copy_work(entry.metrics);
            if !prefix_work_available(work + SLOTS - i - 1) {
                return Lookup::Bypass;
            }
            if !matches!(same_levels(entry.levels.as_deref(), levels), Some(true)) {
                continue;
            }
            // Account for the transient copy as well as the retained cache. The
            // copying helpers reserve each actual capacity before filling it.
            if PREFIX_MEMORY.with(|m| {
                m.get()
                    .live
                    .checked_add(entry.metrics.bytes)
                    .is_none_or(|n| !prefix_combined_fits(n))
            }) {
                return Lookup::Bypass;
            }
            let Some((body, temporary)) = prefix_copy(&entry.body) else {
                return Lookup::Bypass;
            };
            let touched = table.definitions.touch();
            table.definitions.entries[i]
                .as_mut()
                .expect("present syntax entry")
                .touched = touched;
            // The result now belongs to the ordinary evaluator. Only this copy's
            // auxiliary reservation ends; the cache's original ticket remains.
            drop(temporary);
            return Lookup::Hit(body);
        }
        Lookup::Miss
    })
}

fn measure_key(levels: Option<&[Universe]>) -> Option<KeyMetrics> {
    fn level(u: &Universe, depth: usize, m: &mut KeyMetrics) -> Option<()> {
        if depth > PREFIX_DEPTH {
            return None;
        }
        m.nodes = m.nodes.checked_add(1)?;
        if m.nodes > PREFIX_NODES || !nat_order_tick() {
            return None;
        }
        match u {
            Universe::Var(name) => {
                if name.len() > 256 {
                    return None;
                }
                m.payload = m.payload.checked_add(name.len())?;
                if m.payload > 16384 || !nat_order_spend(name.len() + 1) {
                    return None;
                }
                m.bytes = m.bytes.checked_add(name.len())?;
            }
            Universe::Succ(a) => {
                m.bytes = m.bytes.checked_add(std::mem::size_of::<Universe>())?;
                level(a, depth + 1, m)?;
            }
            Universe::Max(a, b) | Universe::IMax(a, b) => {
                m.bytes = m.bytes.checked_add(2 * std::mem::size_of::<Universe>())?;
                level(a, depth + 1, m)?;
                level(b, depth + 1, m)?;
            }
            Universe::SProp | Universe::Prop | Universe::Type(_) => {}
        }
        (m.bytes <= PREFIX_TREE_BYTES).then_some(())
    }
    let mut m = KeyMetrics::default();
    if let Some(levels) = levels {
        if levels.len() > PREFIX_NODES {
            return None;
        }
        m.bytes = levels.len().checked_mul(std::mem::size_of::<Universe>())?;
        if m.bytes > PREFIX_TREE_BYTES {
            return None;
        }
        for u in levels {
            level(u, 1, &mut m)?;
        }
    }
    Some(m)
}

// Remove victims outside the table borrow so all owned syntax and its tickets
// are destroyed before a replacement allocates. At most SLOTS entries retire.
fn make_room(ctx: &Context, bytes: usize, copy_bytes: usize) -> Option<usize> {
    for _ in 0..=SLOTS {
        if !nat_order_spend(SLOTS + 1) {
            return None;
        }
        let decision = PREFIX_TABLE.with(|state| {
            let mut table = state.borrow_mut();
            if !active(&table, ctx) {
                return None;
            }
            let cache = &mut table.definitions;
            let headroom = cache
                .entries
                .iter()
                .flatten()
                .map(|e| e.metrics.bytes)
                .max()
                .unwrap_or(0)
                .max(copy_bytes);
            let fits = PREFIX_MEMORY.with(|m| {
                m.get()
                    .live
                    .checked_add(bytes)
                    .and_then(|n| n.checked_add(headroom))
                    .is_some_and(prefix_combined_fits)
            });
            if fits {
                if let Some(slot) = cache.entries.iter().position(Option::is_none) {
                    return Some((Some(slot), None));
                }
            }
            let victim = cache.oldest()?;
            Some((None, cache.entries[victim].take()))
        })?;
        match decision {
            (Some(slot), _) => return Some(slot),
            (None, retired) => drop(retired),
        }
    }
    None
}

fn remember(ctx: &Context, source: usize, levels: Option<&[Universe]>, body: &Db) -> Option<()> {
    if !PREFIX_TABLE.with(|s| active(&s.borrow(), ctx)) || !prefix_work_available(ADMISSION_WORK) {
        return None;
    }
    let key_metrics = measure_key(levels)?;
    let metrics = prefix_metrics(body)?;
    if !metrics.closed {
        return None;
    }
    let bytes = key_metrics.bytes.checked_add(metrics.bytes)?;
    let slot = make_room(ctx, bytes, metrics.bytes)?;
    let mut key_memory = PrefixTicket::new(0)?;
    let key = match levels {
        None => None,
        Some(levels) => {
            let mut out = prefix_vec(levels.len(), &mut key_memory)?;
            for level in levels {
                out.push(prefix_level(level, &mut key_memory)?);
            }
            Some(out)
        }
    };
    let (body, body_memory) = prefix_copy(body)?;
    PREFIX_TABLE.with(|state| {
        let mut table = state.borrow_mut();
        if !active(&table, ctx) {
            return None;
        }
        let touched = table.definitions.touch();
        table.definitions.entries[slot] = Some(Entry {
            source,
            levels: key,
            body,
            metrics,
            key_metrics,
            touched,
            _key_memory: key_memory,
            _body_memory: body_memory,
        });
        Some(())
    })
}

/// Resolve headers outside this helper and lower only from an empty local scope.
/// No failed lowering, inferred type, or checked equality is stored here.
pub(super) fn lower(
    ctx: &Context,
    original: &Term,
    levels: Option<&[Universe]>,
    lower: impl FnOnce() -> Option<Db>,
) -> Option<Db> {
    let source = original as *const Term as usize;
    let admit = match lookup(ctx, source, levels) {
        Lookup::Hit(body) => return Some(body),
        Lookup::Miss => true,
        Lookup::Bypass => false,
    };
    let body = lower()?;
    if admit {
        let _ = remember(ctx, source, levels, &body);
    }
    Some(body)
}
