//! ★ LOCK — `applyPatches(bytes)` drives the DOM from a buffer, in ONE call. ★
//!
//! The patch stream reaches the language. A program builds a buffer of operations and hands it
//! over once, instead of making a host call per operation — which is what lets the browser host
//! be a `for` loop over a stream rather than fifty entry points with behaviour behind them.
//!
//! The buffer is DATA, and data written by a program can be wrong, so the failure cases are the
//! ones that matter: a malformed buffer must fail LOUDLY, at the call, naming what is wrong.
//! Silently applying the prefix that happened to parse would leave a half-built page and nothing
//! anywhere saying so.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::semantics::dom::{self, DomOp};
use logicaffeine_compile::semantics::patch;

/// The LOGOS source of a byte list, as a literal.
fn bytes_literal(bytes: &[u8]) -> String {
    let items: Vec<String> = bytes.iter().map(|b| b.to_string()).collect();
    format!("[{}]", items.join(", "))
}

/// Run a program that applies `ops` as a stream, and return what it rendered.
fn apply_via_logos(ops: &[DomOp]) -> String {
    let src = format!(
        "## Main\nLet patches be {}.\nCall applyPatches with patches.\n",
        bytes_literal(&patch::encode(ops))
    );
    dom::reset();
    let out = tw_outcome(&src);
    assert_eq!(out.error, None, "the program must run cleanly:\n{src}");
    dom::render()
}

/// What the same ops render when applied directly — the oracle.
fn apply_directly(ops: &[DomOp]) -> String {
    dom::reset();
    patch::apply(ops).expect("the host honours every id the stream names");
    dom::render()
}

#[test]
fn a_buffer_builds_the_document_it_describes() {
    let ops = vec![
        DomOp::CreateElement {
            node: 1,
            tag: "p".into(),
        },
        DomOp::SetText {
            node: 1,
            text: "hello".into(),
        },
        DomOp::AppendChild {
            parent: 0,
            child: 1,
        },
    ];
    assert_eq!(apply_via_logos(&ops), "<body><p>hello</p></body>");
}

#[test]
fn the_buffer_and_the_direct_calls_agree() {
    // The stream is only an interface if it means the same thing as the calls it replaces.
    let ops = vec![
        DomOp::CreateElement {
            node: 1,
            tag: "div".into(),
        },
        DomOp::SetAttribute {
            node: 1,
            name: "class".into(),
            value: "card".into(),
        },
        DomOp::CreateElement {
            node: 2,
            tag: "input".into(),
        },
        DomOp::SetAttribute {
            node: 2,
            name: "type".into(),
            value: "checkbox".into(),
        },
        DomOp::SetChecked {
            node: 2,
            checked: true,
        },
        DomOp::AppendChild {
            parent: 1,
            child: 2,
        },
        DomOp::AppendChild {
            parent: 0,
            child: 1,
        },
    ];
    assert_eq!(apply_via_logos(&ops), apply_directly(&ops));
}

#[test]
fn an_empty_buffer_is_a_no_op_and_not_a_failure() {
    // A paint that changed nothing is normal; failing on it would make "nothing to do"
    // indistinguishable from a bug.
    assert_eq!(apply_via_logos(&[]), "<body></body>");
}

#[test]
fn text_with_awkward_bytes_arrives_intact() {
    let ops = vec![
        DomOp::CreateElement {
            node: 1,
            tag: "p".into(),
        },
        DomOp::SetText {
            node: 1,
            text: "héllo → 世界 \"quoted\"".into(),
        },
        DomOp::AppendChild {
            parent: 0,
            child: 1,
        },
    ];
    assert_eq!(apply_via_logos(&ops), apply_directly(&ops));
}

#[test]
fn a_malformed_buffer_fails_loudly_at_the_call() {
    // The case this lock exists for. Applying the prefix that happened to parse would leave a
    // half-built page with nothing anywhere saying so.
    let src = "## Main\nCall applyPatches with [99, 1, 2, 3].\n";
    dom::reset();
    let err = tw_outcome(src)
        .error
        .unwrap_or_else(|| panic!("a bad buffer must fail:\n{src}"));
    assert!(
        err.contains("opcode") || err.contains("patch"),
        "the failure says what is wrong with the buffer: {err}"
    );
}

#[test]
fn a_truncated_buffer_applies_nothing_at_all() {
    // All-or-nothing: the document must not be left carrying the records that happened to
    // precede the damage.
    let mut bytes = patch::encode(&[
        DomOp::CreateElement {
            node: 1,
            tag: "p".into(),
        },
        DomOp::AppendChild {
            parent: 0,
            child: 1,
        },
    ]);
    bytes.truncate(bytes.len() - 2);
    let src = format!(
        "## Main\nCall applyPatches with {}.\n",
        bytes_literal(&bytes)
    );
    dom::reset();
    let err = tw_outcome(&src)
        .error
        .unwrap_or_else(|| panic!("a truncated buffer must fail"));
    assert!(!err.is_empty());
    assert_eq!(dom::render(), "<body></body>", "nothing was applied");
}

#[test]
fn a_byte_outside_a_bytes_range_is_refused() {
    // The buffer is a `Seq of Int`, so it can hold values a byte cannot. A silent truncation to
    // the low 8 bits would turn a wrong number into a different, valid-looking operation.
    let src = "## Main\nCall applyPatches with [300].\n";
    dom::reset();
    let err = tw_outcome(src)
        .error
        .unwrap_or_else(|| panic!("an out-of-range byte must fail"));
    assert!(
        err.contains("300") || err.contains("byte"),
        "it names the problem: {err}"
    );
}

#[test]
fn a_negative_byte_is_refused() {
    let src = "## Main\nCall applyPatches with [0 - 1].\n";
    dom::reset();
    let err = tw_outcome(src)
        .error
        .unwrap_or_else(|| panic!("a negative byte must fail"));
    assert!(
        err.contains("byte") || err.contains("-1"),
        "it names the problem: {err}"
    );
}

#[test]
fn both_tiers_apply_the_same_buffer_the_same_way() {
    // The stream crosses tiers, so the tiers have to agree about it.
    let ops = vec![
        DomOp::ClearChildren { node: 0 },
        DomOp::CreateElement {
            node: 1,
            tag: "h1".into(),
        },
        DomOp::SetText {
            node: 1,
            text: "title".into(),
        },
        DomOp::AppendChild {
            parent: 0,
            child: 1,
        },
    ];
    let src = format!(
        "## Main\nLet patches be {}.\nCall applyPatches with patches.\n",
        bytes_literal(&patch::encode(&ops))
    );
    dom::reset();
    assert_eq!(tw_outcome(&src).error, None);
    let tw_render = dom::render();
    let tw_log = dom::op_log();
    dom::reset();
    assert_eq!(vm_outcome(&src).error, None, "the VM must apply it too");
    assert_eq!(
        dom::render(),
        tw_render,
        "the tiers built different documents"
    );
    assert_eq!(dom::op_log(), tw_log, "the tiers issued different ops");
}

#[test]
fn both_tiers_refuse_a_malformed_buffer_the_same_way() {
    let src = "## Main\nCall applyPatches with [99, 1].\n";
    dom::reset();
    let tw = tw_outcome(src)
        .error
        .unwrap_or_else(|| panic!("tw must refuse"));
    dom::reset();
    let vm = vm_outcome(src)
        .error
        .unwrap_or_else(|| panic!("the VM must refuse too"));
    assert_eq!(
        tw, vm,
        "the two tiers refuse the same buffer in different words"
    );
}
