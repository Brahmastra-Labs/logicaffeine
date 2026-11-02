//! B1–B4 — the LOGOS→WASM DOM tier.
//!
//! DOM functions are `BuiltinId`s; on the tree-walker and VM they land in the
//! thread-local `semantics::dom::DomRecorder`, on wasm in `dom_*` host imports.
//! The sacred invariant is that all three tiers produce the SAME DOM op-log for
//! the same program — `assert_dom_parity` checks it (the wasm arm needs
//! `--features wasm-jit`; without it the interp/VM parity still runs).

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::semantics::dom::{self, DomOp};

#[path = "compile_common/mod.rs"]
mod common;

/// The first id a page's patch stream names (`vdom.lg`'s `vdWire`): a node the stream draws is
/// `S + k`, apart from every id the host mints itself.
const S: u32 = dom::STREAM_ID_BASE;

/// Run `src` on the tree-walker, capturing the DOM op-log it built.
fn interp_log(src: &str) -> Vec<DomOp> {
    dom::reset();
    let _ = tw_outcome(src);
    dom::op_log()
}

/// Run `src` on the register VM, capturing the DOM op-log it built.
fn vm_log(src: &str) -> Vec<DomOp> {
    dom::reset();
    let _ = vm_outcome(src);
    dom::op_log()
}

/// The ops `src` records after its mount, on the tree-walker: its log past what the same program
/// cut after its `Mount` line records. The two runs share that prefix op for op, so what is left
/// is exactly the drives' work — a turn's own ops, however the paint that follows is spelled.
fn after_the_mount(src: &str) -> Vec<DomOp> {
    let cut = src
        .find("\nMount ")
        .and_then(|at| src[at + 1..].find('\n').map(|end| at + 1 + end + 1));
    let mount_only = &src[..cut.unwrap_or_else(|| panic!("the program mounts:\n{src}"))];
    let mount = interp_log(mount_only);
    let whole = interp_log(src);
    assert_eq!(
        &whole[..mount.len()],
        mount.as_slice(),
        "the drives start from the mount's own document:\n{src}"
    );
    whole[mount.len()..].to_vec()
}

/// Assert the tree-walker and VM build the identical DOM op-log, and that it
/// equals `expected`. (The wasm(MockDom) arm is added under `--features wasm-jit`.)
fn assert_dom_parity(src: &str, expected: &[DomOp]) {
    let interp = interp_log(src);
    let vm = vm_log(src);
    assert_eq!(interp, vm, "tree-walker vs VM DOM op-log diverged:\n{src}");
    assert_eq!(interp, expected, "DOM op-log mismatch:\n{src}");
}

#[test]
fn dom_tree_builds_identically_on_interp_and_vm() {
    let src = "## Main\n\
        Let d be createElement(\"div\").\n\
        Call setText with d and \"hi\".\n\
        Let b be body().\n\
        Call appendChild with b and d.\n";
    assert_dom_parity(
        src,
        &[
            DomOp::CreateElement {
                node: 1,
                tag: "div".into(),
            },
            DomOp::SetText {
                node: 1,
                text: "hi".into(),
            },
            DomOp::AppendChild {
                parent: 0,
                child: 1,
            },
        ],
    );
}

#[test]
fn dom_attributes_and_nested_children() {
    let src = "## Main\n\
        Let ul be createElement(\"ul\").\n\
        Call setAttribute with ul and \"id\" and \"list\".\n\
        Let li be createElement(\"li\").\n\
        Call setText with li and \"item\".\n\
        Call appendChild with ul and li.\n\
        Call appendChild with body() and ul.\n";
    assert_dom_parity(
        src,
        &[
            DomOp::CreateElement {
                node: 1,
                tag: "ul".into(),
            },
            DomOp::SetAttribute {
                node: 1,
                name: "id".into(),
                value: "list".into(),
            },
            DomOp::CreateElement {
                node: 2,
                tag: "li".into(),
            },
            DomOp::SetText {
                node: 2,
                text: "item".into(),
            },
            DomOp::AppendChild {
                parent: 1,
                child: 2,
            },
            DomOp::AppendChild {
                parent: 0,
                child: 1,
            },
        ],
    );
}

#[test]
fn set_html_and_clear_children_record_ops_on_both_tiers() {
    // `setHtml` drops raw markup into an element (innerHTML); `clearChildren`
    // empties it — the immediate-mode re-render primitive. Both record identically
    // on the tree-walker and the VM.
    let src = "## Main\n\
        Let root be createElement(\"div\").\n\
        Call setHtml with root and \"<b>hi</b>\".\n\
        Call appendChild with body() and root.\n\
        Call clearChildren with root.\n";
    assert_dom_parity(
        src,
        &[
            DomOp::CreateElement {
                node: 1,
                tag: "div".into(),
            },
            DomOp::SetHtml {
                node: 1,
                html: "<b>hi</b>".into(),
            },
            DomOp::AppendChild {
                parent: 0,
                child: 1,
            },
            DomOp::ClearChildren { node: 1 },
        ],
    );
}

// ---- Elegant English DOM sugar (desugars to the builtins). ----

#[test]
fn set_the_text_of_sugar_desugars_to_set_text() {
    // The sugary `Set the text of X to Y` and the explicit `Call setText …` produce
    // the identical DOM op-log on both tiers.
    let sugar = "## Main\n\
        Let heading be createElement(\"h1\").\n\
        Set the text of heading to \"Welcome\".\n\
        Call appendChild with body() and heading.\n";
    let explicit = "## Main\n\
        Let heading be createElement(\"h1\").\n\
        Call setText with heading and \"Welcome\".\n\
        Call appendChild with body() and heading.\n";
    let expected = [
        DomOp::CreateElement {
            node: 1,
            tag: "h1".into(),
        },
        DomOp::SetText {
            node: 1,
            text: "Welcome".into(),
        },
        DomOp::AppendChild {
            parent: 0,
            child: 1,
        },
    ];
    assert_dom_parity(sugar, &expected);
    assert_dom_parity(explicit, &expected);
}

// ---- LGX: the same DOM, written in the language that REPLACES the dialect above. ----
//
// Every test in this section is the 1:1 twin of one above it, asserting the same DOM the same
// way — `assert_dom_parity`, so the tree-walker and the VM must agree op for op. Keeping both
// forms side by side is what makes retiring the dialect a deletion rather than a leap: while
// both are here, the twin proves LGX renders what its predecessor rendered.
//
// Three representational differences are deliberate and better, not regressions:
//   * an interpolated hole becomes its OWN text node (`"Clicks: "` then `"3"`) rather than one
//     `setText` of the joined string, which is what lets a later fine-grained update patch the
//     hole alone;
//   * a mount CLEARS its mount point first, so mounting is idempotent and cannot double the page;
//     and
//   * a component's nodes arrive as its paint's committed stream (`vdom.lg`): named by the ids
//     the stream gives them (`S + k`), each attached as it is drawn, its children after it, and a
//     turn as only the writes to the nodes that changed.

/// The LGX twin of `view_card_desugars_to_dom_builtins`.
#[test]
fn lgx_card_renders_a_div_with_a_heading_and_an_interpolated_paragraph() {
    let src = concat!(
        "## Component App\n",
        "State:\n",
        "    Let count be 3.\n",
        "View:\n",
        "    <div class=\"card\"><h1>Welcome back</h1><p class=\"muted\">Clicks: {count}</p></div>\n",
        "\n## Main\nMount App.\n",
    );
    assert_dom_parity(
        src,
        &[
            DomOp::ClearChildren { node: 0 },
            DomOp::CreateElement {
                node: S + 3,
                tag: "div".into(),
            },
            DomOp::SetAttribute {
                node: S + 3,
                name: "class".into(),
                value: "card".into(),
            },
            DomOp::AppendChild {
                parent: 0,
                child: S + 3,
            },
            DomOp::CreateElement {
                node: S + 4,
                tag: "h1".into(),
            },
            DomOp::CreateElement {
                node: S + 5,
                tag: "p".into(),
            },
            DomOp::SetAttribute {
                node: S + 5,
                name: "class".into(),
                value: "muted".into(),
            },
            DomOp::AppendChild {
                parent: S + 3,
                child: S + 4,
            },
            DomOp::AppendChild {
                parent: S + 3,
                child: S + 5,
            },
            DomOp::CreateTextNode {
                node: S + 6,
                text: "Clicks: ".into(),
            },
            DomOp::CreateTextNode {
                node: S + 7,
                text: "3".into(),
            },
            DomOp::AppendChild {
                parent: S + 5,
                child: S + 6,
            },
            DomOp::AppendChild {
                parent: S + 5,
                child: S + 7,
            },
            DomOp::CreateTextNode {
                node: S + 8,
                text: "Welcome back".into(),
            },
            DomOp::AppendChild {
                parent: S + 4,
                child: S + 8,
            },
        ],
    );
}

/// The LGX twin of `view_inline_click_handler_registers_a_listener`.
///
/// The registration is the whole point: a page reaches a browser as a stream of DOM calls, so a
/// handler that exists only in the runtime's own table is a handler the browser never hears
/// about. Both tiers must SAY `listenFlags` — the registration every tier records, whose event the
/// page's node dispatcher answers.
#[test]
fn lgx_on_click_registers_a_listener_on_both_tiers() {
    let src = concat!(
        "## Component App\n",
        "State:\n",
        "    Let n be 0.\n",
        "Actions:\n",
        "    To go:\n",
        "        Set n to n + 1.\n",
        "View:\n",
        "    <button class=\"primary\" on-click={go}>go</button>\n",
        "\n## Main\nMount App.\n",
    );
    assert_dom_parity(
        src,
        &[
            DomOp::ClearChildren { node: 0 },
            DomOp::CreateElement {
                node: S + 3,
                tag: "button".into(),
            },
            DomOp::SetAttribute {
                node: S + 3,
                name: "class".into(),
                value: "primary".into(),
            },
            DomOp::Listen {
                node: S + 3,
                event: "click".into(),
                flags: 0,
            },
            DomOp::AppendChild {
                parent: 0,
                child: S + 3,
            },
            DomOp::CreateTextNode {
                node: S + 4,
                text: "go".into(),
            },
            DomOp::AppendChild {
                parent: S + 3,
                child: S + 4,
            },
        ],
    );
}

/// The two ways to say "when this is clicked, do that" — the attribute on the element and the
/// sentence in `Actions:` — attach a click listener to the same element, at the same moment,
/// on both tiers; the next three tests pin that.
///
/// The attribute form attaches WHILE THE ELEMENT IS BUILT: after the element's attributes,
/// before its children.
#[test]
fn lgx_an_on_click_attribute_attaches_while_the_element_is_built() {
    let src = concat!(
        "## Component App\n",
        "State:\n    Let n be 0.\n",
        "Actions:\n    To go:\n        Set n to n + 1.\n",
        "View:\n    <button id=\"b\" on-click={go}>go</button>\n",
        "\n## Main\nMount App.\n",
    );
    assert_dom_parity(
        src,
        &[
            DomOp::ClearChildren { node: 0 },
            DomOp::CreateElement {
                node: S + 3,
                tag: "button".into(),
            },
            DomOp::SetAttribute {
                node: S + 3,
                name: "id".into(),
                value: "b".into(),
            },
            DomOp::Listen {
                node: S + 3,
                event: "click".into(),
                flags: 0,
            },
            DomOp::AppendChild {
                parent: 0,
                child: S + 3,
            },
            DomOp::CreateTextNode {
                node: S + 4,
                text: "go".into(),
            },
            DomOp::AppendChild {
                parent: S + 3,
                child: S + 4,
            },
        ],
    );
}

/// The sentence form names its element by SELECTOR, and the paint answers the selector from the
/// records it has just drawn — so the element carries the listener when it is committed, and the
/// sentence attaches exactly where the attribute form does, op for op.
#[test]
fn lgx_a_when_sentence_attaches_where_the_attribute_form_does() {
    let src = concat!(
        "## Component App\n",
        "State:\n    Let n be 0.\n",
        "Actions:\n    When \"#b\" is clicked:\n        Set n to n + 1.\n",
        "View:\n    <button id=\"b\">go</button>\n",
        "\n## Main\nMount App.\n",
    );
    assert_dom_parity(
        src,
        &[
            DomOp::ClearChildren { node: 0 },
            DomOp::CreateElement {
                node: S + 3,
                tag: "button".into(),
            },
            DomOp::SetAttribute {
                node: S + 3,
                name: "id".into(),
                value: "b".into(),
            },
            DomOp::Listen {
                node: S + 3,
                event: "click".into(),
                flags: 0,
            },
            DomOp::AppendChild {
                parent: 0,
                child: S + 3,
            },
            DomOp::CreateTextNode {
                node: S + 4,
                text: "go".into(),
            },
            DomOp::AppendChild {
                parent: S + 3,
                child: S + 4,
            },
        ],
    );
}

/// What the two forms SHARE, which is the part that matters to whoever writes one: a click runs
/// the action and the page shows the result, on both tiers, whichever way it was written. This
/// is the assertion the merged test was really making; the op ORDER never was.
#[test]
fn lgx_both_handler_forms_answer_a_click_on_both_tiers() {
    let page = |actions: &str| -> String {
        let src = format!(
            "## Component App\nState:\n    Let n be 0.\n{actions}\
             View:\n    <button id=\"b\"{attr}>go</button>\n    <p>{{n}}</p>\n\
             \n## Main\nMount App.\nClick \"#b\".\n",
            attr = if actions.contains("To go") {
                " on-click={go}"
            } else {
                ""
            },
        );
        dom::reset();
        let tw = tw_outcome(&src);
        assert_eq!(tw.error, None, "the tree-walker must run it:\n{src}");
        let rendered = dom::render();
        dom::reset();
        let vm = vm_outcome(&src);
        assert_eq!(vm.error, None, "the VM must run it:\n{src}");
        assert_eq!(
            dom::render(),
            rendered,
            "the two tiers rendered differently:\n{src}"
        );
        rendered
    };
    let attribute = page("Actions:\n    To go:\n        Set n to n + 1.\n");
    let sentence = page("Actions:\n    When \"#b\" is clicked:\n        Set n to n + 1.\n");
    assert!(
        attribute.contains("<p>1</p>"),
        "the attribute form ran its action: {attribute}"
    );
    assert_eq!(
        attribute, sentence,
        "both forms must leave the page in the same state"
    );
}

/// The LGX twin of `view_raw_child_drops_html_into_the_element`: `{raw h}` is the
/// HTML-interchange escape hatch, and it is one `setHtml` call on both tiers.
#[test]
fn lgx_raw_hole_sets_the_elements_html_on_both_tiers() {
    let src = concat!(
        "## Component App\n",
        "State:\n",
        "    Let h be \"<b>hi</b>\".\n",
        "View:\n",
        "    <div>{raw h}</div>\n",
        "\n## Main\nMount App.\n",
    );
    assert_dom_parity(
        src,
        &[
            DomOp::ClearChildren { node: 0 },
            DomOp::CreateElement {
                node: S + 3,
                tag: "div".into(),
            },
            DomOp::SetHtml {
                node: S + 3,
                html: "<b>hi</b>".into(),
            },
            DomOp::AppendChild {
                parent: 0,
                child: S + 3,
            },
        ],
    );
}

/// The LGX twin of `view_css_child_makes_a_style_element_with_verbatim_braces` and of
/// `css_plain_string_with_braces_needs_no_raw_prefix`.
///
/// `Style:` is REAL CSS in the file, so the papercut those two tests were written about — CSS
/// braces colliding with interpolation — cannot arise: there is no string literal to escape.
/// The rules arrive scoped, which is the next twin's subject.
#[test]
fn lgx_style_block_emits_one_scoped_style_element_on_both_tiers() {
    let src = concat!(
        "## Component App\n",
        "View:\n",
        "    <div><p class=\"box\">a</p></div>\n",
        "Style:\n",
        "    .box { color: red }\n",
        "\n## Main\nMount App.\n",
    );
    assert_dom_parity(
        src,
        &[
            DomOp::ClearChildren { node: 0 },
            DomOp::CreateElement {
                node: S + 2,
                tag: "style".into(),
            },
            DomOp::AppendChild {
                parent: 0,
                child: S + 2,
            },
            DomOp::CreateElement {
                node: S + 4,
                tag: "div".into(),
            },
            DomOp::SetAttribute {
                node: S + 4,
                name: "class".into(),
                value: "lgx-app".into(),
            },
            DomOp::AppendChild {
                parent: 0,
                child: S + 4,
            },
            DomOp::CreateElement {
                node: S + 5,
                tag: "p".into(),
            },
            DomOp::SetAttribute {
                node: S + 5,
                name: "class".into(),
                value: "box lgx-app".into(),
            },
            DomOp::AppendChild {
                parent: S + 4,
                child: S + 5,
            },
            DomOp::CreateTextNode {
                node: S + 6,
                text: "a".into(),
            },
            DomOp::AppendChild {
                parent: S + 5,
                child: S + 6,
            },
            DomOp::CreateTextNode {
                node: S + 7,
                text: ".box.lgx-app { color: red }".into(),
            },
            DomOp::AppendChild {
                parent: S + 2,
                child: S + 7,
            },
        ],
    );
}

/// The LGX twin of `view_for_each_renders_one_element_per_item`.
#[test]
fn lgx_for_each_renders_one_element_per_item_on_both_tiers() {
    let src = concat!(
        "## Component App\n",
        "State:\n",
        "    Let items be [\"a\", \"b\", \"c\"].\n",
        "View:\n",
        "    <ul>{for each item in items:}<li>{item}</li>{end}</ul>\n",
        "\n## Main\nMount App.\n",
    );
    assert_dom_parity(
        src,
        &[
            DomOp::ClearChildren { node: 0 },
            DomOp::CreateElement {
                node: S + 3,
                tag: "ul".into(),
            },
            DomOp::AppendChild {
                parent: 0,
                child: S + 3,
            },
            DomOp::CreateElement {
                node: S + 4,
                tag: "li".into(),
            },
            DomOp::CreateElement {
                node: S + 5,
                tag: "li".into(),
            },
            DomOp::CreateElement {
                node: S + 6,
                tag: "li".into(),
            },
            DomOp::AppendChild {
                parent: S + 3,
                child: S + 4,
            },
            DomOp::AppendChild {
                parent: S + 3,
                child: S + 5,
            },
            DomOp::AppendChild {
                parent: S + 3,
                child: S + 6,
            },
            DomOp::CreateTextNode {
                node: S + 7,
                text: "c".into(),
            },
            DomOp::AppendChild {
                parent: S + 6,
                child: S + 7,
            },
            DomOp::CreateTextNode {
                node: S + 8,
                text: "b".into(),
            },
            DomOp::AppendChild {
                parent: S + 5,
                child: S + 8,
            },
            DomOp::CreateTextNode {
                node: S + 9,
                text: "a".into(),
            },
            DomOp::AppendChild {
                parent: S + 4,
                child: S + 9,
            },
        ],
    );
}

/// The LGX twin of `view_when_otherwise_renders_the_matching_branch`, both ways round.
#[test]
fn lgx_if_else_renders_the_matching_branch_on_both_tiers() {
    let page = |ready: &str| {
        format!(
            "## Component App\nState:\n    Let ready be {ready}.\nView:\n    \
             <div>{{if ready:}}<p>yes</p>{{else:}}<span>no</span>{{end}}</div>\n\n## Main\nMount App.\n"
        )
    };
    assert_dom_parity(
        &page("true"),
        &[
            DomOp::ClearChildren { node: 0 },
            DomOp::CreateElement {
                node: S + 3,
                tag: "div".into(),
            },
            DomOp::AppendChild {
                parent: 0,
                child: S + 3,
            },
            DomOp::CreateElement {
                node: S + 4,
                tag: "p".into(),
            },
            DomOp::AppendChild {
                parent: S + 3,
                child: S + 4,
            },
            DomOp::CreateTextNode {
                node: S + 5,
                text: "yes".into(),
            },
            DomOp::AppendChild {
                parent: S + 4,
                child: S + 5,
            },
        ],
    );
    assert_dom_parity(
        &page("false"),
        &[
            DomOp::ClearChildren { node: 0 },
            DomOp::CreateElement {
                node: S + 3,
                tag: "div".into(),
            },
            DomOp::AppendChild {
                parent: 0,
                child: S + 3,
            },
            DomOp::CreateElement {
                node: S + 4,
                tag: "span".into(),
            },
            DomOp::AppendChild {
                parent: S + 3,
                child: S + 4,
            },
            DomOp::CreateTextNode {
                node: S + 5,
                text: "no".into(),
            },
            DomOp::AppendChild {
                parent: S + 4,
                child: S + 5,
            },
        ],
    );
}

/// ★ THE BULK CALL, END TO END on the wasm tier: a program builds a patch buffer, hands it over
/// in ONE host crossing, and the host shim walks it. This is the path Tristen's constraint
/// describes — the framework in compiled LOGOS, JavaScript a `for` loop over a stream — and it is
/// the only test that exercises the emitter, the wire format, the wasm import and a host decoder
/// together. Each half round-trips in `patch_stream_lock`; only this says they MEET.
// The wasm half lives behind the same gate its `MockDom` host does.
#[cfg(feature = "wasm-jit")]
#[test]
fn a_patch_buffer_applies_identically_on_all_three_tiers() {
    use logicaffeine_compile::semantics::patch;
    let ops = vec![
        DomOp::ClearChildren { node: 0 },
        DomOp::CreateElement {
            node: 1,
            tag: "div".into(),
        },
        DomOp::SetAttribute {
            node: 1,
            name: "class".into(),
            value: "card".into(),
        },
        DomOp::CreateTextNode {
            node: 2,
            text: "héllo → 世界".into(),
        },
        DomOp::AppendChild {
            parent: 1,
            child: 2,
        },
        DomOp::CreateElement {
            node: 3,
            tag: "input".into(),
        },
        DomOp::SetValue {
            node: 3,
            value: "typed".into(),
        },
        DomOp::SetChecked {
            node: 3,
            checked: true,
        },
        DomOp::AppendChild {
            parent: 1,
            child: 3,
        },
        DomOp::AppendChild {
            parent: 0,
            child: 1,
        },
    ];
    let literal: Vec<String> = patch::encode(&ops).iter().map(|b| b.to_string()).collect();
    let src = format!(
        "## Main\nLet patches be [{}].\nCall applyPatches with patches.\n",
        literal.join(", ")
    );
    assert_eq!(interp_log(&src), ops, "tree-walker");
    assert_eq!(vm_log(&src), ops, "VM");
    assert_eq!(wasm_tier::wasm_log(&src), ops, "wasm");
}

/// The LGX twin of `bound_to_wires_two_way_form_binding`: `bind={field}` seeds the input from
/// the field on every paint (`SetValue`) and routes the keystroke back to it.
#[test]
fn lgx_bind_seeds_the_input_from_its_field_on_both_tiers() {
    let src = concat!(
        "## Component App\n",
        "State:\n",
        "    Let title be \"hi\".\n",
        "View:\n",
        "    <input id=\"t\" type=\"text\" bind={title}>\n",
        "\n## Main\nMount App.\n",
    );
    assert_dom_parity(
        src,
        &[
            DomOp::ClearChildren { node: 0 },
            DomOp::CreateElement {
                node: S + 3,
                tag: "input".into(),
            },
            DomOp::SetAttribute {
                node: S + 3,
                name: "id".into(),
                value: "t".into(),
            },
            DomOp::SetAttribute {
                node: S + 3,
                name: "type".into(),
                value: "text".into(),
            },
            DomOp::SetValue {
                node: S + 3,
                value: "hi".into(),
            },
            DomOp::Listen {
                node: S + 3,
                event: "input".into(),
                flags: 0,
            },
            DomOp::AppendChild {
                parent: 0,
                child: S + 3,
            },
        ],
    );
}

/// The checkbox twin. A checkbox's state is `checked`, a PRESENCE on the live PROPERTY — not
/// `value` (which a checkbox ignores) and not the `checked` ATTRIBUTE (which is `defaultChecked`,
/// so writing it leaves the box the visitor sees exactly as it was).
///
/// This op-log is the assertion that can see the difference. A rendered tree cannot: both tiers
/// agreed on `value="true"` for months, which is why the parity lock never caught it. What a
/// browser is TOLD to do is the contract.
#[test]
fn lgx_a_bound_checkbox_sets_checked_and_never_a_value_on_both_tiers() {
    let src = concat!(
        "## Component App\n",
        "State:\n",
        "    Let on be true.\n",
        "View:\n",
        "    <input id=\"c\" type=\"checkbox\" bind={on}>\n",
        "\n## Main\nMount App.\n",
    );
    assert_dom_parity(
        src,
        &[
            DomOp::ClearChildren { node: 0 },
            DomOp::CreateElement {
                node: S + 3,
                tag: "input".into(),
            },
            DomOp::SetAttribute {
                node: S + 3,
                name: "id".into(),
                value: "c".into(),
            },
            DomOp::SetAttribute {
                node: S + 3,
                name: "type".into(),
                value: "checkbox".into(),
            },
            DomOp::SetChecked {
                node: S + 3,
                checked: true,
            },
            DomOp::Listen {
                node: S + 3,
                event: "input".into(),
                flags: 0,
            },
            DomOp::AppendChild {
                parent: 0,
                child: S + 3,
            },
        ],
    );
}

/// A `bind` on an element with NO id is still a page that renders — the write-back has no
/// selector to name the element by, but showing the field must not depend on that.
///
/// This is the shape that found a VM bug worth more than the feature: a component whose only
/// use of its mount point was a WRITE left that binding unpromoted, so the write landed on a
/// global that was never created. `a_function_that_only_writes_an_outer_name_still_binds_it`
/// in `vm_global_promotion_lock.rs` is that bug in plain LOGOS.
#[test]
fn lgx_bind_without_an_id_still_renders_on_both_tiers() {
    let src = concat!(
        "## Component App\n",
        "State:\n",
        "    Let title be \"hi\".\n",
        "View:\n",
        "    <input type=\"text\" bind={title}>\n",
        "\n## Main\nMount App.\n",
    );
    assert_dom_parity(
        src,
        &[
            DomOp::ClearChildren { node: 0 },
            DomOp::CreateElement {
                node: S + 3,
                tag: "input".into(),
            },
            DomOp::SetAttribute {
                node: S + 3,
                name: "type".into(),
                value: "text".into(),
            },
            DomOp::SetValue {
                node: S + 3,
                value: "hi".into(),
            },
            DomOp::Listen {
                node: S + 3,
                event: "input".into(),
                flags: 0,
            },
            DomOp::AppendChild {
                parent: 0,
                child: S + 3,
            },
        ],
    );
}

/// The LGX twin of `mount_without_at_defaults_to_hash_root`, turned the right way up.
///
/// The dialect's `Mount App` FOUND OR CREATED a `#root` div. LGX mounts under the page body,
/// and `Mount App at "#sel"` names a mount point that must already exist. What matters, and
/// what this pins, is that the two tiers agree about which it is.
#[test]
fn lgx_mount_without_at_renders_under_the_body_on_both_tiers() {
    let src = concat!(
        "## Component App\n",
        "State:\n    Let count be 0.\n",
        "View:\n    <h1>Count: {count}</h1>\n",
        "\n## Main\nMount App.\n",
    );
    assert_dom_parity(
        src,
        &[
            DomOp::ClearChildren { node: 0 },
            DomOp::CreateElement {
                node: S + 3,
                tag: "h1".into(),
            },
            DomOp::AppendChild {
                parent: 0,
                child: S + 3,
            },
            DomOp::CreateTextNode {
                node: S + 4,
                text: "Count: ".into(),
            },
            DomOp::CreateTextNode {
                node: S + 5,
                text: "0".into(),
            },
            DomOp::AppendChild {
                parent: S + 3,
                child: S + 4,
            },
            DomOp::AppendChild {
                parent: S + 3,
                child: S + 5,
            },
        ],
    );
}

/// `Mount App at "#root"` when nothing matches `#root` must FAIL, identically, on both tiers.
///
/// The alternative is what the bytecode tier used to do: take the "no such element" handle,
/// append the whole page to it, and report success — a blank page and nothing said. A page
/// that silently goes nowhere is the worst outcome this model can produce, so the two tiers
/// are held to the same words.
#[test]
fn lgx_mount_at_a_selector_that_matches_nothing_fails_the_same_way_on_both_tiers() {
    let src = concat!(
        "## Component App\n",
        "State:\n    Let count be 0.\n",
        "View:\n    <h1>Count: {count}</h1>\n",
        "\n## Main\nMount App at \"#root\".\n",
    );
    let expected = "Mount App at \"#root\": no element matches that selector. Create the mount \
                    point before mounting, or mount without `at` to render under the page body.";
    dom::reset();
    let tw = tw_outcome(src);
    assert_eq!(
        tw.error.as_deref(),
        Some(expected),
        "tree-walker must refuse"
    );
    assert!(
        dom::op_log().is_empty(),
        "and must not have painted anything first"
    );
    dom::reset();
    let vm = vm_outcome(src);
    assert_eq!(
        vm.error.as_deref(),
        Some(expected),
        "the VM must refuse in the same words"
    );
    assert!(
        dom::op_log().is_empty(),
        "and must not have painted anything first"
    );
}

/// `Mount App at "#root"` when `#root` DOES exist paints into it, on both tiers.
#[test]
fn lgx_mount_at_an_existing_selector_paints_into_it_on_both_tiers() {
    let src = concat!(
        "## Component App\n",
        "View:\n    <h1>hi</h1>\n",
        "\n## Main\n",
        "Let root be createElement(\"div\").\n",
        "Call setAttribute with root and \"id\" and \"root\".\n",
        "Call appendChild with body() and root.\n",
        "Mount App at \"#root\".\n",
    );
    assert_dom_parity(
        src,
        &[
            DomOp::CreateElement {
                node: 1,
                tag: "div".into(),
            },
            DomOp::SetAttribute {
                node: 1,
                name: "id".into(),
                value: "root".into(),
            },
            DomOp::AppendChild {
                parent: 0,
                child: 1,
            },
            DomOp::ClearChildren { node: 1 },
            DomOp::CreateElement {
                node: S + 3,
                tag: "h1".into(),
            },
            DomOp::AppendChild {
                parent: 1,
                child: S + 3,
            },
            DomOp::CreateTextNode {
                node: S + 4,
                text: "hi".into(),
            },
            DomOp::AppendChild {
                parent: S + 3,
                child: S + 4,
            },
        ],
    );
}

/// The LGX twin of `mount_makes_a_ceremony_free_reactive_app` and
/// `capstone_reactive_counter_app_re_renders_on_click`: a click runs a turn and the page is
/// painted again, whole, from the new state.
#[test]
fn lgx_a_click_runs_a_turn_and_patches_on_both_tiers() {
    // ★ THE TURN SHOWS ITS FIELD ON PURPOSE, so the twin asserts what an update actually costs:
    // one `setText` into the node that changed, and nothing else.
    //
    // The op-log is the contract, not the rendered tree: `lgx_vm_parity_lock` compares trees and
    // the trees agreed all along, while the two tiers' ops diverged underneath them. This is the
    // assertion that says the compiled patcher and the reference patcher do the SAME thing.
    let src = concat!(
        "## Component App\n",
        "State:\n    Let count be 0.\n",
        "Actions:\n    To bump:\n        Set count to count + 1.\n",
        "View:\n",
        "    <h1>Reactive Counter</h1>\n",
        "    <p id=\"n\">{count}</p>\n",
        "    <button id=\"inc\" on-click={bump}>+</button>\n",
        "\n## Main\nMount App.\nClick \"#inc\".\n",
    );
    assert_dom_parity(
        src,
        &[
            DomOp::ClearChildren { node: 0 },
            DomOp::CreateElement {
                node: S + 3,
                tag: "h1".into(),
            },
            DomOp::CreateElement {
                node: S + 4,
                tag: "p".into(),
            },
            DomOp::SetAttribute {
                node: S + 4,
                name: "id".into(),
                value: "n".into(),
            },
            DomOp::CreateElement {
                node: S + 5,
                tag: "button".into(),
            },
            DomOp::SetAttribute {
                node: S + 5,
                name: "id".into(),
                value: "inc".into(),
            },
            DomOp::Listen {
                node: S + 5,
                event: "click".into(),
                flags: 0,
            },
            DomOp::AppendChild {
                parent: 0,
                child: S + 3,
            },
            DomOp::AppendChild {
                parent: 0,
                child: S + 4,
            },
            DomOp::AppendChild {
                parent: 0,
                child: S + 5,
            },
            DomOp::CreateTextNode {
                node: S + 6,
                text: "+".into(),
            },
            DomOp::AppendChild {
                parent: S + 5,
                child: S + 6,
            },
            DomOp::CreateTextNode {
                node: S + 7,
                text: "0".into(),
            },
            DomOp::AppendChild {
                parent: S + 4,
                child: S + 7,
            },
            DomOp::CreateTextNode {
                node: S + 8,
                text: "Reactive Counter".into(),
            },
            DomOp::AppendChild {
                parent: S + 3,
                child: S + 8,
            },
            // The turn: no rebuild, no new nodes — the hole's own node is written.
            DomOp::SetText {
                node: S + 7,
                text: "1".into(),
            },
        ],
    );
}

/// A turn that moves a field NO site displays costs nothing at all.
///
/// ★ The other half of the same rule, and the reason the fixture above shows its field: `count`
/// here is state the page holds and never renders, so there is nothing on screen that could have
/// changed. Both tiers must agree that the answer is no ops — an engine that repainted on one tier
/// and skipped on the other would be a divergence on the most ordinary shape there is.
#[test]
fn lgx_a_turn_that_changes_nothing_shown_emits_no_ops_on_both_tiers() {
    let src = concat!(
        "## Component App\n",
        "State:\n    Let count be 0.\n",
        "Actions:\n    To bump:\n        Set count to count + 1.\n",
        "View:\n",
        "    <h1>Reactive Counter</h1>\n",
        "    <button id=\"inc\" on-click={bump}>+</button>\n",
        "\n## Main\nMount App.\nClick \"#inc\".\n",
    );
    let interp = interp_log(src);
    let vm = vm_log(src);
    assert_eq!(interp, vm, "tree-walker vs VM:\n{src}");
    // The mount draws the page; the turn, which moves nothing the view shows, adds nothing.
    let turn = after_the_mount(src);
    assert!(
        turn.is_empty(),
        "the turn must not paint at all:\n{turn:#?}"
    );
}

/// The LGX twin of `reactive_counter_shows_the_incremented_value_on_wasm`: the VALUE the hole
/// shows moves, which is the part a tree comparison can see and an op-log comparison must too.
#[test]
fn lgx_the_hole_shows_the_new_value_after_the_turn_on_both_tiers() {
    let src = concat!(
        "## Component App\n",
        "State:\n    Let count be 0.\n",
        "Actions:\n    To bump:\n        Set count to count + 1.\n",
        "View:\n    <p id=\"label\">Count: {count}</p>\n",
        "    <button id=\"inc\" on-click={bump}>+</button>\n",
        "\n## Main\nMount App.\nClick \"#inc\".\n",
    );
    // ★ FOLLOW THE VALUE, NOT HOW IT ARRIVED. This used to collect `CreateTextNode` ops and
    // demand ["0", "1"] — which measured the MECHANISM: it only passed while every turn rebuilt
    // the subtree, so the second value could only appear by creating a second node. A patch
    // writes into the node that is already there, so the value moves with no creation at all, and
    // the old measure reported a working update as a broken one.
    //
    // The claim in this test's name is about what the hole SHOWS. So: whatever the last op to
    // touch that hole was — the creation on the first paint, a `setText` on the turn — the text
    // it ends up carrying is what a visitor reads.
    let shown = |log: &[DomOp]| -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for op in log {
            match op {
                DomOp::CreateTextNode { text, .. } if text == "0" || text == "1" => {
                    out.push(text.clone())
                }
                DomOp::SetText { text, .. } if text == "0" || text == "1" => out.push(text.clone()),
                _ => {}
            }
        }
        out
    };
    let interp = interp_log(src);
    let vm = vm_log(src);
    assert_eq!(interp, vm, "tree-walker vs VM:\n{src}");
    assert_eq!(
        shown(&interp),
        vec!["0", "1"],
        "the hole must show the value the turn wrote"
    );
    // And it must arrive WITHOUT rebuilding: the point of the update is that the node survives.
    let turn = after_the_mount(src);
    assert!(
        !turn.iter().any(|o| matches!(
            o,
            DomOp::ClearChildren { .. }
                | DomOp::CreateElement { .. }
                | DomOp::CreateTextNode { .. }
        )),
        "the turn rebuilt instead of writing the hole:\n{turn:#?}"
    );
    assert!(!turn.is_empty(), "the turn wrote the hole:\n{turn:#?}");
}

/// The LGX twin of `reactive_list_signal_re_renders_on_push`: a `Push` inside an action grows
/// the list the view iterates, and the repaint shows the new item.
#[test]
fn lgx_a_push_in_an_action_grows_the_painted_list_on_both_tiers() {
    let src = concat!(
        "## Component App\n",
        "State:\n    Let todos be a new Seq of Text.\n",
        "Actions:\n    To add:\n        Push \"x\" to todos.\n",
        "View:\n",
        "    <button id=\"add\" on-click={add}>add</button>\n",
        "    <ul>{for each t in todos:}<li>{t}</li>{end}</ul>\n",
        "\n## Main\nMount App.\nClick \"#add\".\n",
    );
    let interp = interp_log(src);
    assert_eq!(vm_log(src), interp, "tree-walker vs VM:\n{src}");
    let items: Vec<&str> = interp
        .iter()
        .filter_map(|op| match op {
            DomOp::CreateTextNode { text, .. } if text == "x" => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        items,
        vec!["x"],
        "one click pushes one item, and the repaint shows it"
    );
}

/// The LGX twin of `component_block_composes_as_a_fragment_on_all_tiers`: a child component
/// renders its nodes INTO the parent, with no wrapper element of its own.
#[test]
fn lgx_a_child_component_renders_into_its_parent_with_no_wrapper_on_both_tiers() {
    let src = concat!(
        "## Component Counter\n",
        "State:\n    Let count be 0.\n",
        "View:\n    <p>Count: {count}</p>\n",
        "\n## Component App\n",
        "View:\n    <Counter/>\n",
        "\n## Main\nMount App.\n",
    );
    assert_dom_parity(
        src,
        &[
            DomOp::ClearChildren { node: 0 },
            DomOp::CreateElement {
                node: S + 4,
                tag: "p".into(),
            },
            DomOp::AppendChild {
                parent: 0,
                child: S + 4,
            },
            DomOp::CreateTextNode {
                node: S + 5,
                text: "Count: ".into(),
            },
            DomOp::CreateTextNode {
                node: S + 6,
                text: "0".into(),
            },
            DomOp::AppendChild {
                parent: S + 4,
                child: S + 5,
            },
            DomOp::AppendChild {
                parent: S + 4,
                child: S + 6,
            },
        ],
    );
}

/// The LGX twin of `scoped_css_isolates_identically_named_component_classes`: two components
/// that both style `.box` do not collide, because each stamps its own scope class.
#[test]
fn lgx_two_components_styling_the_same_class_do_not_collide_on_both_tiers() {
    let src = concat!(
        "## Component Alpha\n",
        "View:\n    <p class=\"box\">a</p>\n",
        "Style:\n    .box { color: red }\n",
        "\n## Component Beta\n",
        "View:\n    <p class=\"box\">b</p>\n",
        "Style:\n    .box { color: blue }\n",
        "\n## Component App\n",
        "View:\n    <div><Alpha/><Beta/></div>\n",
        "\n## Main\nMount App.\n",
    );
    let log = interp_log(src);
    assert_eq!(vm_log(src), log, "tree-walker vs VM:\n{src}");
    let styles: Vec<&str> = log
        .iter()
        .filter_map(|op| match op {
            DomOp::SetText { text, .. } | DomOp::CreateTextNode { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        styles.contains(&".box.lgx-alpha { color: red }"),
        "Alpha's rule is scoped to Alpha:\n{styles:?}"
    );
    assert!(
        styles.contains(&".box.lgx-beta { color: blue }"),
        "Beta's rule is scoped to Beta:\n{styles:?}"
    );
    let classes: Vec<&str> = log
        .iter()
        .filter_map(|op| match op {
            DomOp::SetAttribute { name, value, .. } if name == "class" => Some(value.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        classes.contains(&"box lgx-alpha"),
        "Alpha's element carries its scope:\n{classes:?}"
    );
    assert!(
        classes.contains(&"box lgx-beta"),
        "Beta's element carries its scope:\n{classes:?}"
    );
}

/// The LGX twin of `component_scoped_css_preserves_at_rules_and_scopes_only_nested_selectors`.
/// A prefixed at-rule is one a browser drops in silence, so nothing may ever prefix one.
#[test]
fn lgx_style_scoping_leaves_at_rules_alone_on_both_tiers() {
    let src = concat!(
        "## Component Card\n",
        "View:\n    <p class=\"card\">c</p>\n",
        "Style:\n",
        "    .card { color: red }\n",
        "    @media (max-width: 600px) { .card { padding: 0 } }\n",
        "    @keyframes pulse { 0% { opacity: 0 } to { opacity: 1 } }\n",
        "\n## Main\nMount Card.\n",
    );
    let log = interp_log(src);
    assert_eq!(vm_log(src), log, "tree-walker vs VM:\n{src}");
    let style = log
        .iter()
        .find_map(|op| match op {
            DomOp::SetText { text, .. } | DomOp::CreateTextNode { text, .. }
                if text.contains("@media") =>
            {
                Some(text.clone())
            }
            _ => None,
        })
        .expect("the scoped stylesheet is in the log");
    assert!(
        style.contains(".card.lgx-card { color: red }"),
        "the plain rule is scoped:\n{style}"
    );
    assert!(
        style.contains(".card.lgx-card { padding: 0 }"),
        "a rule NESTED in an at-rule is scoped:\n{style}"
    );
    assert!(
        !style.contains(".lgx-card @"),
        "no at-rule is ever selector-prefixed:\n{style}"
    );
    assert!(
        !style.contains(".lgx-card 0%"),
        "keyframe offsets are never scoped:\n{style}"
    );
}

/// The LGX twin of `view_mistakes_get_socratic_guidance`: a template mistake names the
/// construct and the fix, rather than reporting a bare unexpected token.
#[test]
fn lgx_template_mistakes_get_socratic_guidance() {
    let no_in = concat!(
        "## Component App\n",
        "State:\n    Let items be [\"a\"].\n",
        "View:\n    <ul>{for each item items:}<li>x</li>{end}</ul>\n",
        "\n## Main\nMount App.\n",
    );
    let err = format!("{:?}", tw_outcome(no_in).error);
    assert!(
        err.contains("for each") && err.contains(" in "),
        "the `for each` error names the missing `in`:\n{err}"
    );
    let unclosed = concat!(
        "## Component App\n",
        "View:\n    <div><p>x</div>\n",
        "\n## Main\nMount App.\n",
    );
    let err2 = format!("{:?}", tw_outcome(unclosed).error);
    assert!(
        err2.contains("</p>") || err2.contains("<p>"),
        "the unclosed-tag error names the tag:\n{err2}"
    );
}

// ---- The wasm tier: the same DOM ops through a `MockDom` wasmi host. ----
#[cfg(feature = "wasm-jit")]
mod wasm_tier {
    use super::*;
    use logicaffeine_compile::compile::compile_to_wasm;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// One recorded navigation in the MockDom's router — the wasm mirror of the
    /// native `dom::RouteOp` (pushState `replace=false` vs replaceState `replace=true`).
    #[derive(Clone, Debug, PartialEq, Eq)]
    struct MockRouteOp {
        path: String,
        replace: bool,
    }

    /// One request in the MockDom's correlated fetch engine — request side as the
    /// program sent it, response side as the test delivered it. `done` is the
    /// FINALITY bit: an aborted/failed/delivered request can never deliver again.
    #[derive(Clone, Debug, Default)]
    struct MockFetchReq {
        method: String,
        url: String,
        body: String,
        headers: Vec<(String, String)>,
        closure: i32,
        timeout_ms: i64,
        aborted: bool,
        done: bool,
        status: i64,
        resp_headers: Vec<(String, String)>,
        resp_body: String,
        error: String,
    }

    /// The size caps + request-header allowlist — the SAME constants the native
    /// recorder enforces, so the tiers cannot drift.
    use logicaffeine_compile::semantics::dom::{
        FETCH_ALLOWED_REQUEST_HEADERS, FETCH_REQUEST_BODY_CAP, FETCH_RESPONSE_BODY_CAP,
    };

    /// The mock DOM the emitted `dom_*` host imports drive — the wasm analog of the
    /// thread-local `DomRecorder`, keyed by the same node-creation ordinals.
    #[derive(Default)]
    struct MockDom {
        node_count: u32,
        log: Vec<DomOp>,
        /// `(node, event) → closure handle` (the i32 obj ptr the trampoline fires) — a closure
        /// imperative code attached with `addEventListener`.
        listeners: Vec<(u32, String, i32)>,
        /// `(node, event)` for every `listenFlags` registration — where a component's view listens;
        /// what answers is the page's own listener table, reached through the node dispatcher.
        listens: Vec<(u32, String)>,
        /// `(topic, message)` for every frame the page handed to the transport — the outward leg
        /// of `Publish M on "t"`, which nothing else in this log can show.
        published: Vec<(String, String)>,
        /// The correlated fetch engine — the host-side mirror of the native
        /// recorder (`semantics::dom`): requests keyed by id (index + 1), each
        /// carrying its OWN response state, resolved on command.
        fetch_reqs: Vec<MockFetchReq>,
        /// The request currently being delivered — what `dom_response_text` (the
        /// sugar) reads.
        current_fetch: Option<usize>,
        /// A bump pointer for host-allocated Text (well past a small program's heap).
        text_bump: usize,
        /// `node → tag` and `id → node`, so `dom_query_selector` resolves `#id` and
        /// bare-tag selectors exactly like the native `DomRecorder`.
        tags: Vec<(u32, String)>,
        ids: Vec<(String, u32)>,
        /// The router — the wasm mirror of the native recorder's location model: the
        /// current path (window.location.pathname), the query string, the ordered nav
        /// op-log (pushState/replaceState), and the registered popstate handler closure.
        route_path: String,
        route_query: String,
        route_ops: Vec<MockRouteOp>,
        route_handler: Option<i32>,
        /// Browser storage, keyed by `(area, key)` — the wasm mirror of
        /// `semantics::storage`. Two areas in one map because the AREA is part of the
        /// identity: a key written to `local` must not be readable from `session`, or
        /// "preserved across reloads" would quietly outlive the session it named.
        storage: std::collections::BTreeMap<(String, String), String>,
        /// The document title, so a test can read back what `Set the title to …` wrote.
        title: String,
        /// How many times the page was told to reload — a browser tears the document down, which
        /// a mock cannot do, so it counts the command instead.
        reloads: usize,
        /// How many times the page asked to step back — a real browser answers with a popstate.
        backs: usize,
        /// `(node, name, value)` in write order, so `dom_get_attribute` answers what
        /// `dom_set_attribute` last wrote. The op log alone cannot serve a READ.
        attrs: Vec<(u32, String, String)>,
        /// `(node, command)` for every `dom_element_command` — a mock has no cursor and no
        /// viewport, so what it can honestly record is that the page asked.
        commands: Vec<(u32, String)>,
        /// Every delay, in milliseconds, the page asked to be woken after (`dom_wake_after`) — the
        /// mock has no clock, so what it records is when the page asked to be called.
        wakes: Vec<i64>,
    }
    impl MockDom {
        fn new() -> Self {
            // Node 0 is `body`, matching the recorder.
            MockDom {
                node_count: 1,
                log: Vec::new(),
                listeners: Vec::new(),
                listens: Vec::new(),
                published: Vec::new(),
                fetch_reqs: Vec::new(),
                current_fetch: None,
                text_bump: 2_000_000,
                tags: Vec::new(),
                ids: Vec::new(),
                route_path: "/".to_string(),
                route_query: String::new(),
                route_ops: Vec::new(),
                route_handler: None,
                storage: std::collections::BTreeMap::new(),
                title: String::new(),
                reloads: 0,
                backs: 0,
                attrs: Vec::new(),
                commands: Vec::new(),
                wakes: Vec::new(),
            }
        }
        // --- The router (mirrors the native recorder's location model). ---
        fn init_route(&mut self, path: &str, query: &str) {
            self.route_path = path.to_string();
            self.route_query = query.to_string();
        }
        fn set_route_path(&mut self, path: &str) {
            self.route_path = path.to_string();
        }
        fn route_handler(&self) -> Option<i32> {
            self.route_handler
        }
        fn navigate(&mut self, path: &str, replace: bool) {
            self.route_ops.push(MockRouteOp {
                path: path.to_string(),
                replace,
            });
            self.route_path = path.to_string();
        }
        /// A query param by name (`""` absent) — the same tiny `k=v&k=v` scan the
        /// native recorder and the browser host use.
        fn route_query_value(&self, name: &str) -> String {
            for pair in self.route_query.split('&') {
                if let Some((k, v)) = pair.split_once('=') {
                    if k == name {
                        return v.to_string();
                    }
                }
            }
            String::new()
        }
        fn query_selector(&self, sel: &str) -> i64 {
            if let Some(id) = sel.strip_prefix('#') {
                return self
                    .ids
                    .iter()
                    .find(|(i, _)| i == id)
                    .map(|(_, n)| *n as i64)
                    .unwrap_or(-1);
            }
            self.tags
                .iter()
                .find(|(_, t)| t == sel)
                .map(|(n, _)| *n as i64)
                .unwrap_or(-1)
        }
        fn fresh(&mut self) -> u32 {
            let h = self.node_count;
            self.node_count += 1;
            h
        }
        fn listener(&self, node: u32, event: &str) -> Option<i32> {
            self.listeners
                .iter()
                .find(|(n, e, _)| *n == node && e == event)
                .map(|(_, _, h)| *h)
        }
        fn listens_for(&self, node: u32, event: &str) -> bool {
            self.listens.iter().any(|(n, e)| *n == node && e == event)
        }
        /// One op a patch stream carried, logged and remembered where a later read looks for it —
        /// the tag and id a selector resolves, the attribute a read answers, the pair the page
        /// listens on — exactly as the per-op import that op stands for would have.
        fn record(&mut self, op: DomOp) {
            match &op {
                DomOp::CreateElement { node, tag } => self.tags.push((*node, tag.clone())),
                DomOp::SetAttribute { node, name, value } => {
                    if name == "id" {
                        self.ids.push((value.clone(), *node));
                    }
                    self.attrs.push((*node, name.clone(), value.clone()));
                }
                DomOp::Listen { node, event, .. } => {
                    if !self.listens_for(*node, event) {
                        self.listens.push((*node, event.clone()));
                    }
                }
                DomOp::Unlisten { node, event } => {
                    self.listens.retain(|(n, e)| !(*n == *node && e == event))
                }
                _ => {}
            }
            self.log.push(op);
        }

        // --- The correlated fetch engine (mirrors `semantics::dom` exactly). ---

        /// Register a request and mint its id (index + 1). Contract violations
        /// (oversized body, disallowed header) fail it immediately — FINAL before
        /// it would touch any network.
        fn fetch_send(
            &mut self,
            method: &str,
            url: &str,
            body: &str,
            headers: Vec<(String, String)>,
            closure: i32,
        ) -> i64 {
            let mut req = MockFetchReq {
                method: method.to_string(),
                url: url.to_string(),
                body: body.to_string(),
                headers,
                closure,
                ..MockFetchReq::default()
            };
            if req.body.len() > FETCH_REQUEST_BODY_CAP {
                req.done = true;
                req.error = "toolarge".to_string();
            } else if let Some((name, _)) = req.headers.iter().find(|(n, _)| {
                !FETCH_ALLOWED_REQUEST_HEADERS.contains(&n.to_ascii_lowercase().as_str())
            }) {
                req.done = true;
                req.error = format!("network: header not allowed: {}", name.to_ascii_lowercase());
            }
            self.fetch_reqs.push(req);
            self.fetch_reqs.len() as i64
        }

        fn fetch_req(&self, req: i64) -> Option<&MockFetchReq> {
            (req >= 1)
                .then(|| self.fetch_reqs.get(req as usize - 1))
                .flatten()
        }

        /// Deliver a response: `None` if unknown or already FINAL (no double
        /// delivery; an aborted request never delivers). An oversized body becomes
        /// a delivered `"toolarge"` failure. Returns the closure handle to fire.
        fn resolve_fetch(
            &mut self,
            req: i64,
            status: i64,
            headers: &[(String, String)],
            body: &str,
        ) -> Option<i32> {
            let idx = usize::try_from(req).ok()?.checked_sub(1)?;
            let q = self.fetch_reqs.get_mut(idx)?;
            if q.done {
                return None;
            }
            q.done = true;
            if body.len() > FETCH_RESPONSE_BODY_CAP {
                q.error = "toolarge".to_string();
            } else {
                q.status = status;
                q.resp_headers = headers.to_vec();
                q.resp_body = body.to_string();
            }
            self.current_fetch = Some(idx);
            Some(self.fetch_reqs[idx].closure)
        }

        /// Deliver a structured transport failure (same finality rule).
        fn fail_fetch(&mut self, req: i64, error: &str) -> Option<i32> {
            let idx = usize::try_from(req).ok()?.checked_sub(1)?;
            let q = self.fetch_reqs.get_mut(idx)?;
            if q.done {
                return None;
            }
            q.done = true;
            q.error = error.to_string();
            self.current_fetch = Some(idx);
            Some(q.closure)
        }

        fn abort_fetch(&mut self, req: i64) {
            if let Some(idx) = usize::try_from(req).ok().and_then(|r| r.checked_sub(1)) {
                if let Some(q) = self.fetch_reqs.get_mut(idx) {
                    q.aborted = true;
                    if !q.done {
                        q.done = true;
                        q.error = "abort".to_string();
                    }
                }
            }
        }

        fn set_fetch_timeout(&mut self, req: i64, millis: i64) {
            if let Some(idx) = usize::try_from(req).ok().and_then(|r| r.checked_sub(1)) {
                if let Some(q) = self.fetch_reqs.get_mut(idx) {
                    q.timeout_ms = millis.max(0);
                }
            }
        }

        fn response_text(&self) -> String {
            self.current_fetch
                .and_then(|i| self.fetch_reqs.get(i))
                .map(|q| q.resp_body.clone())
                .unwrap_or_default()
        }
    }
    type Data = Rc<RefCell<MockDom>>;

    /// Build the `dom_*` host linker over a fresh `MockDom`.
    fn dom_linker(engine: &wasmi::Engine) -> (Data, wasmi::Linker<Data>) {
        let dom: Data = Rc::new(RefCell::new(MockDom::new()));
        let mut l = wasmi::Linker::<Data>::new(engine);
        // Every import whose answer the ABI fixes — one implementation, shared, so no test can get
        // `fmt_i64_into`'s return value wrong again (see `wasm_host_stub_lock`).
        crate::common::wasm_host::link_value_imports::<Data>(&mut l);
        // Every program that CAN fail declares `logos_rt_fail`, and a module importing a
        // function its host does not define FAILS TO INSTANTIATE — silently, with no
        // compile error anywhere. This host does not care why; it must simply be here.
        // ★ A TRAP HERE PRINTS ITS REASON. `wasmi` reports only `UnreachableCodeReached`, which
        // says a LOGOS check failed and nothing about which — and the compiler hands the message
        // over on this very call, immediately before trapping. Dropping it meant every failure in
        // a wasm test looked identical; printing it turned one such trap into
        // "Unknown function: `__lgx_tick`" in a single run.
        l.func_wrap(
            "env",
            "logos_rt_fail",
            |c: wasmi::Caller<'_, Data>, h: i32| {
                eprintln!("[wasm_dom] LOGOS runtime failure: {}", read_text(&c, h));
            },
        )
        .unwrap();
        l.func_wrap(
            "env",
            "logos_rt_fail_at",
            |_c: wasmi::Caller<'_, Data>, code: i32, v: i64| {
                eprintln!("[wasm_dom] LOGOS runtime failure: code {code}, value {v}");
            },
        )
        .unwrap();
        install_dom_hosts(&mut l);
        (dom, l)
    }

    /// Read a `Text` handle (`[len][cap][data_ptr]`) from the module's memory.
    fn read_text(c: &wasmi::Caller<'_, Data>, handle: i32) -> String {
        let mem = c.get_export("memory").unwrap().into_memory().unwrap();
        let d = mem.data(c);
        let h = handle as usize;
        let len = i32::from_le_bytes(d[h..h + 4].try_into().unwrap()) as usize;
        let dp = i32::from_le_bytes(d[h + 8..h + 12].try_into().unwrap()) as usize;
        String::from_utf8_lossy(&d[dp..dp + len]).into_owned()
    }

    /// Read a `Seq of Text` handle: `[len][cap][data_ptr]`, each 8-byte slot
    /// holding a Text handle (little-endian, low 4 bytes).
    /// A `Seq of Int` as raw bytes — the patch buffer `dom_apply` receives. Same layout as
    /// `read_text_seq` walks: length at the handle, data pointer eight bytes in, 8-byte stride.
    fn read_byte_seq(c: &mut wasmi::Caller<'_, Data>, handle: i32) -> Vec<u8> {
        let mem = c
            .get_export("memory")
            .and_then(|e| e.into_memory())
            .expect("memory");
        let data = mem.data(&c);
        let at = handle as usize;
        let len = i32::from_le_bytes(data[at..at + 4].try_into().unwrap()) as usize;
        let ptr = i32::from_le_bytes(data[at + 8..at + 12].try_into().unwrap()) as usize;
        (0..len)
            .map(|i| {
                let e = ptr + i * 8;
                i64::from_le_bytes(data[e..e + 8].try_into().unwrap()) as u8
            })
            .collect()
    }

    fn read_text_seq(c: &wasmi::Caller<'_, Data>, handle: i32) -> Vec<String> {
        let mem = c.get_export("memory").unwrap().into_memory().unwrap();
        let d = mem.data(c);
        let h = handle as usize;
        let len = i32::from_le_bytes(d[h..h + 4].try_into().unwrap()) as usize;
        let dp = i32::from_le_bytes(d[h + 8..h + 12].try_into().unwrap()) as usize;
        let mut out = Vec::with_capacity(len);
        for i in 0..len {
            let slot = dp + i * 8;
            let th = i32::from_le_bytes(d[slot..slot + 4].try_into().unwrap());
            out.push(read_text(c, th));
        }
        out
    }

    /// Host-allocate `s` as a Text in the module's memory at the mock's bump
    /// pointer, returning its handle — the shape every Text-returning host uses.
    fn alloc_host_text(c: &mut wasmi::Caller<'_, Data>, s: &str) -> i32 {
        let bump = c.data().borrow().text_bump;
        let mem = c.get_export("memory").unwrap().into_memory().unwrap();
        let (handle, new_bump) = {
            let data = mem.data_mut(&mut *c);
            write_text(data, bump, s)
        };
        c.data().borrow_mut().text_bump = new_bump;
        handle
    }

    /// Compile `src` to wasm, run `main` through a `MockDom`, and return its op-log.
    pub(super) fn wasm_log(src: &str) -> Vec<DomOp> {
        let module = compile_to_wasm(src).expect("compiles to wasm");
        let engine = wasmi::Engine::default();
        let m = wasmi::Module::new(&engine, &module[..]).expect("valid wasm");
        let (dom, l) = dom_linker(&engine);
        let mut store = wasmi::Store::new(&engine, dom.clone());
        let inst = l
            .instantiate(&mut store, &m)
            .unwrap()
            .start(&mut store)
            .unwrap();
        inst.get_typed_func::<(), ()>(&store, "main")
            .unwrap()
            .call(&mut store, ())
            .expect("main runs");
        let out = dom.borrow().log.clone();
        out
    }

    /// Compile `src`, run `main`, then simulate an `event` on `node` as a browser delivers it: a
    /// closure imperative code attached fires through the B0 `__logos_call_closure` trampoline, and
    /// an element a component's view listens on hands the event to the page's node dispatcher.
    /// Returns the op-log (which now includes whatever the handler mutated).
    fn wasm_build_and_fire(src: &str, node: u32, event: &str, event_arg: i64) -> Vec<DomOp> {
        wasm_build_and_fire_where(src, |_| node, event, event_arg)
    }

    /// [`wasm_build_and_fire`] on the element `selector` finds once `main` has drawn the page — a
    /// page's nodes are named by the stream that drew them, so a test finds one the way a visitor
    /// does rather than by a number.
    fn wasm_build_and_fire_at(
        src: &str,
        selector: &str,
        event: &str,
        event_arg: i64,
    ) -> Vec<DomOp> {
        wasm_build_and_fire_where(
            src,
            |dom| {
                let found = dom.query_selector(selector);
                u32::try_from(found).unwrap_or_else(|_| panic!("the page draws `{selector}`"))
            },
            event,
            event_arg,
        )
    }

    fn wasm_build_and_fire_where(
        src: &str,
        target: impl FnOnce(&MockDom) -> u32,
        event: &str,
        event_arg: i64,
    ) -> Vec<DomOp> {
        let module = compile_to_wasm(src).expect("compiles to wasm");
        let engine = wasmi::Engine::default();
        let m = wasmi::Module::new(&engine, &module[..]).expect("valid wasm");
        let (dom, l) = dom_linker(&engine);
        let mut store = wasmi::Store::new(&engine, dom.clone());
        let inst = l
            .instantiate(&mut store, &m)
            .unwrap()
            .start(&mut store)
            .unwrap();
        inst.get_typed_func::<(), ()>(&store, "main")
            .unwrap()
            .call(&mut store, ())
            .expect("main runs");
        let node = target(&dom.borrow());
        let closure = dom.borrow().listener(node, event);
        match closure {
            Some(handle) => {
                inst.get_typed_func::<(i32, i64), i64>(&store, "__logos_call_closure")
                    .expect("trampoline exported")
                    .call(&mut store, (handle, event_arg))
                    .expect("handler fires without trapping");
            }
            None => {
                assert!(
                    dom.borrow().listens_for(node, event),
                    "a handler is registered"
                );
                let answered = crate::common::wasm_host::dispatch_node(
                    &mut store,
                    &inst,
                    i64::from(node),
                    event,
                    "",
                )
                .expect("handler fires without trapping");
                assert!(
                    answered,
                    "the page listens for {event} on node {node}, so its dispatcher answers it"
                );
            }
        }
        let out = dom.borrow().log.clone();
        out
    }

    /// Register every `dom_*` host over the `MockDom` store data.
    fn install_dom_hosts(l: &mut wasmi::Linker<Data>) {
        l.func_wrap("env", "dom_body", |_c: wasmi::Caller<'_, Data>| -> i64 {
            0
        })
        .unwrap();
        // A program's own driving verb enters its page through the host; a mock records no failure.
        crate::common::wasm_host::link_page_entry::<Data>(l, |_c| String::new(), |_c, _message| {});
        // A no-op, matching `runtime.js`'s own: `the journal`/`Replay` are a `## Test`'s concept
        // (verified through `common::wasm_host::InteractivePage::turns()`, not this mock), but
        // EVERY lowered `__act_`/event/bind now imports it (R3), so any
        // module this linker instantiates that has one action needs it satisfied to link at all.
        l.func_wrap(
            "env",
            "dom_note_turn",
            |_c: wasmi::Caller<'_, Data>, _name: i32, _key: i32, _changes: i32, _failure: i32| {},
        )
        .unwrap();
        // A mock records no journal, so it asks for no detail.
        l.func_wrap(
            "env",
            "dom_journal_detail",
            |_c: wasmi::Caller<'_, Data>| -> i64 { 0 },
        )
        .unwrap();
        l.func_wrap(
            "env",
            "dom_wake_after",
            |c: wasmi::Caller<'_, Data>, ms: i64| {
                c.data().borrow_mut().wakes.push(ms);
            },
        )
        .unwrap();
        l.func_wrap(
            "env",
            "dom_create_element",
            |c: wasmi::Caller<'_, Data>, tag: i32| -> i64 {
                let tag = read_text(&c, tag);
                let d = c.data().clone();
                let mut m = d.borrow_mut();
                let h = m.fresh();
                m.tags.push((h, tag.clone()));
                m.log.push(DomOp::CreateElement { node: h, tag });
                h as i64
            },
        )
        .unwrap();
        l.func_wrap(
            "env",
            "dom_create_text_node",
            |c: wasmi::Caller<'_, Data>, txt: i32| -> i64 {
                let text = read_text(&c, txt);
                let d = c.data().clone();
                let mut m = d.borrow_mut();
                let h = m.fresh();
                m.log.push(DomOp::CreateTextNode { node: h, text });
                h as i64
            },
        )
        .unwrap();
        l.func_wrap(
            "env",
            "dom_set_text",
            |c: wasmi::Caller<'_, Data>, el: i64, txt: i32| {
                let text = read_text(&c, txt);
                c.data().borrow_mut().log.push(DomOp::SetText {
                    node: el as u32,
                    text,
                });
            },
        )
        .unwrap();
        l.func_wrap(
            "env",
            "dom_set_html",
            |c: wasmi::Caller<'_, Data>, el: i64, html: i32| {
                let html = read_text(&c, html);
                c.data().borrow_mut().log.push(DomOp::SetHtml {
                    node: el as u32,
                    html,
                });
            },
        )
        .unwrap();
        l.func_wrap(
            "env",
            "dom_clear_children",
            |c: wasmi::Caller<'_, Data>, el: i64| {
                c.data()
                    .borrow_mut()
                    .log
                    .push(DomOp::ClearChildren { node: el as u32 });
            },
        )
        .unwrap();
        l.func_wrap(
            "env",
            "dom_on_message",
            |c: wasmi::Caller<'_, Data>, topic: i32, handler: i32| {
                // The mock has no relay; it records the subscription so a test could fire it, the same
                // way it records a window listener against node 0.
                let topic = read_text(&c, topic);
                c.data()
                    .borrow_mut()
                    .listeners
                    .push((0, format!("message:{topic}"), handler));
            },
        )
        .unwrap();
        l.func_wrap(
            "env",
            "dom_publish",
            |c: wasmi::Caller<'_, Data>, topic: i32, message: i32| {
                // The mock has no socket; it records the frame the page handed out, which is the only
                // way the OUTWARD leg of a publish can be observed at all — the local leg shows up in
                // the page's own ops, so without this a publish that sent nothing would look complete.
                let (topic, message) = (read_text(&c, topic), read_text(&c, message));
                c.data().borrow_mut().published.push((topic, message));
            },
        )
        .unwrap();
        l.func_wrap(
            "env",
            "dom_set_attribute",
            |c: wasmi::Caller<'_, Data>, el: i64, name: i32, val: i32| {
                let name = read_text(&c, name);
                let value = read_text(&c, val);
                let mut m = c.data().borrow_mut();
                if name == "id" {
                    m.ids.push((value.clone(), el as u32));
                }
                m.attrs.push((el as u32, name.clone(), value.clone()));
                m.log.push(DomOp::SetAttribute {
                    node: el as u32,
                    name,
                    value,
                });
            },
        )
        .unwrap();
        // ★ THE READ HALF MUST SEE WHAT THE WRITE HALF WROTE, or `Add class` would compute every
        // new class list from an empty one and the mock would agree with a page that is wrong.
        l.func_wrap(
            "env",
            "dom_scroll_gap",
            |_c: wasmi::Caller<'_, Data>, _el: i64| -> i64 { 0 },
        )
        .unwrap();
        l.func_wrap(
            "env",
            "dom_event_file_name",
            |_c: wasmi::Caller<'_, Data>, _ev: i64| -> i32 { 0 },
        )
        .unwrap();
        l.func_wrap(
            "env",
            "dom_event_file_size",
            |_c: wasmi::Caller<'_, Data>, _ev: i64| -> i64 { 0 },
        )
        .unwrap();
        l.func_wrap(
            "env",
            "dom_event_file_type",
            |_c: wasmi::Caller<'_, Data>, _ev: i64| -> i32 { 0 },
        )
        .unwrap();
        l.func_wrap(
            "env",
            "dom_clipboard_write",
            |_c: wasmi::Caller<'_, Data>, _t: i32| {},
        )
        .unwrap();
        l.func_wrap(
            "env",
            "dom_download_file",
            |_c: wasmi::Caller<'_, Data>, _c2: i32, _n: i32| {},
        )
        .unwrap();
        l.func_wrap(
            "env",
            "dom_notify",
            |_c: wasmi::Caller<'_, Data>, _m: i32, _t: i32| {},
        )
        .unwrap();
        l.func_wrap(
            "env",
            "dom_element_command",
            |c: wasmi::Caller<'_, Data>, el: i64, cmd: i32| {
                let cmd = read_text(&c, cmd);
                c.data().borrow_mut().commands.push((el as u32, cmd));
            },
        )
        .unwrap();
        l.func_wrap(
            "env",
            "dom_set_interval",
            |c: wasmi::Caller<'_, Data>, _ms: i64, handler: i32| {
                // The mock has no clock; it records a program's own interval closure so a test can
                // fire it.
                c.data()
                    .borrow_mut()
                    .listeners
                    .push((0, "interval".to_string(), handler));
            },
        )
        .unwrap();
        l.func_wrap(
            "env",
            "dom_on_window_event",
            |c: wasmi::Caller<'_, Data>, event: i32, handler: i32| {
                let event = read_text(&c, event);
                // Node 0 is the page: the mock has no window, so a window listener is recorded against
                // it and a test fires it exactly as it fires any other listener.
                c.data().borrow_mut().listeners.push((0, event, handler));
            },
        )
        .unwrap();
        l.func_wrap(
            "env",
            "dom_get_attribute",
            |mut c: wasmi::Caller<'_, Data>, el: i64, name: i32| -> i32 {
                let name = read_text(&c, name);
                let current = {
                    let d = c.data().borrow();
                    d.attrs
                        .iter()
                        .rev()
                        .find(|(n, k, _)| *n == el as u32 && *k == name)
                        .map(|(_, _, v)| v.clone())
                        .unwrap_or_default()
                };
                alloc_host_text(&mut c, &current)
            },
        )
        .unwrap();
        l.func_wrap(
            "env",
            "dom_append_child",
            |c: wasmi::Caller<'_, Data>, parent: i64, child: i64| {
                c.data().borrow_mut().log.push(DomOp::AppendChild {
                    parent: parent as u32,
                    child: child as u32,
                });
            },
        )
        .unwrap();
        l.func_wrap(
            "env",
            "dom_set_value",
            |c: wasmi::Caller<'_, Data>, el: i64, txt: i32| {
                let value = read_text(&c, txt);
                c.data().borrow_mut().log.push(DomOp::SetValue {
                    node: el as u32,
                    value,
                });
            },
        )
        .unwrap();
        // A Bool arrives as an i64 in the value model. Recorded as a PRESENCE, like the native
        // recorder, so the op-log comparison can see a checkbox that is drawn wrong.
        l.func_wrap(
            "env",
            "dom_apply",
            |mut c: wasmi::Caller<'_, Data>, patches: i32| {
                // The MockDom is a shim too: it decodes the stream with the SAME codec the native
                // recorder uses and records the ops, so a page driven through the bulk call and a page
                // driven through per-op calls are comparable op-log for op-log.
                let bytes = read_byte_seq(&mut c, patches);
                let ops =
                    logicaffeine_compile::semantics::patch::decode(&bytes).unwrap_or_else(|e| {
                        panic!("the module emitted an undecodable patch stream: {e}")
                    });
                let mut m = c.data().borrow_mut();
                for op in ops {
                    m.record(op);
                }
            },
        )
        .unwrap();
        // A paint's wire-v2 stream, decoded by the shared reader and recorded op by op.
        l.func_wrap(
            "env",
            "dom_apply_words",
            |c: wasmi::Caller<'_, Data>,
             words: i32,
             word_count: i64,
             strings: i32,
             string_count: i64|
             -> i64 {
                let ops = crate::common::wasm_host::applied_words(
                    &c,
                    words,
                    word_count,
                    strings,
                    string_count,
                );
                let mut m = c.data().borrow_mut();
                for op in ops {
                    m.record(op);
                }
                0
            },
        )
        .unwrap();
        // Browser storage. Text in, text out, and NOTHING IS SPECIAL — the API is
        // length-delimited, so no byte needs escaping. An absent key answers handle `0`,
        // which the compiled side turns into `Nothing`: a page must be able to tell
        // "nothing is saved" from "an empty string is saved".
        l.func_wrap(
            "env",
            "storage_set",
            |c: wasmi::Caller<'_, Data>, area: i32, key: i32, value: i32| {
                let (area, key, value) = (
                    read_text(&c, area),
                    read_text(&c, key),
                    read_text(&c, value),
                );
                c.data().borrow_mut().storage.insert((area, key), value);
            },
        )
        .unwrap();
        l.func_wrap(
            "env",
            "storage_get",
            |mut c: wasmi::Caller<'_, Data>, area: i32, key: i32| -> i32 {
                let (area, key) = (read_text(&c, area), read_text(&c, key));
                let found = c.data().borrow().storage.get(&(area, key)).cloned();
                match found {
                    Some(v) => alloc_host_text(&mut c, &v),
                    None => 0,
                }
            },
        )
        .unwrap();
        l.func_wrap(
            "env",
            "storage_remove",
            |c: wasmi::Caller<'_, Data>, area: i32, key: i32| {
                let (area, key) = (read_text(&c, area), read_text(&c, key));
                c.data().borrow_mut().storage.remove(&(area, key));
            },
        )
        .unwrap();
        l.func_wrap(
            "env",
            "dom_set_checked",
            |c: wasmi::Caller<'_, Data>, el: i64, on: i64| {
                c.data().borrow_mut().log.push(DomOp::SetChecked {
                    node: el as u32,
                    checked: on != 0,
                });
            },
        )
        .unwrap();
        // querySelector resolves `#id` / bare-tag exactly like the native recorder.
        l.func_wrap(
            "env",
            "dom_query_selector",
            |c: wasmi::Caller<'_, Data>, sel: i32| -> i64 {
                let sel = read_text(&c, sel);
                c.data().borrow().query_selector(&sel)
            },
        )
        .unwrap();
        // Getters aren't exercised by the build-only parity program; inert stubs.
        l.func_wrap(
            "env",
            "dom_get_text",
            |_c: wasmi::Caller<'_, Data>, _el: i64| -> i32 { 0 },
        )
        .unwrap();
        l.func_wrap(
            "env",
            "dom_get_value",
            |_c: wasmi::Caller<'_, Data>, _el: i64| -> i32 { 0 },
        )
        .unwrap();
        // Register a handler: record `(node, event) → closure handle` and log it.
        // The closure handle is the i32 obj ptr the B0 trampoline later fires.
        l.func_wrap(
            "env",
            "dom_add_event_listener",
            |c: wasmi::Caller<'_, Data>, el: i64, event: i32, handler: i32| {
                let event = read_text(&c, event);
                let mut d = c.data().borrow_mut();
                d.listeners.push((el as u32, event.clone(), handler));
                d.log.push(DomOp::AddEventListener {
                    node: el as u32,
                    event,
                });
            },
        )
        .unwrap();
        // Event accessors — inert in the build-only harness (the fire test's handler
        // doesn't read them); real values arrive with the browser host.
        l.func_wrap(
            "env",
            "dom_event_value",
            |_c: wasmi::Caller<'_, Data>, _ev: i64| -> i32 { 0 },
        )
        .unwrap();
        // A key modifier's gate reads this. Nothing here fires a keyboard event, so it reports the
        // empty key a non-keyboard event has — the same answer a browser gives for a click.
        l.func_wrap(
            "env",
            "dom_event_key",
            |_c: wasmi::Caller<'_, Data>, _ev: i64| -> i32 { 0 },
        )
        .unwrap();
        l.func_wrap(
            "env",
            "dom_event_target",
            |_c: wasmi::Caller<'_, Data>, _ev: i64| -> i64 { -1 },
        )
        .unwrap();
        // Fetch — the correlated engine. `dom_fetch_start` (the `fetchText` sugar)
        // rides the SAME engine as `dom_fetch_send`; both are resolved on command
        // by the test, per REQUEST ID.
        l.func_wrap(
            "env",
            "dom_fetch_start",
            |c: wasmi::Caller<'_, Data>, url: i32, handler: i32| {
                let url = read_text(&c, url);
                c.data()
                    .borrow_mut()
                    .fetch_send("GET", &url, "", Vec::new(), handler);
            },
        )
        .unwrap();
        l.func_wrap(
            "env",
            "dom_fetch_send",
            |c: wasmi::Caller<'_, Data>,
             method: i32,
             url: i32,
             body: i32,
             headers: i32,
             handler: i32|
             -> i64 {
                let method = read_text(&c, method);
                let url = read_text(&c, url);
                let body = read_text(&c, body);
                let flat = read_text_seq(&c, headers);
                let pairs = flat
                    .chunks(2)
                    .map(|p| (p[0].clone(), p.get(1).cloned().unwrap_or_default()))
                    .collect();
                c.data()
                    .borrow_mut()
                    .fetch_send(&method, &url, &body, pairs, handler)
            },
        )
        .unwrap();
        l.func_wrap(
            "env",
            "dom_fetch_abort",
            |c: wasmi::Caller<'_, Data>, req: i64| {
                c.data().borrow_mut().abort_fetch(req);
            },
        )
        .unwrap();
        l.func_wrap(
            "env",
            "dom_fetch_set_timeout",
            |c: wasmi::Caller<'_, Data>, req: i64, millis: i64| {
                c.data().borrow_mut().set_fetch_timeout(req, millis);
            },
        )
        .unwrap();
        l.func_wrap(
            "env",
            "dom_response_status",
            |c: wasmi::Caller<'_, Data>, req: i64| -> i64 {
                c.data()
                    .borrow()
                    .fetch_req(req)
                    .map(|q| q.status)
                    .unwrap_or(0)
            },
        )
        .unwrap();
        l.func_wrap(
            "env",
            "dom_response_body",
            |mut c: wasmi::Caller<'_, Data>, req: i64| -> i32 {
                let body = c
                    .data()
                    .borrow()
                    .fetch_req(req)
                    .map(|q| q.resp_body.clone())
                    .unwrap_or_default();
                alloc_host_text(&mut c, &body)
            },
        )
        .unwrap();
        l.func_wrap(
            "env",
            "dom_response_header",
            |mut c: wasmi::Caller<'_, Data>, req: i64, name: i32| -> i32 {
                let name = read_text(&c, name);
                let value = c
                    .data()
                    .borrow()
                    .fetch_req(req)
                    .and_then(|q| {
                        q.resp_headers
                            .iter()
                            .find(|(n, _)| n.eq_ignore_ascii_case(&name))
                            .map(|(_, v)| v.clone())
                    })
                    .unwrap_or_default();
                alloc_host_text(&mut c, &value)
            },
        )
        .unwrap();
        l.func_wrap(
            "env",
            "dom_response_error",
            |mut c: wasmi::Caller<'_, Data>, req: i64| -> i32 {
                let error = c
                    .data()
                    .borrow()
                    .fetch_req(req)
                    .map(|q| q.error.clone())
                    .unwrap_or_default();
                alloc_host_text(&mut c, &error)
            },
        )
        .unwrap();
        // The `responseText()` sugar — the body of the request CURRENTLY delivering.
        l.func_wrap(
            "env",
            "dom_response_text",
            |mut c: wasmi::Caller<'_, Data>| -> i32 {
                let body = c.data().borrow().response_text();
                alloc_host_text(&mut c, &body)
            },
        )
        .unwrap();
        // The router hosts (G4). `dom_route_path`/`dom_route_query` read the MockDom's
        // location; `dom_navigate`/`dom_replace_route` record the navigation and move
        // the current path; `dom_on_route` registers the popstate handler closure the
        // pop driver later fires through the trampoline.
        l.func_wrap(
            "env",
            "dom_route_path",
            |mut c: wasmi::Caller<'_, Data>| -> i32 {
                let path = c.data().borrow().route_path.clone();
                alloc_host_text(&mut c, &path)
            },
        )
        .unwrap();
        l.func_wrap(
            "env",
            "dom_route_query",
            |mut c: wasmi::Caller<'_, Data>, name: i32| -> i32 {
                let name = read_text(&c, name);
                let value = c.data().borrow().route_query_value(&name);
                alloc_host_text(&mut c, &value)
            },
        )
        .unwrap();
        l.func_wrap(
            "env",
            "dom_navigate",
            |c: wasmi::Caller<'_, Data>, path: i32| {
                let path = read_text(&c, path);
                c.data().borrow_mut().navigate(&path, false);
            },
        )
        .unwrap();
        l.func_wrap(
            "env",
            "dom_replace_route",
            |c: wasmi::Caller<'_, Data>, path: i32| {
                let path = read_text(&c, path);
                c.data().borrow_mut().navigate(&path, true);
            },
        )
        .unwrap();
        l.func_wrap(
            "env",
            "dom_on_route",
            |c: wasmi::Caller<'_, Data>, handler: i32| {
                c.data().borrow_mut().route_handler = Some(handler);
            },
        )
        .unwrap();
        l.func_wrap(
            "env",
            "dom_set_title",
            |c: wasmi::Caller<'_, Data>, title: i32| {
                let title = read_text(&c, title);
                c.data().borrow_mut().title = title;
            },
        )
        .unwrap();
        l.func_wrap("env", "dom_reload", |c: wasmi::Caller<'_, Data>| {
            c.data().borrow_mut().reloads += 1;
        })
        .unwrap();
        l.func_wrap("env", "dom_go_back", |c: wasmi::Caller<'_, Data>| {
            c.data().borrow_mut().backs += 1;
        })
        .unwrap();
        // `listenFlags(el, event, flags)`: the page listens for `event` on `el` — recorded for the
        // driver and logged as the op every tier records for a registration. A mock attaches no
        // real listener, so there is no phase or passivity for the modifier bits to shape.
        l.func_wrap(
            "env",
            "dom_listen_flags",
            |c: wasmi::Caller<'_, Data>, el: i64, event: i32, flags: i64| {
                let event = read_text(&c, event);
                let mut d = c.data().borrow_mut();
                if !d.listens_for(el as u32, &event) {
                    d.listens.push((el as u32, event.clone()));
                }
                d.log.push(DomOp::Listen {
                    node: el as u32,
                    event,
                    flags: flags as u8,
                });
            },
        )
        .unwrap();
        // Another document loading ends this one; a mock has no second document to load.
        l.func_wrap(
            "env",
            "dom_assign_location",
            |_c: wasmi::Caller<'_, Data>, _url: i32| {},
        )
        .unwrap();
        // Last: every declared import the mock does not model links as a trap naming it, so a host
        // import added to the table never stops a page instantiating here.
        crate::common::wasm_host::link_unmodelled_imports::<Data>(l);
        // Format hosts for string interpolation — write the value's text at `buf`
        // and return its byte length. The DOM tier needs these the moment a view
        // interpolates a value (`"Count: {count}"`). (The float hosts take f64,
        // which this wasmi build won't wrap as a host param — they are exercised in
        // the real browser JS host instead; DOM interpolation is Int/Bool/Text.)
    }

    /// Write `s`'s bytes into the module's memory at `buf`, returning the length —
    /// the shape every `fmt_*_into` host uses to build an interpolated string.
    fn fmt_into(c: &mut wasmi::Caller<'_, Data>, buf: i32, s: &str) -> i32 {
        let mem = c.get_export("memory").unwrap().into_memory().unwrap();
        let bytes = s.as_bytes();
        let data = mem.data_mut(c);
        data[buf as usize..buf as usize + bytes.len()].copy_from_slice(bytes);
        bytes.len() as i32
    }

    /// Write a `Text` (`[len][cap][data_ptr]` header + UTF-8 bytes) at `bump` in the
    /// module's memory, returning `(handle, next_bump)` — the host analog of the
    /// emitter's Text allocation, matching the `args()` layout.
    fn write_text(data: &mut [u8], bump: usize, s: &str) -> (i32, usize) {
        let b = s.as_bytes();
        let dp = bump + 16;
        data[bump..bump + 4].copy_from_slice(&(b.len() as i32).to_le_bytes());
        data[bump + 4..bump + 8].copy_from_slice(&(b.len() as i32).to_le_bytes());
        data[bump + 8..bump + 12].copy_from_slice(&(dp as i32).to_le_bytes());
        data[dp..dp + b.len()].copy_from_slice(b);
        (bump as i32, (dp + b.len() + 7) & !7)
    }

    /// A compiled-and-mounted app plus its MockDom — the driver for the correlated
    /// fetch tests: deliver/fail requests BY ID (in any order), firing each
    /// completion closure through the trampoline with ITS OWN reqId, exactly as
    /// the browser host does.
    struct FetchWorld {
        dom: Data,
        store: wasmi::Store<Data>,
        inst: wasmi::Instance,
    }

    fn fetch_world(src: &str) -> FetchWorld {
        let module = compile_to_wasm(src).expect("compiles to wasm");
        let engine = wasmi::Engine::default();
        let m = wasmi::Module::new(&engine, &module[..]).expect("valid wasm");
        let (dom, l) = dom_linker(&engine);
        let mut store = wasmi::Store::new(&engine, dom.clone());
        let inst = l
            .instantiate(&mut store, &m)
            .unwrap()
            .start(&mut store)
            .unwrap();
        inst.get_typed_func::<(), ()>(&store, "main")
            .unwrap()
            .call(&mut store, ())
            .expect("main runs");
        FetchWorld { dom, store, inst }
    }

    impl FetchWorld {
        /// Deliver `req` with `status`/`headers`/`body` and fire its callback.
        /// Returns whether a delivery actually happened (finality suppresses).
        fn resolve(&mut self, req: i64, status: i64, headers: &[(&str, &str)], body: &str) -> bool {
            let pairs: Vec<(String, String)> = headers
                .iter()
                .map(|(n, v)| (n.to_string(), v.to_string()))
                .collect();
            let Some(handle) = self
                .dom
                .borrow_mut()
                .resolve_fetch(req, status, &pairs, body)
            else {
                return false;
            };
            self.fire(handle, req);
            true
        }

        /// Deliver a structured transport failure and fire the callback.
        fn fail(&mut self, req: i64, error: &str) -> bool {
            let Some(handle) = self.dom.borrow_mut().fail_fetch(req, error) else {
                return false;
            };
            self.fire(handle, req);
            true
        }

        fn fire(&mut self, handle: i32, req: i64) {
            self.inst
                .get_typed_func::<(i32, i64), i64>(&self.store, "__logos_call_closure")
                .expect("trampoline exported")
                .call(&mut self.store, (handle, req))
                .expect("completion fires without trapping");
        }

        fn log(&self) -> Vec<DomOp> {
            self.dom.borrow().log.clone()
        }
    }

    /// Run `main`, then deliver the `index`th fetch with `body` — firing its
    /// completion closure through the trampoline (as the browser's `fetch().then`
    /// does). Returns the op-log (now including whatever the handler mutated).
    fn wasm_build_and_resolve_fetch(src: &str, index: usize, body: &str) -> Vec<DomOp> {
        let mut w = fetch_world(src);
        assert!(
            w.resolve(index as i64 + 1, 200, &[], body),
            "an outstanding fetch"
        );
        w.log()
    }

    /// Compile, mount, then fire one registered closure `times` times against the SAME instance —
    /// which is what a repeating interval is. Firing a fresh instance each time would restart the
    /// countdown and prove nothing about accumulation. Answers the op-log and the step, in
    /// milliseconds, of every interval the page armed.
    /// Build `src`, mount it, then hand the page's clock each of `steps` milliseconds in turn
    /// through its own `__lgx_tick` export, as `runtime.js` does: the op log, and every delay the
    /// page asked to be woken after.
    fn wasm_build_and_pass_time(src: &str, steps: &[i64]) -> (Vec<DomOp>, Vec<i64>) {
        let module = compile_to_wasm(src).expect("compiles to wasm");
        let engine = wasmi::Engine::default();
        let m = wasmi::Module::new(&engine, &module[..]).expect("valid wasm");
        let (dom, l) = dom_linker(&engine);
        let mut store = wasmi::Store::new(&engine, dom.clone());
        let inst = l
            .instantiate(&mut store, &m)
            .unwrap()
            .start(&mut store)
            .unwrap();
        inst.get_typed_func::<(), ()>(&store, "main")
            .unwrap()
            .call(&mut store, ())
            .expect("main runs");
        let tick = inst
            .get_typed_func::<i64, i64>(&store, "__lgx_tick")
            .expect("the page exports its clock");
        for ms in steps {
            tick.call(&mut store, *ms).expect("a tick never traps");
        }
        let d = dom.borrow();
        (d.log.clone(), d.wakes.clone())
    }

    /// `log` replayed onto a tree: each node's own text, its children in document order, and the
    /// node each `id` names — so a repaint that rebuilds an element, patches its text node in
    /// place, or moves it reads the same.
    struct Replayed {
        own: std::collections::HashMap<u32, String>,
        kids: std::collections::HashMap<u32, Vec<u32>>,
        by_id: std::collections::HashMap<String, u32>,
    }

    impl Replayed {
        fn of(log: &[DomOp]) -> Self {
            let mut r = Replayed {
                own: Default::default(),
                kids: Default::default(),
                by_id: Default::default(),
            };
            for op in log {
                match op {
                    DomOp::CreateElement { node, .. } => {
                        r.own.insert(*node, String::new());
                    }
                    DomOp::CreateTextNode { node, text } => {
                        r.own.insert(*node, text.clone());
                    }
                    DomOp::SetText { node, text } => {
                        r.own.insert(*node, text.clone());
                        r.kids.remove(node);
                    }
                    DomOp::ClearChildren { node } => {
                        r.kids.remove(node);
                    }
                    DomOp::AppendChild { parent, child } => {
                        r.detach(*child);
                        r.kids.entry(*parent).or_default().push(*child);
                    }
                    DomOp::InsertBefore {
                        parent,
                        child,
                        before,
                    } => {
                        r.detach(*child);
                        let siblings = r.kids.entry(*parent).or_default();
                        match siblings.iter().position(|c| c == before) {
                            Some(at) => siblings.insert(at, *child),
                            None => siblings.push(*child),
                        }
                    }
                    DomOp::RemoveNode { node } | DomOp::FreeNode { node } => r.detach(*node),
                    DomOp::SetAttribute { node, name, value } if name == "id" => {
                        r.by_id.insert(value.clone(), *node);
                    }
                    _ => {}
                }
            }
            r
        }

        fn detach(&mut self, node: u32) {
            for siblings in self.kids.values_mut() {
                siblings.retain(|c| *c != node);
            }
        }

        /// `node`'s own text plus every child's, in document order.
        fn text(&self, node: u32) -> String {
            let mut t = self.own.get(&node).cloned().unwrap_or_default();
            for k in self.kids.get(&node).into_iter().flatten() {
                t.push_str(&self.text(*k));
            }
            t
        }
    }

    /// The first text the element `#id` shows as `log` is applied op by op — what its first paint
    /// drew there, whatever order the paint's own ops came in — or `None` if it never shows any.
    fn first_shown(log: &[DomOp], id: &str) -> Option<String> {
        (1..=log.len()).find_map(|n| {
            let r = Replayed::of(&log[..n]);
            r.by_id
                .get(id)
                .map(|&node| r.text(node))
                .filter(|t| !t.is_empty())
        })
    }

    /// The text the element `#id` shows at the end of `log`.
    fn shown(log: &[DomOp], id: &str) -> String {
        let r = Replayed::of(log);
        let node = *r
            .by_id
            .get(id)
            .unwrap_or_else(|| panic!("no element #{id} in {log:?}"));
        r.text(node)
    }

    /// ★ A PAGE TIMER FIRES THROUGH THE REAL MODULE — the tier a visitor uses, not a model of it.
    ///
    /// `Advance time by …` is gated to `## Test` blocks and a `## Test` block's page queries are
    /// refused on the bytecode tier, so a timer has no cross-tier DOM comparison available. This is
    /// the evidence that stands in its place, and it is stronger: the page is compiled to real
    /// WebAssembly, instantiated, mounted, and then its OWN clock is handed the time that passed
    /// through the same `__lgx_tick` export a browser calls.
    ///
    /// ★ AND IT PROVES THE ARITHMETIC, not just the wiring. The page keeps its own clock and asks
    /// the host to wake it when its earliest timer is due (`dom_wake_after`) — the browser's whole
    /// contribution is saying how much time passed — and `__lgx_tick` decides what that makes due.
    /// Timers every 4 and every 6 seconds ask first to be woken at 4 s. Each then fires once per ITS
    /// OWN period whatever steps the clock takes: in 2-second steps one step moves nothing, the
    /// 4-second timer first fires on the second, the 6-second one on the third, and after six steps
    /// (12 s) they have fired three and two times — exactly what one 12-second step does too. A tick
    /// that fired every time, or never, or on a grid, fails here rather than passing a test that
    /// only checked "something happened".
    #[test]
    fn a_page_timer_fires_through_the_real_module() {
        const ONE: &str = "## Component App\nState:\n    Let n be 0.\nActions:\n    \
            Every 1 seconds as refresh:\n        Set n to n + 1.\nView:\n    <p id=\"n\">{n}</p>\n\n\
            ## Main\nMount App.\n";
        let (_, wakes) = wasm_build_and_pass_time(ONE, &[]);
        assert_eq!(
            wakes.last(),
            Some(&1000),
            "a lone 1-second timer asks to be woken a second on: {wakes:?}"
        );
        for (steps, n) in [(0, "0"), (1, "1"), (3, "3")] {
            let (log, wakes) = wasm_build_and_pass_time(ONE, &vec![1000; steps]);
            assert_eq!(
                shown(&log, "n"),
                n,
                "after {steps} one-second step(s): {log:?}"
            );
            assert_eq!(
                wakes.last(),
                Some(&1000),
                "and asks to be woken for the next beat: {wakes:?}"
            );
        }

        const TWO: &str =
            "## Component App\nState:\n    Let a be 0.\n    Let b be 0.\nActions:\n    \
            Every 4 seconds as quick:\n        Set a to a + 1.\n    \
            Every 6 seconds as slow:\n        Set b to b + 1.\n\
            View:\n    <p id=\"a\">{a}</p>\n    <p id=\"b\">{b}</p>\n\n\
            ## Main\nMount App.\n";
        let (_, wakes) = wasm_build_and_pass_time(TWO, &[]);
        assert_eq!(
            wakes.last(),
            Some(&4000),
            "the earliest of 4 s and 6 s is when the page asks to wake: {wakes:?}"
        );
        for (steps, a, b) in [
            (0, "0", "0"),
            (1, "0", "0"),
            (2, "1", "0"),
            (3, "1", "1"),
            (6, "3", "2"),
        ] {
            let (log, _) = wasm_build_and_pass_time(TWO, &vec![2000; steps]);
            assert_eq!(
                (shown(&log, "a").as_str(), shown(&log, "b").as_str()),
                (a, b),
                "after {steps} two-second step(s), each timer has fired once per its own period: {log:?}"
            );
        }
        let (log, wakes) = wasm_build_and_pass_time(TWO, &[12_000]);
        assert_eq!(
            (shown(&log, "a").as_str(), shown(&log, "b").as_str()),
            ("3", "2"),
            "one twelve-second step fires what six two-second steps do: {log:?}"
        );
        assert_eq!(
            wakes.last(),
            Some(&4000),
            "and the next due is 16 s, four on from 12: {wakes:?}"
        );
    }

    #[test]
    fn dom_tree_builds_identically_on_wasm() {
        let src = "## Main\n\
            Let d be createElement(\"div\").\n\
            Call setText with d and \"hi\".\n\
            Let s be createElement(\"span\").\n\
            Call setAttribute with s and \"class\" and \"x\".\n\
            Call appendChild with d and s.\n\
            Call appendChild with body() and d.\n";
        let expected = vec![
            DomOp::CreateElement {
                node: 1,
                tag: "div".into(),
            },
            DomOp::SetText {
                node: 1,
                text: "hi".into(),
            },
            DomOp::CreateElement {
                node: 2,
                tag: "span".into(),
            },
            DomOp::SetAttribute {
                node: 2,
                name: "class".into(),
                value: "x".into(),
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
        // All three tiers must agree.
        assert_eq!(interp_log(src), expected, "tree-walker");
        assert_eq!(vm_log(src), expected, "VM");
        assert_eq!(wasm_log(src), expected, "wasm(MockDom)");
    }

    #[test]
    fn an_lgx_loop_index_counts_from_one_on_all_three_tiers() {
        // `at index i` is a position in the list, so it counts from 1 like `item i of xs` — and
        // the compiled module paints the same numbers the two in-process engines do.
        let src = "## Component App\nState:\n    Let xs be [\"a\", \"b\", \"c\"].\nView:\n    \
            <ul>{for each x in xs, at index i:}<li>{i}:{x}={item i of xs}</li>{end}</ul>\n\n\
            ## Main\nMount App.\n";
        let wasm = wasm_log(src);
        assert_eq!(interp_log(src), vm_log(src), "tree-walker vs VM");
        assert_eq!(vm_log(src), wasm, "VM vs wasm(MockDom)");
        let painted = Replayed::of(&wasm).text(0);
        assert_eq!(
            painted, "1:a=a2:b=b3:c=c",
            "the painted text, in document order: {wasm:?}"
        );
    }

    #[test]
    fn a_stored_value_comes_back_on_all_three_tiers() {
        // ★ STORAGE EXECUTED, not merely imported. WebAssembly cannot reach `localStorage`
        // itself, so this is the only test that says the whole path works: the emitted
        // `storage_set`/`storage_get` imports, a real host holding the value, and the
        // compiled side turning the host's answer back into a LOGOS `Option of Text`.
        //
        // The value is READ INTO THE PAGE rather than printed, because this harness compares
        // op-logs — and a stored value that reaches the document is the thing a persisted
        // store actually needs.
        let src = "## Main\n\
            Call storageSet with \"local\" and \"draft\" and \"restored\".\n\
            Let v be storageGetOr(\"local\", \"draft\", \"MISSING\").\n\
            Let p be createElement(\"p\").\n\
            Call setText with p and v.\n\
            Call appendChild with body() and p.\n";
        let expected = vec![
            DomOp::CreateElement {
                node: 1,
                tag: "p".into(),
            },
            DomOp::SetText {
                node: 1,
                text: "restored".into(),
            },
            DomOp::AppendChild {
                parent: 0,
                child: 1,
            },
        ];
        assert_eq!(interp_log(src), expected, "tree-walker");
        assert_eq!(vm_log(src), expected, "VM");
        assert_eq!(wasm_log(src), expected, "wasm(MockDom)");
    }

    #[test]
    fn a_key_that_is_not_there_reads_as_absent_on_all_three_tiers() {
        // The other half, and the one a naive host gets wrong: an absent key answers `Nothing`,
        // not `""`. A page that could not tell them apart would overwrite a saved empty draft
        // with a default, or resurrect a value nobody ever saved. On wasm the absence is host
        // handle `0`, boxed by the lowering into a real `Nothing` — so the branch below must be
        // skipped on every tier alike.
        let src = "## Main\n\
            Call storageSet with \"local\" and \"other\" and \"\".\n\
            Let v be storageGetOr(\"local\", \"draft\", \"absent\").\n\
            Let p be createElement(\"p\").\n\
            Call setText with p and v.\n\
            Call appendChild with body() and p.\n";
        let expected = vec![
            DomOp::CreateElement {
                node: 1,
                tag: "p".into(),
            },
            DomOp::SetText {
                node: 1,
                text: "absent".into(),
            },
            DomOp::AppendChild {
                parent: 0,
                child: 1,
            },
        ];
        assert_eq!(interp_log(src), expected, "tree-walker");
        assert_eq!(vm_log(src), expected, "VM");
        assert_eq!(wasm_log(src), expected, "wasm(MockDom)");
    }

    #[test]
    fn the_two_storage_areas_stay_apart_on_all_three_tiers() {
        // The area is part of a key's IDENTITY. If `local` and `session` shared a table, a
        // store `preserved across reloads` would quietly outlive the session it named — on a
        // visitor's machine, where nothing would ever say so.
        let src = "## Main\n\
            Call storageSet with \"local\" and \"k\" and \"long-lived\".\n\
            Call storageSet with \"session\" and \"k\" and \"this-tab\".\n\
            Let a be storageGetOr(\"local\", \"k\", \"?\").\n\
            Let b be storageGetOr(\"session\", \"k\", \"?\").\n\
            Let p be createElement(\"p\").\n\
            Call setText with p and a.\n\
            Let q be createElement(\"p\").\n\
            Call setText with q and b.\n\
            Call appendChild with body() and p.\n\
            Call appendChild with body() and q.\n";
        let expected = vec![
            DomOp::CreateElement {
                node: 1,
                tag: "p".into(),
            },
            DomOp::SetText {
                node: 1,
                text: "long-lived".into(),
            },
            DomOp::CreateElement {
                node: 2,
                tag: "p".into(),
            },
            DomOp::SetText {
                node: 2,
                text: "this-tab".into(),
            },
            DomOp::AppendChild {
                parent: 0,
                child: 1,
            },
            DomOp::AppendChild {
                parent: 0,
                child: 2,
            },
        ];
        assert_eq!(interp_log(src), expected, "tree-walker");
        assert_eq!(vm_log(src), expected, "VM");
        assert_eq!(wasm_log(src), expected, "wasm(MockDom)");
    }

    #[test]
    fn a_removed_key_reads_as_absent_on_all_three_tiers() {
        let src = "## Main\n\
            Call storageSet with \"local\" and \"k\" and \"v\".\n\
            Call storageRemove with \"local\" and \"k\".\n\
            Let v be storageGetOr(\"local\", \"k\", \"gone\").\n\
            Let p be createElement(\"p\").\n\
            Call setText with p and v.\n\
            Call appendChild with body() and p.\n";
        let expected = vec![
            DomOp::CreateElement {
                node: 1,
                tag: "p".into(),
            },
            DomOp::SetText {
                node: 1,
                text: "gone".into(),
            },
            DomOp::AppendChild {
                parent: 0,
                child: 1,
            },
        ];
        assert_eq!(interp_log(src), expected, "tree-walker");
        assert_eq!(vm_log(src), expected, "VM");
        assert_eq!(wasm_log(src), expected, "wasm(MockDom)");
    }

    /// Run `src` TWICE against ONE host — a page load, then a reload. The document is rebuilt
    /// (a fresh `MockDom`'s node ids restart) but the STORAGE carries over, which is exactly what
    /// a browser does: `localStorage` outlives the page that wrote it.
    fn wasm_log_after_reload(src: &str) -> Vec<DomOp> {
        let module = compile_to_wasm(src).expect("compiles to wasm");
        let engine = wasmi::Engine::default();
        let m = wasmi::Module::new(&engine, &module[..]).expect("valid wasm");
        let run = |carry: std::collections::BTreeMap<(String, String), String>| {
            let (dom, l) = dom_linker(&engine);
            dom.borrow_mut().storage = carry;
            let mut store = wasmi::Store::new(&engine, dom.clone());
            let inst = l
                .instantiate(&mut store, &m)
                .unwrap()
                .start(&mut store)
                .unwrap();
            inst.get_typed_func::<(), ()>(&store, "main")
                .unwrap()
                .call(&mut store, ())
                .expect("main runs");
            let saved = dom.borrow().storage.clone();
            let log = dom.borrow().log.clone();
            (saved, log)
        };
        let (saved, _first) = run(std::collections::BTreeMap::new());
        run(saved).1
    }

    #[test]
    fn a_persisted_store_survives_a_reload_in_the_browser() {
        // ★★ THE LOAD-BEARING TEST for storage on the tier that matters. Everything else about
        // persistence — the host imports, the write-through, the read-back — can be right while
        // the page still comes back empty, and an import list cannot tell you. This runs a real
        // compiled module twice against one host and asks the only question a visitor asks:
        // after the reload, is what I typed still on the page?
        //
        // The first load starts from the initializer and an action writes "typed" through to
        // storage. The second load reads it back over the initializer, so the page paints the
        // stored text on its FIRST paint — no click, no event, nothing to re-run.
        let src = "## Store Draft persisted as \"draft\"\n\
            State:\n\
            \x20   Let text be \"empty\".\n\
            Actions:\n\
            \x20   To type (t: Text):\n\
            \x20       Set text to t.\n\
            \n\
            ## Component App\n\
            Use Draft.\n\
            Actions:\n\
            \x20   To fill:\n\
            \x20       Call Draft's type with \"typed\".\n\
            View:\n\
            \x20   <p id=\"t\">{Draft's text}</p>\n\
            \x20   <button id=\"go\" on-click={fill}>x</button>\n\
            \n\
            ## Main\n\
            Mount App.\n\
            Click \"#go\".\n";
        let reloaded = wasm_log_after_reload(src);
        assert_eq!(
            first_shown(&reloaded, "t").as_deref(),
            Some("typed"),
            "a persisted store comes back after a reload, on the FIRST paint — not after some later \
             event: {reloaded:?}"
        );
    }

    #[test]
    fn a_local_store_does_not_survive_a_reload_in_the_browser() {
        // The other half. Persistence is opt-in, and a local store that came back would be
        // keeping a visitor's working state on their machine without the page ever saying so.
        let src = "## Store Draft\n\
            State:\n\
            \x20   Let text be \"empty\".\n\
            Actions:\n\
            \x20   To type (t: Text):\n\
            \x20       Set text to t.\n\
            \n\
            ## Component App\n\
            Use Draft.\n\
            Actions:\n\
            \x20   To fill:\n\
            \x20       Call Draft's type with \"typed\".\n\
            View:\n\
            \x20   <p id=\"t\">{Draft's text}</p>\n\
            \x20   <button id=\"go\" on-click={fill}>x</button>\n\
            \n\
            ## Main\n\
            Mount App.\n\
            Click \"#go\".\n";
        let reloaded = wasm_log_after_reload(src);
        assert_eq!(
            first_shown(&reloaded, "t").as_deref(),
            Some("empty"),
            "a local store starts from its own initializer on every load: {reloaded:?}"
        );
    }

    /// A click program: build a counter, register a click handler that captures it
    /// and sets its text. The handler shape is `(Event) -> Unit`.
    const CLICK_SRC: &str = "## Main\n\
        Let counter be createElement(\"span\").\n\
        Call appendChild with body() and counter.\n\
        Let onClick be (e: Int) -> setText(counter, \"clicked\").\n\
        Call addEventListener with counter and \"click\" and onClick.\n";

    #[test]
    fn click_registration_logs_identically_on_all_tiers() {
        // Before any event fires, all three tiers record the same build + register.
        let expected = vec![
            DomOp::CreateElement {
                node: 1,
                tag: "span".into(),
            },
            DomOp::AppendChild {
                parent: 0,
                child: 1,
            },
            DomOp::AddEventListener {
                node: 1,
                event: "click".into(),
            },
        ];
        assert_eq!(interp_log(CLICK_SRC), expected, "tree-walker");
        assert_eq!(vm_log(CLICK_SRC), expected, "VM");
        assert_eq!(wasm_log(CLICK_SRC), expected, "wasm(MockDom)");
    }

    #[test]
    fn click_handler_mutates_dom_through_the_trampoline() {
        // Fire the click through `__logos_call_closure` — exactly what a browser
        // event does. The trailing `SetText` proves the LOGOS handler ran in wasm
        // and drove the DOM: the JS→WASM→DOM path, end to end.
        let log = wasm_build_and_fire(CLICK_SRC, 1, "click", 0);
        assert_eq!(
            log,
            vec![
                DomOp::CreateElement {
                    node: 1,
                    tag: "span".into()
                },
                DomOp::AppendChild {
                    parent: 0,
                    child: 1
                },
                DomOp::AddEventListener {
                    node: 1,
                    event: "click".into()
                },
                DomOp::SetText {
                    node: 1,
                    text: "clicked".into()
                },
            ]
        );
    }

    /// The contract shared with the backend (`site.lg` in the CLI capstone test):
    /// what `GET /api/schedule` returns and what the frontend renders.
    const SCHEDULE: &str = "9am Keynote / 10am Lunch / 2pm Hack";

    #[test]
    fn capstone_frontend_builds_page_and_renders_fetched_schedule() {
        // The real hackathon frontend: build a heading + a schedule slot, fetch the
        // schedule from the LOGOS backend, and render it into the DOM on arrival.
        let app = "# App\n\n\
            ## Main\n\
            Let heading be createElement(\"h1\").\n\
            Call setText with heading and \"LOGOS Hackathon\".\n\
            Call appendChild with body() and heading.\n\
            Let schedule be createElement(\"div\").\n\
            Call setAttribute with schedule and \"id\" and \"schedule\".\n\
            Call appendChild with body() and schedule.\n\
            Let onSchedule be (reqId: Int) -> setText(schedule, responseText()).\n\
            Call fetchText with \"/api/schedule\" and onSchedule.\n";
        let log = wasm_build_and_resolve_fetch(app, 0, SCHEDULE);
        assert_eq!(
            log,
            vec![
                DomOp::CreateElement {
                    node: 1,
                    tag: "h1".into()
                },
                DomOp::SetText {
                    node: 1,
                    text: "LOGOS Hackathon".into()
                },
                DomOp::AppendChild {
                    parent: 0,
                    child: 1
                },
                DomOp::CreateElement {
                    node: 2,
                    tag: "div".into()
                },
                DomOp::SetAttribute {
                    node: 2,
                    name: "id".into(),
                    value: "schedule".into()
                },
                DomOp::AppendChild {
                    parent: 0,
                    child: 2
                },
                // the fetch resolves and the schedule is rendered into #schedule:
                DomOp::SetText {
                    node: 2,
                    text: SCHEDULE.into()
                },
            ]
        );
    }

    #[test]
    fn fetch_completion_updates_dom_with_the_response() {
        // The client fetches from the LOGOS backend; when the response arrives the
        // completion closure reads the body via responseText() and drives the DOM.
        let src = "## Main\n\
            Let target be createElement(\"div\").\n\
            Call appendChild with body() and target.\n\
            Let onDone be (reqId: Int) -> setText(target, responseText()).\n\
            Call fetchText with \"/api/schedule\" and onDone.\n";
        let log = wasm_build_and_resolve_fetch(src, 0, "9am keynote");
        assert_eq!(
            log,
            vec![
                DomOp::CreateElement {
                    node: 1,
                    tag: "div".into()
                },
                DomOp::AppendChild {
                    parent: 0,
                    child: 1
                },
                // fetchText logs nothing (not a DOM mutation); the response arrives:
                DomOp::SetText {
                    node: 1,
                    text: "9am keynote".into()
                },
            ]
        );
    }

    /// The LGX twin of the retired dialect's `reactive_fetch_into_a_text_signal_re_renders_
    /// with_the_body_on_wasm`: a `State:` field starts as a placeholder, `When the page loads:`
    /// sends a typed fetch, and the completion action writes the field with the DECODED VALUE —
    /// which the FRAMEWORK'S OWN automatic repaint shows, not a hand-written `setText` the way
    /// `fetch_completion_updates_dom_with_the_response` above drives a plain `## Main` script.
    ///
    /// ★ A TEXT REACHES THE PAGE TWO WAYS. The first paint builds the hole's text node already
    /// holding the placeholder (`CreateTextNode`); a later repaint patches that node in place
    /// (`SetText`). Reading only one of them would miss the other half of the story, so the oracle
    /// reads both, in order — and then asks the page itself: `#msg` shows the placeholder up to the
    /// moment the answer arrives, and the fetched body after it.
    #[test]
    fn lgx_a_fetch_completion_re_renders_the_view_with_the_body_on_wasm() {
        const SRC: &str = concat!(
            "## Component App\n",
            "State:\n",
            "    Let msg be \"loading\".\n",
            "Actions:\n",
            "    When the page loads:\n",
            "        Get \"/api\" as Text, then loaded.\n",
            "    To loaded (r: Result of Text and HttpError):\n",
            "        Inspect r:\n",
            "            When Ok (v):\n",
            "                Set msg to v.\n",
            "            When Err (e):\n",
            "                Set msg to e's message.\n",
            "View:\n",
            "    <p id=\"msg\">{msg}</p>\n",
            "\n## Main\n",
            "Mount App.\n",
        );
        assert_eq!(
            tw_outcome(SRC).error,
            None,
            "the tree-walker must run this source cleanly (the failure below is wasm-only)"
        );
        let log = wasm_build_and_resolve_fetch(SRC, 0, "hello from LOGOS");
        let texts: Vec<String> = log
            .iter()
            .filter_map(|op| match op {
                DomOp::CreateTextNode { text, .. } | DomOp::SetText { text, .. } => {
                    Some(text.clone())
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            texts.first(),
            Some(&"loading".to_string()),
            "the initial paint shows the placeholder: {texts:?}"
        );
        assert_eq!(
            texts.last(),
            Some(&"hello from LOGOS".to_string()),
            "the completion repainted the view with the fetched body: {texts:?}"
        );
        let answered = log
            .iter()
            .position(|op| {
                matches!(op, DomOp::CreateTextNode { text, .. } | DomOp::SetText { text, .. }
                    if text == "hello from LOGOS")
            })
            .expect("the body reaches the page");
        assert_eq!(
            shown(&log[..answered], "msg"),
            "loading",
            "until the answer arrives, `#msg` shows the placeholder: {log:?}"
        );
        assert_eq!(
            shown(&log, "msg"),
            "hello from LOGOS",
            "and then the fetched body: {log:?}"
        );
    }

    // ---- The correlated fetch contract (G1): typed requests, per-request
    // responses, structured failures — the anti-misdelivery locks. ----

    /// Every `SetText` in the log, as `(node, text)` — the observation surface
    /// for "which callback wrote what where".
    fn set_texts(log: &[DomOp]) -> Vec<(u32, String)> {
        log.iter()
            .filter_map(|op| match op {
                DomOp::SetText { node, text } => Some((*node, text.clone())),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn concurrent_fetches_resolved_out_of_order_deliver_to_their_own_callbacks() {
        // THE anti-misdelivery lock: two requests in flight, resolved OUT OF
        // ORDER. Each callback receives ITS OWN reqId and must see ITS OWN
        // status+body. Under the old single-global-response host, resolving B
        // would overwrite A's body and both callbacks would read whatever
        // arrived last; the status gate would misfire entirely (status was
        // unobservable). The `If` on the status proves the status ALSO
        // correlated correctly — a swapped delivery writes nothing.
        let src = concat!(
            "## Main\n",
            "Let elA be createElement(\"div\").\n",
            "Call appendChild with body() and elA.\n",
            "Let elB be createElement(\"div\").\n",
            "Call appendChild with body() and elB.\n",
            "Let onA be (r: Int) ->:\n",
            "    Let s be responseStatus(r).\n",
            "    If s is 200:\n",
            "        Call setText with elA and responseBody(r).\n",
            "Let onB be (r: Int) ->:\n",
            "    Let s be responseStatus(r).\n",
            "    If s is 409:\n",
            "        Call setText with elB and responseBody(r).\n",
            "Let hs be a new Seq of Text.\n",
            "Let a be fetchSend(\"GET\", \"/a\", \"\", hs, onA).\n",
            "Let b be fetchSend(\"GET\", \"/b\", \"\", hs, onB).\n",
        );
        let mut w = fetch_world(src);
        assert!(
            w.resolve(2, 409, &[], "conflict-b"),
            "request 2 delivers first"
        );
        assert!(w.resolve(1, 200, &[], "ok-a"), "request 1 delivers second");
        assert_eq!(
            set_texts(&w.log()),
            vec![(2, "conflict-b".to_string()), (1, "ok-a".to_string())],
            "each callback saw its OWN status and body, in delivery order"
        );
    }

    #[test]
    fn denial_conflict_and_validation_statuses_reach_their_callbacks() {
        // 403 / 409 / 422 — the REST failure triptych. Three requests, each
        // callback branching on its own status; each branch fires exactly once.
        let src = concat!(
            "## Main\n",
            "Let el1 be createElement(\"p\").\n",
            "Call appendChild with body() and el1.\n",
            "Let el2 be createElement(\"p\").\n",
            "Call appendChild with body() and el2.\n",
            "Let el3 be createElement(\"p\").\n",
            "Call appendChild with body() and el3.\n",
            "Let on1 be (r: Int) ->:\n",
            "    Let s be responseStatus(r).\n",
            "    If s is 403:\n",
            "        Call setText with el1 and \"denied\".\n",
            "Let on2 be (r: Int) ->:\n",
            "    Let s be responseStatus(r).\n",
            "    If s is 409:\n",
            "        Call setText with el2 and \"conflict\".\n",
            "Let on3 be (r: Int) ->:\n",
            "    Let s be responseStatus(r).\n",
            "    If s is 422:\n",
            "        Call setText with el3 and \"invalid\".\n",
            "Let hs be a new Seq of Text.\n",
            "Let a be fetchSend(\"POST\", \"/orders\", \"n=1\", hs, on1).\n",
            "Let b be fetchSend(\"PUT\", \"/orders/7\", \"n=2\", hs, on2).\n",
            "Let c be fetchSend(\"POST\", \"/orders\", \"bad\", hs, on3).\n",
        );
        let mut w = fetch_world(src);
        assert!(w.resolve(1, 403, &[], "forbidden"));
        assert!(w.resolve(2, 409, &[], "version moved"));
        assert!(w.resolve(3, 422, &[], "missing field"));
        assert_eq!(
            set_texts(&w.log()),
            vec![
                (1, "denied".to_string()),
                (2, "conflict".to_string()),
                (3, "invalid".to_string())
            ],
        );
    }

    #[test]
    fn timeout_fires_the_structured_error_and_the_callback() {
        // `fetchSetTimeout` arms the request's timeout (recorded per-request);
        // firing delivers the `"timeout"` error THROUGH the callback — the app
        // observes it, it is never a silent nothing.
        let src = concat!(
            "## Main\n",
            "Let el be createElement(\"p\").\n",
            "Call appendChild with body() and el.\n",
            "Let on be (r: Int) ->:\n",
            "    Call setText with el and responseError(r).\n",
            "Let hs be a new Seq of Text.\n",
            "Let a be fetchSend(\"GET\", \"/slow\", \"\", hs, on).\n",
            "Call fetchSetTimeout with a and 50.\n",
        );
        let mut w = fetch_world(src);
        assert_eq!(
            w.dom.borrow().fetch_req(1).unwrap().timeout_ms,
            50,
            "the timeout is armed per-request"
        );
        assert!(w.fail(1, "timeout"), "the timer fires the failure");
        assert_eq!(set_texts(&w.log()), vec![(1, "timeout".to_string())]);
        assert!(
            !w.resolve(1, 200, &[], "late"),
            "a late response after the timeout never delivers"
        );
        assert_eq!(
            set_texts(&w.log()).len(),
            1,
            "and the callback never re-fires"
        );
    }

    #[test]
    fn abort_suppresses_delivery_entirely() {
        // `fetchAbort` finalizes the request with the `"abort"` error; the
        // resolve that would have delivered is SUPPRESSED — no callback, no op.
        let src = concat!(
            "## Main\n",
            "Let el be createElement(\"p\").\n",
            "Call appendChild with body() and el.\n",
            "Let on be (r: Int) ->:\n",
            "    Call setText with el and responseBody(r).\n",
            "Let hs be a new Seq of Text.\n",
            "Let a be fetchSend(\"GET\", \"/feed\", \"\", hs, on).\n",
            "Call fetchAbort with a.\n",
        );
        let mut w = fetch_world(src);
        assert!(
            !w.resolve(1, 200, &[], "late body"),
            "an aborted request never delivers"
        );
        assert!(set_texts(&w.log()).is_empty(), "the callback never ran");
        let q = w.dom.borrow().fetch_req(1).unwrap().clone();
        assert!(q.aborted && q.done);
        assert_eq!(q.error, "abort");
    }

    #[test]
    fn transport_failure_surfaces_as_a_structured_network_error() {
        // A malformed / refused / dropped connection is `.catch` in the browser —
        // here the structured `network: <detail>` error, delivered through the
        // callback (the old host had NO rejection path: failure = nothing, ever).
        let src = concat!(
            "## Main\n",
            "Let el be createElement(\"p\").\n",
            "Call appendChild with body() and el.\n",
            "Let on be (r: Int) ->:\n",
            "    Call setText with el and responseError(r).\n",
            "Let hs be a new Seq of Text.\n",
            "Let a be fetchSend(\"GET\", \"/down\", \"\", hs, on).\n",
        );
        let mut w = fetch_world(src);
        assert!(w.fail(1, "network: connection refused"));
        assert_eq!(
            set_texts(&w.log()),
            vec![(1, "network: connection refused".to_string())]
        );
        assert!(
            !w.fail(1, "network: again"),
            "failure is final — no double delivery"
        );
    }

    #[test]
    fn oversized_response_becomes_a_delivered_toolarge_failure() {
        // A body past the response cap is NEVER truncated-and-delivered: the
        // request fails with `"toolarge"`, the body reads empty, and the
        // callback still runs so the app can react.
        let src = concat!(
            "## Main\n",
            "Let el1 be createElement(\"p\").\n",
            "Call appendChild with body() and el1.\n",
            "Let el2 be createElement(\"p\").\n",
            "Call appendChild with body() and el2.\n",
            "Let on be (r: Int) ->:\n",
            "    Call setText with el1 and responseError(r).\n",
            "    Call setText with el2 and responseBody(r).\n",
            "Let hs be a new Seq of Text.\n",
            "Let a be fetchSend(\"GET\", \"/huge\", \"\", hs, on).\n",
        );
        let mut w = fetch_world(src);
        let huge = "x".repeat(FETCH_RESPONSE_BODY_CAP + 1);
        assert!(w.resolve(1, 200, &[], &huge), "the failure IS delivered");
        assert_eq!(
            set_texts(&w.log()),
            vec![(1, "toolarge".to_string()), (2, String::new())],
        );
    }

    #[test]
    fn response_headers_read_case_insensitively_and_absent_reads_empty() {
        let src = concat!(
            "## Main\n",
            "Let el1 be createElement(\"p\").\n",
            "Call appendChild with body() and el1.\n",
            "Let el2 be createElement(\"p\").\n",
            "Call appendChild with body() and el2.\n",
            "Let on be (r: Int) ->:\n",
            "    Call setText with el1 and responseHeader(r, \"content-type\").\n",
            "    Call setText with el2 and responseHeader(r, \"x-absent\").\n",
            "Let hs be a new Seq of Text.\n",
            "Let a be fetchSend(\"GET\", \"/api\", \"\", hs, on).\n",
        );
        let mut w = fetch_world(src);
        assert!(w.resolve(1, 200, &[("Content-Type", "application/json")], "{}"));
        assert_eq!(
            set_texts(&w.log()),
            vec![(1, "application/json".to_string()), (2, String::new())],
        );
    }

    #[test]
    fn disallowed_request_header_fails_the_request_before_any_network() {
        // The allowlist is enforced at SEND: the request is final immediately,
        // its error is readable synchronously, and no later resolve can deliver.
        let src = concat!(
            "## Main\n",
            "Let el be createElement(\"p\").\n",
            "Call appendChild with body() and el.\n",
            "Let on be (r: Int) ->:\n",
            "    Call setText with el and \"ran\".\n",
            "Let hs be a new Seq of Text.\n",
            "Push \"x-evil\" to hs.\n",
            "Push \"1\" to hs.\n",
            "Let a be fetchSend(\"GET\", \"/x\", \"\", hs, on).\n",
            "Call setText with el and responseError(a).\n",
        );
        let mut w = fetch_world(src);
        assert_eq!(
            set_texts(&w.log()),
            vec![(1, "network: header not allowed: x-evil".to_string())],
            "the denial is synchronous and structured"
        );
        assert!(
            !w.resolve(1, 200, &[], "late"),
            "the pre-failed request is final"
        );
    }

    #[test]
    fn fetch_text_sugar_rides_the_same_correlated_engine() {
        // `fetchText` and `fetchSend` share ONE id space and one request table —
        // the sugar can never behave differently from the core.
        let src = concat!(
            "## Main\n",
            "Let el be createElement(\"p\").\n",
            "Call appendChild with body() and el.\n",
            "Let onText be (r: Int) -> setText(el, responseText()).\n",
            "Let onSend be (r: Int) -> setText(el, responseBody(r)).\n",
            "Call fetchText with \"/legacy\" and onText.\n",
            "Let hs be a new Seq of Text.\n",
            "Let b be fetchSend(\"POST\", \"/new\", \"n=1\", hs, onSend).\n",
        );
        let mut w = fetch_world(src);
        {
            let d = w.dom.borrow();
            let q1 = d.fetch_req(1).unwrap();
            assert_eq!(
                (q1.method.as_str(), q1.url.as_str(), q1.body.as_str()),
                ("GET", "/legacy", "")
            );
            let q2 = d.fetch_req(2).unwrap();
            assert_eq!(
                (q2.method.as_str(), q2.url.as_str(), q2.body.as_str()),
                ("POST", "/new", "n=1")
            );
        }
        // Resolve the CORE request first, then the sugar one — both correlate.
        assert!(w.resolve(2, 201, &[], "created"));
        assert!(w.resolve(1, 200, &[], "legacy body"));
        assert_eq!(
            set_texts(&w.log()),
            vec![(1, "created".to_string()), (1, "legacy body".to_string())],
        );
    }

    /// A compiled-and-mounted app whose MockDom carries the router state — the driver
    /// for the pop (back/forward) tests: `deliver_pop(path)` moves the current path and
    /// fires the registered handler through the trampoline with the new path Text handle,
    /// exactly as the browser host's popstate listener does.
    struct RouteWorld {
        dom: Data,
        store: wasmi::Store<Data>,
        inst: wasmi::Instance,
    }

    /// Compile + run `main` with the MockDom initialized at `path`/`query` (a refresh /
    /// deep-link: the app boots already AT that location).
    fn route_world(src: &str, path: &str, query: &str) -> RouteWorld {
        let module = compile_to_wasm(src).expect("compiles to wasm");
        let engine = wasmi::Engine::default();
        let m = wasmi::Module::new(&engine, &module[..]).expect("valid wasm");
        let (dom, l) = dom_linker(&engine);
        dom.borrow_mut().init_route(path, query);
        let mut store = wasmi::Store::new(&engine, dom.clone());
        let inst = l
            .instantiate(&mut store, &m)
            .unwrap()
            .start(&mut store)
            .unwrap();
        inst.get_typed_func::<(), ()>(&store, "main")
            .unwrap()
            .call(&mut store, ())
            .expect("main runs");
        RouteWorld { dom, store, inst }
    }

    impl RouteWorld {
        /// Deliver a popstate (back/forward) at `path`: move the current path FIRST,
        /// then fire the registered route handler through the trampoline with an Int
        /// event tick (`0`) — the handler reads the new path via `routePath()`, mirroring
        /// fetch's `onDone(reqId: Int)`. Returns whether a handler was registered to fire.
        fn deliver_pop(&mut self, path: &str) -> bool {
            let handle = {
                let mut d = self.dom.borrow_mut();
                d.set_route_path(path);
                d.route_handler()
            };
            let Some(handle) = handle else { return false };
            self.inst
                .get_typed_func::<(i32, i64), i64>(&self.store, "__logos_call_closure")
                .expect("trampoline exported")
                .call(&mut self.store, (handle, 0))
                .expect("route handler fires without trapping");
            true
        }

        fn log(&self) -> Vec<DomOp> {
            self.dom.borrow().log.clone()
        }
        fn route_ops(&self) -> Vec<MockRouteOp> {
            self.dom.borrow().route_ops.clone()
        }
    }

    #[test]
    fn navigate_and_replace_lower_to_the_router_hosts_on_wasm() {
        // The emitted module drives the `dom_navigate`/`dom_replace_route` hosts; the
        // MockDom records them per navigation (pushState vs replaceState) and advances
        // the current path, exactly like the native recorder.
        let src = concat!(
            "## Main\n",
            "Call navigate with \"/todos\".\n",
            "Call replaceRoute with \"/todos/8\".\n",
        );
        let w = route_world(src, "/", "");
        assert_eq!(
            w.route_ops(),
            vec![
                MockRouteOp {
                    path: "/todos".into(),
                    replace: false
                },
                MockRouteOp {
                    path: "/todos/8".into(),
                    replace: true
                },
            ],
        );
        assert_eq!(
            w.dom.borrow().route_path,
            "/todos/8",
            "the current path followed"
        );
    }

    #[test]
    fn refresh_at_a_deep_link_dispatches_the_initial_route_on_wasm() {
        // Refresh / deep-link: the app boots already AT `/todos/42?tab=open`. routePath
        // and routeQuery read the initial location, and the initial render shows the
        // deep-linked route — the whole point of a real router over in-memory nav.
        let src = concat!(
            "## Main\n",
            "Let root be createElement(\"div\").\n",
            "Call setAttribute with root and \"id\" and \"root\".\n",
            "Call appendChild with body() and root.\n",
            "Let path be routePath().\n",
            "Let id be routeParam(\"/todos/:id\", \"id\", path).\n",
            "Let tab be routeQuery(\"tab\").\n",
            "Let label be createElement(\"p\").\n",
            "Set the text of label to id.\n",
            "Call appendChild with root and label.\n",
            "Let label2 be createElement(\"p\").\n",
            "Set the text of label2 to tab.\n",
            "Call appendChild with root and label2.\n",
        );
        let w = route_world(src, "/todos/42", "tab=open");
        let texts: Vec<String> = w
            .log()
            .iter()
            .filter_map(|op| match op {
                DomOp::SetText { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(texts, vec!["42".to_string(), "open".to_string()]);
    }

    /// The LGX twin of the retired dialect's `on_route_handler_fires_on_back_forward_and_
    /// re_renders_the_matched_view`: a `## Component` registers `When the route changes:`
    /// (proven to reach the browser by `trigger_tier_lock`) and a back/forward pop must both
    /// run the handler AND repaint the view it changed, exactly like an element event turn.
    #[test]
    fn lgx_when_the_route_changes_re_renders_on_back_forward_on_wasm() {
        const SRC: &str = "## Component App\nState:\n    Let n be 0.\nActions:\n    \
            When the route changes:\n        Set n to n + 1.\nView:\n    <p id=\"n\">{n}</p>\n\n\
            ## Main\nMount App.\n";
        let mut w = route_world(SRC, "/", "");
        let fired = w.deliver_pop("/b");
        assert!(fired, "onRoute registered a handler for the pop to fire");
        let texts: Vec<String> = w
            .log()
            .iter()
            .filter_map(|op| match op {
                DomOp::SetText { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            texts.last(),
            Some(&"1".to_string()),
            "the pop ran the turn AND repainted the view it changed: {texts:?}"
        );
    }

    /// `Mount App at "#nope"` where nothing matches `#nope` — a loud, named failure on the WASM
    /// tier too, not a silent no-op that swallows the whole page. The VM bytecode compiler
    /// (`vm/compiler.rs`'s `Stmt::Ui(UiStmt::Mount)` arm) already compiles the `querySelector` /
    /// `< 0` / fail-text check into every program that mounts `at` a selector, and the WASM
    /// backend lowers that SAME bytecode — so this proves the check survives all the way to a
    /// real WASM trap, with the compiler's own message, rather than assuming it from reading the
    /// VM compiler alone.
    #[test]
    fn mount_at_a_selector_matching_nothing_traps_by_name_on_wasm() {
        let src = "## Component App\nView:\n    <p>hi</p>\n\n## Main\nMount App at \"#nope\".\n";
        let module = compile_to_wasm(src).expect("compiles to wasm");
        let engine = wasmi::Engine::default();
        let m = wasmi::Module::new(&engine, &module[..]).expect("valid wasm");
        let dom: Data = Rc::new(RefCell::new(MockDom::new()));
        let mut l = wasmi::Linker::<Data>::new(&engine);
        crate::common::wasm_host::link_value_imports::<Data>(&mut l);
        let captured: std::sync::Arc<std::sync::Mutex<Option<String>>> =
            std::sync::Arc::new(std::sync::Mutex::new(None));
        let sink = captured.clone();
        l.func_wrap(
            "env",
            "logos_rt_fail",
            move |c: wasmi::Caller<'_, Data>, h: i32| {
                *sink.lock().unwrap() = Some(read_text(&c, h));
            },
        )
        .unwrap();
        l.func_wrap(
            "env",
            "logos_rt_fail_at",
            |_c: wasmi::Caller<'_, Data>, _code: i32, _v: i64| {},
        )
        .unwrap();
        install_dom_hosts(&mut l);
        let mut store = wasmi::Store::new(&engine, dom.clone());
        let inst = l
            .instantiate(&mut store, &m)
            .unwrap()
            .start(&mut store)
            .unwrap();
        let result = inst
            .get_typed_func::<(), ()>(&store, "main")
            .unwrap()
            .call(&mut store, ());
        assert!(
            result.is_err(),
            "mounting at a selector matching nothing must trap, not silently paint nothing"
        );
        let msg = captured.lock().unwrap().clone();
        let msg = msg.expect("the compiled check must report its failure text before trapping");
        assert!(
            msg.contains("#nope"),
            "the failure must name the selector that matched nothing: {msg}"
        );
        assert!(
            msg.to_lowercase().contains("no element matches")
                || msg.to_lowercase().contains("no such element"),
            "the failure must say NOTHING matched, not just that mounting failed: {msg}"
        );
        assert!(
            dom.borrow().log.is_empty(),
            "a refused mount must not have painted anything first"
        );
    }

    /// ★ LOCK — POSSIBLE DATA LOSS, the wasm-tier half of `early_return_turn_epilogue_lock.rs`
    /// (`logicaffeine_tests`, the lowered-VM half). `noteTurn` and the state write-back both sit
    /// AFTER a turn's own body in the SAME compiled function; a `Return` lowered as a native wasm
    /// `return` used to leave that function immediately, skipping both. `Set n to n + 1. If n is
    /// greater than 0: Return.` must still show `n` as `1`, not `0` (never written back) or `999`
    /// (the guard's own point — the line after `Return` must not run either).
    #[test]
    fn an_early_return_still_commits_its_write_back_on_wasm() {
        const SRC: &str = "## Component App\nState:\n    Let n be 0.\nActions:\n    \
            To bump:\n        Set n to n + 1.\n        If n is greater than 0:\n            Return.\n        Set n to 999.\n\
            View:\n    <p id=\"n\">{n}</p><button id=\"b\" on-click={bump}>x</button>\n\n\
            ## Main\nMount App.\n";
        let log = wasm_build_and_fire_at(SRC, "#b", "click", 0);
        assert!(
            log.iter()
                .any(|op| matches!(op, DomOp::SetText { text, .. } if text == "1")),
            "the write-back before the early Return must still commit (n=1), not stay at 0 or \
             reach the guarded 999: {log:?}"
        );
        assert!(
            !log.iter()
                .any(|op| matches!(op, DomOp::SetText { text, .. } if text == "999")),
            "the statement AFTER the early Return must not run: {log:?}"
        );
    }

    /// ★ LOCK — Tristen's rule, the wasm-runtime half of `duplicate_keyed_loop_lowering_lock.rs`
    /// (`logicaffeine_tests`, which only proves the page still COMPILES). This fires `main` for
    /// real and catches the `Fail` the duplicate-key pre-pass raises during the first render,
    /// through the SAME `logos_rt_fail` capture `mount_at_a_selector_matching_nothing_traps_by_
    /// name_on_wasm` uses, and checks the message is BYTE-IDENTICAL to what the VM tier gives —
    /// the whole point of building it from `duplicate_keyed_loop_message_parts` on both sides.
    #[test]
    fn a_duplicate_keyed_loop_traps_on_wasm_with_the_same_message_as_the_vm() {
        let src = "## Component App\nState:\n    Let items be [1, 2, 1].\nView:\n    \
            <main><ul>{for each x in items, keyed by x:}<li>{x}</li>{end}</ul></main>\n\n\
            ## Main\nMount App.\n";
        let module =
            compile_to_wasm(src).expect("a duplicate key is a RUNTIME Fail, not a compile refusal");
        let engine = wasmi::Engine::default();
        let m = wasmi::Module::new(&engine, &module[..]).expect("valid wasm");
        let dom: Data = Rc::new(RefCell::new(MockDom::new()));
        let mut l = wasmi::Linker::<Data>::new(&engine);
        crate::common::wasm_host::link_value_imports::<Data>(&mut l);
        let captured: std::sync::Arc<std::sync::Mutex<Option<String>>> =
            std::sync::Arc::new(std::sync::Mutex::new(None));
        let sink = captured.clone();
        l.func_wrap(
            "env",
            "logos_rt_fail",
            move |c: wasmi::Caller<'_, Data>, h: i32| {
                *sink.lock().unwrap() = Some(read_text(&c, h));
            },
        )
        .unwrap();
        l.func_wrap(
            "env",
            "logos_rt_fail_at",
            |_c: wasmi::Caller<'_, Data>, _code: i32, _v: i64| {},
        )
        .unwrap();
        install_dom_hosts(&mut l);
        let mut store = wasmi::Store::new(&engine, dom.clone());
        let inst = l
            .instantiate(&mut store, &m)
            .unwrap()
            .start(&mut store)
            .unwrap();
        let result = inst
            .get_typed_func::<(), ()>(&store, "main")
            .unwrap()
            .call(&mut store, ());
        assert!(
            result.is_err(),
            "a duplicate key must trap the first render, not silently paint it"
        );
        let msg = captured.lock().unwrap().clone();
        let msg = msg.expect("the compiled check must report its failure text before trapping");
        assert_eq!(
            msg,
            "the loop over items keys item 1 and item 3 both \"1\" — a `keyed by` loop's keys \
             must be unique among its own siblings; give the duplicate its own key, or drop \
             `keyed by` if position alone already identifies each row here.",
            "must be BYTE-IDENTICAL to the VM tier's own message"
        );
    }

    /// ★ LOCK — the listener-site ledger's gate (`has_any_listener_site`, `lgx_lower.rs`) covers a
    /// component-level `When … :` trigger, not only a view-level `on-click`/`bind`. Those two
    /// registration paths share `ListenerSite::Action`, but `has_listener_site` only ever walked
    /// the VIEW tree — so a component whose ONLY listener is `When the user presses "…":` (a
    /// `def.events` entry, no `ViewNode::Handler` anywhere) was found, while writing this gate, to
    /// still call `register_listener_with` and push into `__lgx_paint_sites` while the gate would
    /// have skipped declaring it: an undeclared-local trap on the one tier that enforces it. This
    /// page has no view-level listener at all, so it exists only to keep that combination covered.
    #[test]
    fn a_component_whose_only_listener_is_a_page_level_trigger_still_runs_on_wasm() {
        const SRC: &str = "## Component App\nState:\n    Let n be 0.\nActions:\n    \
            When the user presses \"Escape\":\n        Set n to n + 1.\nView:\n    <p id=\"n\">{n}</p>\n\n\
            ## Main\nMount App.\n";
        let log = wasm_log(SRC);
        assert!(
            log.iter().any(|op| matches!(op, DomOp::CreateTextNode { text, .. } if text == "0")),
            "a page-level-trigger-only component must still mount and paint its first render: {log:?}"
        );
    }

    /// ★ THE MOCK LINKS THE WHOLE DECLARED TABLE: a module importing every row of the host-import
    /// table instantiates against it, the rows it models with their own stubs and the rest as traps
    /// naming the import — so a host import added to the table never stops a page loading here.
    #[test]
    fn every_declared_host_import_links_against_the_mock_dom() {
        let table = logicaffeine_compile::vm::wasm::host_import_signatures();
        assert!(
            table.len() >= 150,
            "the host-import table read back only {} rows",
            table.len()
        );
        let engine = wasmi::Engine::default();
        let module = crate::common::wasm_host::import_every_row(&table);
        let m = wasmi::Module::new(&engine, &module[..]).expect("the import-only module is valid");
        let (dom, l) = dom_linker(&engine);
        let mut store = wasmi::Store::new(&engine, dom);
        l.instantiate(&mut store, &m)
            .unwrap_or_else(|e| panic!("the mock does not link the whole declared table: {e}"));
    }
}

#[test]
fn dom_query_selector_and_get_text_agree() {
    // querySelector by tag finds the created node; getText reads it back — the
    // returned Text flows into a Show so we also confirm the value crosses tiers.
    let src = "## Main\n\
        Let p be createElement(\"p\").\n\
        Call setText with p and \"hello\".\n\
        Call appendChild with body() and p.\n\
        Let found be querySelector(\"p\").\n\
        Show getText(found).\n";
    // Both tiers must Show the same text AND build the same op-log.
    assert_eq!(tw_outcome(src).output.trim(), "hello");
    assert_eq!(vm_outcome(src).output.trim(), "hello");
    assert_dom_parity(
        src,
        &[
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
        ],
    );
}

// ---- The correlated fetch contract on the NATIVE tiers (tw≡VM parity over the
// shared recorder in `semantics::dom` — the same engine the wasm MockDom and the
// browser host mirror). ----

#[test]
fn fetch_send_records_the_typed_request_identically_on_interp_and_vm() {
    // The request side of the contract: method + url + body + headers land in the
    // recorder, ids are sequential from 1, and the tree-walker and the VM agree
    // on every field (the parity surface `dom::fetch_requests()`).
    let src = concat!(
        "## Main\n",
        "Let onDone be (r: Int) -> responseStatus(r).\n",
        "Let hs be a new Seq of Text.\n",
        "Push \"content-type\" to hs.\n",
        "Push \"application/json\" to hs.\n",
        "Push \"idempotency-key\" to hs.\n",
        "Push \"key-1\" to hs.\n",
        "Let a be fetchSend(\"POST\", \"/api/orders\", \"n=1\", hs, onDone).\n",
        "Let b be fetchSend(\"PUT\", \"/api/orders/7\", \"n=2\", hs, onDone).\n",
        "Show a.\n",
        "Show b.\n",
    );
    dom::reset();
    let tw = tw_outcome(src);
    let tw_reqs = dom::fetch_requests();
    dom::reset();
    let vm = vm_outcome(src);
    let vm_reqs = dom::fetch_requests();
    assert_eq!(
        tw.output.trim(),
        vm.output.trim(),
        "tw vs VM outputs diverged"
    );
    assert_eq!(
        tw.output.trim(),
        "1\n2",
        "request ids are sequential from 1"
    );
    assert_eq!(tw_reqs, vm_reqs, "tw vs VM recorded requests diverged");
    assert_eq!(tw_reqs.len(), 2);
    assert_eq!(tw_reqs[0].method, "POST");
    assert_eq!(tw_reqs[0].url, "/api/orders");
    assert_eq!(tw_reqs[0].body, "n=1");
    assert_eq!(
        tw_reqs[0].headers,
        vec![
            ("content-type".to_string(), "application/json".to_string()),
            ("idempotency-key".to_string(), "key-1".to_string()),
        ],
    );
    assert_eq!(tw_reqs[1].method, "PUT");
    assert_eq!(tw_reqs[1].url, "/api/orders/7");
}

#[test]
fn disallowed_header_and_oversized_body_prefail_identically_on_both_tiers() {
    // Contract violations fail the request AT SEND, synchronously and
    // structurally — observable in the same program run, identical on tw and VM.
    let evil = concat!(
        "## Main\n",
        "Let onDone be (r: Int) -> responseStatus(r).\n",
        "Let hs be a new Seq of Text.\n",
        "Push \"x-evil\" to hs.\n",
        "Push \"1\" to hs.\n",
        "Let a be fetchSend(\"GET\", \"/x\", \"\", hs, onDone).\n",
        "Show responseError(a).\n",
        "Show responseStatus(a).\n",
    );
    dom::reset();
    let tw = tw_outcome(evil);
    dom::reset();
    let vm = vm_outcome(evil);
    assert_eq!(tw.output.trim(), vm.output.trim());
    assert_eq!(tw.output.trim(), "network: header not allowed: x-evil\n0");

    // 8 bytes doubled 20 times = 8 MiB — past the 4 MiB request cap.
    let big = concat!(
        "## Main\n",
        "Let onDone be (r: Int) -> responseStatus(r).\n",
        "Let hs be a new Seq of Text.\n",
        "Let mutable payload be \"xxxxxxxx\".\n",
        "Repeat for i from 1 to 20:\n",
        "    Set payload to payload + payload.\n",
        "Let a be fetchSend(\"POST\", \"/big\", payload, hs, onDone).\n",
        "Show responseError(a).\n",
    );
    dom::reset();
    let tw = tw_outcome(big);
    dom::reset();
    let vm = vm_outcome(big);
    assert_eq!(tw.output.trim(), vm.output.trim());
    assert_eq!(tw.output.trim(), "toolarge");
}

#[test]
fn the_native_recorder_delivers_per_request_with_caps_and_finality() {
    // The driver API the tests (and any native host) use: out-of-order delivery
    // lands per-request, response headers read case-insensitively, an oversized
    // body becomes a DELIVERED "toolarge" failure, and a final request never
    // delivers twice.
    let src = concat!(
        "## Main\n",
        "Let onA be (r: Int) -> responseStatus(r).\n",
        "Let onB be (r: Int) -> responseStatus(r).\n",
        "Call fetchText with \"/a\" and onA.\n",
        "Call fetchText with \"/b\" and onB.\n",
    );
    dom::reset();
    let _ = tw_outcome(src);
    // Deliver request 2 FIRST.
    let headers = vec![("X-Trace".to_string(), "t2".to_string())];
    let c2 = dom::begin_fetch_resolve(2, 200, &headers, "second");
    assert!(c2.is_some(), "request 2 delivers");
    assert_eq!(
        dom::response_text(),
        "second",
        "the sugar reads the CURRENT delivery"
    );
    dom::end_fetch_resolve();
    // Request 1 gets a response past the cap → a delivered "toolarge" failure.
    let huge = "y".repeat(dom::FETCH_RESPONSE_BODY_CAP + 1);
    let c1 = dom::begin_fetch_resolve(1, 200, &[], &huge);
    assert!(
        c1.is_some(),
        "the failure IS delivered (the callback would run)"
    );
    dom::end_fetch_resolve();
    // Per-request state, correlated — nothing overwrote anything.
    assert_eq!(dom::response_status(2), 200);
    assert_eq!(dom::response_body(2), "second");
    assert_eq!(
        dom::response_header(2, "x-trace"),
        "t2",
        "response headers are case-insensitive"
    );
    assert_eq!(dom::response_header(2, "x-missing"), "");
    assert_eq!(dom::response_error(2), "");
    assert_eq!(dom::response_status(1), 0);
    assert_eq!(dom::response_body(1), "");
    assert_eq!(dom::response_error(1), "toolarge");
    // Finality: no double delivery, and failing a delivered request is a no-op.
    assert!(dom::begin_fetch_resolve(2, 500, &[], "again").is_none());
    assert!(dom::fail_fetch(2, "network: late").is_none());
    // An unknown request id resolves to nothing and reads as pending.
    assert!(dom::begin_fetch_resolve(99, 200, &[], "ghost").is_none());
    assert_eq!(dom::response_status(99), 0);
    assert_eq!(dom::response_error(99), "");
}

#[test]
fn abort_and_timeout_control_native_requests_by_id() {
    let src = concat!(
        "## Main\n",
        "Let onA be (r: Int) -> responseStatus(r).\n",
        "Let onB be (r: Int) -> responseStatus(r).\n",
        "Let hs be a new Seq of Text.\n",
        "Let a be fetchSend(\"GET\", \"/a\", \"\", hs, onA).\n",
        "Let b be fetchSend(\"GET\", \"/b\", \"\", hs, onB).\n",
        "Call fetchAbort with a.\n",
        "Call fetchSetTimeout with b and 250.\n",
        "Show responseError(a).\n",
    );
    dom::reset();
    let tw = tw_outcome(src);
    let tw_reqs = dom::fetch_requests();
    dom::reset();
    let vm = vm_outcome(src);
    let vm_reqs = dom::fetch_requests();
    assert_eq!(tw.output.trim(), vm.output.trim());
    assert_eq!(
        tw.output.trim(),
        "abort",
        "abort is observable synchronously"
    );
    assert_eq!(tw_reqs, vm_reqs);
    assert!(tw_reqs[0].aborted);
    assert_eq!(
        tw_reqs[1].timeout_ms, 250,
        "the timeout is armed per-request"
    );
    // The aborted request never delivers; the timed one delivers "timeout" when fired.
    assert!(dom::begin_fetch_resolve(1, 200, &[], "late").is_none());
    assert!(dom::fail_fetch(2, "timeout").is_some());
    assert_eq!(dom::response_error(2), "timeout");
    dom::end_fetch_resolve();
}

#[test]
fn web_lg_wrappers_send_typed_requests_through_the_correlated_core() {
    // The stdlib wrappers (assets/std/web.lg, auto-imported): thin, pure-LOGOS
    // layers over `fetchSend`. `postJsonIdempotent` sets the Idempotency-Key
    // header (safe retries are the CALLER's decision — no implicit retry
    // anywhere); `putJsonExpecting` sets x-expected-version (optimistic
    // concurrency); `webGet`/`deleteAt` are the bare verbs. Identical on tw and
    // the VM, ids sequential.
    let src = concat!(
        "## Main\n",
        "Let onDone be (r: Int) -> responseStatus(r).\n",
        "Let a be webGet(\"/api/items\", onDone).\n",
        "Let b be postJson(\"/api/items\", \"[1]\", onDone).\n",
        "Let c be postJsonIdempotent(\"/api/orders\", \"[2]\", \"key-9\", onDone).\n",
        "Let d be putJsonExpecting(\"/api/orders/7\", \"[3]\", \"v41\", onDone).\n",
        "Let e be deleteAt(\"/api/orders/7\", onDone).\n",
        "Show a.\n",
        "Show b.\n",
        "Show c.\n",
        "Show d.\n",
        "Show e.\n",
    );
    dom::reset();
    let tw = tw_outcome(src);
    let tw_reqs = dom::fetch_requests();
    dom::reset();
    let vm = vm_outcome(src);
    let vm_reqs = dom::fetch_requests();
    assert_eq!(
        tw.output.trim(),
        vm.output.trim(),
        "tw vs VM outputs diverged"
    );
    assert_eq!(tw.output.trim(), "1\n2\n3\n4\n5");
    assert_eq!(tw_reqs, vm_reqs, "tw vs VM recorded requests diverged");
    let shape: Vec<(&str, &str, &str)> = tw_reqs
        .iter()
        .map(|r| (r.method.as_str(), r.url.as_str(), r.body.as_str()))
        .collect();
    assert_eq!(
        shape,
        vec![
            ("GET", "/api/items", ""),
            ("POST", "/api/items", "[1]"),
            ("POST", "/api/orders", "[2]"),
            ("PUT", "/api/orders/7", "[3]"),
            ("DELETE", "/api/orders/7", ""),
        ],
    );
    let json = ("content-type".to_string(), "application/json".to_string());
    assert_eq!(tw_reqs[1].headers, vec![json.clone()]);
    assert_eq!(
        tw_reqs[2].headers,
        vec![
            json.clone(),
            ("idempotency-key".to_string(), "key-9".to_string())
        ],
    );
    assert_eq!(
        tw_reqs[3].headers,
        vec![json, ("x-expected-version".to_string(), "v41".to_string())],
    );
    assert!(tw_reqs[0].headers.is_empty() && tw_reqs[4].headers.is_empty());
}

// ---- G4 — the WASM-DOM history/location router. `routePath`/`routeQuery` read
// window.location; `navigate`/`replaceRoute` are history.pushState/replaceState
// (recorded per navigation); `onRoute` registers a popstate handler the browser
// fires with the new path. The `matchRoute`/`routeParam` pattern matcher is
// pure-LOGOS in web.lg over `routePath`. The native tiers land in the thread-local
// recorder (tw≡VM parity of the nav op-log + the current path), and — mirroring
// the correlated-fetch driver — a `deliver_pop(path)` fires the registered handler
// on command; the wasm(MockDom) arm proves the trampoline + back/forward. ----

#[test]
fn navigate_and_replace_record_ops_and_move_the_current_path_on_both_tiers() {
    // pushState vs replaceState is a real distinction — a `navigate` adds a history
    // entry, a `replaceRoute` swaps the current one — and both advance routePath().
    // The tree-walker and the VM must record the identical nav op-log and end at the
    // same path.
    let src = concat!(
        "## Main\n",
        "Show routePath().\n",
        "Call navigate with \"/todos\".\n",
        "Show routePath().\n",
        "Call navigate with \"/todos/7\".\n",
        "Call replaceRoute with \"/todos/8\".\n",
        "Show routePath().\n",
    );
    dom::reset();
    let tw = tw_outcome(src);
    let tw_nav = dom::route_ops();
    dom::reset();
    let vm = vm_outcome(src);
    let vm_nav = dom::route_ops();
    assert_eq!(
        tw.output.trim(),
        vm.output.trim(),
        "tw vs VM outputs diverged"
    );
    // The initial path defaults to "/" (SPA root), then follows the navigations.
    assert_eq!(tw.output.trim(), "/\n/todos\n/todos/8");
    assert_eq!(tw_nav, vm_nav, "tw vs VM nav op-log diverged");
    assert_eq!(
        tw_nav,
        vec![
            dom::RouteOp::Navigate {
                path: "/todos".into(),
                replace: false
            },
            dom::RouteOp::Navigate {
                path: "/todos/7".into(),
                replace: false
            },
            dom::RouteOp::Navigate {
                path: "/todos/8".into(),
                replace: true
            },
        ],
    );
}

#[test]
fn route_query_reads_a_param_and_is_empty_when_absent_on_both_tiers() {
    // routeQuery(name) reads the query string set at init (refresh/deep-link);
    // an absent param is "" (never a trap), identical on tw and the VM.
    let src = concat!(
        "## Main\n",
        "Show routeQuery(\"tab\").\n",
        "Show routeQuery(\"missing\").\n",
    );
    dom::reset();
    dom::init_route("/settings", "tab=profile&sort=asc");
    let tw = tw_outcome(src);
    dom::reset();
    dom::init_route("/settings", "tab=profile&sort=asc");
    let vm = vm_outcome(src);
    assert_eq!(
        tw.output.trim(),
        vm.output.trim(),
        "tw vs VM outputs diverged"
    );
    // `.trim()` drops the trailing blank line from the empty second `Show`.
    assert_eq!(tw.output.trim(), "profile");
}

#[test]
fn on_route_registers_a_handler_the_pop_driver_fires_with_the_new_path() {
    // `onRoute(handler)` registers a popstate handler (like addEventListener/
    // fetchText, it stores the LOGOS closure). Back/forward is simulated by
    // `deliver_pop(path)`: it advances routePath and hands back the closure to
    // fire — exactly the correlated-fetch driver shape, proven on both tiers.
    let src = concat!(
        "## Main\n",
        "Let onNav be (tick: Int) -> routePath().\n",
        "Call onRoute with onNav.\n",
    );
    dom::reset();
    dom::init_route("/a", "");
    let _ = tw_outcome(src);
    assert_eq!(dom::route_path(), "/a", "the init path is current");
    // No handler is fired yet — registration alone records nothing navigational.
    assert!(dom::route_ops().is_empty(), "onRoute records no nav op");
    // A back/forward event delivers the popped path and yields the handler closure.
    let fired = dom::deliver_pop("/b");
    assert!(!fired.is_empty(), "the registered handler fires on a pop");
    assert_eq!(dom::route_path(), "/b", "the pop moved the current path");
    // Same registration + firing on the VM tier.
    dom::reset();
    dom::init_route("/a", "");
    let _ = vm_outcome(src);
    assert!(
        !dom::deliver_pop("/c").is_empty(),
        "the VM registers the same handler"
    );
    assert_eq!(dom::route_path(), "/c");
    // A pop with no registered handler is a no-op (still moves the path).
    dom::reset();
    assert!(
        dom::deliver_pop("/d").is_empty(),
        "no handler ⇒ nothing to fire"
    );
    assert_eq!(dom::route_path(), "/d");
}

#[test]
fn route_param_matcher_extracts_named_segments_pure_logos_on_both_tiers() {
    // The pattern matcher is pure-LOGOS (web.lg) over routePath: matchRoute checks a
    // `/todos/:id` template against the current path; routeParam extracts a named
    // segment. A non-matching pattern yields "" (routeParam) / false (matchRoute).
    let src = concat!(
        "## Main\n",
        "Let p be routePath().\n",
        "Show matchRoute(\"/todos/:id\", p).\n",
        "Show routeParam(\"/todos/:id\", \"id\", p).\n",
        "Show matchRoute(\"/users/:id\", p).\n",
        "Show routeParam(\"/users/:id\", \"id\", p).\n",
        "Show matchRoute(\"/todos/:id/edit\", p).\n",
        "Show routeParam(\"/todos/:id\", \"missing\", p).\n",
    );
    dom::reset();
    dom::init_route("/todos/42", "");
    let tw = tw_outcome(src);
    dom::reset();
    dom::init_route("/todos/42", "");
    let vm = vm_outcome(src);
    assert_eq!(
        tw.output.trim(),
        vm.output.trim(),
        "tw vs VM outputs diverged"
    );
    assert_eq!(
        tw.output.trim(),
        // matches /todos/:id → id=42; /users/:id no-match → false/""; wrong arity
        // (/todos/:id/edit) no-match → false; an unknown param name → "" (the final
        // blank line is trimmed off).
        "true\n42\nfalse\n\nfalse",
    );
}
