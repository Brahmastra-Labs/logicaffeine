//! ★ LOCK — the page seam (`with_page`/`with_statement_observer`): with no page attached, every
//! driving verb runs on the page this interpreter hosts itself; with one attached, every driving
//! verb is carried out on IT instead — never on this interpreter's own DOM — while every other
//! statement (`Let`, `Show`, …) keeps running here. ★
use crate::page_host::{Drive, DriveError, DrivenPage};
use std::cell::RefCell;
use std::rc::Rc;

/// A page that records every statement driven into it and answers nothing of its own — the
/// minimal double for proving WHERE a driving verb ran, not what a real page does with it.
#[derive(Default)]
struct RecordingPage {
    drives: Vec<Drive>,
}

impl DrivenPage for RecordingPage {
    fn drive(&mut self, d: &Drive) -> Result<(), DriveError> {
        self.drives.push(d.clone());
        Ok(())
    }
    fn unhandled_failure(&self) -> Option<String> {
        None
    }
    fn snapshot(&mut self) -> Result<crate::page_host::DocSnapshot, String> {
        Ok(crate::page_host::DocSnapshot::default())
    }
}

/// Parse `source`, build an `Interpreter` over it (optionally wired to a page and an
/// observer), and run it to completion.
fn run_with(
    source: &str,
    page: Option<Rc<RefCell<RecordingPage>>>,
    observed: Option<Rc<RefCell<Vec<usize>>>>,
) -> Result<Vec<String>, String> {
    crate::ui_bridge::with_parsed_program(source, |parsed, interner| {
        let (stmts, types, policies) = parsed.map_err(|e| e.to_string())?;
        let mut interp = crate::interpreter::Interpreter::new(interner)
            .with_type_registry(types)
            .with_policies(policies);
        if let Some(p) = page {
            interp = interp.with_page(p);
        }
        if let Some(seen) = observed {
            interp = interp.with_statement_observer(Box::new(move |i| seen.borrow_mut().push(i)));
        }
        interp.run_sync(stmts)?;
        Ok(interp.output.clone())
    })
}

#[test]
fn with_no_page_attached_driving_verbs_run_locally_exactly_as_before() {
    crate::semantics::dom::reset();
    let src = "\
## Component App
State:
    Let n be 0.
Actions:
    To add:
        Set n to n + 1.
View:
    <p id=\"n\">{n}</p>
    <button id=\"go\" on-click={add}>go</button>

## Main
Mount App.
Click \"#go\".
";
    let out = run_with(src, None, None);
    assert_eq!(out, Ok(Vec::new()));
    assert!(
        crate::semantics::dom::render().contains("<p id=\"n\">1</p>"),
        "the click ran against this interpreter's own DOM, unaffected by the seam: {}",
        crate::semantics::dom::render()
    );
}

#[test]
fn an_attached_page_receives_every_driving_verb_and_the_local_dom_never_moves() {
    crate::semantics::dom::reset();
    let page = Rc::new(RefCell::new(RecordingPage::default()));
    let src = "\
## Component App
State:
    Let n be 0.
Actions:
    To add:
        Set n to n + 1.
View:
    <p id=\"n\">{n}</p>
    <button id=\"go\" on-click={add}>go</button>

## Main
Mount App.
Click \"#go\".
";
    let out = run_with(src, Some(page.clone()), None);
    assert_eq!(out, Ok(Vec::new()));
    assert_eq!(
        page.borrow().drives,
        vec![
            Drive::Mount {
                component: "App".to_string(),
                at: None
            },
            Drive::Click {
                selector: "#go".to_string()
            },
        ],
        "every driving verb reached the attached page, in order"
    );
    assert_eq!(
        crate::semantics::dom::render(),
        "<body></body>",
        "a driving verb delegated to the page must never touch this interpreter's own DOM: {}",
        crate::semantics::dom::render()
    );
}

#[test]
fn an_attached_page_receives_replay_and_the_local_runtime_never_runs_it() {
    crate::semantics::dom::reset();
    let page = Rc::new(RefCell::new(RecordingPage::default()));
    let src = "\
## Component App
State:
    Let n be 0.
Actions:
    To bump:
        Set n to n + 1.
    To grow:
        Set n to n * 2.
View:
    <p id=\"n\">{n}</p>

## Main
Mount App.
Replay these actions: bump, grow, bump.
";
    let out = run_with(src, Some(page.clone()), None);
    assert_eq!(
        out,
        Ok(Vec::new()),
        "the replay ran on the page, which answered cleanly"
    );
    assert_eq!(
        page.borrow().drives,
        vec![
            Drive::Mount {
                component: "App".to_string(),
                at: None
            },
            Drive::Replay {
                actions: vec!["bump".to_string(), "grow".to_string(), "bump".to_string()]
            },
        ],
        "`Replay these actions:` is a driving statement like any other: the attached page \
         receives it, with every action by name, in order"
    );
    assert_eq!(
        crate::semantics::dom::render(),
        "<body></body>",
        "the replay must never run against the page this interpreter would host itself: {}",
        crate::semantics::dom::render()
    );
}

#[test]
fn a_replay_the_attached_page_refuses_is_the_statements_failure() {
    crate::semantics::dom::reset();
    struct RefusingPage;
    impl DrivenPage for RefusingPage {
        fn drive(&mut self, d: &Drive) -> Result<(), DriveError> {
            match d {
                Drive::Replay { .. } => Err(DriveError::Failed("no page to replay".to_string())),
                _ => Ok(()),
            }
        }
        fn unhandled_failure(&self) -> Option<String> {
            None
        }
        fn snapshot(&mut self) -> Result<crate::page_host::DocSnapshot, String> {
            Ok(crate::page_host::DocSnapshot::default())
        }
    }
    let src = "\
## Component App
State:
    Let n be 0.
Actions:
    To bump:
        Set n to n + 1.
View:
    <p id=\"n\">{n}</p>

## Main
Mount App.
Replay these actions: bump.
Show \"after\".
";
    let result = crate::ui_bridge::with_parsed_program(src, |parsed, interner| {
        let (stmts, types, policies) = parsed.map_err(|e| e.to_string())?;
        let mut interp = crate::interpreter::Interpreter::new(interner)
            .with_type_registry(types)
            .with_policies(policies)
            .with_page(Rc::new(RefCell::new(RefusingPage)));
        interp.run_sync(stmts)?;
        Ok::<_, String>(interp.output.clone())
    });
    assert_eq!(
        result,
        Err("no page to replay".to_string()),
        "the page's own answer to the replay is the statement's failure, and nothing after it runs"
    );
}

#[test]
fn non_driving_statements_still_run_locally_with_a_page_attached() {
    crate::semantics::dom::reset();
    let page = Rc::new(RefCell::new(RecordingPage::default()));
    let src = "## Main\nLet x be 1 + 1.\nShow x.\n";
    let out = run_with(src, Some(page.clone()), None);
    assert_eq!(
        out,
        Ok(vec!["2".to_string()]),
        "ordinary computation still runs here"
    );
    assert!(
        page.borrow().drives.is_empty(),
        "nothing here is a driving verb"
    );
}

#[test]
fn the_statement_observer_fires_once_per_top_level_statement_in_order() {
    crate::semantics::dom::reset();
    let seen = Rc::new(RefCell::new(Vec::new()));
    let src = "## Main\nShow 1.\nShow 2.\nShow 3.\n";
    let out = run_with(src, None, Some(seen.clone()));
    assert_eq!(
        out,
        Ok(vec!["1".to_string(), "2".to_string(), "3".to_string()])
    );
    assert_eq!(*seen.borrow(), vec![0, 1, 2]);
}
