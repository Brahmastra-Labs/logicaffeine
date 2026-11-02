//! Running one scenario — a program's own statements, then a `## Test` body — on a page tier,
//! and holding the tiers' runs side by side.
//!
//! The reference trace is the tree-walker's: for every `k`, the program and the body's first `k`
//! statements run in a fresh interpreter, a fresh sealed world and a fresh document, and the page
//! is read afterwards. Runs are deterministic, so this is exactly the page a single run shows
//! after its `k`-th statement.
//!
//! A driven tier (the lowered page on the VM, a browser) is walked statement by statement: a
//! statement that acts on the page is carried out by the driver, and one that only reads it has
//! nothing to do on that tier — the test's assertions are the reference tier's. What the driven
//! walk cannot carry it REFUSES, naming the statement and why, rather than guessing: a statement
//! whose operand is computed (evaluating it is the interpreter's job), a driving statement nested
//! in a block, a call that may write the page.

use super::lowered::{DriveError, LoweredPage};
use super::snapshot::DocSnapshot;
use super::trace::{Divergence, PageTrace, Stop, TraceStep};
use super::{Drive, DrivenPage, PageTier, RemotePage};
use crate::analysis::{PolicyRegistry, TypeRegistry};
use crate::ast::stmt::{Expr, Literal, Stmt, UiStmt};
use crate::intern::Interner;
use crate::semantics::dom;
use crate::semantics::world::PageWorld;
use serde::Serialize;
use std::path::Path;
use std::time::Duration;

/// Which tier executes a scenario's page — the configuration-side name of a [`super::PageTier`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PageTierKind {
    /// The page's lowered LOGOS on the tree-walker — the same interpreter that runs the test. The
    /// reference every other tier is held to.
    LoweredHere,
    /// The page's lowered LOGOS on the bytecode VM.
    LoweredVm,
    /// A real browser, through a [`RemotePageLauncher`].
    Browser,
}

impl PageTierKind {
    pub fn label(self) -> &'static str {
        match self {
            PageTierKind::LoweredHere => "lowered tree-walker",
            PageTierKind::LoweredVm => "lowered VM",
            PageTierKind::Browser => "browser",
        }
    }
}

/// Starts a page in another process for one scenario.
pub trait RemotePageLauncher {
    /// Start the page `arrival` describes, fresh — visited at its route, its load-time answers
    /// already in place, its main run — ready to be driven.
    fn launch(&self, arrival: &Arrival) -> Result<Box<dyn RemotePage>, String>;
}

/// How a scenario's page arrives: everything that happens before statement 0 is observed.
///
/// A visitor lands on a page at a URL, in a world that already answers the requests the page makes
/// as it loads; the page's own `main` runs and a `## Page` mounts itself. Every tier arrives this
/// way — the in-process tiers by driving the arranged body's arrival ([`Arranged::arrival`]), a
/// remote page by being handed this and loading.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Arrival<'s> {
    /// The program the page ships (its `## Main`, if it had one, already stripped).
    pub source: &'s str,
    /// The URL the page is visited at: the auto-mounted `## Page`'s declared path, else `/`.
    pub route: String,
    /// The test's `Given that … answers …` statements that come before the page loads, in order.
    pub answers: Vec<GivenAnswer>,
    /// Every component the test mounts at a statement of its own — what the page's host must be
    /// able to mount on demand, read from the test's own `Mount` statements.
    pub mounts: Vec<String>,
}

/// `Given that METHOD "url" answers STATUS with BODY.`
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GivenAnswer {
    pub method: String,
    pub url: String,
    pub status: i64,
    pub body: String,
}

/// The sealed world a scenario runs in.
#[derive(Clone, Copy, Debug)]
pub struct World {
    pub caps: logicaffeine_system::testworld::Capabilities,
    pub seed: u64,
    /// The deterministic step budget for each tree-walker run.
    pub budget: u64,
}

impl World {
    /// The world a test runs in: its entropy is keyed by the test's identity, so adding or
    /// reordering tests never shifts another test's stream.
    pub fn for_test(
        name: &str,
        suite_seed: u64,
        caps: logicaffeine_system::testworld::Capabilities,
        budget: u64,
    ) -> World {
        let mut h: u64 = 0xCBF2_9CE4_8422_2325 ^ suite_seed;
        for b in name.bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0000_0100_0000_01B3);
        }
        World {
            caps,
            seed: h,
            budget,
        }
    }

    /// The property-generation seed derived from the world's.
    pub fn gen_seed(&self) -> u64 {
        self.seed ^ 0x5DEE_CE66_1234_ABCD
    }

    /// Enter the world: a fresh document, a fresh page runtime, fresh storage, no network, and
    /// the sealed clock/entropy/filesystem — SCOPED, not destructive: whatever the caller's own
    /// document/runtime/clock/sealed world held a moment ago is saved in the returned guard and
    /// comes back the instant it drops. A candidate page tier paints into the very same
    /// thread-locals the caller's own page just finished painting into (that sharing is what lets
    /// a page query mean the same thing on every tier), so entering a nested world without saving
    /// what it is about to overwrite would hand the caller back the candidate's leftovers instead
    /// of its own.
    ///
    /// The fresh document is the one a browser loads the page in: the shell [`super::PAGE_STEM`]
    /// names, served by the same function the browser tier's bundle is built from
    /// ([`crate::web_host::browser_host_html`]), so every tier arrives at the same head and title.
    /// `route`, when the scenario auto-mounts a `## Page`, is that page's own declared URL — the
    /// fresh document boots already AT that path (`dom::init_route`), exactly as a real visitor
    /// arrives at it, rather than at the boot default `"/"`.
    fn enter(&self, route: Option<&str>) -> WorldGuard {
        let guard = WorldGuard(Some(PageWorld::take()));
        crate::concurrency::net_inbox::set_net_offline(true);
        logicaffeine_system::testworld::install(logicaffeine_system::testworld::TestWorld::new(
            self.caps, self.seed,
        ));
        dom::arrive(&crate::web_host::browser_host_html(super::PAGE_STEM));
        if let Some(path) = route {
            dom::init_route(path, "");
        }
        guard
    }
}

/// The caller's own page world, saved by [`World::enter`] and put back when this drops.
struct WorldGuard(Option<PageWorld>);

impl Drop for WorldGuard {
    fn drop(&mut self) {
        if let Some(world) = self.0.take() {
            world.restore();
        }
    }
}

/// The lowered page's run on the tree-walker — the reference — once: the trace to where the program
/// stopped, and the statement it could not carry and why, when that is where it stopped.
#[derive(Clone, Debug)]
pub struct HereRun {
    pub trace: PageTrace,
    pub refused: Option<(usize, String)>,
}

impl HereRun {
    /// The run as a candidate tier's: refused at the statement it could not carry, traced otherwise.
    pub fn tier_run(&self) -> TierRun {
        match &self.refused {
            Some((statement, reason)) => TierRun::Refused {
                statement: *statement,
                reason: reason.clone(),
            },
            None => TierRun::Traced(self.trace.clone()),
        }
    }
}

/// A tier's run of a scenario.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TierRun {
    Traced(PageTrace),
    /// The tier cannot carry the scenario; `statement` is the one it cannot carry (`0` for the
    /// program itself).
    Refused {
        statement: usize,
        reason: String,
    },
}

/// What one top-level test statement is, to a tier that drives the page from outside.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Classified {
    /// It acts on the page.
    Drive(Drive),
    /// It only reads the page (or nothing at all).
    Read,
    /// A driven tier cannot carry it; the reason says why.
    Unsupported(String),
}

/// A test's statements in the order they run.
///
/// A `## Page` mounts itself, and that mount belongs to the program — which would run it before any
/// test statement, so a test could never arrange what the page finds when it loads. A browser test
/// sets the scene and THEN opens the page; so does this: the test's leading scene statements
/// ([`is_scene`]) run first, then the page's own mount. `Reload the page.` is the page loading
/// again, its own mount included — every tier's reload mounts the page again itself, so nothing is
/// placed after one. A program that mounts nothing of its own — a `## Component` a test mounts for
/// itself — runs exactly as written.
#[derive(Clone, Debug)]
pub struct Arranged<'a> {
    /// The program without the page's own mount.
    pub defs: Vec<Stmt<'a>>,
    /// The test's statements with the page's mount placed where it runs.
    pub body: Vec<Stmt<'a>>,
    /// For each statement of `body`, the authored test statement it is (1-based), or `None` for
    /// the page's own mount the runner placed. A placed mount belongs to the step of the
    /// statement before it (step 0 when it comes first).
    pub authored: Vec<Option<usize>>,
    /// How many statements at the head of `body` are the page's ARRIVAL: the leading scene and the
    /// page's own mount after it. A visitor never sees the page before it has loaded, so every one
    /// of them belongs to step 0 — the page as it arrives — and a step they carry an authored number
    /// for shows that same arrived page. `0` when the program mounts no page of its own.
    pub arrival: usize,
}

/// Whether `stmt` arranges the world a page meets rather than acting on the page.
pub fn is_scene(stmt: &Stmt) -> bool {
    matches!(
        stmt,
        Stmt::GivenFetch { .. } | Stmt::ServeFetches | Stmt::GivenFile { .. }
    )
}

/// The names of every `## Page`-kind component `defs` declares — the parser gives each one an
/// auto-inserted `Mount <page>.` in `defs` itself (a page "mounts itself"), which this recognizes
/// by name rather than by a test author having written the word `Mount` anywhere.
fn page_component_names(defs: &[Stmt]) -> Vec<logicaffeine_base::Symbol> {
    defs.iter()
        .filter_map(|s| match s {
            Stmt::ComponentDef(d)
                if matches!(d.kind, crate::ast::component::ComponentKind::Page { .. }) =>
            {
                Some(d.name)
            }
            _ => None,
        })
        .collect()
}

/// Whether `defs` auto-mounts a `## Page` — the parser's own inserted `Mount <page>.` [`arrange`]
/// relocates into the test body, not a statement the test's own body ever writes. A test against
/// such a program drives a mounted page exactly as much as an explicit `Mount` in its body does;
/// `largo test`'s cross-tier check (`testrun.rs`) uses this SAME signal for that reason, so the two
/// cannot drift on what "this test drives a mounted page" means.
pub fn defs_auto_mount_a_page(defs: &[Stmt]) -> bool {
    let pages = page_component_names(defs);
    defs.iter()
        .any(|s| matches!(s, Stmt::Ui(UiStmt::Mount { component, selector: None }) if pages.contains(component)))
}

/// The `## Page` component `defs` auto-mounts, and its own declared path — the parser's own
/// inserted `Mount <page>.` [`arrange`] relocates into the test body. `None` when `defs` auto-
/// mounts nothing (a `## Component` a test mounts for itself, if anything).
fn auto_mounted_page(
    defs: &[Stmt],
) -> Option<(logicaffeine_base::Symbol, logicaffeine_base::Symbol)> {
    let paths: std::collections::HashMap<logicaffeine_base::Symbol, logicaffeine_base::Symbol> =
        defs.iter()
            .filter_map(|s| match s {
                Stmt::ComponentDef(d) => match d.kind {
                    crate::ast::component::ComponentKind::Page { path, .. } => Some((d.name, path)),
                    _ => None,
                },
                _ => None,
            })
            .collect();
    defs.iter().find_map(|s| match s {
        Stmt::Ui(UiStmt::Mount {
            component,
            selector: None,
        }) => paths.get(component).map(|p| (*component, *p)),
        _ => None,
    })
}

/// The declared URL of the `## Page` `defs` auto-mounts, if any, resolved through `interner`.
/// `routePath()` must read this from the moment a page's own auto-mount runs, not the boot default
/// `"/"`: a page's own test, and every tier that mounts it, is that page AT its own URL, exactly as
/// a visitor who navigated there directly would find it.
pub fn auto_mounted_page_path(defs: &[Stmt], interner: &Interner) -> Option<String> {
    auto_mounted_page(defs).map(|(_, path)| interner.resolve(path).to_string())
}

/// Boot the document at the URL the `## Page` `defs` auto-mounts declares, so `routePath()`
/// answers that URL from the page's first statement — the page as a visitor who navigated to it
/// finds it. A program that auto-mounts no page leaves the route where it is.
pub fn boot_at_declared_route(defs: &[Stmt], interner: &Interner) {
    if let Some(path) = auto_mounted_page_path(defs, interner) {
        dom::init_route(&path, "");
    }
}

/// The name of the `## Page` component `defs` auto-mounts, if any, resolved through `interner` —
/// what a driven tier's `Replay these actions:` runs against before any explicit `Mount` in the
/// test body ever ran (a page mounts itself).
pub fn auto_mounted_page_component(defs: &[Stmt], interner: &Interner) -> Option<String> {
    auto_mounted_page(defs).map(|(component, _)| interner.resolve(component).to_string())
}

/// Arrange one test against its program. See [`Arranged`].
pub fn arrange<'a>(defs: &[Stmt<'a>], body: &[Stmt<'a>]) -> Arranged<'a> {
    let pages = page_component_names(defs);
    let is_page_mount = |s: &Stmt| matches!(s, Stmt::Ui(UiStmt::Mount { component, selector: None }) if pages.contains(component));
    let mounts: Vec<Stmt<'a>> = defs.iter().filter(|s| is_page_mount(s)).cloned().collect();
    let program: Vec<Stmt<'a>> = defs.iter().filter(|s| !is_page_mount(s)).cloned().collect();
    let mut arranged = Vec::with_capacity(body.len() + mounts.len());
    let mut authored = Vec::with_capacity(body.len() + mounts.len());
    let scene = body.iter().take_while(|s| is_scene(s)).count();
    for (i, stmt) in body.iter().enumerate() {
        if i == scene {
            arranged.extend(mounts.iter().cloned());
            authored.extend(mounts.iter().map(|_| None));
        }
        arranged.push(stmt.clone());
        authored.push(Some(i + 1));
    }
    if scene == body.len() {
        arranged.extend(mounts.iter().cloned());
        authored.extend(mounts.iter().map(|_| None));
    }
    let arrival = if mounts.is_empty() {
        0
    } else {
        scene + mounts.len()
    };
    Arranged {
        defs: program,
        body: arranged,
        authored,
        arrival,
    }
}

/// A program and a test body to run against it, arranged ([`arrange`]).
pub struct Scenario<'a> {
    pub interner: &'a Interner,
    pub types: &'a TypeRegistry,
    pub policies: &'a PolicyRegistry,
    pub arranged: Arranged<'a>,
    pub world: World,
}

impl<'a> Scenario<'a> {
    pub fn new(
        interner: &'a Interner,
        types: &'a TypeRegistry,
        policies: &'a PolicyRegistry,
        defs: &[Stmt<'a>],
        body: &[Stmt<'a>],
        world: World,
    ) -> Self {
        Scenario {
            interner,
            types,
            policies,
            arranged: arrange(defs, body),
            world,
        }
    }

    /// How many statements the test's author wrote.
    fn authored_len(&self) -> usize {
        self.arranged.authored.iter().flatten().count()
    }

    /// `arranged.defs` and `arranged.body` combined, in that order — `arrange` relocates a page's
    /// own `Mount` OUT of `defs` and into `body` (see [`Arranged`]), so anything hunting for it
    /// (a page's own auto-mount, its declared component or route) must search both, the way
    /// [`Scenario::reference_trace`]'s own interpreter run does by construction.
    fn full_program(&self) -> Vec<Stmt<'a>> {
        self.arranged
            .defs
            .iter()
            .chain(&self.arranged.body)
            .cloned()
            .collect()
    }

    /// This scenario's own auto-mounted page's declared URL, if it has one — see
    /// [`auto_mounted_page_path`].
    fn route(&self) -> Option<String> {
        auto_mounted_page_path(&self.full_program(), self.interner)
    }

    /// Where step `k`'s statements end in the arranged body: everything before the next authored
    /// statement, and never before the page has arrived.
    fn step_end(&self, k: usize) -> usize {
        let next = self
            .arranged
            .authored
            .iter()
            .position(|a| *a == Some(k + 1))
            .unwrap_or(self.arranged.body.len());
        next.max(self.arranged.arrival)
    }

    /// Whether a driven tier carries step `k` out itself: the program's own step always, and an
    /// authored statement when it drives the page (a statement that only reads has nothing to do).
    fn executes(&self, k: usize, steps: &[Classified]) -> bool {
        k == 0
            || self
                .arranged
                .authored
                .iter()
                .position(|a| *a == Some(k))
                .is_some_and(|i| matches!(steps[i], Classified::Drive(_)))
    }

    /// The reference: the lowered page on the tree-walker ([`Scenario::lowered_here_run`]), where a
    /// statement the page cannot carry is where the program stops.
    pub fn reference_trace(&self) -> PageTrace {
        self.lowered_here_run().trace
    }

    /// The run on the lowered page, on the tree-walker that runs the test: ONE interpreter runs
    /// the program as the page's main — its lowered prelude and definitions — and then the arranged
    /// body, one step at a time, every driving statement carried out on the page by the driver
    /// every lowered tier shares, with that same interpreter as its engine. The page is read after
    /// each step's statements, where `Scenario::step_end` ends it — the boundaries the reference
    /// trace runs to — in a single run.
    pub fn lowered_here_trace(&self) -> TierRun {
        self.lowered_here_run().tier_run()
    }

    /// The run on the lowered page on the tree-walker, once: its trace and, when it stopped on a
    /// statement the page cannot carry, that statement and why.
    pub fn lowered_here_run(&self) -> HereRun {
        crate::interpreter::on_session_stack(|| self.lowered_here_run_in_session())
    }

    /// [`Scenario::lowered_here_run`], on the session's own thread.
    fn lowered_here_run_in_session(&self) -> HereRun {
        let _guard = self.world.enter(self.route().as_deref());
        let mut interp = crate::interpreter::Interpreter::new(self.interner)
            .with_type_registry(self.types)
            .with_policies(self.policies.clone());
        if !self.world.caps.filesystem {
            interp = interp.with_vfs(std::sync::Arc::new(logicaffeine_system::fs::MemVfs::new()));
        }
        interp.enable_test_mode(self.world.budget);
        interp.set_gen_seed(self.world.gen_seed());
        let auto_mounted = auto_mounted_page_component(&self.full_program(), self.interner);
        let mut trace = PageTrace::default();
        match interp.host_lowered_page(&self.arranged.defs, auto_mounted) {
            Ok(()) => {}
            Err(DriveError::Refused(reason)) => {
                trace.stopped = Some(Stop {
                    statement: 0,
                    message: reason.clone(),
                });
                return HereRun {
                    trace,
                    refused: Some((0, reason)),
                };
            }
            Err(DriveError::Failed(message)) => {
                trace.steps.push(TraceStep {
                    statement: 0,
                    executed: true,
                    snapshot: DocSnapshot::capture(interp.hosted_page_failure()),
                });
                trace.stopped = Some(Stop {
                    statement: 0,
                    message,
                });
                return HereRun {
                    trace,
                    refused: None,
                };
            }
        }
        let mut from = 0;
        let mut returned = false;
        for k in 0..=self.authored_len() {
            let end = self.step_end(k);
            let step = &self.arranged.body[from..end.max(from)];
            from = end.max(from);
            let outcome = if returned {
                Ok(crate::interpreter::ControlFlow::Continue)
            } else if crate::interpreter::needs_async(step, self.interner) {
                futures::executor::block_on(interp.run_statements_async(step))
            } else {
                interp.run_statements(step)
            };
            let refusal = interp.hosted_page_refusal().map(|(d, why)| match d {
                Drive::Mount { component, .. } => self
                    .why_not_lowered(component)
                    .unwrap_or_else(|| why.to_string()),
                _ => why.to_string(),
            });
            trace.steps.push(TraceStep {
                statement: k,
                executed: true,
                snapshot: DocSnapshot::capture(interp.hosted_page_failure()),
            });
            if let Some(reason) = refusal {
                trace.stopped = Some(Stop {
                    statement: k,
                    message: reason.clone(),
                });
                return HereRun {
                    trace,
                    refused: Some((k, reason)),
                };
            }
            match outcome {
                Ok(crate::interpreter::ControlFlow::Continue) => {}
                Ok(_) => returned = true,
                Err(message) => {
                    trace.stopped = Some(Stop {
                        statement: k,
                        message,
                    });
                    break;
                }
            }
        }
        HereRun {
            trace,
            refused: None,
        }
    }

    /// The run on the lowered page, on the bytecode VM.
    pub fn lowered_vm_trace(&self) -> TierRun {
        let steps = match self.classify_body() {
            Ok(steps) => steps,
            Err(refused) => return refused,
        };
        let program = match crate::vm::Compiler::compile_with_types(
            &self.arranged.defs,
            self.interner,
            Some(self.types),
        ) {
            Ok(p) => p,
            Err(e) => {
                return TierRun::Refused {
                    statement: 0,
                    reason: format!("the program does not compile for the bytecode VM: {e}"),
                }
            }
        };
        let _guard = self.world.enter(self.route().as_deref());
        let vm = crate::vm::Vm::new(&program).with_policy_ctx(self.policies, self.interner);
        let mut page = LoweredPage::new(crate::vm::session::Session::new(vm, self.interner));
        let auto_mounted = auto_mounted_page_component(&self.full_program(), self.interner);
        let started = page.start(auto_mounted, |s| s.run_main());
        let run = self.walk(&mut PageTier::LoweredVm(page), started, &steps, 0);
        run
    }

    /// The run on a page another process executes, `source` being the program it ships.
    ///
    /// The page is launched already arrived ([`Arrival`]): at its own route, with the scene's
    /// answers in place, its `main` run — which performs a `## Page`'s own mount, exactly as a
    /// browser loading it does. The walk then starts after the arrival, and a `Mount` the test
    /// writes at a statement of its own is carried out at that statement, through the entry the
    /// arrival asked the page's host to expose.
    pub fn remote_trace(&self, launcher: &dyn RemotePageLauncher, source: &str) -> TierRun {
        let steps = match self.classify_body() {
            Ok(steps) => steps,
            Err(refused) => return refused,
        };
        let mut answers = Vec::new();
        for (i, step) in steps[..self.arranged.arrival].iter().enumerate() {
            match step {
                Classified::Drive(Drive::AnswerFetch {
                    method,
                    url,
                    status,
                    body,
                }) => answers.push(GivenAnswer {
                    method: method.clone(),
                    url: url.clone(),
                    status: *status,
                    body: body.clone(),
                }),
                Classified::Drive(Drive::Mount { .. }) if self.arranged.authored[i].is_none() => {}
                _ => {
                    return TierRun::Refused {
                        statement: self.owner(i),
                        reason: format!(
                            "{} cannot be arranged before a page in another process loads: only \
                             `Given that … answers …` travels with the page to where it runs",
                            stmt_label(&self.arranged.body[i])
                        ),
                    }
                }
            }
        }
        let mut mounts: Vec<String> = steps[self.arranged.arrival..]
            .iter()
            .filter_map(|s| match s {
                Classified::Drive(Drive::Mount { component, .. }) => Some(component.clone()),
                _ => None,
            })
            .collect();
        mounts.sort();
        mounts.dedup();
        let arrival = Arrival {
            source,
            route: self.route().unwrap_or_else(|| "/".to_string()),
            answers,
            mounts,
        };
        let _guard = self.world.enter(self.route().as_deref());
        match launcher.launch(&arrival) {
            Ok(page) => self.walk(
                &mut PageTier::Browser(page),
                Ok(()),
                &steps,
                self.arranged.arrival,
            ),
            Err(e) => TierRun::Refused {
                statement: 0,
                reason: format!("the page did not start: {e}"),
            },
        }
    }

    /// The step a statement of the arranged body belongs to: its own authored number, or the
    /// step of the authored statement before a placed mount.
    fn owner(&self, i: usize) -> usize {
        self.arranged.authored[..=i]
            .iter()
            .rev()
            .flatten()
            .next()
            .copied()
            .unwrap_or(0)
    }

    /// Classify every statement, or refuse at the first one a driven tier cannot carry — before
    /// anything runs, so a refusal never follows half a run.
    fn classify_body(&self) -> Result<Vec<Classified>, TierRun> {
        let mut steps = Vec::with_capacity(self.arranged.body.len());
        for (i, stmt) in self.arranged.body.iter().enumerate() {
            match classify(stmt, self.interner) {
                Classified::Unsupported(reason) => {
                    return Err(TierRun::Refused {
                        statement: self.owner(i),
                        reason,
                    })
                }
                c => steps.push(c),
            }
        }
        Ok(steps)
    }

    /// Walk a tier's page through the arranged body, one snapshot per step, each taken where
    /// [`Scenario::step_end`] ends it — the same boundaries the reference trace runs to. `from` is
    /// how much of the body the tier has already carried out by the time it is handed over (a
    /// remote page launched already arrived).
    fn walk(
        &self,
        tier: &mut PageTier<'_>,
        started: Result<(), DriveError>,
        steps: &[Classified],
        from: usize,
    ) -> TierRun {
        let page = tier.driven();
        let mut trace = PageTrace::default();
        let snap = |page: &mut dyn DrivenPage| {
            page.snapshot().unwrap_or_else(|e| DocSnapshot {
                document: format!("(the page could not be read: {e})"),
                ..DocSnapshot::default()
            })
        };
        match started {
            Ok(()) => {}
            Err(DriveError::Refused(reason)) => {
                return TierRun::Refused {
                    statement: 0,
                    reason,
                }
            }
            Err(DriveError::Failed(message)) => {
                trace.steps.push(TraceStep {
                    statement: 0,
                    executed: true,
                    snapshot: snap(page),
                });
                trace.stopped = Some(Stop {
                    statement: 0,
                    message,
                });
                return TierRun::Traced(trace);
            }
        }
        let mut i = from;
        for k in 0..=self.authored_len() {
            let executed = self.executes(k, steps);
            while i < self.step_end(k) {
                let step = &steps[i];
                let outcome = match step {
                    Classified::Drive(d) => page.drive(d),
                    Classified::Read => Ok(()),
                    Classified::Unsupported(reason) => {
                        return TierRun::Refused {
                            statement: self.owner(i),
                            reason: reason.clone(),
                        }
                    }
                };
                match outcome {
                    Ok(()) => {}
                    Err(DriveError::Refused(reason)) => {
                        let reason = match step {
                            Classified::Drive(Drive::Mount { component, .. }) => {
                                self.why_not_lowered(component).unwrap_or(reason)
                            }
                            _ => reason,
                        };
                        return TierRun::Refused {
                            statement: self.owner(i),
                            reason,
                        };
                    }
                    Err(DriveError::Failed(message)) => {
                        trace.steps.push(TraceStep {
                            statement: k,
                            executed,
                            snapshot: snap(page),
                        });
                        trace.stopped = Some(Stop {
                            statement: k,
                            message,
                        });
                        return TierRun::Traced(trace);
                    }
                }
                i += 1;
            }
            trace.steps.push(TraceStep {
                statement: k,
                executed,
                snapshot: snap(page),
            });
        }
        TierRun::Traced(trace)
    }

    /// Why the lowering left `component` without a mount entry, in the lowering's own words.
    fn why_not_lowered(&self, component: &str) -> Option<String> {
        why_not_lowered(&self.arranged.defs, self.types, self.interner, component)
    }
}

/// Why the lowering left `component` without a mount entry, in the lowering's own words — the
/// compiler emits `defs` as a whole rather than failing it outright, so a construct one component
/// cannot carry (a store shared across replicas, an event modifier with no lowered driver, …)
/// shows up as a MISSING export, not a compile error; this is the one place that turns that
/// absence back into the reason, for every caller that drives a lowered page from outside it
/// (`Scenario::walk`'s own [`Scenario::why_not_lowered`], and [`drive_main`]).
fn why_not_lowered(
    defs: &[Stmt],
    types: &TypeRegistry,
    interner: &Interner,
    component: &str,
) -> Option<String> {
    let component_defs: Vec<_> = defs
        .iter()
        .filter_map(|s| match s {
            Stmt::ComponentDef(d) => Some((**d).clone()),
            _ => None,
        })
        .collect();
    let def = component_defs
        .iter()
        .find(|d| interner.resolve(d.name) == component)?;
    match logicaffeine_language::parser::lgx_lower::lowerable(
        def,
        &component_defs,
        Some(types),
        interner,
    ) {
        Err(why) => Some(format!(
            "`Mount {component}` cannot run on this tier: {}",
            why.message
        )),
        Ok(()) => None,
    }
}

/// `## Main` driving a page — the plain, non-sealed twin of [`Scenario::lowered_vm_trace`]. There
/// is no test to seal a world for and no reference trace to hold against: `compile.rs::vm_outcome`
/// hands over a program's own definitions and its own driving script (its gate has already made
/// sure `drive_stmts` is nothing but driving statements), and this drives them exactly as a
/// `## Test` drives its candidate tier — one statement at a time, through the session's guarded
/// door, settling after each — so a trapping turn recovers here and a typed fetch completes here,
/// the same as they do under `## Test`.
pub fn drive_main<'a>(
    defs: &[Stmt<'a>],
    drive_stmts: &[Stmt<'a>],
    interner: &Interner,
    types: &TypeRegistry,
    policies: &PolicyRegistry,
) -> Result<String, String> {
    let program = crate::vm::Compiler::compile_with_types(defs, interner, Some(types))
        .map_err(|e| format!("the VM compiler cannot compile this program: {e}"))?;
    let vm = crate::vm::Vm::new(&program).with_policy_ctx(policies, interner);
    let mut page = LoweredPage::new(crate::vm::session::Session::new(vm, interner));
    let to_message = |e: DriveError| match e {
        DriveError::Failed(m) | DriveError::Refused(m) => m,
    };
    let auto_mounted = auto_mounted_page_component(&[defs, drive_stmts].concat(), interner);
    page.start(auto_mounted, |s| s.run_main())
        .map_err(to_message)?;
    for stmt in drive_stmts {
        let d = match classify(stmt, interner) {
            Classified::Drive(d) => d,
            _ => {
                return Err(format!(
                    "{} is not a driving statement — the gate that routed this program's `## \
                     Main` here is out of step with `classify`",
                    stmt_label(stmt)
                ))
            }
        };
        if let Err(e) = page.drive(&d) {
            // The compiler emits `defs` as a whole rather than failing it outright, so a
            // construct one component cannot carry shows up as a MISSING mount export, not a
            // compile error (see `why_not_lowered`) — the generic `DriveError::Refused` this
            // produces on its own names no more than that absence; ask the lowering directly for
            // the reason, exactly as `Scenario::walk` already does for a `## Test`'s Mount.
            let message = match (&d, e) {
                (Drive::Mount { component, .. }, DriveError::Refused(reason)) => {
                    why_not_lowered(defs, types, interner, component).unwrap_or(reason)
                }
                (_, e) => to_message(e),
            };
            return Err(message);
        }
    }
    // An action failure NO handler absorbed does not stop the script — the reference tree-walker
    // keeps running the rest of `## Main` past a trapping turn exactly the same way (a later click
    // still lands) — but it IS the program's own failure once nothing is left to run, the same way
    // an entirely unhandled failure is fatal to the tree-walker's own `RunOutcome` (confirmed
    // directly: `tw_outcome` on a click with no `When an action fails:` handler reports the trap as
    // its error even though the click after it still committed).
    match page.unhandled_failure() {
        Some(message) => Err(message.to_string()),
        None => Ok(page.output()),
    }
}

/// Whether `stmt` acts on a page — a statement the page host carries out ([`Drive`]), whatever its
/// operands — rather than one that reads the page or never touches it.
pub fn drives_the_page(stmt: &Stmt, interner: &Interner) -> bool {
    match stmt {
        Stmt::Ui(ui) => ui.role() != crate::ast::stmt::Role::Action,
        _ => to_drive(stmt, interner).is_some(),
    }
}

/// What `stmt` is to a tier that drives the page from outside.
pub fn classify(stmt: &Stmt, interner: &Interner) -> Classified {
    match to_drive(stmt, interner) {
        Some(Ok(d)) => Classified::Drive(d),
        Some(Err(why)) => Classified::Unsupported(why),
        None if reads_only_stmt(stmt) => Classified::Read,
        None => Classified::Unsupported(format!(
            "{} is carried out by the tree-walker running the test; a driven tier can carry only \
             driving statements and statements that read the page",
            stmt_label(stmt)
        )),
    }
}

/// The driving statement `stmt` is, with its operands, or `None` when it drives nothing.
fn to_drive(stmt: &Stmt, interner: &Interner) -> Option<Result<Drive, String>> {
    let t = |e: &Expr| operand_text(e, interner);
    Some(match stmt {
        Stmt::Ui(ui) => match ui {
            UiStmt::Mount {
                component,
                selector,
            } => (|| {
                Ok(Drive::Mount {
                    component: interner.resolve(*component).to_string(),
                    at: selector.map(t).transpose()?,
                })
            })(),
            UiStmt::Click { selector } => t(selector).map(|selector| Drive::Click { selector }),
            UiStmt::Submit { selector } => t(selector).map(|selector| Drive::Submit { selector }),
            UiStmt::Press { key, selector } => (|| {
                Ok(Drive::Press {
                    key: t(key)?,
                    selector: match selector {
                        Some(s) => t(s)?,
                        None => "body".to_string(),
                    },
                })
            })(),
            UiStmt::TypeInto { text, selector } => (|| {
                Ok(Drive::Type {
                    text: t(text)?,
                    selector: t(selector)?,
                })
            })(),
            UiStmt::Choose { value, selector } => (|| {
                Ok(Drive::Choose {
                    value: t(value)?,
                    selector: t(selector)?,
                })
            })(),
            UiStmt::Hover { selector } => t(selector).map(|selector| Drive::Hover { selector }),
            UiStmt::Focus { selector } => t(selector).map(|selector| Drive::Focus { selector }),
            UiStmt::Blur { selector } => t(selector).map(|selector| Drive::Blur { selector }),
            // `Raise "filedropped" on SEL with FILE.` drops a file: the file travels in the drive, so
            // each drop delivers its own file on whichever tier carries it out.
            UiStmt::RaiseWindowEvent {
                event,
                target,
                value,
            } => (|| {
                let event = t(event)?;
                let target = t(target)?;
                if event == "filedropped" {
                    return Ok(Drive::DropFile {
                        target,
                        file: dropped_file(value, interner)?,
                    });
                }
                Ok(Drive::Raise {
                    event,
                    target,
                    value: t(value)?,
                })
            })(),
            UiStmt::Navigate { url } => t(url).map(|url| Drive::Navigate { url }),
            UiStmt::Reload => Ok(Drive::Reload),
            other => Err(format!(
                "`{}` in a test is not a driving statement the page host maps",
                ui_label(other)
            )),
        },
        Stmt::AdvanceTime { millis } => match millis {
            Expr::Literal(Literal::Number(n)) => Ok(Drive::AdvanceTime {
                ms: (*n).max(0) as u64,
            }),
            Expr::Literal(Literal::Duration(ns)) => Ok(Drive::AdvanceTime {
                ms: ((*ns).max(0) / 1_000_000) as u64,
            }),
            other => Err(computed(other, interner)),
        },
        Stmt::GivenFetch {
            method,
            url,
            status,
            body,
        } => (|| {
            let status = match status {
                Expr::Literal(Literal::Number(n)) => *n,
                other => return Err(computed(other, interner)),
            };
            Ok(Drive::AnswerFetch {
                method: interner.resolve(*method).to_string(),
                url: t(url)?,
                status,
                body: t(body)?,
            })
        })(),
        Stmt::ServeFetches => Ok(Drive::ServeFetchesWithRoutes),
        Stmt::Replay { actions } => Ok(Drive::Replay {
            actions: actions
                .iter()
                .map(|a| interner.resolve(*a).to_string())
                .collect(),
        }),
        _ => return None,
    })
}

/// The file a drop's operand declares: `a new File with name … and size … and type … and bytes …`
/// with literal fields, read by [`dom::DroppedFile::declared`]'s rule, or a literal Text naming it.
fn dropped_file(value: &Expr, interner: &Interner) -> Result<dom::DroppedFile, String> {
    let Expr::New {
        type_name,
        init_fields,
        ..
    } = value
    else {
        return Ok(dom::DroppedFile::named(&operand_text(value, interner)?));
    };
    if interner.resolve(*type_name) != "File" {
        return Err(format!(
            "a dropped file is a `File` or its name as Text, and this is a new {}",
            interner.resolve(*type_name)
        ));
    }
    let field = |name: &str| {
        init_fields
            .iter()
            .find(|(f, _)| interner.resolve(*f) == name)
            .map(|(_, e)| *e)
    };
    let text = |name: &str| field(name).map(|e| operand_text(e, interner)).transpose();
    let size = match field("size") {
        Some(Expr::Literal(Literal::Number(n))) => Some(*n),
        Some(other) => return Err(computed(other, interner)),
        None => None,
    };
    let bytes = match field("bytes") {
        Some(Expr::List(items)) => Some(
            items
                .iter()
                .map(|item| match item {
                    Expr::Literal(Literal::Number(n)) => Ok(*n),
                    other => Err(computed(other, interner)),
                })
                .collect::<Result<Vec<i64>, String>>()?,
        ),
        Some(other) => return Err(computed(other, interner)),
        None => None,
    };
    dom::DroppedFile::declared(
        text("name")?.unwrap_or_default(),
        size,
        text("type")?,
        bytes,
    )
}

/// A driving statement's operand, when it is written as a literal.
fn operand_text(e: &Expr, interner: &Interner) -> Result<String, String> {
    match e {
        Expr::Literal(Literal::Text(s)) => Ok(interner.resolve(*s).to_string()),
        Expr::Literal(Literal::Number(n)) => Ok(n.to_string()),
        other => Err(computed(other, interner)),
    }
}

fn computed(e: &Expr, interner: &Interner) -> String {
    format!(
        "its operand `{}` is computed, and evaluating it is the tree-walker's job; a driven tier \
         takes literal operands until the interpreter hands it evaluated statements",
        logicaffeine_language::printer::render_expr(e, interner)
    )
}

/// Whether `stmt` only reads — the forms a test uses to look at a page, over expressions that
/// cannot write it.
fn reads_only_stmt(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Expect { condition } => reads_only(condition),
        Stmt::ExpectOutput { .. } => true,
        Stmt::Let { value, .. } | Stmt::Set { value, .. } => reads_only(value),
        Stmt::Show { object, recipient } => {
            reads_only(object) && matches!(recipient, Expr::Identifier(_))
        }
        Stmt::RuntimeAssert {
            condition, reason, ..
        } => reads_only(condition) && reason.map_or(true, reads_only),
        _ => false,
    }
}

/// Whether evaluating `e` can only read. An allowlist: a shape not named here is not claimed.
fn reads_only(e: &Expr) -> bool {
    match e {
        Expr::Literal(_) | Expr::Identifier(_) | Expr::OptionNone => true,
        Expr::BinaryOp { left, right, .. }
        | Expr::Index {
            collection: left,
            index: right,
        }
        | Expr::Contains {
            collection: left,
            value: right,
        }
        | Expr::Range {
            start: left,
            end: right,
        }
        | Expr::Union { left, right }
        | Expr::Intersection { left, right } => reads_only(left) && reads_only(right),
        Expr::Not { operand }
        | Expr::Length {
            collection: operand,
        }
        | Expr::FieldAccess {
            object: operand, ..
        }
        | Expr::OptionSome { value: operand }
        | Expr::Copy { expr: operand } => reads_only(operand),
        Expr::Slice {
            collection,
            start,
            end,
        } => reads_only(collection) && reads_only(start) && reads_only(end),
        Expr::List(items) | Expr::Tuple(items) => items.iter().all(|i| reads_only(i)),
        Expr::InterpolatedString(parts) => parts.iter().all(|p| match p {
            crate::ast::stmt::StringPart::Literal(_) => true,
            crate::ast::stmt::StringPart::Expr { value, .. } => reads_only(value),
        }),
        Expr::Dom(query) => query.selector().map_or(true, reads_only),
        _ => false,
    }
}

fn stmt_label(stmt: &Stmt) -> &'static str {
    match stmt {
        Stmt::If { .. } => "an `If`",
        Stmt::While { .. } => "a `While`",
        Stmt::Repeat { .. } => "a `Repeat`",
        Stmt::Call { .. } => "a `Call`",
        Stmt::ExpectFail { .. } => "an `Expect this to fail:` block",
        Stmt::ForAll { .. } => "a `For all` property",
        Stmt::Inspect { .. } => "an `Inspect`",
        Stmt::GivenFile { .. } => "`Given a file`",
        Stmt::Let { .. } => "a `Let` that calls a function",
        Stmt::Set { .. } => "a `Set` that calls a function",
        Stmt::Expect { .. } => "an `Expect` that calls a function",
        Stmt::Show { .. } => "a `Show` that calls a function",
        _ => "this statement",
    }
}

fn ui_label(ui: &UiStmt) -> &'static str {
    match ui {
        UiStmt::ScrollTo { .. } => "Scroll to",
        UiStmt::AddClass { .. } => "Add class",
        UiStmt::RemoveClass { .. } => "Remove class",
        UiStmt::SelectText { .. } => "Select the text in",
        UiStmt::CopyToClipboard { .. } => "Copy … to the clipboard",
        UiStmt::Download { .. } => "Download",
        UiStmt::ReplaceUrl { .. } => "Replace the URL with",
        UiStmt::GoBack => "Go back",
        UiStmt::SetTitle { .. } => "Set the title to",
        UiStmt::Save { .. } => "Save",
        UiStmt::Load { .. } => "Load",
        UiStmt::Forget { .. } => "Forget",
        UiStmt::Notify { .. } => "Notify",
        UiStmt::Publish { .. } => "Publish",
        UiStmt::Toggle { .. } => "Toggle",
        UiStmt::CallStore { .. } => "Call <Store>'s",
        UiStmt::StopTimer { .. } => "Stop the timer",
        UiStmt::StartTimer { .. } => "Start the timer",
        _ => "this verb",
    }
}

/// One `## Test`, run on the reference tier and on each candidate tier.
#[derive(Clone, Debug, Serialize)]
pub struct TestTierComparison {
    pub test: String,
    /// The page's lowered LOGOS on the tree-walker's run — the reference.
    pub reference: PageTrace,
    /// Every tier asked for, in the order asked — the reference tier among them answering with the
    /// reference's own run.
    pub candidates: Vec<(PageTierKind, TierRun)>,
}

impl TestTierComparison {
    /// How `kind`'s run differs from the reference; `None` when it did not run a trace.
    pub fn divergence(&self, kind: PageTierKind) -> Option<Divergence> {
        self.candidates
            .iter()
            .find(|(k, _)| *k == kind)
            .and_then(|(_, run)| match run {
                TierRun::Traced(t) => Some(self.reference.compare(t)),
                TierRun::Refused { .. } => None,
            })
    }
}

/// Parse `source`, and run every `## Test` in it that works a page on the reference tier and on
/// each of `candidates`. A test whose program mounts nothing is not a page scenario and is left
/// out. `## Main` is the program's entry point, never test fixture, and is stripped as the test
/// runner strips it.
pub fn compare_tests(
    source: &str,
    candidates: &[PageTierKind],
    suite_seed: u64,
    budget: u64,
    launcher: Option<&dyn RemotePageLauncher>,
) -> Result<Vec<TestTierComparison>, String> {
    let stripped = crate::analysis::assemble::strip_main_block(source);
    crate::ui_bridge::with_parsed_program(&stripped, |parsed, interner| {
        let (stmts, types, policies) = parsed?;
        let defs: Vec<Stmt> = stmts
            .iter()
            .filter(|s| !matches!(s, Stmt::TestDef { .. }))
            .cloned()
            .collect();
        let mut out = Vec::new();
        for s in stmts.iter() {
            let Stmt::TestDef {
                name,
                body,
                cases: None,
                skip: None,
                capabilities,
                budget: b,
            } = s
            else {
                continue;
            };
            if !works_a_page(&defs, body) {
                continue;
            }
            let caps = logicaffeine_system::testworld::Capabilities {
                filesystem: capabilities.filesystem,
                network: capabilities.network,
                environment: capabilities.environment,
                clock: capabilities.clock,
            };
            let test = interner.resolve(*name).to_string();
            let scenario = Scenario::new(
                interner,
                types,
                &policies,
                &defs,
                body,
                World::for_test(&test, suite_seed, caps, b.unwrap_or(budget)),
            );
            let here = scenario.lowered_here_run();
            let candidates = candidates
                .iter()
                .map(|k| match k {
                    PageTierKind::LoweredHere => (*k, here.tier_run()),
                    _ => (*k, scenario.run(*k, launcher, &stripped)),
                })
                .collect();
            out.push(TestTierComparison {
                test,
                reference: here.trace,
                candidates,
            });
        }
        Ok(out)
    })
}

/// An example page's assembled source — `entry`'s project closure through
/// [`crate::analysis::assemble_project`], the same function `largo build --static` compiles from,
/// not a raw single-file read. A page that shares a layout or a type across files (`Import
/// Layout.` + `{extends Site}`, `Import Shared.`) is a normal project; a discovery path that reads
/// only the one file it happened to be pointed at is what misdiagnoses that page, not the page.
///
/// Retried like a single-file `read_stable`: assembly reads the entry AND every file it imports
/// straight off disk itself, so a page beside another process's mid-write (a live, multi-file
/// corpus) must not read as a spurious failure any more than a single-file read would. Returns the
/// underlying assembly error only once 20 attempts, 5ms apart, never produce a reading that repeats
/// — a real, persistent problem, not a torn one.
pub fn assemble_stable(entry: &Path) -> Result<String, String> {
    let mut last_ok: Option<String> = None;
    let mut last_err: Option<String> = None;
    for attempt in 0..20 {
        if attempt > 0 {
            std::thread::sleep(Duration::from_millis(5));
        }
        match crate::analysis::assemble_project(entry) {
            Ok(assembled) if last_ok.as_deref() == Some(assembled.source.as_str()) => {
                return Ok(assembled.source);
            }
            Ok(assembled) => last_ok = Some(assembled.source),
            Err(e) => last_err = Some(e),
        }
    }
    last_ok.ok_or_else(|| {
        last_err.unwrap_or_else(|| format!("could not stably assemble {}", entry.display()))
    })
}

impl<'a> Scenario<'a> {
    /// The run on `kind`. The reference tier's run is [`Scenario::reference_trace`].
    pub fn run(
        &self,
        kind: PageTierKind,
        launcher: Option<&dyn RemotePageLauncher>,
        source: &str,
    ) -> TierRun {
        match kind {
            PageTierKind::LoweredHere => self.lowered_here_trace(),
            PageTierKind::LoweredVm => self.lowered_vm_trace(),
            PageTierKind::Browser => match launcher {
                Some(l) => self.remote_trace(l, source),
                None => TierRun::Refused {
                    statement: 0,
                    reason: "no browser is attached to this run".to_string(),
                },
            },
        }
    }
}

/// Whether a scenario works a page: something in the program or the body mounts one.
pub fn works_a_page(defs: &[Stmt], body: &[Stmt]) -> bool {
    defs.iter()
        .chain(body)
        .any(|s| matches!(s, Stmt::Ui(UiStmt::Mount { .. })))
}
