//! The page world: everything a page's run reads and writes besides the program's own values — the
//! document, the journal of the turns it took, the page clock, the sealed test world, and the
//! browser's storage.
//!
//! Each lives in a thread-local of its own module; this is where they are handled as ONE world. A
//! program run starts from a fresh one ([`begin_run`]); a session carried onto a thread of its own
//! takes its caller's world with it and hands it back ([`PageWorld`]); a comparison of tiers enters
//! a fresh one per tier and restores the caller's afterwards.

use std::cell::{Cell, RefCell};

/// One entry of the journal: a turn that ran, the instance it ran against, what it changed, and —
/// when it trapped — the failure that rolled it back.
#[derive(Clone, Debug)]
pub struct JournalEntry {
    pub instance: String,
    pub action: String,
    /// `(field, before, after)` for every field the turn changed.
    pub changes: Vec<(String, String, String)>,
    /// The failure that rolled the turn back, if it did not commit.
    pub failed: Option<String>,
}

thread_local! {
    static JOURNAL: RefCell<Vec<JournalEntry>> = const { RefCell::new(Vec::new()) };
    /// Whether the NEXT program run is a hot reload rather than a fresh start ([`arm_hot_reload`]).
    static HOT_RELOAD: Cell<bool> = const { Cell::new(false) };
}

/// Record a turn.
pub fn journal_push(entry: JournalEntry) {
    JOURNAL.with(|j| j.borrow_mut().push(entry));
}

/// Every turn recorded since the world began, in order.
pub fn journal() -> Vec<JournalEntry> {
    JOURNAL.with(|j| j.borrow().clone())
}

/// A FRESH WORLD, the document aside (its owner resets it: [`crate::semantics::dom::reset`]): an
/// empty journal, and a browser opened for the first time — storage is what a page left on a
/// visitor's machine, and a run that inherited a previous run's would make every test depend on
/// the order the tests happened to run in.
pub fn reset() {
    JOURNAL.with(|j| j.borrow_mut().clear());
    HOT_RELOAD.with(|f| f.set(false));
    crate::semantics::storage::reset();
}

/// What a program run starts from: [`reset`] — a run of a program is a fresh process — unless a hot
/// reload was armed ([`arm_hot_reload`]): then the journal starts again and the browser's storage,
/// where preserved state lives, is kept.
pub fn begin_run() {
    if HOT_RELOAD.with(|f| f.replace(false)) {
        JOURNAL.with(|j| j.borrow_mut().clear());
    } else {
        reset();
    }
}

/// A HOT RELOAD: `largo serve` rebuilds a page when its file changes and runs it again, and the
/// run it arms keeps the browser's storage — the form half filled in stays filled in — where a cold
/// start would begin with nothing stored. Armed by whoever performs the reload, consumed by the
/// next run.
pub fn arm_hot_reload() {
    HOT_RELOAD.with(|f| f.set(true));
}

/// A page world, taken out of the thread that held it: what [`PageWorld::take`] leaves a fresh one
/// in place of and [`PageWorld::restore`] puts back.
#[derive(Default)]
pub struct PageWorld {
    dom: crate::semantics::dom::DomState,
    journal: Vec<JournalEntry>,
    hot_reload: bool,
    clock: crate::semantics::drive::ClockState,
    testworld: Option<logicaffeine_system::testworld::TestWorld>,
    storage: crate::semantics::storage::StorageState,
}

impl PageWorld {
    /// This thread's world, leaving a fresh one in its place.
    pub fn take() -> Self {
        PageWorld {
            dom: crate::semantics::dom::take(),
            journal: JOURNAL.with(RefCell::take),
            hot_reload: HOT_RELOAD.with(Cell::take),
            clock: crate::semantics::drive::take(),
            testworld: logicaffeine_system::testworld::take(),
            storage: crate::semantics::storage::take(),
        }
    }

    /// Install this world as this thread's.
    pub fn restore(self) {
        crate::semantics::dom::restore(self.dom);
        JOURNAL.with(|j| *j.borrow_mut() = self.journal);
        HOT_RELOAD.with(|f| f.set(self.hot_reload));
        crate::semantics::drive::restore(self.clock);
        logicaffeine_system::testworld::restore(self.testworld);
        crate::semantics::storage::restore(self.storage);
    }
}
