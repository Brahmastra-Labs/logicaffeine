//! ★ LOCK — the host a browser keeps for a lowered page OWNS NO ENGINE. ★
//!
//! [`LoweredHost`] carries every piece of state a browser keeps for a page — its clock and armed
//! intervals, the requests it has decided about, whether it serves fetches from the program's
//! routes, the first failure no handler absorbed, the component `Replay` runs against — and borrows
//! the engine only for the one statement it is carrying out. That is what lets an engine that is
//! ALSO running the test (the tree-walker) own its page host with no cycle between the two: the
//! engine holds the host, and hands itself to it one statement at a time.
use super::{DriveError, LoweredCalls, LoweredHost};
use crate::interpreter::RuntimeValue;
use crate::page_host::Drive;
use crate::semantics::dom;
use std::collections::BTreeMap;
use std::rc::Rc;

/// What one entry answers when the host calls it.
type Answer = fn(&[RuntimeValue]) -> Result<RuntimeValue, String>;

/// An engine that defines a fixed table of entries and records every call the host makes — the
/// minimal double for proving what the host asked of the engine, and that the engine stays the
/// caller's.
#[derive(Default)]
struct ScriptedEngine {
    entries: BTreeMap<&'static str, Answer>,
    calls: Vec<(String, Vec<RuntimeValue>)>,
    reloads: usize,
    lines: Vec<String>,
}

impl ScriptedEngine {
    fn with(entries: &[(&'static str, Answer)]) -> Self {
        ScriptedEngine {
            entries: entries.iter().copied().collect(),
            ..ScriptedEngine::default()
        }
    }

    fn called(&self) -> Vec<&str> {
        self.calls.iter().map(|(name, _)| name.as_str()).collect()
    }
}

impl LoweredCalls for ScriptedEngine {
    fn call(&mut self, name: &str, args: Vec<RuntimeValue>) -> Result<RuntimeValue, String> {
        let answer = *self
            .entries
            .get(name)
            .ok_or_else(|| format!("no entry `{name}`"))?;
        self.calls.push((name.to_string(), args.clone()));
        answer(&args)
    }
    fn call_value(
        &mut self,
        callee: &RuntimeValue,
        _args: Vec<RuntimeValue>,
    ) -> Result<RuntimeValue, String> {
        Err(format!("Cannot call value of type {}", callee.type_name()))
    }
    fn has(&self, name: &str) -> bool {
        self.entries.contains_key(name)
    }
    fn reload(&mut self) -> Result<(), String> {
        self.reloads += 1;
        Ok(())
    }
    fn lines(&self) -> &[String] {
        &self.lines
    }
}

fn nothing(_: &[RuntimeValue]) -> Result<RuntimeValue, String> {
    Ok(RuntimeValue::Nothing)
}

fn boom(_: &[RuntimeValue]) -> Result<RuntimeValue, String> {
    Err("boom".to_string())
}

fn bang(_: &[RuntimeValue]) -> Result<RuntimeValue, String> {
    Err("bang".to_string())
}

fn text(s: &str) -> RuntimeValue {
    RuntimeValue::Text(Rc::new(s.to_string()))
}

#[test]
fn a_host_drives_an_engine_it_does_not_own() {
    dom::reset();
    let mut engine = ScriptedEngine::with(&[("__mount_App", nothing), ("__act_App_bump", nothing)]);
    let mut host = LoweredHost::new();
    host.start(&mut engine, None, |_| Ok(()))
        .expect("the page starts");
    host.drive(
        &mut engine,
        &Drive::Mount {
            component: "App".to_string(),
            at: None,
        },
    )
    .expect("the mount is carried out");
    assert_eq!(
        engine.calls,
        vec![(
            "__mount_App".to_string(),
            vec![RuntimeValue::Int(dom::body())]
        )],
        "the host called the engine's own mount entry with the page's body"
    );
    // The engine is still the caller's between statements: the test reads and drives it directly.
    engine
        .lines
        .push("the caller still holds the engine".to_string());
    // The host remembers what it mounted across statements — its own state, not the engine's.
    host.drive(
        &mut engine,
        &Drive::Replay {
            actions: vec!["bump".to_string()],
        },
    )
    .expect("the replay is carried out against the mounted component");
    assert_eq!(engine.called(), ["__mount_App", "__act_App_bump"]);
    assert_eq!(
        engine.calls[1].1,
        vec![text("")],
        "a replayed action runs on the root instance"
    );
}

#[test]
fn an_engine_behind_a_trait_object_is_driven_the_same_way() {
    dom::reset();
    let mut engine = ScriptedEngine::with(&[("__mount_App", nothing)]);
    let mut host = LoweredHost::new();
    {
        let dynamic: &mut dyn LoweredCalls = &mut engine;
        host.start(dynamic, None, |_| Ok(())).unwrap();
        host.drive(
            dynamic,
            &Drive::Mount {
                component: "App".to_string(),
                at: None,
            },
        )
        .unwrap();
    }
    assert_eq!(engine.called(), ["__mount_App"]);
}

/// The page mounted, then `actions` replayed against it: each one a turn the page is handed.
fn replayed(engine: &mut ScriptedEngine, host: &mut LoweredHost, actions: &[&str]) -> Result<(), DriveError> {
    host.drive(
        engine,
        &Drive::Replay {
            actions: actions.iter().map(|a| a.to_string()).collect(),
        },
    )
}

fn mounted(engine: &mut ScriptedEngine) -> LoweredHost {
    let mut host = LoweredHost::new();
    host.start(engine, None, |_| Ok(())).unwrap();
    host.drive(
        engine,
        &Drive::Mount {
            component: "App".to_string(),
            at: None,
        },
    )
    .expect("the page mounts");
    host
}

#[test]
fn a_failure_no_handler_absorbs_is_the_hosts_record_and_the_first_one_stays() {
    dom::reset();
    let mut engine = ScriptedEngine::with(&[
        ("__mount_App", nothing),
        ("__act_App_fails", boom),
        ("__act_App_fails_too", bang),
    ]);
    let mut host = mounted(&mut engine);
    replayed(&mut engine, &mut host, &["fails"])
        .expect("a failing turn is handed to the failure path, not returned as the statement's");
    assert_eq!(host.unhandled_failure(), Some("boom"));
    replayed(&mut engine, &mut host, &["fails_too"]).unwrap();
    assert_eq!(
        host.unhandled_failure(),
        Some("boom"),
        "the first unabsorbed failure is the one kept"
    );
}

#[test]
fn a_failure_the_page_absorbs_is_not_recorded() {
    dom::reset();
    fn absorbed(_: &[RuntimeValue]) -> Result<RuntimeValue, String> {
        Ok(RuntimeValue::Bool(true))
    }
    let mut engine = ScriptedEngine::with(&[
        ("__mount_App", nothing),
        ("__act_App_fails", boom),
        ("__lgx_on_failure", absorbed),
    ]);
    let mut host = mounted(&mut engine);
    replayed(&mut engine, &mut host, &["fails"]).unwrap();
    assert_eq!(host.unhandled_failure(), None);
    assert_eq!(
        engine.calls[2],
        ("__lgx_on_failure".to_string(), vec![text("boom")]),
        "the page's failure hook heard the failure's own message"
    );
}

#[test]
fn a_mount_that_fails_is_the_statements_failure() {
    dom::reset();
    let mut engine = ScriptedEngine::with(&[("__mount_App", boom), ("__lgx_on_failure", nothing)]);
    let mut host = LoweredHost::new();
    host.start(&mut engine, None, |_| Ok(())).unwrap();
    let outcome = host.drive(
        &mut engine,
        &Drive::Mount {
            component: "App".to_string(),
            at: None,
        },
    );
    assert!(
        matches!(outcome, Err(DriveError::Failed(ref why)) if why == "boom"),
        "`When the page loads:` runs as the page mounts, not as a turn: {outcome:?}"
    );
    assert_eq!(engine.called(), ["__mount_App"], "no turn's failure hook hears a mount's failure");
    assert_eq!(host.unhandled_failure(), None);
}

#[test]
fn a_page_that_fails_to_start_reports_its_mains_failure() {
    dom::reset();
    let mut engine = ScriptedEngine::default();
    let mut host = LoweredHost::new();
    let started = host.start(&mut engine, None, |_| Err("main trapped".to_string()));
    assert_eq!(started, Err(DriveError::Failed("main trapped".to_string())));
}

#[test]
fn a_reload_restarts_the_engine_and_the_hosts_clock() {
    dom::reset();
    let mut engine = ScriptedEngine::default();
    let mut host = LoweredHost::new();
    host.start(&mut engine, None, |_| Ok(())).unwrap();
    host.drive(&mut engine, &Drive::AdvanceTime { ms: 500 })
        .unwrap();
    host.drive(&mut engine, &Drive::Reload).unwrap();
    assert_eq!(engine.reloads, 1, "the engine's main runs again");
    assert_eq!(host.now_ms, 0, "and the page's clock starts over with it");
}
