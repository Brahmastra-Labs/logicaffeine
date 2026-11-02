//! The core beside a [`Context`]: a [`Kernel::logos`] per thread, kept in step with the context it
//! last mirrored through that context's admission log.
//!
//! A `Context` is persistent and shared between threads; the core is a mutable, single-threaded
//! kernel. So the core is not a field of the context but a mirror of one context lineage per
//! thread, and [`sync`] brings it to a context:
//!
//! - the context's log head is the mirror's: nothing changed;
//! - the log reaches the mirror's head: the entries since name what was registered, and those
//!   declarations are admitted in dependency order, unless the entries change something the mirror
//!   holds (a held name removed or registered again, a constructor added to a held family, a name a
//!   refused declaration referenced, a change to how `Int` literals read), which is a branch. The
//!   quotient needs no rule of its own: its certification logs all four primitives, so they arrive
//!   together and are installed whole, or a held one is registered again;
//! - the log reaches the standard library's head instead: the mirror restarts from a snapshot of
//!   the standard library's core, kept per thread, and advances from there;
//! - anything else, a branch included: the context is replayed whole ([`logos_env::replay`]).
//!
//! Every route ends where a replay of the context ends: the same declarations admitted, with the
//! same verdicts.
//!
//! A thread's mirror lives on a core thread of its own: an admission recurses as deep as the
//! declaration it checks, deeper than a caller's stack may reach, and the core is single-threaded
//! state that never moves between threads. [`sync`] and [`with`] run there and answer the caller.

use super::logos_env::{self, Declaration, Replayed};
use super::tc::Kernel;
use crate::context::{Context, LogEntry};
use crate::prelude::StandardLibrary;
use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;

/// How [`sync`] brought the mirror to a context.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    /// The mirror already mirrored the context.
    Current,
    /// The context extends the mirrored one; only its new declarations were admitted.
    Advanced,
    /// The context extends the standard library on another branch; the mirror restarted from the
    /// standard library's snapshot and admitted the rest.
    Rebased,
    /// The context was replayed whole.
    Replayed,
}

/// What one [`sync`] did: its route, and the core's verdict on each declaration it admitted.
#[derive(Debug)]
pub struct Synced {
    pub route: Route,
    pub verdicts: Vec<Replayed>,
    /// The refused among `verdicts` that reference a declaration the core had already refused: the
    /// refusal follows from that one's.
    pub downstream: BTreeSet<String>,
}

#[derive(Clone)]
struct Mirror {
    kernel: Kernel,
    /// The head of the mirrored context's log.
    head: Option<Arc<LogEntry>>,
    /// Every name the kernel's declarations declare, refused ones included.
    held: HashSet<String>,
    /// The names refused declarations reference that the mirror does not hold: a replay would
    /// decide those declarations again once these are registered.
    awaited: HashSet<String>,
    /// Every name of a declaration the core refused.
    refused: HashSet<String>,
}

thread_local! {
    /// On a core thread: its mirror.
    static MIRROR: RefCell<Option<Mirror>> = const { RefCell::new(None) };
    /// On a core thread: the standard library's mirror, built on first use and never advanced.
    static BASE: RefCell<Option<Mirror>> = const { RefCell::new(None) };
    /// On a calling thread: the core thread that holds its mirror, started on first use.
    static CORE: RefCell<Option<CoreThread>> = const { RefCell::new(None) };
    /// Whether this thread is a core thread.
    static ON_CORE: Cell<bool> = const { Cell::new(false) };
}

/// The stack a core thread runs on, the one the core's own suites check on.
const CORE_STACK: usize = 1 << 30;

/// A calling thread's core thread. It ends when the calling thread does: dropping the queue ends
/// its loop.
struct CoreThread {
    queue: std::sync::mpsc::Sender<Box<dyn FnOnce() + Send>>,
}

fn start_core() -> CoreThread {
    let (queue, jobs) = std::sync::mpsc::channel::<Box<dyn FnOnce() + Send>>();
    std::thread::Builder::new()
        .name("logos-core-mirror".to_string())
        .stack_size(CORE_STACK)
        .spawn(move || {
            ON_CORE.with(|on| on.set(true));
            for job in jobs {
                job();
            }
        })
        .expect("a core thread starts");
    CoreThread { queue }
}

/// Run `job` on this thread's core thread and return its answer; a panic in `job` is the caller's.
/// On a core thread itself, `job` runs in place.
fn on_core<R: Send + 'static>(job: impl FnOnce() -> R + Send + 'static) -> R {
    if ON_CORE.with(Cell::get) {
        return job();
    }
    let (answer, answered) = std::sync::mpsc::channel();
    let job: Box<dyn FnOnce() + Send> = Box::new(move || {
        let _ = answer.send(std::panic::catch_unwind(std::panic::AssertUnwindSafe(job)));
    });
    CORE.with(|cell| {
        cell.borrow_mut()
            .get_or_insert_with(start_core)
            .queue
            .send(job)
            .expect("the core thread runs")
    });
    match answered.recv().expect("the core thread answers every job") {
        Ok(result) => result,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

/// Bring this thread's mirror to `ctx`.
pub fn sync(ctx: &Context) -> Synced {
    let ctx = ctx.clone();
    on_core(move || {
        let (mirror, synced) = advance(MIRROR.with(|cell| cell.borrow_mut().take()), &ctx);
        MIRROR.with(|cell| *cell.borrow_mut() = Some(mirror));
        synced
    })
}

/// Bring this thread's mirror to `ctx` and run `f` on its kernel. The mirror is out of its slot
/// while `f` runs, so a sync inside `f` builds its own and this one is put back after.
pub fn with<R: Send + 'static>(
    ctx: &Context,
    f: impl FnOnce(&mut Kernel) -> R + Send + 'static,
) -> (Synced, R) {
    let ctx = ctx.clone();
    on_core(move || {
        let (mut mirror, synced) = advance(MIRROR.with(|cell| cell.borrow_mut().take()), &ctx);
        let result = f(&mut mirror.kernel);
        MIRROR.with(|cell| *cell.borrow_mut() = Some(mirror));
        (synced, result)
    })
}

fn same(a: Option<&Arc<LogEntry>>, b: Option<&Arc<LogEntry>>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        (None, None) => true,
        _ => false,
    }
}

/// Walk back from `at`, pushing each entry passed onto `suffix` (newest first), until `stop` holds
/// of the position reached: that position, or `None` when the log ended first.
fn walk<'a>(
    mut at: Option<&'a Arc<LogEntry>>,
    suffix: &mut Vec<&'a LogEntry>,
    stop: impl Fn(Option<&Arc<LogEntry>>) -> bool,
) -> Option<Option<&'a Arc<LogEntry>>> {
    loop {
        if stop(at) {
            return Some(at);
        }
        let entry = at?;
        suffix.push(entry);
        at = entry.prev.as_ref();
    }
}

fn advance(mirror: Option<Mirror>, ctx: &Context) -> (Mirror, Synced) {
    let base_head = StandardLibrary::registered().map(Context::admissions);
    let at_base = |at: Option<&Arc<LogEntry>>| base_head.is_some_and(|h| same(at, h));
    let mut suffix: Vec<&LogEntry> = Vec::new();
    let mut at = ctx.admissions();
    if let Some(mut mirror) = mirror {
        let Some(stopped) = walk(at, &mut suffix, |at| {
            same(at, mirror.head.as_ref()) || at_base(at)
        }) else {
            return replay(ctx);
        };
        if same(stopped, mirror.head.as_ref()) {
            if suffix.is_empty() {
                mirror.kernel.follow(ctx);
                return (
                    mirror,
                    Synced {
                        route: Route::Current,
                        verdicts: Vec::new(),
                        downstream: BTreeSet::new(),
                    },
                );
            }
            if let Ok((mirror, verdicts, downstream)) = extend(mirror, ctx, &suffix) {
                return (
                    mirror,
                    Synced {
                        route: Route::Advanced,
                        verdicts,
                        downstream,
                    },
                );
            }
        }
        at = stopped;
    }
    if walk(at, &mut suffix, at_base).is_none() {
        return replay(ctx);
    }
    let (base, built) = base();
    let route = if built.is_some() {
        Route::Replayed
    } else {
        Route::Rebased
    };
    match extend(base, ctx, &suffix) {
        Ok((mirror, more, more_downstream)) => {
            let (mut verdicts, mut downstream) = built.unwrap_or_default();
            verdicts.extend(more);
            downstream.extend(more_downstream);
            (
                mirror,
                Synced {
                    route,
                    verdicts,
                    downstream,
                },
            )
        }
        Err(()) => replay(ctx),
    }
}

/// The standard library's mirror, and its verdicts when this call built it.
fn base() -> (Mirror, Option<(Vec<Replayed>, BTreeSet<String>)>) {
    BASE.with(|cell| {
        let mut slot = cell.borrow_mut();
        if let Some(base) = slot.as_ref() {
            return (base.clone(), None);
        }
        let ctx = StandardLibrary::registered().expect("a context met the standard library's log");
        let (base, synced) = replay(ctx);
        *slot = Some(base.clone());
        (base, Some((synced.verdicts, synced.downstream)))
    })
}

fn replay(ctx: &Context) -> (Mirror, Synced) {
    let (kernel, items, verdicts) = logos_env::replay_items(ctx);
    let mut mirror = Mirror {
        kernel,
        head: ctx.admissions().cloned(),
        held: HashSet::new(),
        awaited: HashSet::new(),
        refused: HashSet::new(),
    };
    let downstream = mirror.hold(&items, &verdicts);
    (
        mirror,
        Synced {
            route: Route::Replayed,
            verdicts,
            downstream,
        },
    )
}

/// Admit what `suffix` (newest first) registered on top of `mirror`; `Err` when the suffix is a
/// branch from it.
fn extend(
    mut mirror: Mirror,
    ctx: &Context,
    suffix: &[&LogEntry],
) -> Result<(Mirror, Vec<Replayed>, BTreeSet<String>), ()> {
    if ctx.canonical_numerals() != mirror.kernel.numerals() {
        return Err(());
    }
    let mut changed: BTreeSet<String> = BTreeSet::new();
    for entry in suffix.iter().rev() {
        if mirror.held.contains(&entry.name) || mirror.awaited.contains(&entry.name) {
            return Err(());
        }
        if entry.removed {
            continue;
        }
        let owner = ctx
            .constructor_inductive(&entry.name)
            .unwrap_or(&entry.name);
        if mirror.held.contains(owner) {
            return Err(());
        }
        changed.insert(owner.to_string());
    }
    let items = logos_env::items_named(ctx, &changed);
    mirror.kernel.follow(ctx);
    let verdicts = logos_env::admit_items(&mut mirror.kernel, ctx, &items);
    let downstream = mirror.hold(&items, &verdicts);
    mirror.head = ctx.admissions().cloned();
    Ok((mirror, verdicts, downstream))
}

impl Mirror {
    /// Record `items` (in admission order) as held, what the refused ones among them reference and
    /// the mirror lacks, and which refused ones reference a declaration refused before them: those
    /// are returned.
    fn hold(&mut self, items: &[Declaration], verdicts: &[Replayed]) -> BTreeSet<String> {
        for item in items {
            self.held
                .extend(item.names().into_iter().map(str::to_string));
        }
        let refused: HashSet<&str> = verdicts
            .iter()
            .filter(|v| v.verdict.is_err())
            .map(|v| v.name.as_str())
            .collect();
        let mut downstream = BTreeSet::new();
        for item in items {
            let names = item.names();
            if names.iter().any(|n| refused.contains(n)) {
                let globals = logos_env::item_globals(item);
                if globals
                    .iter()
                    .any(|g| self.refused.contains(g) && !names.contains(&g.as_str()))
                {
                    downstream.extend(names.iter().map(|n| n.to_string()));
                }
                self.refused.extend(names.iter().map(|n| n.to_string()));
                self.awaited
                    .extend(globals.into_iter().filter(|n| !self.held.contains(n)));
            }
        }
        self.awaited.retain(|n| !self.held.contains(n));
        downstream
    }
}

/// Shadow mode, before a door decides: the mirror is brought to the context as the door finds it, so
/// the sync after the door reports exactly what the door registered.
#[cfg(feature = "shadow")]
pub(crate) fn shadow_enter(ctx: &Context) -> Entry {
    if compares(ctx) {
        sync(ctx);
    }
    Entry(ctx.admissions().cloned())
}

/// The admission log's head as a door found it: what the door registers is what the log holds
/// past it.
#[cfg(feature = "shadow")]
pub(crate) struct Entry(Option<Arc<LogEntry>>);

/// The names registered in `ctx` since `entry`.
#[cfg(feature = "shadow")]
fn registered_since<'a>(ctx: &'a Context, entry: &Entry) -> HashSet<&'a str> {
    let mut suffix = Vec::new();
    walk(ctx.admissions(), &mut suffix, |at| {
        same(at, entry.0.as_ref())
    });
    suffix
        .into_iter()
        .filter(|e| !e.removed)
        .map(|e| e.name.as_str())
        .collect()
}

/// Whether the shadow compares the doors' verdicts at `ctx`: not while the standard library
/// registers, and not in a context with local bindings, where a declaration may be open in them and
/// the core, which admits closed global declarations, has no counterpart for it.
#[cfg(feature = "shadow")]
fn compares(ctx: &Context) -> bool {
    !crate::prelude::registering() && !ctx.has_local_bindings()
}

/// Where the core's verdicts on what a door registered differ from the context's: a declaration the
/// context holds is one the core must admit, and one it holds pending (its dependencies not yet
/// registered) one the core must refuse. A refusal `downstream` of one the core made before is that
/// one's: it was compared where that declaration was decided, or no checked door decided it.
#[cfg(feature = "shadow")]
fn admission_disagreements<'a>(
    verdicts: impl IntoIterator<Item = &'a Replayed>,
    pending: &HashSet<&str>,
    downstream: &BTreeSet<String>,
) -> Vec<String> {
    verdicts
        .into_iter()
        .filter_map(|v| match (&v.verdict, pending.contains(v.name.as_str())) {
            (Ok(()), false) | (Err(_), true) => None,
            (Err(_), false) if downstream.contains(&v.name) => None,
            (Err(e), false) => Some(format!(
                "`{}`: the Term kernel admits it, the core refuses it: {e}",
                v.name
            )),
            (Ok(()), true) => Some(format!(
                "`{}`: the Term kernel holds it pending, the core admits it",
                v.name
            )),
        })
        .collect()
}

/// Shadow mode, after a door decided a declaration: `entry` is the log as the door found it
/// (`shadow_enter`), `refused` the `Term` kernel's refusal of `candidate`, or `None` when the door
/// registered it. The core's verdict must be the same: on every
/// name the door registered, read by a sync (a name the door left pending is a refusal), or on the
/// refused candidate, judged in the unchanged context. Only a refusal of the theory is compared; a
/// door's own policy (a name already taken, a redeclaration others depend on) has no counterpart in
/// the core. Stands down where `compares` says the core has no counterpart.
#[cfg(feature = "shadow")]
pub(crate) fn shadow(
    ctx: &Context,
    entry: &Entry,
    refused: Option<&crate::error::KernelError>,
    candidate: impl FnOnce() -> Vec<Declaration>,
) {
    if !compares(ctx) {
        return;
    }
    match refused {
        None => {
            let synced = sync(ctx);
            let registered = registered_since(ctx, entry);
            let pending: HashSet<&str> = ctx
                .pending_obligations()
                .into_iter()
                .map(|(n, _)| n)
                .collect();
            let decided = synced
                .verdicts
                .iter()
                .filter(|v| registered.contains(v.name.as_str()));
            let disagreements = admission_disagreements(decided, &pending, &synced.downstream);
            if !disagreements.is_empty() {
                panic!(
                    "shadow mode: the kernels disagree ({:?}):\n{}",
                    synced.route,
                    disagreements.join("\n")
                );
            }
        }
        Some(error) if theory_refusal(error) => {
            let block = candidate();
            let names: Vec<String> = block
                .iter()
                .flat_map(|d| d.names().into_iter().map(str::to_string))
                .collect();
            let judged = ctx.clone();
            let (_, verdict) = with(ctx, move |k| logos_env::judge(k, &judged, &block));
            if verdict.is_ok() {
                panic!(
                    "shadow mode: the Term kernel refuses {}, the core admits it.\nThe Term kernel: {error}",
                    names.join(", ")
                );
            }
        }
        Some(_) => {}
    }
}

/// Whether `error` is a refusal by a rule of the theory, which the core must make too.
#[cfg(feature = "shadow")]
fn theory_refusal(error: &crate::error::KernelError) -> bool {
    use crate::error::KernelError::*;
    matches!(
        error,
        UnboundVariable(_)
            | NotAFunction(_)
            | NotAType(_)
            | TypeMismatch { .. }
            | NotAnInductive(_)
            | InvalidMotive(_)
            | WrongNumberOfCases { .. }
            | InvalidDeclaration(_)
            | TerminationViolation { .. }
            | PositivityViolation { .. }
    )
}

#[cfg(all(test, feature = "shadow"))]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_kernel/dag/mirror/shadow_controls.rs"]
mod shadow_controls;
