//! The Studio bottom **debug drawer** — an IDE-style debugger for imperative
//! (Code-mode) LOGOS, driven by the zero-cost bytecode debugger in
//! `logicaffeine_compile::debug`. A self-contained, additive panel: it docks below
//! the editor/output and slides away on Stop, leaving the rest of the Studio
//! untouched.
//!
//! Anatomy (classic IDE Debug panel): a step toolbar (Step Into / Over / Out /
//! Continue / **Step Back** / Restart / Stop), a status line (state · pc · current
//! op · step counter), and Variables / Call Stack / Breakpoints / Bytecode tabs.
//! Breakpoints are set on the bytecode tape (this is a bytecode-level debugger —
//! you watch the actual VM). Step Back is true time-travel, free from the
//! debugger's snapshot history.

use dioxus::prelude::*;
use logicaffeine_compile::debug::{CausalNode, DebugSnapshot, Debugger, ProofVerdict, VarTimeline};
use logicaffeine_compile::interpreter::InterpreterResult;

#[derive(Clone, Copy, PartialEq)]
enum DebugTab {
    Variables,
    Stack,
    Heap,
    Timeline,
    Prove,
    CallStack,
    Breakpoints,
    Bytecode,
}

/// The bottom debug drawer. `source` is the Code-mode program; `output` receives what the
/// program has printed at the cursor, for the Studio's Output column; `on_close` stops the
/// session (the Studio hides the drawer).
#[component]
pub fn DebugDrawer(
    source: String,
    height: Signal<f64>,
    output: Signal<InterpreterResult>,
    on_close: EventHandler<()>,
) -> Element {
    // Dragging the grip on the drawer's top edge paints each frame straight onto the drawer's
    // height and commits it once, on release (see ui::drag). The panel row above gives way.
    let start_grip_drag = move |evt: PointerEvent| {
        #[cfg(target_arch = "wasm32")]
        {
            use crate::ui::pages::studio_layout::drawer_dragged;
            use dioxus::html::input_data::MouseButton;
            use wasm_bindgen::JsCast;
            let mut height = height;
            if evt
                .data()
                .trigger_button()
                .is_some_and(|b| b != MouseButton::Primary)
            {
                return;
            }
            evt.prevent_default();
            let Some(document) = web_sys::window().and_then(|w| w.document()) else {
                return;
            };
            let find = |selector: &str| document.query_selector(selector).ok().flatten();
            let (Some(grip), Some(drawer), Some(panels)) = (
                find(".dbg-grip"),
                find(".dbg-drawer").and_then(|el| el.dyn_into::<web_sys::HtmlElement>().ok()),
                find(".studio-content"),
            ) else {
                return;
            };
            let start = drawer.get_bounding_client_rect().height();
            let room = start + panels.get_bounding_client_rect().height();
            let point = evt.data().client_coordinates();
            let _ = grip.class_list().add_1("active");
            let released = grip.clone();
            crate::ui::drag::begin(
                grip,
                evt.data().pointer_id(),
                (point.x, point.y),
                crate::ui::drag::Axis::Rows,
                move |_, dy| {
                    let _ = drawer
                        .style()
                        .set_property("height", &format!("{}px", drawer_dragged(start, dy, room)));
                },
                move |_, dy| {
                    let _ = released.class_list().remove_1("active");
                    height.set(drawer_dragged(start, dy, room));
                },
            );
        }
        #[cfg(not(target_arch = "wasm32"))]
        let _ = (evt, height);
    };
    let drawer_style = format!("height: {}px;", height());

    // The debugger is built once from the source the session opened with; `armed_source`
    // remembers that source so we can tell when the editor/file has since changed.
    let mut dbg = use_signal(|| Debugger::from_source(&source));
    let mut armed_source = use_signal(|| source.clone());
    // What the program has printed at the cursor shows in the Studio's Output column, and a
    // program that cannot be debugged says why there too.
    use_effect(move || {
        let printed = match &*dbg.read() {
            Ok(d) => InterpreterResult {
                lines: d.output().to_vec(),
                error: d.error().map(str::to_string),
            },
            Err(message) => InterpreterResult {
                lines: Vec::new(),
                error: Some(message.clone()),
            },
        };
        let mut output = output;
        output.set(printed);
    });
    let tab = use_signal(|| DebugTab::Variables);
    // The variable whose causal provenance is being traced (click a value to set it).
    let mut traced = use_signal(|| None::<u16>);
    // The live-proof query typed into the Prove tab.
    let mut prove_query = use_signal(String::new);
    // The "virtual hardware" easter egg — off by default, bundle-light.
    let mut hw_open = use_signal(|| false);
    // Auto-play: a timer steps the program so you can watch it run.
    let mut playing = use_signal(|| false);
    let speed = use_signal(|| 650u32);
    use_future(move || async move {
        loop {
            #[cfg(target_arch = "wasm32")]
            {
                gloo_timers::future::TimeoutFuture::new(speed().max(60)).await;
                if playing() {
                    let mut still = false;
                    if let Ok(d) = dbg.write().as_mut() {
                        if d.is_running() {
                            d.step();
                            still = d.is_running();
                        }
                    }
                    if !still {
                        playing.set(false);
                    }
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                let _ = (speed, playing, dbg);
                break;
            }
        }
    });

    // The timeline is wider than the drawer: keep its playhead in view as the cursor moves.
    use_effect(move || {
        let _ = dbg.read();
        if tab() == DebugTab::Timeline {
            #[cfg(target_arch = "wasm32")]
            keep_playhead_in_view();
        }
    });

    // Compile error → a minimal error drawer (nothing to step).
    if let Err(msg) = &*dbg.read() {
        let msg = msg.clone();
        return rsx! {
            style { "{DEBUG_DRAWER_STYLE}" }
            div { class: "dbg-drawer", style: "{drawer_style}",
                div { class: "dbg-grip", onpointerdown: move |evt| start_grip_drag(evt) }
                div { class: "dbg-bar",
                    span { class: "dbg-title", "Debugger" }
                    button { class: "dbg-btn dbg-stop", onclick: move |_| on_close.call(()), "Close" }
                }
                div { class: "dbg-error", "Cannot debug: {msg}" }
            }
        };
    }

    // Pull a fresh snapshot + disassembly for this render.
    let snap = dbg.read().as_ref().ok().map(|d| d.snapshot());
    let snap = match snap {
        Some(s) => s,
        None => return rsx! {},
    };
    let disasm = dbg
        .read()
        .as_ref()
        .ok()
        .map(|d| {
            d.disassembly()
                .iter()
                .map(|l| (l.pc, l.text.clone()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let bps: std::collections::BTreeSet<usize> = dbg
        .read()
        .as_ref()
        .ok()
        .map(|d| d.breakpoints().into_iter().collect())
        .unwrap_or_default();

    let paused = snap.state == "paused";
    let at_start = snap.step == 0;
    let cur_tab = *tab.read();

    // ── step toolbar handlers (each mutates the debugger in place) ─────────────
    let act = move |f: fn(&mut Debugger)| {
        move |_| {
            if let Ok(d) = dbg.write().as_mut() {
                f(d);
            }
        }
    };

    // The editor/file changed since we armed this session (a different program).
    let stale = armed_source() != source;
    let reload_source = source.clone();

    rsx! {
        style { "{DEBUG_DRAWER_STYLE}" }
        div { class: "dbg-drawer", style: "{drawer_style}",
            div { class: "dbg-grip", onpointerdown: move |evt| start_grip_drag(evt) }
            // Toolbar + status line
            div { class: "dbg-bar",
                span { class: "dbg-title",
                    span { class: "dbg-bug", dangerous_inner_html: IC_BUG }
                    "Debug"
                }
                div { class: "dbg-controls",
                    button { class: "dbg-btn dbg-rev", disabled: at_start, title: "Reverse continue (back to the previous breakpoint)",
                        onclick: act(Debugger::reverse_resume), dangerous_inner_html: IC_REVERSE }
                    button { class: "dbg-btn dbg-rev", disabled: at_start, title: "Step back (time-travel one op)",
                        onclick: act(Debugger::step_back), dangerous_inner_html: IC_STEP_BACK }
                    span { class: "dbg-divider" }
                    button { class: if playing() { "dbg-btn dbg-play on" } else { "dbg-btn dbg-play" },
                        disabled: !paused && !playing(),
                        title: if playing() { "Pause" } else { "Auto-play: watch it run" },
                        onclick: move |_| { let p = playing(); playing.set(!p); },
                        dangerous_inner_html: if playing() { IC_PAUSE } else { IC_PLAY } }
                    button { class: "dbg-btn dbg-go", disabled: !paused, title: "Continue",
                        onclick: act(Debugger::resume), dangerous_inner_html: IC_CONTINUE }
                    button { class: "dbg-btn", disabled: !paused, title: "Step over",
                        onclick: act(Debugger::step_over), dangerous_inner_html: IC_STEP_OVER }
                    button { class: "dbg-btn", disabled: !paused, title: "Step into",
                        onclick: act(Debugger::step), dangerous_inner_html: IC_STEP_INTO }
                    button { class: "dbg-btn", disabled: !paused, title: "Step out",
                        onclick: act(Debugger::step_out), dangerous_inner_html: IC_STEP_OUT }
                    span { class: "dbg-divider" }
                    button { class: "dbg-btn", title: "Restart (rewind to the entry)",
                        onclick: act(Debugger::restart), dangerous_inner_html: IC_RESTART }
                    button { class: if hw_open() { "dbg-btn dbg-toggle on" } else { "dbg-btn dbg-toggle" },
                        title: "Virtual hardware view",
                        onclick: move |_| { let v = hw_open(); hw_open.set(!v); }, dangerous_inner_html: IC_GEAR }
                }
                div { class: "dbg-status",
                    if playing() {
                        span { class: "dbg-state dbg-state-playing", "playing" }
                    } else {
                        span { class: "dbg-state dbg-state-{snap.state}", "{snap.state}" }
                    }
                    span { class: "dbg-sep", "\u{00B7}" }
                    span { "pc {snap.pc}/{snap.total_ops}" }
                    if !snap.op_text.is_empty() {
                        span { class: "dbg-sep", "\u{00B7}" }
                        span { class: "dbg-op", "{snap.op_text}" }
                    }
                }
                button { class: "dbg-close", title: "Close the debugger",
                    onclick: move |_| on_close.call(()), dangerous_inner_html: IC_CLOSE }
            }

            // The editor changed since this session armed (a file switch or an edit) —
            // non-destructive: keep debugging the old program, or reload the new one.
            if stale {
                div { class: "dbg-reload",
                    span { class: "dbg-reload-msg", "\u{26A0} The code changed. This session is the previous version." }
                    button { class: "dbg-reload-btn",
                        onclick: move |_| {
                            dbg.set(Debugger::from_source(&reload_source));
                            armed_source.set(reload_source.clone());
                            playing.set(false);
                        },
                        "Reload" }
                }
            }

            // Time-travel scrubber — drag through every op you have run.
            div { class: "dbg-scrub",
                span { class: "dbg-scrub-cap", dangerous_inner_html: IC_CLOCK }
                input {
                    class: "dbg-scrub-range",
                    r#type: "range",
                    min: "0",
                    max: "{snap.total_steps}",
                    value: "{snap.step}",
                    oninput: move |e| {
                        if let Ok(v) = e.value().parse::<usize>() {
                            if let Ok(d) = dbg.write().as_mut() { d.seek(v); }
                        }
                    },
                }
                span { class: "dbg-scrub-label", "{snap.step} / {snap.total_steps}" }
            }

            // Teaching line: a question to predict the step when the engine has one, otherwise
            // what the step does (the op itself where there are no plainer words), with the
            // step's first-order-logic meaning alongside (sum = x + y, t ⟺ (i < n)). One row,
            // always present, so stepping never moves the panel below it.
            {
                let ask = !snap.socratic.is_empty();
                let text = if ask {
                    snap.socratic.clone()
                } else if !snap.narration.is_empty() {
                    snap.narration.clone()
                } else {
                    snap.op_text.clone()
                };
                rsx! {
                    div { class: if ask { "dbg-narr dbg-narr-socratic".to_string() } else { format!("dbg-narr dbg-narr-{}", snap.state) },
                        span { class: "dbg-narr-ico", dangerous_inner_html: if ask { IC_SOCRATIC } else { IC_BULB } }
                        span { class: "dbg-narr-text", title: "{text}", "{text}" }
                        if !snap.fol.is_empty() {
                            span { class: "dbg-fol", title: "first-order-logic meaning of this step",
                                span { class: "dbg-fol-tag", "\u{22A8}" }
                                span { class: "dbg-fol-text", "{snap.fol}" }
                            }
                        }
                    }
                }
            }

            // The "virtual hardware" easter egg — an animated datapath of the live VM.
            if hw_open() {
                div { class: "dbg-hw", dangerous_inner_html: "{datapath_svg(&snap)}" }
            }

            // Tabs
            div { class: "dbg-tabs",
                DebugTabBtn { label: "Variables", this: DebugTab::Variables, cur: cur_tab, tab }
                DebugTabBtn { label: "Stack", this: DebugTab::Stack, cur: cur_tab, tab }
                DebugTabBtn { label: "Heap", this: DebugTab::Heap, cur: cur_tab, tab }
                DebugTabBtn { label: "Timeline", this: DebugTab::Timeline, cur: cur_tab, tab }
                DebugTabBtn { label: "Prove", this: DebugTab::Prove, cur: cur_tab, tab }
                DebugTabBtn { label: "Call Stack", this: DebugTab::CallStack, cur: cur_tab, tab }
                DebugTabBtn { label: "Breakpoints", this: DebugTab::Breakpoints, cur: cur_tab, tab }
                DebugTabBtn { label: "Bytecode", this: DebugTab::Bytecode, cur: cur_tab, tab }
            }

            div { class: "dbg-body",
                match cur_tab {
                    DebugTab::Variables => rsx! {
                        if let Some(frame) = snap.frames.last() {
                            div { class: "dbg-vars",
                                if !frame.registers.iter().any(|r| r.name.is_some()) {
                                    div { class: "dbg-empty", "no variables in this frame yet. The Stack tab shows every register" }
                                }
                                for reg in frame.registers.iter().filter(|r| r.name.is_some()) {
                                    {
                                        let idx = reg.index;
                                        let on = traced() == Some(idx);
                                        rsx! {
                                            div {
                                                class: if on { "dbg-var traced" } else if reg.changed { "dbg-var changed" } else { "dbg-var" },
                                                title: "Trace where this value came from",
                                                onclick: move |_| {
                                                    if traced() == Some(idx) { traced.set(None); } else { traced.set(Some(idx)); }
                                                },
                                                span { class: "dbg-var-name",
                                                    { reg.name.clone().unwrap_or_else(|| format!("R{}", reg.index)) }
                                                }
                                                if !reg.kind.is_empty() {
                                                    span { class: "dbg-var-type", "{reg.kind}" }
                                                }
                                                span { class: "dbg-var-eq", "=" }
                                                span { class: "dbg-var-val", "{reg.value}" }
                                                span { class: "dbg-why", dangerous_inner_html: IC_TRACE }
                                            }
                                        }
                                    }
                                }
                            }
                            // Causal provenance — "why is this value here?" — the exact data-flow
                            // lineage, possible only because execution is deterministically recorded.
                            if let Some(node) = traced().and_then(|r| dbg.read().as_ref().ok().and_then(|d| d.provenance(r))) {
                                div { class: "dbg-prov",
                                    div { class: "dbg-prov-h",
                                        span { class: "dbg-prov-title",
                                            span { class: "dbg-prov-ico", dangerous_inner_html: IC_TRACE }
                                            "why \u{2192} causal lineage"
                                        }
                                        button { class: "dbg-prov-x", onclick: move |_| traced.set(None),
                                            dangerous_inner_html: IC_CLOSE }
                                    }
                                    div { class: "dbg-prov-tree", dangerous_inner_html: "{causal_html(&node)}" }
                                }
                            }
                            if !snap.globals.is_empty() {
                                div { class: "dbg-globals-h", "globals" }
                                for (name, val) in snap.globals.iter() {
                                    div { class: "dbg-var",
                                        span { class: "dbg-var-name", "{name}" }
                                        span { class: "dbg-var-eq", "=" }
                                        span { class: "dbg-var-val", "{val}" }
                                    }
                                }
                            }
                        }
                    },
                    DebugTab::Stack => rsx! {
                        div { class: "dbg-stackmem",
                            div { class: "dbg-mem-hint", "The register file, the VM's stack memory. Each frame's slots at their addresses (current frame on top)." }
                            for frame in snap.frames.iter().rev() {
                                div { class: "dbg-sframe",
                                    div { class: "dbg-sframe-h",
                                        span { class: "dbg-sframe-fn", { frame.function.clone().unwrap_or_else(|| "Main".to_string()) } }
                                        span { class: "dbg-sframe-base", "base @ {frame.base}" }
                                    }
                                    for reg in frame.registers.iter() {
                                        {
                                            let addr = frame.base + reg.index as usize;
                                            let nm = reg.name.clone().unwrap_or_else(|| format!("R{}", reg.index));
                                            rsx! {
                                                div { class: if reg.changed { "dbg-slot changed" } else { "dbg-slot" },
                                                    span { class: "dbg-slot-addr", "{addr:04}" }
                                                    span { class: "dbg-slot-name", "{nm}" }
                                                    if !reg.kind.is_empty() {
                                                        span { class: "dbg-slot-type", "{reg.kind}" }
                                                    }
                                                    span { class: "dbg-slot-val", "{reg.value}" }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    },
                    DebugTab::Heap => rsx! {
                        div { class: "dbg-heap",
                            div { class: "dbg-mem-hint", "Heap objects (lists, maps, structs, text). A box shared by two variables is an alias: one allocation, two names." }
                            if snap.heap.is_empty() {
                                div { class: "dbg-empty", "no heap objects, only scalar values in scope" }
                            }
                            for obj in snap.heap.iter() {
                                div { class: if obj.shared { "dbg-hobj shared" } else { "dbg-hobj" },
                                    div { class: "dbg-hobj-h",
                                        span { class: "dbg-hobj-id", "{obj.id}" }
                                        span { class: "dbg-hobj-kind", "{obj.kind}" }
                                        span { class: "dbg-hobj-store", title: "memory layout", "{obj.storage}" }
                                        if obj.rc > 1 {
                                            span { class: "dbg-hobj-rc", title: "reference count", "rc {obj.rc}" }
                                        }
                                        if obj.shared {
                                            span { class: "dbg-hobj-alias", "aliased" }
                                        }
                                    }
                                    div { class: "dbg-hobj-val", "{obj.summary}" }
                                    div { class: "dbg-hobj-refs",
                                        span { class: "dbg-refs-label", "\u{2190}" }
                                        for r in obj.referenced_by.iter() {
                                            span { class: "dbg-ref", "{r}" }
                                        }
                                    }
                                }
                            }
                        }
                    },
                    DebugTab::Timeline => {
                        let tl = dbg.read().as_ref().ok().map(|d| d.variable_timeline());
                        let insights = dbg.read().as_ref().ok().map(|d| d.observed_invariants()).unwrap_or_default();
                        let proven = dbg.read().as_ref().ok().map(|d| d.proven_invariants()).unwrap_or_default();
                        match tl {
                            Some(tl) if !tl.vars.is_empty() => rsx! {
                                div { class: "dbg-osc-wrap",
                                    div { class: "dbg-mem-hint", "Every variable's value across the whole run: a logic-analyzer for your program. The pink playhead is your time-travel cursor. Drag the scrubber to move through time." }
                                    if tl.truncated {
                                        div { class: "dbg-mem-hint dim", "showing the most recent {tl.steps} steps" }
                                    }
                                    {
                                        let (names, lanes) = timeline_svg(&tl);
                                        rsx! {
                                            div { class: "dbg-osc",
                                                div { class: "dbg-osc-names", dangerous_inner_html: "{names}" }
                                                div { class: "dbg-osc-scroll", dangerous_inner_html: "{lanes}" }
                                            }
                                        }
                                    }
                                    // PROVEN invariants — the Oracle's static guarantees (every run).
                                    if !proven.is_empty() {
                                        div { class: "dbg-insights",
                                            div { class: "dbg-insights-h proven",
                                                span { class: "dbg-proven-ico", dangerous_inner_html: IC_SHIELD }
                                                "proven: holds on every run"
                                            }
                                            for p in proven.iter() {
                                                div { class: "dbg-insight",
                                                    span { class: "dbg-insight-name", "{p.name}" }
                                                    for f in p.facts.iter() {
                                                        span { class: "dbg-insight-fact proven", "{f}" }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    // Daikon-style observed invariants — what held over THIS run.
                                    if !insights.is_empty() {
                                        div { class: "dbg-insights",
                                            div { class: "dbg-insights-h", "observed this run" }
                                            for ins in insights.iter() {
                                                div { class: "dbg-insight",
                                                    span { class: "dbg-insight-name", "{ins.name}" }
                                                    for f in ins.facts.iter() {
                                                        span { class: "dbg-insight-fact", "{f}" }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            },
                            _ => rsx! {
                                div { class: "dbg-empty", "no variables to plot yet. Step the program to record a timeline" }
                            },
                        }
                    },
                    DebugTab::Prove => {
                        let q = prove_query();
                        let result = if q.trim().is_empty() {
                            None
                        } else {
                            dbg.read().as_ref().ok().map(|d| d.assert_at_cursor(&q))
                        };
                        rsx! {
                            div { class: "dbg-prove",
                                div { class: "dbg-mem-hint", "Assert a property of the variables. You get both lenses: whether it holds NOW (live values) and whether it's PROVEN for every run (the Oracle's static facts, with no Z3)." }
                                input {
                                    class: "dbg-prove-input",
                                    r#type: "text",
                                    placeholder: "e.g.  x < y    or    sum >= 0",
                                    value: "{q}",
                                    oninput: move |e| prove_query.set(e.value()),
                                }
                                if let Some(r) = result {
                                    if r.parsed {
                                        div { class: "dbg-prove-result",
                                            div { class: "dbg-prove-row",
                                                span { class: "dbg-prove-label", "now" }
                                                match r.now {
                                                    Some(true) => rsx! { span { class: "dbg-verdict yes", "true" } },
                                                    Some(false) => rsx! { span { class: "dbg-verdict no", "false" } },
                                                    None => rsx! { span { class: "dbg-verdict idk", "not a live value" } },
                                                }
                                                if !r.now_detail.is_empty() {
                                                    span { class: "dbg-prove-detail", "{r.now_detail}" }
                                                }
                                            }
                                            div { class: "dbg-prove-row",
                                                span { class: "dbg-prove-label", "every run" }
                                                {
                                                    let (cls, txt) = match r.verdict {
                                                        ProofVerdict::ProvenTrue => ("yes", "proven"),
                                                        ProofVerdict::ProvenFalse => ("no", "refuted"),
                                                        ProofVerdict::Unknown => ("idk", "unproven"),
                                                    };
                                                    rsx! { span { class: "dbg-verdict {cls}", "{txt}" } }
                                                }
                                                if !r.verdict_detail.is_empty() {
                                                    span { class: "dbg-prove-detail", "{r.verdict_detail}" }
                                                }
                                            }
                                        }
                                    } else {
                                        div { class: "dbg-prove-err", "{r.now_detail}" }
                                    }
                                }
                            }
                        }
                    },
                    DebugTab::CallStack => rsx! {
                        div { class: "dbg-stack",
                            for (i, frame) in snap.frames.iter().enumerate().rev() {
                                div { class: if i + 1 == snap.frames.len() { "dbg-frame current" } else { "dbg-frame" },
                                    span { class: "dbg-frame-fn",
                                        { frame.function.clone().unwrap_or_else(|| "Main".to_string()) }
                                    }
                                }
                            }
                        }
                    },
                    DebugTab::Breakpoints => rsx! {
                        div { class: "dbg-bps",
                            if bps.is_empty() {
                                div { class: "dbg-empty", "no breakpoints. Click a line in the Bytecode tab" }
                            }
                            for pc in bps.iter().copied() {
                                {
                                    let label = disasm.iter().find(|(p, _)| *p == pc)
                                        .map(|(_, t)| t.clone()).unwrap_or_default();
                                    rsx! {
                                        div { class: "dbg-bp",
                                            onclick: move |_| { if let Ok(d) = dbg.write().as_mut() { d.clear_breakpoint(pc); } },
                                            span { class: "dbg-bp-dot", "\u{25CF}" }
                                            span { class: "dbg-bp-pc", "{pc}" }
                                            span { class: "dbg-bp-op", "{label}" }
                                        }
                                    }
                                }
                            }
                        }
                    },
                    DebugTab::Bytecode => rsx! {
                        div { class: "dbg-tape",
                            for (pc, text) in disasm.iter() {
                                {
                                    let pc = *pc;
                                    let is_cur = pc == snap.pc && snap.state != "done";
                                    let has_bp = bps.contains(&pc);
                                    let row = if is_cur { "dbg-row cur" } else { "dbg-row" };
                                    rsx! {
                                        div { class: "{row}",
                                            onclick: move |_| { if let Ok(d) = dbg.write().as_mut() { d.toggle_breakpoint(pc); } },
                                            span { class: if has_bp { "dbg-gutter on" } else { "dbg-gutter" },
                                                if has_bp { "\u{25CF}" } else { "" }
                                            }
                                            span { class: "dbg-pc", "{pc}" }
                                            span { class: "dbg-text", "{text}" }
                                        }
                                    }
                                }
                            }
                        }
                    },
                }
            }
        }
    }
}

#[component]
fn DebugTabBtn(label: String, this: DebugTab, cur: DebugTab, tab: Signal<DebugTab>) -> Element {
    let active = this == cur;
    rsx! {
        button {
            class: if active { "dbg-tab active" } else { "dbg-tab" },
            onclick: move |_| tab.set(this),
            "{label}"
        }
    }
}

/// XML-escape a value for embedding in the datapath SVG.
fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// `s` cut to at most `max` characters, the last one an ellipsis when anything is cut.
fn clip(s: &str, max: usize) -> String {
    if s.chars().count() > max {
        format!("{}\u{2026}", s.chars().take(max - 1).collect::<String>())
    } else {
        s.to_string()
    }
}

/// Scroll the timeline so its playhead sits in view, once the next frame has laid it out.
#[cfg(target_arch = "wasm32")]
fn keep_playhead_in_view() {
    use wasm_bindgen::JsCast;
    let follow = wasm_bindgen::closure::Closure::once_into_js(move || {
        let Some(document) = web_sys::window().and_then(|w| w.document()) else {
            return;
        };
        let find = |selector: &str| document.query_selector(selector).ok().flatten();
        let (Some(view), Some(head)) = (find(".dbg-osc-scroll"), find(".dbg-osc-scroll .tlph"))
        else {
            return;
        };
        let (v, h) = (
            view.get_bounding_client_rect(),
            head.get_bounding_client_rect(),
        );
        let margin = v.width() * 0.15;
        if h.left() < v.left() + margin || h.right() > v.right() - margin {
            let target = view.scroll_left() as f64 + (h.left() - v.left()) - v.width() / 2.0;
            view.set_scroll_left(target.max(0.0) as i32);
        }
    });
    if let Some(window) = web_sys::window() {
        let _ = window.request_animation_frame(follow.unchecked_ref());
    }
}

/// The "virtual hardware" easter egg: an animated datapath of the paused VM — the
/// register file wired into the engine/ALU, the current op lit up, changed registers
/// pulsing. Pure string SVG + CSS keyframes (no deps; nothing renders until opened).
fn datapath_svg(snap: &DebugSnapshot) -> String {
    let regs: &[_] = match snap.frames.last() {
        Some(f) => &f.registers,
        None => &[],
    };
    let cw = 92.0_f64;
    let gap = 8.0_f64;
    // Wide enough for every register — and never narrower than the original 800, so a
    // small frame is not blown up past its designed size.
    let width = (24.0 + regs.len() as f64 * (cw + gap) - gap).max(800.0);
    let engine_x = width / 2.0;
    let engine_y = 104.0_f64;
    let mut s = format!(
        "<svg viewBox='0 0 {width:.0} 168' width='100%' preserveAspectRatio='xMidYMid meet' xmlns='http://www.w3.org/2000/svg'>",
    );
    s.push_str(DATAPATH_CSS);
    let reads: std::collections::BTreeSet<u16> = snap.op_reads.iter().copied().collect();
    let write = snap.op_writes;
    let cell_cx = |pos: usize| 12.0 + pos as f64 * (cw + gap) + cw / 2.0;
    // Operand wires: each SOURCE register of this op flows down into the engine.
    for (pos, r) in regs.iter().enumerate() {
        if reads.contains(&r.index) {
            let sx = cell_cx(pos);
            s.push_str(&format!(
                "<path class='wire live' d='M{sx:.0},44 C{sx:.0},80 {engine_x:.0},80 {engine_x:.0},{ey:.0}'/>",
                ey = engine_y - 4.0,
            ));
        }
    }
    // Result wire: the engine writes its result back up to the DESTINATION register.
    if let Some(w) = write {
        if let Some(pos) = regs.iter().position(|r| r.index == w) {
            let dx = cell_cx(pos);
            s.push_str(&format!(
                "<path class='wire out' d='M{engine_x:.0},{eb:.0} C{engine_x:.0},{lo:.0} {dx:.0},{lo:.0} {dx:.0},44'/>",
                eb = engine_y + 46.0,
                lo = engine_y + 72.0,
            ));
        }
    }
    // Register cells — sources glow cyan, the destination glows green.
    for (pos, r) in regs.iter().enumerate() {
        let x = 12.0 + pos as f64 * (cw + gap);
        let name = r.name.clone().unwrap_or_else(|| format!("R{}", r.index));
        let cls = if Some(r.index) == write {
            "rcell dst"
        } else if reads.contains(&r.index) {
            "rcell src"
        } else if r.changed {
            "rcell hot"
        } else {
            "rcell"
        };
        s.push_str(&format!(
            "<g class='{cls}'><rect x='{x:.0}' y='14' width='{cw:.0}' height='30' rx='6'/>\
             <text x='{tx:.0}' y='33' text-anchor='middle'>{n} = {v}</text></g>",
            tx = x + cw / 2.0,
            n = esc(&name),
            v = esc(&clip(&r.value, 9)),
        ));
    }
    // Engine box — the live operation, narrated in plain English.
    let engine_cls = if snap.state == "paused" {
        "engine live"
    } else {
        "engine"
    };
    let label = if !snap.narration.is_empty() {
        &snap.narration
    } else {
        &snap.op_text
    };
    let label = clip(label, 48);
    s.push_str(&format!(
        "<g class='{engine_cls}'><rect x='{ex:.0}' y='{ey:.0}' width='340' height='46' rx='8'/>\
         <text class='lbl' x='{tx:.0}' y='{ly:.0}' text-anchor='middle'>ALU \u{00B7} pc {pc}/{tot}</text>\
         <text class='op' x='{tx:.0}' y='{oy:.0}' text-anchor='middle'>{op}</text></g>",
        ex = engine_x - 170.0,
        ey = engine_y,
        tx = engine_x,
        ly = engine_y + 18.0,
        oy = engine_y + 37.0,
        pc = snap.pc,
        tot = snap.total_ops,
        op = esc(&label),
    ));
    s.push_str("</svg>");
    s
}

/// The **variable oscilloscope** — a logic-analyzer waveform of every variable's
/// value across the whole recorded run, runs of equal value merged into held "bus"
/// boxes, with a pink playhead pinned at the time-travel cursor. Two SVGs drawn side by
/// side: the lane names, which stay put, and the lanes, which scroll beneath a long run. Reuses the studio's
/// `waveform_svg` idiom; the program's entire observable past on one scope.
fn timeline_svg(tl: &VarTimeline) -> (String, String) {
    let gutter = 96.0_f64;
    let pad = 4.0_f64;
    let col_w = 34.0_f64;
    let row_h = 30.0_f64;
    let top = 22.0_f64;
    let n = tl.steps.max(1);
    let lanes = tl.vars.len().max(1);
    let width = pad + n as f64 * col_w + 14.0;
    let height = top + lanes as f64 * row_h + 16.0;
    let bottom = top + lanes as f64 * row_h;
    let col_x = |step: usize| pad + step as f64 * col_w;

    // The lane names: their own narrow SVG, so they stay put while the lanes scroll.
    let mut names = format!(
        "<svg viewBox='0 0 {gutter:.0} {height:.0}' width='{gutter:.0}' height='{height:.0}' xmlns='http://www.w3.org/2000/svg'>"
    );
    names.push_str(TIMELINE_CSS);
    for (ri, v) in tl.vars.iter().enumerate() {
        let y = top + ri as f64 * row_h;
        let label = if v.kind.is_empty() {
            v.name.clone()
        } else {
            format!("{} : {}", v.name, v.kind)
        };
        names.push_str(&format!(
            "<text class='tln' x='{:.0}' y='{:.0}'>{}</text>",
            gutter - 9.0,
            y + row_h * 0.62,
            esc(&label)
        ));
    }
    names.push_str("</svg>");

    let mut s = format!(
        "<svg viewBox='0 0 {width:.0} {height:.0}' width='{width:.0}' height='{height:.0}' xmlns='http://www.w3.org/2000/svg'>"
    );
    s.push_str(TIMELINE_CSS);

    // Time grid + step labels (thinned so they stay readable on long runs).
    let label_every = (n / 14).max(1);
    for step in 0..=n {
        let x = col_x(step);
        s.push_str(&format!(
            "<line class='tlg' x1='{x:.0}' y1='{top:.0}' x2='{x:.0}' y2='{bottom:.0}'/>"
        ));
        if step < n && step % label_every == 0 {
            s.push_str(&format!(
                "<text class='tltl' x='{:.0}' y='14'>{}</text>",
                x + col_w / 2.0,
                tl.start + step
            ));
        }
    }

    // Playhead at the cursor column.
    let px = col_x(tl.cursor.saturating_sub(tl.start));
    s.push_str(&format!(
        "<rect class='tlph-bg' x='{px:.0}' y='{top:.0}' width='{col_w:.0}' height='{:.0}'/>",
        bottom - top
    ));
    s.push_str(&format!(
        "<line class='tlph' x1='{px:.0}' y1='{:.0}' x2='{px:.0}' y2='{:.0}'/>",
        top - 6.0,
        bottom + 4.0
    ));

    for (ri, v) in tl.vars.iter().enumerate() {
        let y = top + ri as f64 * row_h;
        // Merge consecutive equal samples into one held box (the waveform "bus value").
        let mut i = 0usize;
        while i < v.points.len() {
            let p = &v.points[i];
            let mut j = i + 1;
            while j < v.points.len()
                && v.points[j].present == p.present
                && v.points[j].value == p.value
            {
                j += 1;
            }
            if p.present {
                let x = col_x(i);
                let span = (j - i) as f64 * col_w;
                let cls = if p.changed { "tlbox edge" } else { "tlbox" };
                s.push_str(&format!(
                    "<rect class='{cls}' x='{:.0}' y='{:.0}' width='{:.0}' height='{:.0}' rx='5'/>",
                    x + 1.5,
                    y + 4.0,
                    (span - 3.0).max(2.0),
                    row_h - 9.0
                ));
                let shown: String = p.value.chars().take((span / 7.0) as usize + 1).collect();
                s.push_str(&format!(
                    "<text class='tlv' x='{:.0}' y='{:.0}'>{}</text>",
                    x + span / 2.0,
                    y + row_h * 0.62,
                    esc(&shown)
                ));
            }
            i = j;
        }
    }
    s.push_str("</svg>");
    (names, s)
}

/// Render a [`CausalNode`] provenance tree to indented HTML — each value shown with
/// the op that produced it and the step it happened, its inputs nested below.
fn causal_html(node: &CausalNode) -> String {
    let mut s = String::new();
    causal_html_rec(node, 0, &mut s);
    s
}

fn causal_html_rec(node: &CausalNode, depth: usize, out: &mut String) {
    let name = node
        .name
        .clone()
        .unwrap_or_else(|| format!("R{}", node.reg));
    let how = if !node.narration.is_empty() {
        node.narration.clone()
    } else if !node.op_text.is_empty() {
        node.op_text.clone()
    } else {
        "initial value".to_string()
    };
    out.push_str(&format!(
        "<div class='cz-row' style='padding-left:{}px'>",
        depth * 18
    ));
    if depth > 0 {
        out.push_str("<span class='cz-arm'>\u{2514}\u{2500}</span>");
    }
    out.push_str(&format!(
        "<span class='cz-val'>{} = {}</span>",
        esc(&name),
        esc(&node.value)
    ));
    if !node.kind.is_empty() {
        out.push_str(&format!("<span class='cz-kind'>{}</span>", esc(&node.kind)));
    }
    out.push_str(&format!("<span class='cz-how'>{}</span>", esc(&how)));
    if node.step > 0 {
        out.push_str(&format!("<span class='cz-step'>step {}</span>", node.step));
    }
    out.push_str("</div>");
    for inp in &node.inputs {
        causal_html_rec(inp, depth + 1, out);
    }
}

const TIMELINE_CSS: &str = "<style>\
.tlg{stroke:rgba(255,255,255,0.05);stroke-width:1}\
.tltl{fill:rgba(255,255,255,0.4);font:9px ui-monospace,monospace;text-anchor:middle}\
.tln{fill:#a5b4fc;font:11px ui-monospace,monospace;text-anchor:end}\
.tlbox{fill:rgba(34,211,238,0.10);stroke:#22d3ee;stroke-width:1}\
.tlbox.edge{fill:rgba(129,140,248,0.20);stroke:#818cf8;stroke-width:1.5}\
.tlv{fill:#dbeafe;font:10px ui-monospace,monospace;text-anchor:middle}\
.tlph{stroke:#f472b6;stroke-width:1.5;opacity:0.9}\
.tlph-bg{fill:rgba(244,114,182,0.08)}\
</style>";

const IC_TRACE: &str = "<svg viewBox='0 0 16 16' width='12' height='12' fill='none' stroke='currentColor' stroke-width='1.3' stroke-linecap='round' stroke-linejoin='round'><circle cx='3.4' cy='3.4' r='1.6'/><circle cx='12.6' cy='3.4' r='1.6'/><circle cx='8' cy='12.6' r='1.6'/><path d='M3.4 5v2.2a2 2 0 0 0 2 2h5.2a2 2 0 0 0 2-2V5'/><path d='M8 9.4v1.6'/></svg>";

const IC_SHIELD: &str = "<svg viewBox='0 0 16 16' width='12' height='12' fill='none' stroke='currentColor' stroke-width='1.3' stroke-linecap='round' stroke-linejoin='round'><path d='M8 1.6l5 1.8v4c0 3.2-2.2 5.4-5 6.4-2.8-1-5-3.2-5-6.4v-4z'/><path d='M5.8 8l1.5 1.5L10.4 6'/></svg>";

const IC_SOCRATIC: &str = "<svg viewBox='0 0 16 16' width='14' height='14' fill='none' stroke='currentColor' stroke-width='1.3' stroke-linecap='round' stroke-linejoin='round'><path d='M3 2.8h10a1 1 0 0 1 1 1v6a1 1 0 0 1-1 1H7l-3 2.4V10.8H3a1 1 0 0 1-1-1v-6a1 1 0 0 1 1-1z'/><path d='M6.5 5.6a1.6 1.6 0 0 1 3 .5c0 1-1.5 1.3-1.5 2.2'/><path d='M8 9.9h.01'/></svg>";

const DATAPATH_CSS: &str = "<style>\
.rcell rect{fill:#161b22;stroke:rgba(255,255,255,0.12);stroke-width:1}\
.rcell text{fill:#cbd5e1;font:11px ui-monospace,monospace}\
.rcell.hot rect{stroke:#667eea;fill:#1e2540;animation:dpglow 1s ease-in-out infinite}\
.rcell.hot text{fill:#a5b4fc}\
.rcell.src rect{stroke:#22d3ee;fill:#0e2a33;animation:dpglow 1s ease-in-out infinite}\
.rcell.src text{fill:#67e8f9}\
.rcell.dst rect{stroke:#4ade80;fill:#0e2a1c;animation:dpglow 1s ease-in-out infinite}\
.rcell.dst text{fill:#86efac}\
@keyframes dpglow{0%,100%{stroke-opacity:.45}50%{stroke-opacity:1}}\
.wire{fill:none;stroke:rgba(255,255,255,0.08);stroke-width:1.5}\
.wire.live{stroke:#22d3ee;stroke-dasharray:5 4;animation:dpflow .7s linear infinite}\
.wire.out{fill:none;stroke:#4ade80;stroke-width:1.5;stroke-dasharray:5 4;animation:dpflow .7s linear infinite}\
@keyframes dpflow{to{stroke-dashoffset:-18}}\
.engine rect{fill:#12161c;stroke:rgba(255,255,255,0.18);stroke-width:1.5}\
.engine.live rect{stroke:#4ade80;animation:dpglow 1.4s ease-in-out infinite}\
.engine .lbl{fill:#9ca3af;font:bold 11px ui-monospace,monospace}\
.engine .op{fill:#e8eaed;font:12px ui-monospace,monospace}\
</style>";

// ── Debug-control icons — inline SVG (font-independent, crisp, `currentColor` so
//    they inherit each button's colour). No tofu, unlike obscure Unicode glyphs.
const IC_CONTINUE: &str = "<svg viewBox='0 0 16 16' width='15' height='15' fill='currentColor'><path d='M3.5 3v10l7-5z'/><rect x='11' y='3' width='1.8' height='10' rx='.6'/></svg>";
const IC_REVERSE: &str = "<svg viewBox='0 0 16 16' width='15' height='15' fill='currentColor'><rect x='3.2' y='3' width='1.8' height='10' rx='.6'/><path d='M12.5 3v10l-7-5z'/></svg>";
const IC_STEP_BACK: &str = "<svg viewBox='0 0 16 16' width='15' height='15' fill='currentColor'><rect x='3' y='3.2' width='1.7' height='9.6' rx='.6'/><path d='M12.5 3.2v9.6l-6.6-4.8z'/></svg>";
const IC_STEP_INTO: &str = "<svg viewBox='0 0 16 16' width='15' height='15' fill='none' stroke='currentColor' stroke-width='1.4' stroke-linecap='round' stroke-linejoin='round'><path d='M8 2.5v5'/><path d='M5.4 5L8 7.6 10.6 5'/><circle cx='8' cy='12.2' r='1.5' fill='currentColor' stroke='none'/></svg>";
const IC_STEP_OUT: &str = "<svg viewBox='0 0 16 16' width='15' height='15' fill='none' stroke='currentColor' stroke-width='1.4' stroke-linecap='round' stroke-linejoin='round'><path d='M8 8.1V3'/><path d='M5.4 5.6L8 3l2.6 2.6'/><circle cx='8' cy='12.2' r='1.5' fill='currentColor' stroke='none'/></svg>";
const IC_STEP_OVER: &str = "<svg viewBox='0 0 16 16' width='15' height='15' fill='none' stroke='currentColor' stroke-width='1.4' stroke-linecap='round' stroke-linejoin='round'><path d='M3 8.6c.4-4 9.6-4 10 .4'/><path d='M11 6.7l2 1.1-1.7 1.8'/><circle cx='8' cy='12.2' r='1.5' fill='currentColor' stroke='none'/></svg>";
const IC_RESTART: &str = "<svg viewBox='0 0 16 16' width='15' height='15' fill='none' stroke='currentColor' stroke-width='1.4' stroke-linecap='round' stroke-linejoin='round'><path d='M12.6 8a4.6 4.6 0 1 1-1.5-3.4'/><path d='M12.9 2.3l-.2 2.6-2.6-.4'/></svg>";
const IC_CLOCK: &str = "<svg viewBox='0 0 16 16' width='13' height='13' fill='none' stroke='currentColor' stroke-width='1.3' stroke-linecap='round' stroke-linejoin='round'><circle cx='8' cy='8.6' r='4.8'/><path d='M8 5.6v3l2 1.4'/><path d='M6 1.6h4M8 1.6v2'/></svg>";
const IC_CLOSE: &str = "<svg viewBox='0 0 16 16' width='15' height='15' fill='none' stroke='currentColor' stroke-width='1.7' stroke-linecap='round'><path d='M4 4l8 8M12 4l-8 8'/></svg>";
const IC_BULB: &str = "<svg viewBox='0 0 16 16' width='14' height='14' fill='none' stroke='currentColor' stroke-width='1.3' stroke-linecap='round' stroke-linejoin='round'><path d='M6 12h4M6.4 13.6h3.2'/><path d='M5 7.4a3 3 0 1 1 6 0c0 1.3-.8 2-1.3 2.7-.2.3-.2.6-.2 1.4H6.5c0-.8 0-1.1-.2-1.4C5.8 9.4 5 8.7 5 7.4z'/></svg>";
const IC_PLAY: &str = "<svg viewBox='0 0 16 16' width='15' height='15' fill='currentColor'><path d='M4.5 3v10l8-5z'/></svg>";
const IC_PAUSE: &str = "<svg viewBox='0 0 16 16' width='15' height='15' fill='currentColor'><rect x='4' y='3.5' width='2.7' height='9' rx='.8'/><rect x='9.3' y='3.5' width='2.7' height='9' rx='.8'/></svg>";
const IC_GEAR: &str = "<svg viewBox='0 0 16 16' width='15' height='15' fill='none' stroke='currentColor' stroke-width='1.2' stroke-linecap='round' stroke-linejoin='round'><circle cx='8' cy='8' r='2.1'/><path d='M8 1.7v1.6M8 12.7v1.6M14.3 8h-1.6M3.3 8H1.7M12.45 3.55l-1.13 1.13M4.68 11.32l-1.13 1.13M12.45 12.45l-1.13-1.13M4.68 4.68L3.55 3.55'/></svg>";

/// A ladybug glyph as inline SVG, for the Debug button + drawer title (shared with
/// the Studio toolbar). Explicitly sized so it needs no extra CSS.
pub const IC_BUG: &str = "<svg viewBox='0 0 16 16' width='14' height='14' fill='none' stroke='currentColor' stroke-width='1.2' stroke-linecap='round' stroke-linejoin='round'><ellipse cx='8' cy='9' rx='2.9' ry='3.5'/><path d='M8 5.6v6.8'/><path d='M5.1 9H2.7M10.9 9h2.4M5.3 6.5L3.6 5.2M10.7 6.5l1.7-1.3M5.3 11.6L3.6 12.9M10.7 11.6l1.7 1.3'/><path d='M6.4 4.4a1.6 1.6 0 0 1 3.2 0'/></svg>";

const DEBUG_DRAWER_STYLE: &str = r#"
.dbg-drawer { display:flex; flex-direction:column; background:var(--studio-panel-bg,#12161c);
  border-top:1px solid var(--studio-border,rgba(255,255,255,0.08)); color:var(--studio-text,#e8eaed);
  font-size:13px; flex:0 1 auto; min-height:44px; overflow:hidden; }
/* The grab bars are the column bars turned on their side: solid 8px, in their grey. */
.dbg-grip { flex:none; height:8px; background:var(--studio-resizer,#22272b); cursor:row-resize; touch-action:none; transition:background 0.2s ease; }
.dbg-grip:hover, .dbg-grip.active { background:var(--studio-accent,#667eea); }
/* Under a finger the grip takes a finger: its grab area reaches 18px past what it draws. */
@media (pointer: coarse) {
.dbg-grip { position:relative; }
.dbg-grip::before { content:""; position:absolute; left:0; right:0; top:-18px; bottom:-18px; z-index:1; }
}
.dbg-bar { display:flex; align-items:center; gap:12px; padding:6px 12px; flex-wrap:wrap;
  border-bottom:1px solid var(--studio-border,rgba(255,255,255,0.08)); }
.dbg-title { font-weight:600; }
.dbg-controls { display:flex; gap:4px; }
.dbg-btn { background:rgba(255,255,255,0.05); border:1px solid var(--studio-border,rgba(255,255,255,0.08));
  color:var(--studio-text,#e8eaed); border-radius:6px; padding:3px 8px; cursor:pointer; font-size:13px;
  line-height:1; transition:all .12s ease; }
.dbg-btn:hover:not(:disabled) { background:rgba(255,255,255,0.1); }
.dbg-btn:disabled { opacity:0.35; cursor:default; }
.dbg-go { color:#4ade80; }
.dbg-stop { color:#f87171; }
.dbg-toggle.on { background:rgba(102,126,234,0.3); border-color:var(--studio-accent,#667eea); }
.dbg-status { display:flex; align-items:center; gap:6px; margin-left:auto; color:var(--studio-text-secondary,#9ca3af);
  font-family:ui-monospace,monospace; font-size:12px; }
.dbg-sep { opacity:0.4; }
.dbg-op { color:var(--studio-text,#e8eaed); }
.dbg-state { text-transform:uppercase; font-size:11px; letter-spacing:0.5px; padding:1px 6px; border-radius:4px; }
.dbg-state-paused { background:rgba(102,126,234,0.25); color:#a5b4fc; }
.dbg-state-done { background:rgba(74,222,128,0.2); color:#4ade80; }
.dbg-state-blocked { background:rgba(251,191,36,0.2); color:#fbbf24; }
.dbg-state-error { background:rgba(248,113,113,0.2); color:#f87171; }
.dbg-state-playing { background:rgba(34,211,238,0.18); color:#22d3ee; }
.dbg-tabs { display:flex; gap:2px; padding:4px 8px 0; border-bottom:1px solid var(--studio-border,rgba(255,255,255,0.08)); }
.dbg-tab { background:transparent; border:none; color:var(--studio-text-muted,#6b7280); padding:5px 12px;
  cursor:pointer; border-radius:5px 5px 0 0; font-size:12px; }
.dbg-tab.active { color:var(--studio-text,#e8eaed); background:rgba(255,255,255,0.05); }
.dbg-body { flex:1; min-height:0; overflow:auto; padding:8px 12px; font-family:ui-monospace,monospace; font-size:12px; }
.dbg-empty { color:var(--studio-text-muted,#6b7280); font-style:italic; }
.dbg-var { display:flex; gap:6px; padding:2px 4px; border-radius:4px; }
.dbg-var.changed { background:rgba(102,126,234,0.18); }
.dbg-var-name { color:#a5b4fc; min-width:48px; }
.dbg-var-type { color:#f0abfc; font-size:11px; opacity:0.85; background:rgba(240,171,252,0.1); border-radius:4px; padding:0 5px; align-self:center; }
.dbg-var-eq { opacity:0.4; }
.dbg-var-val { color:#4ade80; }
.dbg-var { cursor:pointer; border-radius:5px; }
.dbg-var:hover { background:rgba(255,255,255,0.04); }
.dbg-var:hover .dbg-why { opacity:0.7; }
.dbg-var.traced { background:rgba(34,211,238,0.12); box-shadow:inset 2px 0 0 #22d3ee; }
.dbg-var.traced .dbg-why { opacity:1; color:#22d3ee; }
.dbg-why { margin-left:auto; display:inline-flex; align-items:center; color:#6b7280; opacity:0; transition:opacity 0.12s; }
.dbg-prov { margin:8px 6px 4px; border:1px solid rgba(34,211,238,0.25); border-radius:8px; background:rgba(34,211,238,0.04); overflow:hidden; }
.dbg-prov-h { display:flex; align-items:center; justify-content:space-between; padding:5px 10px; background:rgba(34,211,238,0.08); border-bottom:1px solid rgba(34,211,238,0.15); }
.dbg-prov-title { display:flex; align-items:center; gap:6px; color:#67e8f9; font-size:11px; text-transform:uppercase; letter-spacing:0.5px; }
.dbg-prov-ico { display:inline-flex; align-items:center; color:#22d3ee; }
.dbg-prov-x { background:transparent; border:none; color:#6b7280; cursor:pointer; display:inline-flex; padding:2px; border-radius:4px; }
.dbg-prov-x:hover { background:rgba(248,113,113,0.15); color:#f87171; }
.dbg-prov-tree { padding:8px 10px; font-family:ui-monospace,monospace; font-size:12px; }
.cz-row { display:flex; align-items:center; gap:8px; padding:2px 0; white-space:nowrap; }
.cz-arm { color:#475569; }
.cz-val { color:#4ade80; font-weight:600; }
.cz-kind { color:#f0abfc; font-size:10px; opacity:0.8; }
.cz-how { color:#cbd5e1; opacity:0.85; }
.cz-step { color:#6b7280; font-size:10px; margin-left:auto; }
.dbg-osc-wrap { display:flex; flex-direction:column; gap:4px; }
.dbg-osc { display:flex; padding:4px 2px 8px; }
.dbg-osc-names { flex:none; }
.dbg-osc-scroll { flex:1 1 auto; min-width:0; overflow-x:auto; }
.dbg-mem-hint.dim { opacity:0.6; font-size:11px; }
.dbg-fol { margin-left:auto; display:inline-flex; align-items:center; gap:8px; min-width:0; flex-shrink:1; font-family:ui-monospace,monospace; }
.dbg-fol-tag { color:#a78bfa; font-size:13px; }
.dbg-fol-text { color:#ddd6fe; font-size:12.5px; white-space:nowrap; overflow:hidden; text-overflow:ellipsis; }
.dbg-prove { display:flex; flex-direction:column; gap:8px; padding:2px 4px; }
.dbg-prove-input { background:var(--studio-panel-bg,#12161c); border:1px solid var(--studio-border,rgba(255,255,255,0.12)); border-radius:6px; color:#e8eaed; font-family:ui-monospace,monospace; font-size:13px; padding:7px 10px; outline:none; }
.dbg-prove-input:focus { border-color:#a78bfa; }
.dbg-prove-result { display:flex; flex-direction:column; gap:6px; padding:4px 2px; }
.dbg-prove-row { display:flex; align-items:center; gap:9px; flex-wrap:wrap; }
.dbg-prove-label { color:#6b7280; font-size:10px; text-transform:uppercase; letter-spacing:0.6px; min-width:62px; }
.dbg-verdict { font-weight:600; font-size:12px; border-radius:10px; padding:1px 9px; }
.dbg-verdict.yes { color:#86efac; background:rgba(74,222,128,0.14); border:1px solid rgba(74,222,128,0.3); }
.dbg-verdict.no { color:#fca5a5; background:rgba(248,113,113,0.14); border:1px solid rgba(248,113,113,0.3); }
.dbg-verdict.idk { color:#9ca3af; background:rgba(156,163,175,0.12); border:1px solid rgba(156,163,175,0.25); }
.dbg-prove-detail { color:#94a3b8; font-family:ui-monospace,monospace; font-size:11px; }
.dbg-prove-err { color:#fbbf24; font-size:12px; padding:4px 2px; }
.dbg-insights { margin:2px 6px 6px; }
.dbg-insights-h { color:#67e8f9; font-size:10px; text-transform:uppercase; letter-spacing:0.6px; margin-bottom:5px; }
.dbg-insights-h.proven { color:#c4b5fd; display:flex; align-items:center; gap:5px; }
.dbg-proven-ico { display:inline-flex; align-items:center; color:#a78bfa; }
.dbg-insight { display:flex; align-items:center; gap:7px; flex-wrap:wrap; padding:2px 0; }
.dbg-insight-name { color:#a5b4fc; font-family:ui-monospace,monospace; font-size:12px; min-width:48px; }
.dbg-insight-fact { color:#86efac; background:rgba(74,222,128,0.1); border:1px solid rgba(74,222,128,0.2); border-radius:10px; padding:0 8px; font-size:11px; }
.dbg-insight-fact.proven { color:#ddd6fe; background:rgba(167,139,250,0.12); border-color:rgba(167,139,250,0.3); font-family:ui-monospace,monospace; }
.dbg-globals-h { color:var(--studio-text-muted,#6b7280); margin-top:8px; text-transform:uppercase; font-size:10px; letter-spacing:0.5px; }
.dbg-frame { padding:3px 6px; border-radius:4px; }
.dbg-frame.current { background:rgba(102,126,234,0.18); }
.dbg-frame-fn { color:#a5b4fc; }
.dbg-bp, .dbg-row { display:flex; gap:8px; align-items:center; padding:2px 4px; cursor:pointer; border-radius:3px; }
.dbg-bp:hover, .dbg-row:hover { background:rgba(255,255,255,0.05); }
.dbg-row.cur { background:rgba(102,126,234,0.22); }
.dbg-gutter { width:12px; color:#f87171; text-align:center; }
.dbg-pc, .dbg-bp-pc { color:var(--studio-text-muted,#6b7280); min-width:28px; text-align:right; }
.dbg-text, .dbg-bp-op { color:var(--studio-text,#e8eaed); }
.dbg-bp-dot { color:#f87171; }
.dbg-error { padding:12px; color:#f87171; }
.dbg-hw { padding:10px 12px; border-bottom:1px solid var(--studio-border,rgba(255,255,255,0.08)); color:var(--studio-text-secondary,#9ca3af); font-style:italic; }
.dbg-btn { display:inline-flex; align-items:center; justify-content:center; }
.dbg-btn svg { display:block; }
.dbg-title { display:inline-flex; align-items:center; gap:6px; }
.dbg-bug { display:inline-flex; color:#f87171; }
.dbg-divider { width:1px; height:18px; background:var(--studio-border,rgba(255,255,255,0.12)); margin:0 3px; align-self:center; }
.dbg-rev { color:#c084fc; }
.dbg-scrub { display:flex; align-items:center; gap:10px; padding:5px 12px; border-bottom:1px solid var(--studio-border,rgba(255,255,255,0.08)); }
.dbg-scrub-cap { color:var(--studio-text-muted,#6b7280); font-size:13px; }
.dbg-scrub-range { flex:1; accent-color:var(--studio-accent,#667eea); height:4px; cursor:pointer; }
.dbg-scrub-label { color:var(--studio-text-secondary,#9ca3af); font-family:ui-monospace,monospace; font-size:11px; min-width:62px; text-align:right; }
.dbg-narr { display:flex; align-items:center; gap:9px; padding:8px 14px; background:linear-gradient(90deg,rgba(102,126,234,0.12),rgba(102,126,234,0.03)); border-bottom:1px solid var(--studio-border,rgba(255,255,255,0.08)); }
.dbg-narr-ico { display:inline-flex; color:#fbbf24; flex-shrink:0; }
.dbg-narr-text { color:var(--studio-text,#e8eaed); font-size:13.5px; flex:0 1 auto; min-width:0; white-space:nowrap; overflow:hidden; text-overflow:ellipsis; }
.dbg-narr-done .dbg-narr-text, .dbg-narr-error .dbg-narr-text { font-style:italic; color:var(--studio-text-secondary,#9ca3af); }
.dbg-narr-error .dbg-narr-ico { color:#f87171; }
.dbg-narr-socratic { background:linear-gradient(90deg,rgba(34,211,238,0.13),rgba(34,211,238,0.03)); }
.dbg-narr-socratic .dbg-narr-ico { color:#22d3ee; }
.dbg-narr-socratic .dbg-narr-text { color:#cffafe; }
.dbg-play { color:#4ade80; }
.dbg-mem-hint { color:var(--studio-text-muted,#6b7280); font-size:11px; margin-bottom:8px; }
.dbg-sframe { margin-bottom:8px; border:1px solid var(--studio-border,rgba(255,255,255,0.08)); border-radius:6px; overflow:hidden; }
.dbg-sframe-h { display:flex; justify-content:space-between; padding:4px 8px; background:rgba(255,255,255,0.04); }
.dbg-sframe-fn { color:#a5b4fc; font-weight:600; }
.dbg-sframe-base { color:var(--studio-text-muted,#6b7280); font-family:ui-monospace,monospace; font-size:11px; }
.dbg-slot { display:flex; gap:10px; padding:1px 8px; }
.dbg-slot.changed { background:rgba(102,126,234,0.18); }
.dbg-slot-addr { color:#6b7280; font-family:ui-monospace,monospace; min-width:36px; }
.dbg-slot-name { color:#cbd5e1; min-width:50px; }
.dbg-slot-type { color:#f0abfc; font-size:10px; opacity:0.75; min-width:42px; }
.dbg-slot-val { color:#4ade80; }
.dbg-heap { display:flex; flex-direction:column; gap:8px; }
.dbg-hobj { border:1px solid var(--studio-border,rgba(255,255,255,0.12)); border-radius:7px; padding:6px 9px; background:rgba(255,255,255,0.02); }
.dbg-hobj.shared { border-color:#fbbf24; background:rgba(251,191,36,0.06); }
.dbg-hobj-h { display:flex; align-items:center; gap:8px; }
.dbg-hobj-id { color:#22d3ee; font-family:ui-monospace,monospace; font-weight:600; }
.dbg-hobj-kind { color:var(--studio-text-secondary,#9ca3af); text-transform:uppercase; font-size:10px; letter-spacing:0.5px; }
.dbg-hobj-store { color:#22d3ee; font-family:ui-monospace,monospace; font-size:10px; background:rgba(34,211,238,0.1); padding:0 5px; border-radius:3px; }
.dbg-hobj-rc { color:#9ca3af; font-size:10px; }
.dbg-hobj-alias { color:#fbbf24; font-size:10px; text-transform:uppercase; letter-spacing:0.5px; }
.dbg-hobj-val { color:#86efac; margin:3px 0; word-break:break-all; }
.dbg-hobj-refs { display:flex; align-items:center; gap:5px; flex-wrap:wrap; }
.dbg-refs-label { color:#6b7280; }
.dbg-ref { background:rgba(102,126,234,0.2); color:#a5b4fc; padding:0 6px; border-radius:4px; font-size:11px; }
.dbg-close { display:inline-flex; align-items:center; justify-content:center; background:transparent; border:none; color:var(--studio-text-muted,#6b7280); cursor:pointer; padding:4px 5px; border-radius:5px; margin-left:8px; }
.dbg-close:hover { background:rgba(248,113,113,0.15); color:#f87171; }
.dbg-reload { display:flex; align-items:center; gap:12px; padding:7px 14px; background:rgba(251,191,36,0.1); border-bottom:1px solid var(--studio-border,rgba(255,255,255,0.08)); }
.dbg-reload-msg { color:#fbbf24; font-size:12.5px; flex:1; }
.dbg-reload-btn { background:#fbbf24; color:#1a1f27; border:none; border-radius:5px; padding:3px 12px; font-weight:600; cursor:pointer; font-size:12px; }
.dbg-reload-btn:hover { filter:brightness(1.1); }
"#;

#[cfg(test)]
#[path = "../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/components/debug_drawer/tests.rs"]
mod tests;
