//! Tabs on a shared topic, for the locks on `## Store … shared on "topic"`: each tab a lowered page
//! with its own document and its own storage profile, swapped in for exactly the call that runs on
//! it, on both lowered engines — the tree-walker hosting the lowered page and the bytecode VM. The
//! lock plays the relay: it reads what one page holds or published and hands it to another through
//! the page's `__lgx_shared`, as a relay host does.

#![allow(dead_code)]

use logicaffeine_compile::interpreter::{Interpreter, RuntimeValue};
use logicaffeine_compile::page_host::{Drive, LoweredCalls, LoweredPage};
use logicaffeine_compile::semantics::{dom, storage};
use logicaffeine_compile::ui_bridge::with_parsed_program;
use logicaffeine_compile::vm::session::Session;
use logicaffeine_compile::vm::{Compiler, Vm};
use logicaffeine_language::parser::lgx_lower::shared_arrival_fn_name;
use std::rc::Rc;

/// A page in a tab, as the relay sees it.
pub trait Tab {
    /// `Click SEL.`, carried out by the host every lowered tier shares; the click fails nothing.
    fn click(&mut self, selector: &str);
    /// `Click SEL.`, answering the failure it leaves the page with, if any.
    fn click_failing(&mut self, selector: &str) -> Option<String>;
    /// A frame another page published on `topic` for `key`, handed to this page.
    fn arrive(&mut self, topic: &str, key: &str, frame: &str);
    /// The frame this page holds for `key`.
    fn held(&mut self, key: &str) -> Option<String>;
    /// What this page wrote to its topics since the last look — what a relay would carry off.
    fn published(&mut self) -> Vec<(String, String, String)>;
    /// The page's document.
    fn document(&mut self) -> String;
}

/// A lowered page with its own document and its own storage profile.
struct Replica<L: LoweredCalls> {
    page: LoweredPage<L>,
    storage: Option<storage::StorageState>,
    dom: Option<dom::DomState>,
}

impl<L: LoweredCalls> Replica<L> {
    /// A fresh tab: the page's main runs and `App` mounts, against an empty document and a storage
    /// profile whose topics hold `topic` — the frames a relay hands a page that subscribes.
    fn open(
        engine: L,
        topic: &[(&str, &str)],
        main: impl FnOnce(&mut L) -> Result<(), String>,
    ) -> Self {
        storage::reset();
        for (key, frame) in topic {
            storage::set(storage::SHARED, key, frame).expect("the shared area exists");
        }
        storage::take_shared_writes();
        let mut tab = Replica {
            page: LoweredPage::new(engine),
            storage: Some(storage::take()),
            dom: Some(dom::DomState::default()),
        };
        tab.inside(|page| {
            page.start(None, main)
                .unwrap_or_else(|e| panic!("the page starts: {e:?}"))
        });
        tab.inside(|page| {
            page.drive(&Drive::Mount {
                component: "App".to_string(),
                at: None,
            })
            .unwrap_or_else(|e| panic!("App mounts: {e:?}"))
        });
        tab
    }

    /// Run `f` with this tab's document and storage in place.
    fn inside<R>(&mut self, f: impl FnOnce(&mut LoweredPage<L>) -> R) -> R {
        storage::restore(
            self.storage
                .take()
                .expect("the tab's storage is put back after every call"),
        );
        dom::restore(
            self.dom
                .take()
                .expect("the tab's document is put back after every call"),
        );
        let out = f(&mut self.page);
        self.dom = Some(dom::take());
        self.storage = Some(storage::take());
        out
    }
}

fn text(s: &str) -> RuntimeValue {
    RuntimeValue::Text(Rc::new(s.to_string()))
}

impl<L: LoweredCalls> Tab for Replica<L> {
    fn click(&mut self, selector: &str) {
        self.inside(|page| {
            page.drive(&Drive::Click {
                selector: selector.to_string(),
            })
            .unwrap_or_else(|e| panic!("`Click \"{selector}\"` runs: {e:?}"));
            assert_eq!(
                page.unhandled_failure(),
                None,
                "`Click \"{selector}\"` fails nothing"
            );
        });
    }

    fn click_failing(&mut self, selector: &str) -> Option<String> {
        self.inside(|page| {
            page.drive(&Drive::Click {
                selector: selector.to_string(),
            })
            .unwrap_or_else(|e| panic!("`Click \"{selector}\"` runs: {e:?}"));
            page.unhandled_failure().map(str::to_string)
        })
    }

    fn arrive(&mut self, topic: &str, key: &str, frame: &str) {
        self.inside(|page| {
            page.calls()
                .call(
                    shared_arrival_fn_name(),
                    vec![text(topic), text(key), text(frame)],
                )
                .unwrap_or_else(|e| panic!("the page takes the frame {frame:?} for {key}: {e}"));
        });
    }

    fn held(&mut self, key: &str) -> Option<String> {
        self.inside(|_| storage::get(storage::SHARED, key).expect("the shared area exists"))
    }

    fn published(&mut self) -> Vec<(String, String, String)> {
        self.inside(|_| storage::take_shared_writes())
    }

    fn document(&mut self) -> String {
        self.inside(|_| dom::render())
    }
}

/// `n` fresh tabs of `src` on each lowered engine, handed to `scenario` with the engine's name.
pub fn on_each_tier(src: &str, n: usize, scenario: impl for<'x> Fn(&str, Vec<Box<dyn Tab + 'x>>)) {
    on_each_tier_subscribed(src, n, &[], scenario);
}

/// [`on_each_tier`], every tab opening on a topic that already holds `topic`.
pub fn on_each_tier_subscribed(
    src: &str,
    n: usize,
    topic: &[(&str, &str)],
    scenario: impl for<'x> Fn(&str, Vec<Box<dyn Tab + 'x>>),
) {
    storage::reset();
    dom::reset();
    with_parsed_program(src, |parsed, interner| {
        let (stmts, types, policies) = parsed.unwrap_or_else(|e| panic!("parses: {e}\n{src}"));
        let tabs: Vec<Box<dyn Tab>> = (0..n)
            .map(|_| {
                let engine = Interpreter::new(interner)
                    .with_type_registry(types)
                    .with_policies(policies.clone());
                Box::new(Replica::open(engine, topic, |i| i.run_page_main(stmts))) as Box<dyn Tab>
            })
            .collect();
        scenario("the tree-walker hosting the lowered page", tabs);
    });
    with_parsed_program(src, |parsed, interner| {
        let (stmts, types, _) = parsed.unwrap_or_else(|e| panic!("parses: {e}\n{src}"));
        let program = Compiler::compile_with_types(stmts, interner, Some(types))
            .unwrap_or_else(|e| panic!("compiles: {e}\n{src}"));
        let tabs: Vec<Box<dyn Tab + '_>> = (0..n)
            .map(|_| {
                let engine = Session::new(Vm::new(&program), interner);
                Box::new(Replica::open(engine, topic, |s| s.run_main())) as Box<dyn Tab + '_>
            })
            .collect();
        scenario("the bytecode VM", tabs);
    });
}

/// The text of the element `id` in `document`.
pub fn shown(document: &str, id: &str) -> String {
    let open = format!("<p id=\"{id}\">");
    let start = document
        .find(&open)
        .unwrap_or_else(|| panic!("#{id} is on the page: {document}"))
        + open.len();
    let end = start + document[start..].find("</p>").expect("the element closes");
    document[start..end].to_string()
}

/// The texts of the `<li>` items inside the element `id` in `document`, in order.
pub fn items(document: &str, id: &str) -> Vec<String> {
    let open = format!(" id=\"{id}\">");
    let start = document
        .find(&open)
        .unwrap_or_else(|| panic!("#{id} is on the page: {document}"))
        + open.len();
    let close = ["</ul>", "</ol>"]
        .iter()
        .filter_map(|c| document[start..].find(c))
        .min()
        .expect("the list closes");
    document[start..start + close]
        .split("<li>")
        .skip(1)
        .map(|item| item.split("</li>").next().unwrap_or("").to_string())
        .collect()
}
