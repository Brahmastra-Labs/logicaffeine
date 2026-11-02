//! Wave 3 — `On <event> from <source>:` — the generalized, statement-level async event handler.
//!
//! Today Logos only has the *view-child* `on click:` form (attached to the enclosing element
//! inside a `Render:`/`View:` block). This wave lifts event handling to a first-class statement
//! with an EXPLICIT source — `On click from button:` — the shape media orchestration needs
//! (`On chunk from encoder:`, `On track from peer:`, `On candidate from connection:`). It
//! desugars to the same `addEventListener(source, "event", closure)` machinery, so all three
//! tiers register the identical listener; here we lock that registration via the DOM op-log.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::semantics::dom::{self, DomOp};

fn interp_log(src: &str) -> Vec<DomOp> {
    dom::reset();
    let _ = tw_outcome(src);
    dom::op_log()
}

fn vm_log(src: &str) -> Vec<DomOp> {
    dom::reset();
    let _ = vm_outcome(src);
    dom::op_log()
}

/// The tree-walker and the register VM must build the identical DOM op-log, and it must equal
/// `expected` — the same parity discipline `wasm_dom.rs` uses for the view-child handlers.
fn assert_dom_parity(src: &str, expected: &[DomOp]) {
    let interp = interp_log(src);
    let vm = vm_log(src);
    assert_eq!(interp, vm, "tree-walker vs VM DOM op-log diverged:\n{src}");
    assert_eq!(interp, expected, "DOM op-log mismatch:\n{src}");
}

/// `On <event> from <source>:` registers a listener on the named source for the named event —
/// the generalized statement form, attaching to an explicit source rather than the enclosing
/// view element.
#[test]
fn on_event_from_source_registers_a_listener() {
    let src = "## Main\n\
        Let button be createElement(\"button\").\n\
        Call appendChild with body() and button.\n\
        On click from button:\n\
        \x20   Call setText with button and \"clicked\".\n";
    assert_dom_parity(
        src,
        &[
            DomOp::CreateElement {
                node: 1,
                tag: "button".into(),
            },
            DomOp::AppendChild {
                parent: 0,
                child: 1,
            },
            DomOp::AddEventListener {
                node: 1,
                event: "click".into(),
            },
        ],
    );
}

/// The event name is taken verbatim, so media/peer events register under their own names.
#[test]
fn on_event_from_source_uses_the_named_event_verbatim() {
    let src = "## Main\n\
        Let el be createElement(\"video\").\n\
        Call appendChild with body() and el.\n\
        On play from el:\n\
        \x20   Call setText with el and \"playing\".\n";
    assert_dom_parity(
        src,
        &[
            DomOp::CreateElement {
                node: 1,
                tag: "video".into(),
            },
            DomOp::AppendChild {
                parent: 0,
                child: 1,
            },
            DomOp::AddEventListener {
                node: 1,
                event: "play".into(),
            },
        ],
    );
}
