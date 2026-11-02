// The emitted module is RUN under the `wasmi` interpreter, gated — like the rest
// of the WASM run-tests — behind the `wasm-jit` feature.
#![cfg(feature = "wasm-jit")]
//! W13 — `obs_wasm_lock`: the direct-WASM backend EMITS observation.
//!
//! The last tier joins the observation channel. In the OBSERVED profile
//! (`compile_to_wasm_observed`) each `Note`/`Warn`/`Record`/`During` lowers to a
//! call to the host `obs(site_id)` import; the host — which compiled the program,
//! so it holds the `observation_sites` table — resolves each site to its
//! category / message / span kind. This lock RUNS the emitted module under
//! `wasmi`, capturing the `obs` calls, and proves the module emits the full event
//! stream in execution order, resolving to the same events the tree-walker and VM
//! emit. (Payload VALUES are a documented follow-up — they need linear-memory
//! marshaling; the event stream itself emits here.)

use std::cell::RefCell;
use std::rc::Rc;

#[path = "compile_common/mod.rs"]
mod common;

use logicaffeine_compile::compile::compile_to_wasm_observed;
use logicaffeine_compile::vm::ObsSiteKind;

/// A scalar, no-`Show` program (so the only host import is `obs`) threaded with a
/// message, a span around real work, and a message inside the span, then a final
/// message — five observation sites, all payload-free.
const FIXTURE: &str = "## Main\n\
    \x20   Let mutable x be 5.\n\
    \x20   Note that we started.\n\
    \x20   During folding:\n\
    \x20       Set x to x plus 1.\n\
    \x20       Record that a step ran.\n\
    \x20   Warn that we finished.\n";

/// One captured `obs` call: the site index and its three scalar-Int payload slots.
type ObsCall = (i32, [i64; 3]);

/// Run an OBSERVED wasm module under `wasmi`, capturing each `obs(site, p0, p1, p2)`
/// call the module makes.
fn run_capturing_obs(module: &[u8]) -> Vec<ObsCall> {
    let engine = wasmi::Engine::default();
    let m = wasmi::Module::new(&engine, module).expect("emitted bytes are valid wasm");
    let recorded: Rc<RefCell<Vec<ObsCall>>> = Rc::new(RefCell::new(Vec::new()));
    let mut store = wasmi::Store::new(&engine, recorded.clone());
    let mut linker = wasmi::Linker::<Rc<RefCell<Vec<ObsCall>>>>::new(&engine);
    // Every program that CAN fail declares `logos_rt_fail`, and a module importing a
    // function its host does not define FAILS TO INSTANTIATE — silently, with no
    // compile error anywhere. This host does not care why; it must simply be here.
    linker
        .func_wrap(
            "env",
            "logos_rt_fail",
            |_c: wasmi::Caller<'_, _>, _h: i32| {},
        )
        .unwrap();
    linker
        .func_wrap(
            "env",
            "logos_rt_fail_at",
            |_c: wasmi::Caller<'_, _>, _code: i32, _v: i64| {},
        )
        .unwrap();
    linker
        .func_wrap(
            "env",
            "obs",
            |c: wasmi::Caller<'_, Rc<RefCell<Vec<ObsCall>>>>,
             site: i32,
             p0: i64,
             p1: i64,
             p2: i64| {
                c.data().borrow_mut().push((site, [p0, p1, p2]));
            },
        )
        .unwrap();
    crate::common::wasm_host::link_unmodelled_imports(&mut linker);
    let instance = linker
        .instantiate(&mut store, &m)
        .unwrap()
        .start(&mut store)
        .unwrap();
    instance
        .get_typed_func::<(), ()>(&store, "main")
        .unwrap()
        .call(&mut store, ())
        .expect("main runs");
    let out = recorded.borrow().clone();
    out
}

/// Just the site-index sequence (dropping payloads).
fn site_sequence(calls: &[ObsCall]) -> Vec<i32> {
    calls.iter().map(|(s, _)| *s).collect()
}

/// The module emits `obs(site)` at every observation point, in execution order,
/// and each site resolves (via the host's `observation_sites`) to exactly the
/// tree-walker/VM event stream: Note, span-enter, Record, span-exit, Warn.
#[test]
fn wasm_emits_the_full_observation_stream() {
    let (wasm, sites) = compile_to_wasm_observed(FIXTURE).expect("compiles to observed wasm");

    // Five compiled sites: the two-channel metadata the host resolves against.
    assert_eq!(
        sites.len(),
        5,
        "Note + During(enter,exit) + Record + Warn = 5 sites"
    );

    let recorded = site_sequence(&run_capturing_obs(&wasm));
    assert_eq!(
        recorded,
        vec![0, 1, 2, 3, 4],
        "obs fires at each site, in execution order"
    );

    // Resolve each emitted site to its event — the stream the host reconstructs.
    let kind = |i: usize| &sites[recorded[i] as usize].kind;
    assert!(matches!(kind(0), ObsSiteKind::Message(_)), "the Note");
    assert!(
        matches!(kind(1), ObsSiteKind::SpanEnter),
        "the During opens"
    );
    assert!(
        matches!(kind(2), ObsSiteKind::Message(_)),
        "the Record inside the span"
    );
    assert!(
        matches!(kind(3), ObsSiteKind::SpanExit),
        "the During closes"
    );
    assert!(matches!(kind(4), ObsSiteKind::Message(_)), "the Warn");

    // And the messages resolve to the actual that-clauses / span label.
    assert_eq!(sites[0].message, "we started");
    assert_eq!(sites[1].message, "folding");
    assert_eq!(sites[2].message, "a step ran");
    assert_eq!(
        sites[3].message, "folding",
        "the span exit carries the same label as its enter"
    );
    assert_eq!(sites[4].message, "we finished");
}

/// Scalar Int payloads cross the WASM boundary: a `Record … with i and total`
/// passes the LIVE values of `i` and `total` as the `obs` call's i64 args, so the
/// host reconstructs the same payloads the tree-walker and VM capture.
#[test]
fn wasm_marshals_scalar_int_payloads() {
    // `i` runs 1,2,3 and `total` accumulates 1,3,6; the Record fires each pass.
    let src = "## Main\n\
        \x20   Let mutable total be 0.\n\
        \x20   Repeat for i from 1 to 3:\n\
        \x20       Set total to total + i.\n\
        \x20       Record that a step ran with i and total.\n";
    let (wasm, sites) = compile_to_wasm_observed(src).expect("compiles");
    // One site — the Record — with two payload names, in order.
    assert_eq!(sites.len(), 1);
    assert_eq!(
        sites[0].payload_names,
        vec!["i".to_string(), "total".to_string()]
    );

    let calls = run_capturing_obs(&wasm);
    // Three emissions (one per loop pass), all at site 0, carrying (i, total).
    let payloads: Vec<(i64, i64)> = calls.iter().map(|(_, p)| (p[0], p[1])).collect();
    assert_eq!(
        payloads,
        vec![(1, 1), (2, 3), (3, 6)],
        "live Int payload values cross to the host"
    );
}

/// Spans are balanced in the emitted module — every `span-enter` site has its
/// matching `span-exit`, exactly as on the tree-walker and VM.
#[test]
fn wasm_spans_are_balanced() {
    let (_wasm, sites) = compile_to_wasm_observed(FIXTURE).expect("compiles");
    let enters = sites
        .iter()
        .filter(|s| matches!(s.kind, ObsSiteKind::SpanEnter))
        .count();
    let exits = sites
        .iter()
        .filter(|s| matches!(s.kind, ObsSiteKind::SpanExit))
        .count();
    assert_eq!(enters, exits, "balanced spans");
    assert_eq!(enters, 1, "one During");
}
