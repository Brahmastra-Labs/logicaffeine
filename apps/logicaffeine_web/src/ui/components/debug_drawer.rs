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
use logicaffeine_compile::debug::{DebugSnapshot, Debugger};

#[derive(Clone, Copy, PartialEq)]
enum DebugTab {
    Variables,
    Stack,
    Heap,
    CallStack,
    Breakpoints,
    Bytecode,
}

/// The bottom debug drawer. `source` is the Code-mode program; `on_close` stops the
/// session (the Studio hides the drawer).
#[component]
pub fn DebugDrawer(source: String, on_close: EventHandler<()>) -> Element {
    // The debugger is built once from the source the session opened with.
    let mut dbg = use_signal(|| Debugger::from_source(&source));
    let mut tab = use_signal(|| DebugTab::Variables);
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

    // Compile error → a minimal error drawer (nothing to step).
    if let Err(msg) = &*dbg.read() {
        let msg = msg.clone();
        return rsx! {
            style { "{DEBUG_DRAWER_STYLE}" }
            div { class: "dbg-drawer",
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
    let bps: std::collections::BTreeSet<usize> =
        dbg.read().as_ref().ok().map(|d| d.breakpoints().into_iter().collect()).unwrap_or_default();

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

    rsx! {
        style { "{DEBUG_DRAWER_STYLE}" }
        div { class: "dbg-drawer",
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
                        title: if playing() { "Pause" } else { "Auto-play \u{2014} watch it run" },
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
                    button { class: "dbg-btn dbg-stop", title: "Stop debugging",
                        onclick: move |_| on_close.call(()), dangerous_inner_html: IC_STOP }
                }
                div { class: "dbg-status",
                    span { class: "dbg-state dbg-state-{snap.state}", "{snap.state}" }
                    span { class: "dbg-sep", "\u{00B7}" }
                    span { "pc {snap.pc}/{snap.total_ops}" }
                    if !snap.op_text.is_empty() {
                        span { class: "dbg-sep", "\u{00B7}" }
                        span { class: "dbg-op", "{snap.op_text}" }
                    }
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

            // Plain-English narration of what this step does — the teaching line.
            if !snap.narration.is_empty() {
                div { class: "dbg-narr dbg-narr-{snap.state}",
                    span { class: "dbg-narr-ico", dangerous_inner_html: IC_BULB }
                    span { class: "dbg-narr-text", "{snap.narration}" }
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
                DebugTabBtn { label: "Call Stack", this: DebugTab::CallStack, cur: cur_tab, tab }
                DebugTabBtn { label: "Breakpoints", this: DebugTab::Breakpoints, cur: cur_tab, tab }
                DebugTabBtn { label: "Bytecode", this: DebugTab::Bytecode, cur: cur_tab, tab }
            }

            div { class: "dbg-body",
                match cur_tab {
                    DebugTab::Variables => rsx! {
                        if let Some(frame) = snap.frames.last() {
                            div { class: "dbg-vars",
                                if frame.registers.is_empty() {
                                    div { class: "dbg-empty", "no locals in this frame" }
                                }
                                for reg in frame.registers.iter() {
                                    div { class: if reg.changed { "dbg-var changed" } else { "dbg-var" },
                                        span { class: "dbg-var-name",
                                            { reg.name.clone().unwrap_or_else(|| format!("R{}", reg.index)) }
                                        }
                                        span { class: "dbg-var-eq", "=" }
                                        span { class: "dbg-var-val", "{reg.value}" }
                                    }
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
                            div { class: "dbg-mem-hint", "The register file \u{2014} the VM's stack memory. Each frame's slots at their addresses (current frame on top)." }
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
                            div { class: "dbg-mem-hint", "Heap objects (lists, maps, structs, text). A box shared by two variables is an alias \u{2014} one allocation, two names." }
                            if snap.heap.is_empty() {
                                div { class: "dbg-empty", "no heap objects \u{2014} only scalar values in scope" }
                            }
                            for obj in snap.heap.iter() {
                                div { class: if obj.shared { "dbg-hobj shared" } else { "dbg-hobj" },
                                    div { class: "dbg-hobj-h",
                                        span { class: "dbg-hobj-id", "{obj.id}" }
                                        span { class: "dbg-hobj-kind", "{obj.kind}" }
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
                                div { class: "dbg-empty", "no breakpoints \u{2014} click a line in the Bytecode tab" }
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

            // Output strip (the Show lines so far).
            if !snap.output.is_empty() {
                div { class: "dbg-out",
                    for line in snap.output.iter() {
                        div { class: "dbg-out-line", "{line}" }
                    }
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
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
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
    let engine_x = 400.0_f64;
    let engine_y = 104.0_f64;
    let mut s = String::from(
        "<svg viewBox='0 0 800 168' width='100%' preserveAspectRatio='xMidYMid meet' xmlns='http://www.w3.org/2000/svg'>",
    );
    s.push_str(DATAPATH_CSS);
    let reads: std::collections::BTreeSet<u16> = snap.op_reads.iter().copied().collect();
    let write = snap.op_writes;
    let cell_cx = |pos: usize| 12.0 + pos as f64 * (cw + gap) + cw / 2.0;
    // Operand wires: each SOURCE register of this op flows down into the engine.
    for (pos, r) in regs.iter().take(8).enumerate() {
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
        if let Some(pos) = regs.iter().take(8).position(|r| r.index == w) {
            let dx = cell_cx(pos);
            s.push_str(&format!(
                "<path class='wire out' d='M{engine_x:.0},{eb:.0} C{engine_x:.0},{lo:.0} {dx:.0},{lo:.0} {dx:.0},44'/>",
                eb = engine_y + 46.0,
                lo = engine_y + 72.0,
            ));
        }
    }
    // Register cells — sources glow cyan, the destination glows green.
    for (pos, r) in regs.iter().take(8).enumerate() {
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
            v = esc(&r.value),
        ));
    }
    // Engine box — the live operation, narrated in plain English.
    let engine_cls = if snap.state == "paused" { "engine live" } else { "engine" };
    let label = if !snap.narration.is_empty() { &snap.narration } else { &snap.op_text };
    let label: String = if label.chars().count() > 48 {
        format!("{}\u{2026}", label.chars().take(47).collect::<String>())
    } else {
        label.clone()
    };
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
const IC_STOP: &str = "<svg viewBox='0 0 16 16' width='15' height='15' fill='currentColor'><rect x='3.5' y='3.5' width='9' height='9' rx='1.6'/></svg>";
const IC_CLOCK: &str = "<svg viewBox='0 0 16 16' width='13' height='13' fill='none' stroke='currentColor' stroke-width='1.3' stroke-linecap='round' stroke-linejoin='round'><circle cx='8' cy='8.6' r='4.8'/><path d='M8 5.6v3l2 1.4'/><path d='M6 1.6h4M8 1.6v2'/></svg>";
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
  font-size:13px; max-height:320px; min-height:160px; flex-shrink:0; }
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
.dbg-tabs { display:flex; gap:2px; padding:4px 8px 0; border-bottom:1px solid var(--studio-border,rgba(255,255,255,0.08)); }
.dbg-tab { background:transparent; border:none; color:var(--studio-text-muted,#6b7280); padding:5px 12px;
  cursor:pointer; border-radius:5px 5px 0 0; font-size:12px; }
.dbg-tab.active { color:var(--studio-text,#e8eaed); background:rgba(255,255,255,0.05); }
.dbg-body { flex:1; min-height:0; overflow:auto; padding:8px 12px; font-family:ui-monospace,monospace; font-size:12px; }
.dbg-empty { color:var(--studio-text-muted,#6b7280); font-style:italic; }
.dbg-var { display:flex; gap:6px; padding:2px 4px; border-radius:4px; }
.dbg-var.changed { background:rgba(102,126,234,0.18); }
.dbg-var-name { color:#a5b4fc; min-width:48px; }
.dbg-var-eq { opacity:0.4; }
.dbg-var-val { color:#4ade80; }
.dbg-globals-h, .dbg-out { color:var(--studio-text-muted,#6b7280); margin-top:8px; text-transform:uppercase; font-size:10px; letter-spacing:0.5px; }
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
.dbg-out { border-top:1px solid var(--studio-border,rgba(255,255,255,0.08)); padding:6px 12px; max-height:80px; overflow:auto; }
.dbg-out-line { color:#4ade80; font-family:ui-monospace,monospace; font-size:12px; }
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
.dbg-narr-text { color:var(--studio-text,#e8eaed); font-size:13.5px; }
.dbg-narr-done .dbg-narr-text, .dbg-narr-error .dbg-narr-text { font-style:italic; color:var(--studio-text-secondary,#9ca3af); }
.dbg-narr-error .dbg-narr-ico { color:#f87171; }
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
.dbg-slot-val { color:#4ade80; }
.dbg-heap { display:flex; flex-direction:column; gap:8px; }
.dbg-hobj { border:1px solid var(--studio-border,rgba(255,255,255,0.12)); border-radius:7px; padding:6px 9px; background:rgba(255,255,255,0.02); }
.dbg-hobj.shared { border-color:#fbbf24; background:rgba(251,191,36,0.06); }
.dbg-hobj-h { display:flex; align-items:center; gap:8px; }
.dbg-hobj-id { color:#22d3ee; font-family:ui-monospace,monospace; font-weight:600; }
.dbg-hobj-kind { color:var(--studio-text-secondary,#9ca3af); text-transform:uppercase; font-size:10px; letter-spacing:0.5px; }
.dbg-hobj-rc { color:#9ca3af; font-size:10px; }
.dbg-hobj-alias { color:#fbbf24; font-size:10px; text-transform:uppercase; letter-spacing:0.5px; }
.dbg-hobj-val { color:#86efac; margin:3px 0; word-break:break-all; }
.dbg-hobj-refs { display:flex; align-items:center; gap:5px; flex-wrap:wrap; }
.dbg-refs-label { color:#6b7280; }
.dbg-ref { background:rgba(102,126,234,0.2); color:#a5b4fc; padding:0 6px; border-radius:4px; font-size:11px; }
"#;
