//! A page tier that disagrees with the reference ON PURPOSE, at the one statement a test names.
//!
//! A lock on the cross-tier check needs a divergence for the check to catch. A real one — a
//! miscompile, a ledgered gap — is a fixture that loses its precondition the day it is fixed, and
//! the lock goes red for being right. This tier cannot be fixed. It replays the reference tier's
//! own trace, snapshot for snapshot, except at `lost`, where it shows the page as it stood one
//! statement earlier: that statement's effect never happened. Every other statement agrees by
//! construction, so a check that works reports exactly `lost` and nothing else.
//!
//! It plugs in where any page in another process does — [`RemotePageLauncher`], run as the
//! `Browser` candidate — so the check under test is the production one, unmodified.

#![allow(dead_code)]

use logicaffeine_compile::page_host::{
    compare_tests, Arrival, DocSnapshot, Drive, DriveError, RemotePage, RemotePageLauncher,
};
use logicaffeine_compile::testrun::TestConfig;
use std::cell::RefCell;
use std::rc::Rc;

/// What the divergent tier was handed, so a test can tell the check actually ran it.
#[derive(Debug, Default)]
pub struct Seen {
    /// Every route a page was launched at, in launch order.
    pub launches: Vec<String>,
    /// Every driving statement handed over, in order.
    pub drives: Vec<Drive>,
    /// How many times the page was observed.
    pub snapshots: usize,
}

/// Launches a page that loses the effect of statement `lost` and agrees everywhere else.
pub struct LosesOneStatement {
    reference: Vec<DocSnapshot>,
    lost: usize,
    seen: Rc<RefCell<Seen>>,
}

impl LosesOneStatement {
    /// Record `source`'s one page test on the reference tier — in the world `run_tests` gives it
    /// under [`TestConfig::default`] — to be replayed with statement `lost`'s effect missing.
    ///
    /// Refuses a fixture that could not show the loss: `lost` must be a test statement (never `0`,
    /// the program itself) and must change the page's DOCUMENT, an axis every cross-tier check
    /// compares. A statement that changed nothing, or only the journal, would make the fault
    /// invisible and the lock a check that cannot fail.
    pub fn at(source: &str, lost: usize) -> LosesOneStatement {
        let budget = TestConfig::default().default_budget;
        let mut all = compare_tests(source, &[], 0, budget, None)
            .unwrap_or_else(|e| panic!("the fixture must parse:\n{source}\n=> {e}"));
        assert_eq!(
            all.len(),
            1,
            "the fixture declares exactly one page test:\n{source}"
        );
        let trace = all.remove(0).reference;
        assert_eq!(
            trace.stopped, None,
            "the reference tier must run the whole fixture:\n{source}"
        );
        let reference: Vec<DocSnapshot> = trace.steps.into_iter().map(|s| s.snapshot).collect();
        assert!(
            (1..reference.len()).contains(&lost),
            "statement {lost} is not a test statement of a {}-step trace",
            reference.len()
        );
        assert_ne!(
            reference[lost].document,
            reference[lost - 1].document,
            "statement {lost} must change the document, or losing its effect would be invisible"
        );
        LosesOneStatement {
            reference,
            lost,
            seen: Rc::new(RefCell::new(Seen::default())),
        }
    }

    /// What the check handed this tier, shared with every page it launched.
    pub fn seen(&self) -> Rc<RefCell<Seen>> {
        self.seen.clone()
    }
}

impl RemotePageLauncher for LosesOneStatement {
    fn launch(&self, arrival: &Arrival) -> Result<Box<dyn RemotePage>, String> {
        self.seen.borrow_mut().launches.push(arrival.route.clone());
        Ok(Box::new(Replaying {
            reference: self.reference.clone(),
            lost: self.lost,
            next: 0,
            seen: self.seen.clone(),
        }))
    }
}

struct Replaying {
    reference: Vec<DocSnapshot>,
    lost: usize,
    next: usize,
    seen: Rc<RefCell<Seen>>,
}

impl RemotePage for Replaying {
    fn drive(&mut self, d: &Drive) -> Result<(), DriveError> {
        self.seen.borrow_mut().drives.push(d.clone());
        Ok(())
    }

    fn unhandled_failure(&self) -> Option<String> {
        None
    }

    fn snapshot(&mut self) -> Result<DocSnapshot, String> {
        let k = self.next;
        self.next += 1;
        self.seen.borrow_mut().snapshots += 1;
        let shown = if k == self.lost { k - 1 } else { k };
        self.reference.get(shown).cloned().ok_or_else(|| {
            format!(
                "observed {} times, but the reference trace has {} steps",
                k + 1,
                self.reference.len()
            )
        })
    }
}
