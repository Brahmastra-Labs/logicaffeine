//! One driver, several page tiers.
//!
//! A `## Test` that works a page has two halves. The TEST — its `Expect`s, its budgets, its
//! sealed world — is the tree-walker's, always. The PAGE is whoever executes it: the page's lowered
//! LOGOS on the tree-walker itself (the reference), the same LOGOS on the bytecode VM (what a
//! browser runs, compiled to WebAssembly), or a real browser. What varies between tiers is only
//! who executes the page, so the statements that act on a page are carried out by ONE driver
//! ([`LoweredHost::drive`]) that does for each exactly what `runtime.js` does in a browser, over
//! whichever engine runs the page's lowered LOGOS:
//!
//! | statement | the page sees |
//! |---|---|
//! | `Mount C [at S]` | `__mount_C(parent)` |
//! | `Click`/`Submit`/`Press`/`Type … into`/`Choose`/`Hover` | each of the verb's events dispatched as a browser dispatches it, along [`crate::semantics::dom::dispatch_path`] — every stop through `__lgx_dispatch_node` — and what came of it worded by [`crate::semantics::drive`] |
//! | `Focus`/`Blur` | the DOM command, then the `focus`/`blur` it raises, dispatched the same way |
//! | `Raise "offline"` | `__lgx_raise` |
//! | `Navigate to U` | `pushState`, then the route handler the page registered |
//! | `Reload the page` | the module re-instantiated, its main run again |
//! | `Advance time by N` | every interval the page armed, fired as it comes due |
//! | `Given that M "u" answers …` | `dom::stub_fetch` |
//!
//! After every statement the page SETTLES: each request that has an answer is completed through
//! the closure the page handed over with it, until no completion sends one that has an answer.
//! Every call into the page goes through one guarded door; a failure is handed to
//! `__lgx_on_failure`, and one no handler absorbs is the page's to report.
//!
//! Page queries (`the text of "#x"`) read the recorder every in-process tier paints into, so a
//! test's assertions mean the same thing whichever tier ran the page. Two tiers are compared by
//! [`PageTrace`] — the page after every top-level statement — per [`Axis`].

mod ledger;
mod lowered;
mod scenario;
mod snapshot;
mod trace;

pub use ledger::{ledger_map, parse_ledger, stale_rows, LedgerRow, LedgerTier};
pub use lowered::{DriveError, LoweredCalls, LoweredHost, LoweredPage};
pub use scenario::{
    arrange, assemble_stable, auto_mounted_page_component, auto_mounted_page_path,
    boot_at_declared_route, classify, compare_tests, defs_auto_mount_a_page, drive_main,
    drives_the_page, is_scene, works_a_page, Arranged, Arrival, Classified, GivenAnswer, HereRun,
    PageTierKind, RemotePageLauncher, Scenario, TestTierComparison, TierRun, World,
};
pub use snapshot::{fetch_record, Axis, DocSnapshot};
pub use trace::{describe_difference, AxisDivergence, Divergence, PageTrace, Stop, TraceStep};

/// How many leading statements of `program` are its page's MAIN: everything before the first
/// statement that acts on the page ([`drives_the_page`]) — the lowered prelude, the program's
/// definitions, and the top-level bindings a page's `State:` may read. What follows is whoever
/// drives the page, the page's own mount among them. A reload runs exactly the main again.
pub fn page_main_len(
    program: &[crate::ast::stmt::Stmt],
    interner: &crate::intern::Interner,
) -> usize {
    program
        .iter()
        .position(|s| drives_the_page(s, interner))
        .unwrap_or(program.len())
}

/// The name every page tier's shell is built with: the browser bundle's module and host
/// (`/app.wasm`, `/app.js`) and the shell's default `<title>`. A page the host drives in process
/// arrives in the shell this names ([`World`]), so it arrives in the very document a browser loads.
pub const PAGE_STEM: &str = "app";

/// One statement that acts on a page, with its operands evaluated — the unit every page tier is
/// driven in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Drive {
    /// `Mount C [at "#sel"].`
    Mount {
        component: String,
        at: Option<String>,
    },
    /// `Click SEL.`
    Click { selector: String },
    /// `Submit SEL.`
    Submit { selector: String },
    /// `Press KEY [in SEL].` — the page itself (`body`) when no element is named.
    Press { key: String, selector: String },
    /// `Type TEXT into SEL.`
    Type { text: String, selector: String },
    /// `Choose VALUE in SEL.`
    Choose { value: String, selector: String },
    /// `Hover SEL.`
    Hover { selector: String },
    /// `Focus SEL.`
    Focus { selector: String },
    /// `Blur SEL.`
    Blur { selector: String },
    /// `Raise "offline".` — a window event, delivered to the page.
    Raise {
        event: String,
        target: String,
        value: String,
    },
    /// `Raise "filedropped" on SEL with FILE.` — `file` dropped on the element: its name, type and
    /// content travel with the drop into the handler it reaches.
    DropFile {
        target: String,
        file: crate::semantics::dom::DroppedFile,
    },
    /// `Navigate to URL.`
    Navigate { url: String },
    /// `Reload the page.`
    Reload,
    /// `Advance time by N.`, in milliseconds.
    AdvanceTime { ms: u64 },
    /// `Given that METHOD "url" answers STATUS with BODY.`
    AnswerFetch {
        method: String,
        url: String,
        status: i64,
        body: String,
    },
    /// `Serve fetches with routes.`
    ServeFetchesWithRoutes,
    /// `Replay these actions: a, b, c.` — re-run the mounted component's own actions, by name, in
    /// order, each a turn of its own.
    Replay { actions: Vec<String> },
}

impl Drive {
    /// The statement's own verb, as a test writes it.
    pub fn verb(&self) -> &'static str {
        match self {
            Drive::Mount { .. } => "Mount",
            Drive::Click { .. } => "Click",
            Drive::Submit { .. } => "Submit",
            Drive::Press { .. } => "Press",
            Drive::Type { .. } => "Type … into",
            Drive::Choose { .. } => "Choose",
            Drive::Hover { .. } => "Hover",
            Drive::Focus { .. } => "Focus",
            Drive::Blur { .. } => "Blur",
            Drive::Raise { .. } | Drive::DropFile { .. } => "Raise",
            Drive::Navigate { .. } => "Navigate to",
            Drive::Reload => "Reload the page",
            Drive::AdvanceTime { .. } => "Advance time by",
            Drive::AnswerFetch { .. } => "Given that … answers",
            Drive::ServeFetchesWithRoutes => "Serve fetches with routes",
            Drive::Replay { .. } => "Replay these actions",
        }
    }
}

/// A page the host drives one statement at a time and can observe between statements.
pub trait DrivenPage {
    fn drive(&mut self, d: &Drive) -> Result<(), DriveError>;
    /// The first action failure no handler absorbed, if any.
    fn unhandled_failure(&self) -> Option<String>;
    /// The page as it stands.
    fn snapshot(&mut self) -> Result<DocSnapshot, String>;
}

impl<L: LoweredCalls> DrivenPage for LoweredPage<L> {
    fn drive(&mut self, d: &Drive) -> Result<(), DriveError> {
        LoweredPage::drive(self, d)
    }
    fn unhandled_failure(&self) -> Option<String> {
        LoweredPage::unhandled_failure(self).map(str::to_string)
    }
    fn snapshot(&mut self) -> Result<DocSnapshot, String> {
        Ok(DocSnapshot::capture(
            LoweredPage::unhandled_failure(self).map(str::to_string),
        ))
    }
}

/// A page executed somewhere other than this process — a real browser, driven over its own
/// transport. Whoever implements it starts the page (serving its bundle, loading it) and then
/// answers the same two questions every tier answers: carry out a statement, show the page.
pub trait RemotePage {
    fn drive(&mut self, d: &Drive) -> Result<(), DriveError>;
    fn unhandled_failure(&self) -> Option<String>;
    fn snapshot(&mut self) -> Result<DocSnapshot, String>;
}

impl DrivenPage for Box<dyn RemotePage + '_> {
    fn drive(&mut self, d: &Drive) -> Result<(), DriveError> {
        (**self).drive(d)
    }
    fn unhandled_failure(&self) -> Option<String> {
        (**self).unhandled_failure()
    }
    fn snapshot(&mut self) -> Result<DocSnapshot, String> {
        (**self).snapshot()
    }
}

/// Who executes the page while a scenario runs.
pub enum PageTier<'p> {
    /// The page's lowered LOGOS on the bytecode VM, driven through its entries.
    LoweredVm(LoweredPage<crate::vm::session::Session<'p>>),
    /// A page in another process, driven over its transport.
    Browser(Box<dyn RemotePage + 'p>),
}

impl<'p> PageTier<'p> {
    /// The page the host drives.
    pub fn driven(&mut self) -> &mut dyn DrivenPage {
        match self {
            PageTier::LoweredVm(page) => page,
            PageTier::Browser(page) => page,
        }
    }

    /// Which tier this is.
    pub fn kind(&self) -> PageTierKind {
        match self {
            PageTier::LoweredVm(_) => PageTierKind::LoweredVm,
            PageTier::Browser(_) => PageTierKind::Browser,
        }
    }
}
