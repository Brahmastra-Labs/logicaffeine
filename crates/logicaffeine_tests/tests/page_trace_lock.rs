//! ★ LOCK — a page's run is a TRACE: the page after every top-level statement, compared axis by
//! axis. ★
//!
//! Two tiers that run one `## Test` are held together by what the page looked like after each
//! statement — document, title, URL, focus, storage, the requests it sent, the journal, the failure
//! it reported, and where the run stopped. This file pins what a trace IS and what comparing two
//! of them reports, so the tier locks built on it (`lowered_vm_tier_lock`,
//! `typed_fetch_completes_on_every_tier`) cannot pass by comparing less than they claim.

use logicaffeine_compile::page_host::{
    classify, compare_tests, Axis, Classified, DocSnapshot, Drive, DriveError, PageTierKind,
    PageTrace, RemotePage, Stop, TestTierComparison, TierRun, TraceStep,
};
use logicaffeine_compile::semantics::dom;

const BUDGET: u64 = 50_000_000;

const COUNTER: &str = "## Component Counter\n\
    State:\n    Let n be 0.\n\
    Actions:\n    To bump:\n        Set n to n + 1.\n\
    View:\n    <p id=\"n\">{n}</p>\n    <button id=\"b\" on-click={bump}>+</button>\n\n";

fn only(src: &str) -> TestTierComparison {
    let mut all = compare_tests(src, &[PageTierKind::LoweredVm], 0, BUDGET, None)
        .unwrap_or_else(|e| panic!("the source must parse:\n{src}\n=> {e}"));
    assert_eq!(all.len(), 1, "exactly one page test:\n{src}");
    all.remove(0)
}

fn lowered(c: &TestTierComparison) -> &PageTrace {
    match &c.candidates[0] {
        (PageTierKind::LoweredVm, TierRun::Traced(t)) => t,
        other => panic!("the lowered tier must trace this scenario, got {other:?}"),
    }
}

#[test]
fn a_trace_has_one_snapshot_per_top_level_statement_and_one_for_the_program() {
    let src = format!(
        "{COUNTER}## Test: clicking counts\n\
         \x20   Mount Counter.\n\
         \x20   Click \"#b\".\n\
         \x20   Expect that the text of \"#n\" is equal to \"1\".\n\
         \x20   Click \"#b\".\n"
    );
    let c = only(&src);
    let statements: Vec<usize> = c.reference.steps.iter().map(|s| s.statement).collect();
    assert_eq!(
        statements,
        [0, 1, 2, 3, 4],
        "step 0 is the program's own page, then one per statement"
    );
    assert_eq!(
        lowered(&c)
            .steps
            .iter()
            .map(|s| s.statement)
            .collect::<Vec<_>>(),
        statements
    );
    assert_eq!(c.reference.stopped, None);
}

#[test]
fn each_step_is_the_page_after_that_statement() {
    let src = format!(
        "{COUNTER}## Test: clicking counts\n\
         \x20   Mount Counter.\n\
         \x20   Click \"#b\".\n\
         \x20   Click \"#b\".\n"
    );
    let c = only(&src);
    for trace in [&c.reference, lowered(&c)] {
        let docs: Vec<&str> = trace
            .steps
            .iter()
            .map(|s| s.snapshot.document.as_str())
            .collect();
        assert_eq!(
            docs[0], "<body></body>",
            "nothing is mounted before the test mounts it"
        );
        assert!(docs[1].contains(">0<"), "{docs:?}");
        assert!(docs[2].contains(">1<"), "{docs:?}");
        assert!(docs[3].contains(">2<"), "{docs:?}");
    }
    // The whole-trace agreement of every page is `lowered_vm_tier_lock`'s, held against its
    // shrink-only ledger; here it is the page each statement leaves.
    let d = c.reference.compare(lowered(&c));
    assert!(
        d.axes
            .iter()
            .all(|a| a.axis != Axis::Document && a.axis != Axis::Stopped),
        "the two tiers paint the same page after every statement:\n{}",
        d.render()
    );
}

#[test]
fn a_statement_that_only_reads_is_executed_by_the_reference_and_not_by_a_driven_tier() {
    let src = format!(
        "{COUNTER}## Test: reads\n\
         \x20   Mount Counter.\n\
         \x20   Expect that the text of \"#n\" is equal to \"0\".\n"
    );
    let c = only(&src);
    assert!(c.reference.steps.iter().all(|s| s.executed));
    let l = lowered(&c);
    assert!(l.steps[1].executed, "the mount drives the page");
    assert!(
        !l.steps[2].executed,
        "the Expect is the test's, and the test runs on the tree-walker"
    );
}

#[test]
fn a_failing_driving_statement_stops_both_runs_at_that_statement_with_the_same_words() {
    let src = format!(
        "{COUNTER}## Test: clicks a button that is not there\n\
         \x20   Mount Counter.\n\
         \x20   Click \"#typo\".\n\
         \x20   Click \"#b\".\n"
    );
    let c = only(&src);
    let stop = c
        .reference
        .stopped
        .clone()
        .expect("the reference stops at the missing element");
    assert_eq!(stop.statement, 2);
    assert!(
        stop.message.starts_with("No element matches \"#typo\"."),
        "{}",
        stop.message
    );
    assert_eq!(
        lowered(&c).stopped.as_ref(),
        Some(&stop),
        "the lowered page answers in the same words"
    );
    let d = c.reference.compare(lowered(&c));
    assert!(
        d.axes.iter().all(|a| a.axis != Axis::Stopped),
        "{}",
        d.render()
    );
}

#[test]
fn an_unanswered_click_is_refused_in_the_reference_tiers_words_on_the_lowered_tier() {
    let src = "## Component App\nView:\n    <p id=\"m\">hi</p>\n\n\
        ## Test: nothing listens\n\
        \x20   Mount App.\n\
        \x20   Click \"#m\".\n";
    let c = only(src);
    let stop = c
        .reference
        .stopped
        .clone()
        .expect("the reference refuses the unanswered click");
    assert!(
        stop.message
            .starts_with("Nothing answers a click on \"#m\"."),
        "{}",
        stop.message
    );
    assert_eq!(
        lowered(&c).stopped.as_ref(),
        Some(&stop),
        "a page with no dispatcher says the same"
    );
}

/// Change `axis`'s value in `snap` — an exhaustive match, so an axis added without a way to
/// perturb it does not compile here.
fn perturb(snap: &mut DocSnapshot, axis: Axis) {
    match axis {
        Axis::Document => snap.document.push('x'),
        Axis::Head => snap.head = Some("<title>x</title>".into()),
        Axis::Title => snap.title.push('x'),
        Axis::Url => snap.url.push('x'),
        Axis::RouteOps => snap.route_ops.push("push /x".into()),
        Axis::Focus => snap.focus = Some("input#x".into()),
        Axis::Scroll => snap.scroll = Some("div#x".into()),
        Axis::Selection => snap.selection = Some("p#x".into()),
        Axis::Clipboard => snap.clipboard.push('x'),
        Axis::Downloads => snap.downloads.push(("a.txt".into(), "x".into())),
        Axis::Notifications => snap.notifications.push(("x".into(), "".into())),
        Axis::Published => snap.published.push(("t".into(), "x".into())),
        Axis::Storage => snap.storage.push(("local".into(), "k".into(), "x".into())),
        Axis::Fetches => snap.fetches.push("GET /x".into()),
        Axis::Journal => snap.journal.push("bump".into()),
        Axis::Failure => snap.failure = Some("boom".into()),
        Axis::Stopped => unreachable!("not a snapshot field"),
    }
}

fn trace_of(n: usize) -> PageTrace {
    PageTrace {
        steps: (0..=n)
            .map(|statement| TraceStep {
                statement,
                executed: true,
                snapshot: DocSnapshot {
                    document: format!("<body>{statement}</body>"),
                    ..DocSnapshot::default()
                },
            })
            .collect(),
        stopped: None,
    }
}

#[test]
fn every_snapshot_axis_is_compared_and_named_at_its_first_statement() {
    for axis in Axis::SNAPSHOT {
        let reference = trace_of(4);
        let mut candidate = reference.clone();
        perturb(&mut candidate.steps[2].snapshot, axis);
        perturb(&mut candidate.steps[3].snapshot, axis);
        let d = reference.compare(&candidate);
        assert_eq!(
            d.axes.len(),
            1,
            "{axis:?}: one axis differs, one is reported: {d:?}"
        );
        assert_eq!(d.axes[0].axis, axis);
        assert_eq!(
            d.axes[0].statement, 2,
            "{axis:?}: named at the FIRST statement it differs"
        );
        assert_eq!(d.first_statement(), Some(2));
    }
}

#[test]
fn two_axes_that_differ_are_both_reported_so_one_cannot_hide_the_other() {
    let reference = trace_of(4);
    let mut candidate = reference.clone();
    perturb(&mut candidate.steps[1].snapshot, Axis::Journal);
    perturb(&mut candidate.steps[3].snapshot, Axis::Document);
    let d = reference.compare(&candidate);
    let got: Vec<(Axis, usize)> = d.axes.iter().map(|a| (a.axis, a.statement)).collect();
    assert_eq!(got, [(Axis::Journal, 1), (Axis::Document, 3)]);
}

#[test]
fn every_axis_has_a_name_a_ledger_can_spell_and_read_back() {
    let mut names = std::collections::BTreeSet::new();
    for axis in Axis::SNAPSHOT.into_iter().chain([Axis::Stopped]) {
        assert_eq!(Axis::from_name(axis.name()), Some(axis));
        assert!(
            names.insert(axis.name()),
            "two axes share the name {}",
            axis.name()
        );
    }
    assert_eq!(Axis::from_name("no such axis"), None);
}

#[test]
fn a_stop_on_one_side_only_is_a_divergence_at_that_statement() {
    let reference = trace_of(3);
    let mut candidate = trace_of(2);
    candidate.stopped = Some(Stop {
        statement: 2,
        message: "boom".into(),
    });
    let d = reference.compare(&candidate);
    assert_eq!(d.axes.len(), 1, "{d:?}");
    assert_eq!((d.axes[0].axis, d.axes[0].statement), (Axis::Stopped, 2));
    assert!(d.axes[0].candidate.contains("boom"));
}

#[test]
fn a_reference_stop_at_a_statement_the_candidate_only_read_ends_the_comparison_there() {
    // The reference failed evaluating a READ (a page query on a missing element). The candidate
    // did not execute reads — but every page before it agreed, so it would have failed the same
    // way. What follows is not compared: the reference never got there.
    let mut reference = trace_of(2);
    reference.stopped = Some(Stop {
        statement: 2,
        message: "No element matches \"#x\", so it has no text.".into(),
    });
    let mut candidate = trace_of(4);
    candidate.steps[2].executed = false;
    perturb(&mut candidate.steps[3].snapshot, Axis::Document);
    assert!(reference.compare(&candidate).is_empty());
    // …but a candidate that EXECUTED that statement and went on is a real divergence.
    candidate.steps[2].executed = true;
    assert!(!reference.compare(&candidate).is_empty());
}

#[test]
fn a_difference_is_shown_where_it_is() {
    let a = format!("<body>{}<p>1</p></body>", "x".repeat(300));
    let b = format!("<body>{}<p>2</p></body>", "x".repeat(300));
    let shown = logicaffeine_compile::page_host::describe_difference(&a, &b);
    assert!(
        shown.contains("<p>1</p>") && shown.contains("<p>2</p>"),
        "{shown}"
    );
    assert!(
        shown.contains("byte 309"),
        "the offset of the first difference: {shown}"
    );
}

// ---- What a driven tier carries, and what it refuses. ----

fn classify_body(src: &str) -> Vec<Classified> {
    logicaffeine_compile::ui_bridge::with_parsed_program(src, |parsed, interner| {
        let (stmts, _, _) = parsed.unwrap_or_else(|e| panic!("parses:\n{src}\n=> {e}"));
        let body = stmts
            .iter()
            .find_map(|s| match s {
                logicaffeine_compile::ast::stmt::Stmt::TestDef { body, .. } => Some(*body),
                _ => None,
            })
            .expect("a test");
        body.iter().map(|s| classify(s, interner)).collect()
    })
}

#[test]
fn every_driving_statement_a_test_can_write_maps_to_one_drive() {
    let src = format!(
        "{COUNTER}## Component Shell\nView:\n    <div id=\"root\"></div>\n\n\
         ## Test: everything\n\
         \x20   Mount Shell.\n\
         \x20   Mount Counter at \"#root\".\n\
         \x20   Click \"#b\".\n\
         \x20   Submit \"#f\".\n\
         \x20   Press \"Enter\" in \"#d\".\n\
         \x20   Type \"milk\" into \"#d\".\n\
         \x20   Choose \"b\" in \"#s\".\n\
         \x20   Hover \"#b\".\n\
         \x20   Focus \"#d\".\n\
         \x20   Blur \"#d\".\n\
         \x20   Raise \"offline\".\n\
         \x20   Navigate to \"/x\".\n\
         \x20   Reload the page.\n\
         \x20   Advance time by 3 seconds.\n\
         \x20   Advance time by 250.\n\
         \x20   Given that GET \"/api\" answers 404 with \"gone\".\n\
         \x20   Serve fetches with routes.\n\
         \x20   Expect that the text of \"#n\" is equal to \"0\".\n"
    );
    let s = |x: &str| x.to_string();
    let want = vec![
        Classified::Drive(Drive::Mount {
            component: s("Shell"),
            at: None,
        }),
        Classified::Drive(Drive::Mount {
            component: s("Counter"),
            at: Some(s("#root")),
        }),
        Classified::Drive(Drive::Click { selector: s("#b") }),
        Classified::Drive(Drive::Submit { selector: s("#f") }),
        Classified::Drive(Drive::Press {
            key: s("Enter"),
            selector: s("#d"),
        }),
        Classified::Drive(Drive::Type {
            text: s("milk"),
            selector: s("#d"),
        }),
        Classified::Drive(Drive::Choose {
            value: s("b"),
            selector: s("#s"),
        }),
        Classified::Drive(Drive::Hover { selector: s("#b") }),
        Classified::Drive(Drive::Focus { selector: s("#d") }),
        Classified::Drive(Drive::Blur { selector: s("#d") }),
        Classified::Drive(Drive::Raise {
            event: s("offline"),
            target: s("body"),
            value: s(""),
        }),
        Classified::Drive(Drive::Navigate { url: s("/x") }),
        Classified::Drive(Drive::Reload),
        Classified::Drive(Drive::AdvanceTime { ms: 3000 }),
        Classified::Drive(Drive::AdvanceTime { ms: 250 }),
        Classified::Drive(Drive::AnswerFetch {
            method: s("GET"),
            url: s("/api"),
            status: 404,
            body: s("gone"),
        }),
        Classified::Drive(Drive::ServeFetchesWithRoutes),
        Classified::Read,
    ];
    assert_eq!(classify_body(&src), want);
}

#[test]
fn a_computed_operand_is_refused_by_a_driven_tier_and_named() {
    let src = format!(
        "{COUNTER}## Test: computed\n\
         \x20   Mount Counter.\n\
         \x20   Let target be \"#b\".\n\
         \x20   Click target.\n"
    );
    let c = only(&src);
    match &c.candidates[0].1 {
        TierRun::Refused { statement, reason } => {
            assert_eq!(*statement, 3, "the Click is statement 3");
            assert!(
                reason.contains("`target`") && reason.contains("computed"),
                "{reason}"
            );
        }
        other => panic!("a computed operand must be refused, not guessed: {other:?}"),
    }
}

#[test]
fn a_driving_statement_inside_a_block_is_refused_rather_than_skipped() {
    let src = format!(
        "{COUNTER}## Test: loop\n\
         \x20   Mount Counter.\n\
         \x20   Repeat for i from 1 to 2:\n\
         \x20       Click \"#b\".\n\
         \x20   Expect that the text of \"#n\" is equal to \"2\".\n"
    );
    let steps = classify_body(&src);
    assert!(
        matches!(&steps[1], Classified::Unsupported(why) if why.contains("`Repeat`")),
        "{steps:?}"
    );
}

#[test]
fn a_read_that_calls_a_function_is_refused_rather_than_assumed_harmless() {
    let src = format!(
        "## To shout (t: Text) -> Text:\n    Return t + \"!\".\n\n{COUNTER}## Test: calls\n\
         \x20   Mount Counter.\n\
         \x20   Let x be shout(\"a\").\n"
    );
    let steps = classify_body(&src);
    assert!(matches!(&steps[1], Classified::Unsupported(_)), "{steps:?}");
}

// ---- A page in another process is walked by the same driver loop. ----

/// A stand-in for a browser: it records every statement it is handed and paints a line for each,
/// so the trace shows the walk handed over each driving statement once, in order.
struct Recording {
    log: std::rc::Rc<std::cell::RefCell<Vec<Drive>>>,
}

impl RemotePage for Recording {
    fn drive(&mut self, d: &Drive) -> Result<(), DriveError> {
        self.log.borrow_mut().push(d.clone());
        if let Drive::Click { selector } = d {
            if selector == "#gone" {
                return Err(DriveError::Failed("the remote page has no #gone".into()));
            }
        }
        let p = dom::create_element("p");
        dom::set_text(p, &format!("{d:?}"));
        dom::append_child(dom::body(), p);
        Ok(())
    }
    fn unhandled_failure(&self) -> Option<String> {
        None
    }
    fn snapshot(&mut self) -> Result<DocSnapshot, String> {
        Ok(DocSnapshot::capture(None))
    }
}

struct Launcher {
    log: std::rc::Rc<std::cell::RefCell<Vec<Drive>>>,
}

impl logicaffeine_compile::page_host::RemotePageLauncher for Launcher {
    fn launch(
        &self,
        _arrival: &logicaffeine_compile::page_host::Arrival,
    ) -> Result<Box<dyn RemotePage>, String> {
        Ok(Box::new(Recording {
            log: self.log.clone(),
        }))
    }
}

#[test]
fn a_remote_page_is_driven_statement_by_statement_through_the_same_walk() {
    let log = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let launcher = Launcher { log: log.clone() };
    let src = format!(
        "{COUNTER}## Test: remote\n\
         \x20   Mount Counter.\n\
         \x20   Expect that the text of \"#n\" is equal to \"0\".\n\
         \x20   Click \"#b\".\n\
         \x20   Click \"#gone\".\n\
         \x20   Click \"#b\".\n"
    );
    let all = compare_tests(&src, &[PageTierKind::Browser], 0, BUDGET, Some(&launcher)).unwrap();
    let TierRun::Traced(t) = &all[0].candidates[0].1 else {
        panic!("{:?}", all[0].candidates)
    };
    assert_eq!(
        *log.borrow(),
        [
            Drive::Mount {
                component: "Counter".into(),
                at: None
            },
            Drive::Click {
                selector: "#b".into()
            },
            Drive::Click {
                selector: "#gone".into()
            },
        ],
        "reads are not handed over, and nothing is driven after the page stops the run"
    );
    assert_eq!(
        t.stopped,
        Some(Stop {
            statement: 4,
            message: "the remote page has no #gone".into()
        })
    );
    assert_eq!(
        t.steps.len(),
        5,
        "program, mount, read, click, the failing click"
    );
    assert!(
        t.steps[3].snapshot.document.contains("Click"),
        "{:?}",
        t.steps[3]
    );
}

#[test]
fn without_a_browser_the_browser_tier_says_so() {
    let src = format!("{COUNTER}## Test: t\n    Mount Counter.\n");
    let all = compare_tests(&src, &[PageTierKind::Browser], 0, BUDGET, None).unwrap();
    assert!(
        matches!(&all[0].candidates[0].1, TierRun::Refused { reason, .. } if reason.contains("no browser"))
    );
}
