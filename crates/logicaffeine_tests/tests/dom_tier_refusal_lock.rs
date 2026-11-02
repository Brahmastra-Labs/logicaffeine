//! ★ G6-full LOCK — which part of reactive DOM reconciliation is shared, and which is WASM-only. ★
//!
//! The cross-tier DOM story has a shared surface and a WASM-only surface, and this lock draws the
//! line by test, never by assumption:
//!
//!  * SHARED — RECONCILIATION ITSELF. Every tier paints through `vdom.lg`, which is the program's
//!    own code: a turn either writes a retained node where it stands — a site the lowering proved
//!    patchable (`logicaffeine_language::lgx::deps::SiteDeps::patchable`: a bare text child or a
//!    two-way binding), written with `vdPatchText`/`vdPatchValue` — or draws the view again and
//!    commits it against the retained tree, which swaps an `{if}`'s arm in place and moves the rows
//!    of a keyed list. The tree-walker and the VM apply each committed stream to the `DomRecorder`
//!    exactly as the browser applies it to its document.
//!
//!  * WASM-ONLY — THE HOST CROSSING. A compiled page hands every element event to one entry,
//!    `__lgx_dispatch_node`, and hands every paint back through one import, `dom_apply_words`.
//!
//! So this file proves:
//!  (a) the reactive program compiles to a WASM module that DEFINES that crossing, and draws
//!      through nothing else — no per-op DOM import, no closure handed to the host;
//!  (b) the tree-walker and VM record the IDENTICAL DOM op stream for the same program;
//!  (b') a turn whose dirty fields are ALL patchable writes the SAME node in place on both native
//!      tiers, while a turn that reaches an `{if}`'s condition swaps the arm in place on both;
//!  (c) a keyed reorder MOVES the rows it matched by key on both native tiers — the keyed-list
//!      reconciler is the program's, so no tier lacks it;
//!  (d) the emitted WASM DOM surface is DETERMINISTIC across two compiles of the same source —
//!      both the module bytes AND the MockDom-observed initial op-log.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::semantics::dom::{self, DomOp};

/// The G17 reactive form, in LGX: a bound `<input>` echoed in a `<p>`, with an
/// `Actions:` turn (`retype`) that mutates `n`, a SEPARATE field the view also shows
/// (`<p id="count">`), so a native-tier `Click` changes something the view reads and
/// can drive a genuine second (and third, and fourth) repaint.
const LGX_FORM_APP: &str = concat!(
    "## Component Form\n",
    "State:\n",
    "    Let name be \"\".\n",
    "    Let n be 0.\n",
    "Actions:\n",
    "    To retype:\n",
    "        Set n to n + 1.\n",
    "View:\n",
    "    <main>\n",
    "    <input id=\"f\" type=\"text\" bind={name}>\n",
    "    <p id=\"echo\">Hello, {name}</p>\n",
    "    <p id=\"count\">{n}</p>\n",
    "    <button id=\"b\" on-click={retype}>go</button>\n",
    "    </main>\n",
    "\n## Main\n",
    "Mount Form.\n",
);

/// `LGX_FORM_APP`'s `## Main`, with `n` successive `Click "#b"` turns appended — each one
/// mutates `n` (the State field, confusingly homonymous with this function's own parameter)
/// and must repaint.
fn form_app_with_clicks(n: usize) -> String {
    let mut src = LGX_FORM_APP.to_string();
    for _ in 0..n {
        src.push_str("Click \"#b\".\n");
    }
    src
}

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

#[test]
fn native_tiers_record_the_identical_op_stream_across_repeated_repaints() {
    // (b) — the SHARED surface: the tree-walker and the VM build the mount AND every
    // click-driven repaint identically. Tier parity holds across the whole run, not
    // only the initial mount.
    let src = form_app_with_clicks(3);
    let interp = interp_log(&src);
    let vm = vm_log(&src);
    assert_eq!(
        interp, vm,
        "tree-walker and VM must record the IDENTICAL DOM op stream across mount + 3 clicks"
    );
    assert!(!interp.is_empty(), "the run must record ops at all");
}

/// (b') THE PATCHABLE HALF — a turn whose only dirty site is a bare text hole
/// (`<p id="count">{n}</p>`) patches that ONE node in place, on both native tiers: a
/// `SetText`, never a `ClearChildren`/second `CreateElement`. `patch_plan` decides it at
/// lowering time from `SiteDeps::patchable()` and bakes it into the action's own compiled
/// body; this runs the fixture on both tiers and reads the op-log back.
#[test]
fn a_patchable_text_hole_is_patched_in_place_never_rebuilt_on_either_native_tier() {
    const PATCH_APP: &str = concat!(
        "## Component App\n",
        "State:\n",
        "    Let n be 0.\n",
        "Actions:\n",
        "    To bump:\n",
        "        Set n to n + 1.\n",
        "View:\n",
        "    <p id=\"count\">{n}</p><button id=\"b\" on-click={bump}>go</button>\n",
        "\n## Main\nMount App.\n",
    );
    let mut src = PATCH_APP.to_string();
    for _ in 0..3 {
        src.push_str("Click \"#b\".\n");
    }
    for (tier, log) in [("tree-walker", interp_log(&src)), ("VM", vm_log(&src))] {
        // The mount creates the `<p>` and the `<button>` once each; every click after it
        // must patch the SAME node rather than recreate anything.
        let creates = log
            .iter()
            .filter(|op| matches!(op, DomOp::CreateElement { .. }))
            .count();
        assert_eq!(
            creates, 2,
            "the {tier} must CreateElement only at mount (the `<p>` and the `<button>`) — a \
             patchable turn creates nothing further: {log:?}"
        );
        let clears = log
            .iter()
            .filter(|op| matches!(op, DomOp::ClearChildren { .. }))
            .count();
        assert_eq!(
            clears, 1,
            "the {tier} must ClearChildren only once, at mount — a turn whose only dirty site \
             is a bare text hole must never ClearChildren again: {log:?}"
        );
        let set_nodes: Vec<u32> = log
            .iter()
            .filter_map(|op| {
                if let DomOp::SetText { node, .. } = op {
                    Some(*node)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(
            set_nodes.len(),
            3,
            "the {tier} must SetText once per click: {log:?}"
        );
        assert!(
            set_nodes.windows(2).all(|w| w[0] == w[1]),
            "the {tier} must SetText the SAME node every time, in place: {log:?}"
        );
    }
}

/// (b') THE STRUCTURAL HALF — a turn whose dirty field reaches an `{if}`'s condition set
/// (`SiteKind::If`) cannot be patched as a value write (which arm renders is a structural
/// decision), so both native tiers draw the view again and the commit swaps the arm in place: the
/// arm that stopped rendering is freed, the one that now renders is created where it stands, and
/// nothing around them is cleared or rebuilt.
#[test]
fn a_structural_if_flip_swaps_the_arm_in_place_on_both_native_tiers() {
    const REBUILD_APP: &str = concat!(
        "## Component App\n",
        "State:\n",
        "    Let show be true.\n",
        "Actions:\n",
        "    To toggle:\n",
        "        Set show to not show.\n",
        "View:\n",
        "    {if show:}<p id=\"a\">yes</p>{else:}<p id=\"b\">no</p>{end}\n",
        "    <button id=\"t\" on-click={toggle}>go</button>\n",
        "\n## Main\nMount App.\nClick \"#t\".\n",
    );
    let mount_only = REBUILD_APP
        .strip_suffix("Click \"#t\".\n")
        .expect("the fixture ends with its drive");
    for (tier, log, mount) in [
        (
            "tree-walker",
            interp_log(REBUILD_APP),
            interp_log(mount_only),
        ),
        ("VM", vm_log(REBUILD_APP), vm_log(mount_only)),
    ] {
        assert_eq!(
            &log[..mount.len()],
            mount.as_slice(),
            "the {tier}'s turn starts from its mount: {log:?}"
        );
        let turn = &log[mount.len()..];
        let created: Vec<&str> = turn
            .iter()
            .filter_map(|op| {
                if let DomOp::CreateElement { tag, .. } = op {
                    Some(tag.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(
            created,
            ["p"],
            "the {tier} creates exactly the arm that now renders: {turn:?}"
        );
        let freed = turn
            .iter()
            .filter(|op| matches!(op, DomOp::FreeNode { .. }))
            .count();
        assert_eq!(
            freed, 1,
            "the {tier} frees exactly the arm that stopped rendering: {turn:?}"
        );
        assert!(
            !turn
                .iter()
                .any(|op| matches!(op, DomOp::ClearChildren { .. })),
            "the {tier} swaps the arm without clearing anything around it: {turn:?}"
        );
        assert!(
            turn.iter().any(|op| matches!(op, DomOp::SetAttribute { name, value, .. } if name == "id" && value == "b")),
            "the {tier} draws the `{{else}}` arm: {turn:?}"
        );
    }
}

/// ★ THE DEPENDENCY-AWARE SKIP, LOCKED DIRECTLY — proven on both native tiers: a turn that writes
/// a field NO site in the view reads must add LITERALLY NOTHING to the op-log — the honest answer
/// to "what changed" is nothing.
#[test]
fn a_turn_that_changes_only_a_field_the_view_never_reads_adds_no_ops_on_either_native_tier() {
    const MOUNT_ONLY: &str = concat!(
        "## Component App\n",
        "State:\n",
        "    Let shown be 0.\n",
        "    Let hidden be 0.\n",
        "Actions:\n",
        "    To bump:\n",
        "        Set hidden to hidden + 1.\n",
        "View:\n",
        "    <p id=\"s\">{shown}</p><button id=\"b\" on-click={bump}>b</button>\n",
        "\n## Main\nMount App.\n",
    );
    let with_clicks = format!("{MOUNT_ONLY}Click \"#b\".\nClick \"#b\".\n");
    for (tier, mount, clicked) in [
        (
            "tree-walker",
            interp_log(MOUNT_ONLY),
            interp_log(&with_clicks),
        ),
        ("VM", vm_log(MOUNT_ONLY), vm_log(&with_clicks)),
    ] {
        assert_eq!(
            mount, clicked,
            "the {tier} must add NOTHING to the op-log for two turns that change only a field \
             the view never reads: mount={mount:?} clicked={clicked:?}"
        );
    }
}

/// (c) THE KEYED-LIST RECONCILER RUNS ON BOTH NATIVE TIERS. A turn that reorders a keyed list
/// draws the view again, and the commit matches each row to its retained node by key and moves the
/// ones off the longest run already in order — `c` from last to first here, one `InsertBefore` —
/// creating, freeing and clearing nothing. Both native tiers make the same move and show the list
/// in its new order.
#[test]
fn a_keyed_reorder_moves_its_rows_on_both_native_tiers() {
    const KEYED_APP: &str = concat!(
        "## Component App\n",
        "State:\n",
        "    Let items be [\"a\", \"b\", \"c\"].\n",
        "Actions:\n",
        "    To flip:\n",
        "        Set items to [\"c\", \"a\", \"b\"].\n",
        "View:\n",
        "    <ul>{for each x in items, keyed by x:}<li>{x}</li>{end}</ul>\n",
        "    <button id=\"go\" on-click={flip}>go</button>\n",
        "\n## Main\nMount App.\nClick \"#go\".\n",
    );
    let mount_only = KEYED_APP
        .strip_suffix("Click \"#go\".\n")
        .expect("the fixture ends with its drive");
    let mut turns = Vec::new();
    for (tier, run) in [
        ("tree-walker", interp_log as fn(&str) -> Vec<DomOp>),
        ("VM", vm_log),
    ] {
        let mount = run(mount_only);
        let log = run(KEYED_APP);
        assert!(
            dom::render().contains("<ul><li>c</li><li>a</li><li>b</li></ul>"),
            "the {tier} shows the list in its new order: {}",
            dom::render()
        );
        assert_eq!(
            &log[..mount.len()],
            mount.as_slice(),
            "the {tier}'s turn starts from its mount: {log:?}"
        );
        let turn = log[mount.len()..].to_vec();
        let moves = turn
            .iter()
            .filter(|op| matches!(op, DomOp::InsertBefore { .. }))
            .count();
        assert_eq!(
            moves, 1,
            "the {tier} moves exactly the one row off the run: {turn:?}"
        );
        assert!(
            !turn.iter().any(|op| matches!(
                op,
                DomOp::CreateElement { .. } | DomOp::CreateTextNode { .. } | DomOp::FreeNode { .. } | DomOp::ClearChildren { .. }
            )),
            "the {tier} keeps every row it matched by key — nothing created, freed or cleared: {turn:?}"
        );
        turns.push(turn);
    }
    assert_eq!(turns[0], turns[1], "both native tiers make the same moves");
}

#[cfg(feature = "wasm-jit")]
mod wasm_only {
    use super::LGX_FORM_APP as FORM_APP;
    use logicaffeine_compile::compile::compile_to_wasm;

    /// Scan a wasm module for the literal bytes of an export/import `name`. The WAT
    /// export/import name section stores names verbatim, so their presence in the module
    /// bytes is a sound (if coarse) capability probe — enough to assert the reconciliation
    /// surface is DEFINED (or, for the native comparison, that it exists only here).
    fn module_names_symbol(wasm: &[u8], name: &str) -> bool {
        wasm.windows(name.len()).any(|w| w == name.as_bytes())
    }

    #[test]
    fn the_reactive_program_compiles_to_the_wasm_only_reconciliation_surface() {
        // (a) + (c) — the WASM tier DEFINES the reconciliation surface the native tiers
        // lack: the `__lgx_dispatch_node` entry the browser host hands every event on an element
        // to, and the DOM patch host imports that action's own re-render calls when it runs.
        // These are what a reactive re-render + keyed diff need; their presence in the WASM
        // module (and absence anywhere on interp/VM) IS the WASM-only classification.
        //
        // ★ `__logos_render` ITSELF IS NOT PART OF THIS SURFACE FOR AN LGX `Mount`ed COMPONENT.
        // That export names a literal top-level function called `render` (`vm/wasm/module.rs`'s
        // own `render_index` lookup: `interner.resolve(f.name) == "render"`) — the OLD
        // immediate-mode idiom `reactive.rs`'s `RERENDER_SRC` still uses, where the HOST
        // re-invokes `render()` on its own tick. An LGX component's re-render is instead driven
        // ENTIRELY FROM INSIDE THE WASM MODULE: the compiled ACTION calls the component's own
        // `__render_<Name>` again itself when it runs — the host only ever hands the event to
        // `__lgx_dispatch_node` and applies the resulting DOM ops.
        //
        // ★ AND AN ELEMENT'S EVENT IS NO CLOSURE. The view registers where it listens
        // (`dom_listen_flags`) and the page's own table says what answers, so this page hands the
        // host no closure at all: no `dom_add_event_listener`, and no `__logos_call_closure`
        // trampoline, which a module exports only where a closure crosses the host.
        let wasm = compile_to_wasm(FORM_APP).expect("the reactive form compiles to wasm");
        assert!(
            module_names_symbol(&wasm, "__lgx_dispatch_node"),
            "the WASM module must export `__lgx_dispatch_node` — the one entry an element's event \
             reaches the page through"
        );
        // Everything a paint changes — the bound input's in-place value, where the page listens —
        // reaches the host as the paint's one committed stream, and nothing is drawn any other way.
        assert!(
            module_names_symbol(&wasm, "dom_apply_words"),
            "the WASM module imports `dom_apply_words` — the one door a paint's patches reach the host by"
        );
        for per_op in ["dom_set_value", "dom_create_element", "dom_append_child"] {
            assert!(
                !module_names_symbol(&wasm, per_op),
                "the page draws only through its stream, so the module carries no `{per_op}`"
            );
        }
        for closure_surface in ["dom_add_event_listener", "__logos_call_closure"] {
            assert!(
                !module_names_symbol(&wasm, closure_surface),
                "the page hands the host no closure, so the module carries no `{closure_surface}`"
            );
        }
    }

    #[test]
    fn emitted_wasm_dom_surface_is_deterministic_across_two_compiles() {
        // (d) — determinism: the same source compiles to byte-identical WASM every time,
        // so the emitted DOM-op stream is reproducible (no nondeterministic ordering leaks
        // into the reconciliation the browser drives). Two compiles, byte-for-byte equal.
        let a = compile_to_wasm(FORM_APP).expect("compiles once");
        let b = compile_to_wasm(FORM_APP).expect("compiles twice");
        assert_eq!(
            a, b,
            "the WASM DOM bundle must be deterministic — two compiles of one source are byte-identical"
        );
        assert!(
            a.starts_with(b"\0asm"),
            "and it is a real wasm module (magic header present)"
        );
    }
}

// ---- The DOM does not exist on a device. ----

/// A program that touches the DOM must be REFUSED by every tier that has no DOM — and the
/// refusal must SAY so.
///
/// Before this, `compile_to_rust`, `compile_to_c` and `compile_to_cuda` all accepted a page and
/// returned an artifact: 380 bytes of Rust, 121KB of C, a CUDA kernel. None of them has a
/// document to build. `largo build --target cuda` on a `## Page` produced something that looked
/// like a successful build and could only ever render nothing. Only the riscv backend refused,
/// and for an unrelated reason ("unsupported non-numeric"), which is luck rather than a rule.
///
/// A silent wrong answer is the worst outcome a compiler can give, and it is worse here than
/// usual: the artifact BUILDS, so the mistake surfaces at run time on a device, far from the
/// `--target` flag that caused it.
const DOM_PROGRAM: &str = "## Main\nLet d be createElement(\"div\").\n\
    Call setText with d and \"hi\".\nCall appendChild with body() and d.\n";

const LGX_PAGE: &str = "## Component App\nView:\n    <p>hi</p>\n\n## Main\nMount App.\n";

/// The refusal must name the DOM, not merely fail. A backend that says "unsupported expression"
/// sends the reader looking at their expression.
fn assert_names_the_dom(tier: &str, src_label: &str, err: String) {
    let lower = err.to_lowercase();
    assert!(
        lower.contains("dom") || lower.contains("browser") || lower.contains("document"),
        "the {tier} refusal of {src_label} must NAME the DOM so the reader knows the tier is \
         wrong for the program, not the program: {err}"
    );
}

#[test]
fn the_device_tiers_refuse_a_dom_program_by_name() {
    use logicaffeine_compile::compile::{compile_to_c, compile_to_cuda, compile_to_rust};
    for (label, src) in [("raw DOM builtins", DOM_PROGRAM), ("an LGX page", LGX_PAGE)] {
        let rust = compile_to_rust(src);
        assert!(
            rust.is_err(),
            "AOT-Rust must refuse {label}: it has no document to build"
        );
        assert_names_the_dom("AOT-Rust", label, format!("{:?}", rust.unwrap_err()));

        let c = compile_to_c(src);
        assert!(
            c.is_err(),
            "AOT-C must refuse {label}: it has no document to build"
        );
        assert_names_the_dom("AOT-C", label, format!("{:?}", c.unwrap_err()));

        let cuda = compile_to_cuda(src);
        assert!(
            cuda.is_err(),
            "CUDA must refuse {label}: a GPU kernel has no document"
        );
        assert_names_the_dom("CUDA", label, format!("{:?}", cuda.unwrap_err()));
    }
}

/// The tiers that DO have a DOM keep working — the refusal must be about the tier, not about the
/// program. A rule that fails closed everywhere is not fail-closed, it is just broken.
#[test]
fn the_browser_tiers_still_accept_the_same_program() {
    use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
    for (label, src) in [("raw DOM builtins", DOM_PROGRAM), ("an LGX page", LGX_PAGE)] {
        logicaffeine_compile::semantics::dom::reset();
        assert_eq!(
            tw_outcome(src).error,
            None,
            "the tree-walker still runs {label}"
        );
        logicaffeine_compile::semantics::dom::reset();
        assert_eq!(vm_outcome(src).error, None, "the VM still runs {label}");
    }
}

// ---- Refusing the DOM must mean refusing a program that USES it. ----

/// ★ A REGRESSION THIS FILE CAUSED, and the shape that makes it serious.
///
/// The refusal scanned the whole LOADED program — including the standard-library functions the
/// prelude brought in and nobody calls. `web.lg` holds the fetch wrappers AND the pure-text
/// helpers `htmlEscape`, `isIntText` and `matchRoute`, so ANY program that escaped a string or
/// matched a route was told it "touches the DOM" and refused.
///
/// That is every real server: `largo build` on a program that renders a page or routes a request
/// stopped working, and the message sent the reader looking for a `## Page` that was not there.
///
/// The rule is REACHABILITY, not presence. A prelude function nobody calls is not part of the
/// program, and a backend must not refuse a program for code it will never emit.
#[test]
fn a_program_using_a_text_helper_from_web_lg_still_builds() {
    for (label, src) in [
        ("htmlEscape", "## Main\nShow htmlEscape(\"<b>x</b>\").\n"),
        ("isIntText", "## Main\nShow isIntText(\"42\").\n"),
        (
            "matchRoute",
            "## Main\nShow matchRoute(\"/a/:id\", \"/a/1\").\n",
        ),
        (
            "routeParam",
            "## Main\nShow routeParam(\"/a/:id\", \"id\", \"/a/1\").\n",
        ),
    ] {
        let out = logicaffeine_compile::compile::compile_to_rust(src);
        assert!(
            out.is_ok(),
            "`{label}` is pure text and must build on AOT-Rust; it was refused: {:?}",
            out.err().map(|e| e.kind)
        );
    }
}

/// The other half, so the fix cannot be "stop refusing": a program that REALLY touches the DOM is
/// still refused, by name.
#[test]
fn a_program_that_really_touches_the_dom_is_still_refused() {
    let src = "## Main\nLet el be createElement(\"div\").\nCall appendChild with body() and el.\n";
    let err = logicaffeine_compile::compile::compile_to_rust(src)
        .err()
        .unwrap_or_else(|| panic!("a DOM program must still be refused"));
    assert!(
        format!("{:?}", err.kind).contains("DOM"),
        "the refusal still names the DOM: {:?}",
        err.kind
    );
}

/// And a program whose own FUNCTION touches the DOM, called from Main — reachability has to
/// follow calls, not just look at the top level.
#[test]
fn a_dom_call_behind_a_function_is_still_refused() {
    let src = "## To build () -> Int:\n    Let el be createElement(\"div\").\n    Return 1.\n\n## Main\nShow build().\n";
    let err = logicaffeine_compile::compile::compile_to_rust(src)
        .err()
        .unwrap_or_else(|| panic!("a DOM call behind a function must still be refused"));
    assert!(
        format!("{:?}", err.kind).contains("DOM"),
        "the refusal still names the DOM: {:?}",
        err.kind
    );
}

/// A page still is a page.
#[test]
fn a_component_is_still_refused() {
    let src = "## Component App\nView:\n    <p>hi</p>\n\n## Main\nMount App.\n";
    assert!(
        logicaffeine_compile::compile::compile_to_rust(src).is_err(),
        "a `## Component` has no AOT-Rust form"
    );
}
