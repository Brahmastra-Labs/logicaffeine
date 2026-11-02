//! Closed let evidence must survive repeated use within one source import.
use logicaffeine_kernel::interface::lean as adapter;

#[test]
fn original_let_ladder_accepts() {
    adapter::check_str(include_str!("fixtures/lean_arena/let-ladder.ndjson"))
        .expect("original Arena let ladder must check without exhausting materialization");
}

use serde_json::{json, Value};

struct Source {
    rows: Vec<Value>,
    next: usize,
    prop: usize,
    ty: usize,
    higher: usize,
    variable: usize,
    choose: usize,
}
impl Source {
    fn new() -> Self {
        let mut s = Self {
            rows: vec![
                json!({"meta":{"format":{"version":"3.1.0"}}}),
                json!({"in":1,"str":{"pre":0,"str":"choose"}}),
                json!({"in":2,"str":{"pre":0,"str":"result"}}),
                json!({"il":1,"succ":0}),
                json!({"il":2,"succ":1}),
            ],
            next: 0,
            prop: 0,
            ty: 0,
            higher: 0,
            variable: 0,
            choose: 0,
        };
        s.prop = s.expr("sort", json!(0));
        s.ty = s.expr("sort", json!(1));
        s.higher = s.expr("sort", json!(2));
        s.variable = s.expr("bvar", json!(0));
        let outer = s.expr("bvar", json!(1));
        let inside = s.binder("lam", s.ty, outer);
        let body = s.binder("lam", s.ty, inside);
        let inside_ty = s.binder("forallE", s.ty, s.ty);
        let function_ty = s.binder("forallE", s.ty, inside_ty);
        s.rows.push(
            json!({"def":{"name":1,"all":[1],"type":function_ty,"value":body,
            "levelParams":[],"safety":"safe","hints":"abbrev"}}),
        );
        s.choose = s.expr("const", json!({"name":1,"us":[]}));
        s
    }
    fn expr(&mut self, kind: &str, value: Value) -> usize {
        let id = self.next;
        self.next += 1;
        self.rows.push(json!({"ie":id,kind:value}));
        id
    }
    fn binder(&mut self, kind: &str, ty: usize, body: usize) -> usize {
        self.expr(
            kind,
            json!({"name":0,"type":ty,"body":body,"binderInfo":"default"}),
        )
    }
    fn let_(&mut self, ty: usize, value: usize, body: usize) -> usize {
        self.expr(
            "letE",
            json!({"name":0,"type":ty,"value":value,"body":body,"nondep":false}),
        )
    }
    fn pair(&mut self, a: usize, b: usize) -> usize {
        let f = self.expr("app", json!({"fn":self.choose,"arg":a}));
        self.expr("app", json!({"fn":f,"arg":b}))
    }
    fn finish(mut self, mut body: usize) -> String {
        // Force the sharing path; the prefix also establishes a valid repeated pair.
        for _ in 0..160 {
            body = self.let_(self.ty, self.prop, body);
        }
        self.rows.push(
            json!({"def":{"name":2,"all":[2],"type":self.ty,"value":body,
            "levelParams":[],"safety":"safe","hints":"abbrev"}}),
        );
        self.rows
            .into_iter()
            .map(|r| r.to_string() + "\n")
            .collect()
    }
}
fn rejects(source: String) {
    assert!(
        matches!(
            adapter::check_str(&source),
            Err(adapter::Verdict::Reject(_))
        ),
        "invalid let must be rejected even when its result is unused"
    );
}
#[test]
fn a_new_annotation_is_checked_even_for_a_previously_checked_value() {
    let mut s = Source::new();
    let body = s.let_(s.prop, s.prop, s.prop);
    rejects(s.finish(body));
}
#[test]
fn a_new_value_is_checked_even_for_a_previously_checked_annotation() {
    let mut s = Source::new();
    let body = s.let_(s.ty, s.ty, s.prop);
    rejects(s.finish(body));
}
#[test]
fn source_open_values_cannot_reuse_checks_after_different_substitutions() {
    let mut s = Source::new();
    let observer = s.let_(s.ty, s.variable, s.prop);
    let good = s.let_(s.ty, s.prop, observer);
    let bad = s.let_(s.higher, s.ty, observer);
    let body = s.pair(good, bad);
    rejects(s.finish(body));
}
#[test]
fn source_open_annotations_cannot_reuse_checks_after_different_substitutions() {
    let mut s = Source::new();
    let observer = s.let_(s.variable, s.prop, s.prop);
    let good = s.let_(s.higher, s.ty, observer);
    let bad = s.let_(s.ty, s.prop, observer);
    let body = s.pair(good, bad);
    rejects(s.finish(body));
}
#[test]
fn repeated_open_pairs_still_accept_when_each_substitution_is_valid() {
    let mut s = Source::new();
    let observer = s.let_(s.ty, s.variable, s.prop);
    let first = s.let_(s.ty, s.prop, observer);
    let second = s.let_(s.ty, s.prop, observer);
    let body = s.pair(first, second);
    adapter::check_str(&s.finish(body)).expect("valid source-local let values");
}
