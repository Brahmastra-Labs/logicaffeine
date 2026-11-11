use crate::ui::components::footer::Footer;
use crate::ui::components::main_nav::{ActivePage, MainNav};
use crate::ui::seo::{
    breadcrumb_schema, organization_schema, pages as seo_pages, webpage_schema, BreadcrumbItem,
    JsonLdMultiple, PageHead,
};
use dioxus::prelude::*;
#[cfg(all(feature = "split", target_arch = "wasm32"))]
use dioxus::wasm_split;
use serde::Deserialize;
use std::collections::HashMap;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::LazyLock;
#[cfg(target_arch = "wasm32")]
use std::sync::OnceLock;

/// The toggle vector (one bool per registry optimization, in discriminant order)
/// as an [`OptimizationConfig`]. Index `i` ↔ `REGISTRY[i].opt` ↔ bit `i`.
fn config_from_toggles(toggles: &[bool]) -> logicaffeine_compile::optimization::OptimizationConfig {
    let mut cfg = logicaffeine_compile::optimization::OptimizationConfig::all_off();
    for (i, m) in logicaffeine_compile::optimization::REGISTRY
        .iter()
        .enumerate()
    {
        if toggles.get(i).copied().unwrap_or(false) {
            cfg.set(m.opt, true);
        }
    }
    cfg
}

/// The inverse of [`config_from_toggles`].
fn toggles_from_config(cfg: &logicaffeine_compile::optimization::OptimizationConfig) -> Vec<bool> {
    logicaffeine_compile::optimization::REGISTRY
        .iter()
        .map(|m| cfg.is_on(m.opt))
        .collect()
}

mod bars;
mod charts;
mod codec;
mod data;
mod eda;
mod interp;
mod jit_code;
mod language;
mod model;
mod solver;
use self::bars::*;
use self::charts::*;
use self::codec::*;
use self::data::*;
use self::eda::*;
use self::interp::*;
use self::jit_code::*;
use self::language::*;
use self::model::*;
use self::solver::*;

#[cfg(test)]
#[path = "../../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/pages/benchmarks/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/pages/benchmarks/solver_data_guard.rs"]
mod solver_data_guard;

#[cfg(test)]
#[path = "../../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/pages/benchmarks/eda_data_guard.rs"]
mod eda_data_guard;

const BENCHMARKS_STYLE: &str = include_str!("style.css");

/// The three top-level benchmark stories, shown as tabs: the programming
/// language itself, the SAT solver, and the transport wire codec.
#[derive(Clone, Copy, PartialEq, Eq)]
enum BenchTab {
    Language,
    Solver,
    Codec,
}

impl BenchTab {
    fn label(self) -> &'static str {
        match self {
            BenchTab::Language => "Programming Language",
            BenchTab::Solver => "SAT Solver",
            BenchTab::Codec => "Transport Codec",
        }
    }

    fn all() -> [BenchTab; 3] {
        [BenchTab::Language, BenchTab::Solver, BenchTab::Codec]
    }
}

/// The output panel's tabs: one per LOGOS tier, each showing what that tier runs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum CodeTab {
    Rust,
    C,
    CBytecode,
    Jit,
    Vm,
    TreeWalker,
}

impl CodeTab {
    fn all() -> [CodeTab; 6] {
        [
            CodeTab::Rust,
            CodeTab::C,
            CodeTab::CBytecode,
            CodeTab::Jit,
            CodeTab::Vm,
            CodeTab::TreeWalker,
        ]
    }

    /// The tier's label on the charts, read from the registry so the tab and the bar always agree.
    fn tier(self) -> &'static str {
        match self {
            CodeTab::Rust => "logos_release",
            CodeTab::C => "logos_c",
            CodeTab::CBytecode => "logos_cbc",
            CodeTab::Jit => "logos_jit",
            CodeTab::Vm => "logos_vm",
            CodeTab::TreeWalker => "logos_tw",
        }
    }

    fn fallback_label(self) -> &'static str {
        match self {
            CodeTab::Rust => "LOGOS (Rust)",
            CodeTab::C => "LOGOS (C)",
            CodeTab::CBytecode => "LOGOS (C, bytecode)",
            CodeTab::Jit => "LOGOS (JIT)",
            CodeTab::Vm => "LOGOS (VM)",
            CodeTab::TreeWalker => "LOGOS (tree-walker)",
        }
    }
}

/// The program part of a generated C file: what follows the runtime the C generator prepends.
fn c_program_part(full: &str) -> String {
    let sentinel = logicaffeine_compile::codegen_c::USER_CODE_SENTINEL;
    match full.find(sentinel) {
        Some(i) => full[i + sentinel.len()..]
            .trim_start_matches('\n')
            .to_string(),
        None => full.to_string(),
    }
}

/// Gate component: guarantees the data feeds are pinned before the page proper
/// renders, so every accessor inside `BenchmarksLoaded` can `expect` its feed.
/// Native builds (tests, SSG prerender) compile the data in and render directly;
/// wasm fetches the staged `/data` bundle first, behind a lightweight shell.
#[component(lazy)]
pub fn Benchmarks() -> Element {
    #[cfg(not(target_arch = "wasm32"))]
    return rsx! { BenchmarksLoaded {} };

    #[cfg(target_arch = "wasm32")]
    {
        let mut bundle = use_resource(load_bench_bundle);
        let state = bundle.read_unchecked();
        match &*state {
            Some(Ok(())) => rsx! { BenchmarksLoaded {} },
            Some(Err(e)) => rsx! {
                BenchShell {
                    p { class: "bench-gate-status error", "Benchmark data failed to load: {e}" }
                    button {
                        class: "bench-gate-retry",
                        onclick: move |_| bundle.restart(),
                        "Retry"
                    }
                }
            },
            None => rsx! {
                BenchShell {
                    p { class: "bench-gate-status", "Loading benchmark data\u{2026}" }
                }
            },
        }
    }
}

/// The nav/meta shell shown while the wasm data bundle is in flight (or failed):
/// full PageHead + nav + footer, so the page keeps its meta and chrome either way.
#[cfg(target_arch = "wasm32")]
#[component]
fn BenchShell(children: Element) -> Element {
    rsx! {
        PageHead {
            title: seo_pages::BENCHMARKS.title,
            description: seo_pages::BENCHMARKS.description,
            canonical_path: seo_pages::BENCHMARKS.canonical_path,
        }
        style { "{BENCHMARKS_STYLE}" }
        div { class: "bench-container",
            MainNav { active: ActivePage::Benchmarks, subtitle: Some("Performance") }
            section { class: "bench-hero",
                h1 { "Benchmarks" }
                {children}
            }
            Footer {}
        }
    }
}

#[component]
fn BenchmarksLoaded() -> Element {
    let data = bench_data().expect("gated: the Benchmarks shell loads the bundle first");
    // Presentation order: same-algorithm benchmarks first, optimizer-collapse
    // benchmarks (recursive fib, two_sum, …) last — so the first result a skeptic
    // double-clicks is a fair, like-for-like comparison, not a compiler collapse.
    // Stable sort keeps the underlying data order within each group; the
    // `data.benchmarks` array itself is untouched (the hero card still reads
    // `benchmarks[0]`).
    let display_order: Vec<usize> = {
        let mut idx: Vec<usize> = (0..data.benchmarks.len()).collect();
        idx.sort_by_key(|&i| data.benchmarks[i].collapse_note().is_some());
        idx
    };
    let first_apples = display_order.first().copied().unwrap_or(0);
    let mut active_bench = use_signal(move || first_apples);
    let mut bench_tab = use_signal(|| BenchTab::Language);
    // Optimization-toggle showcase state: one on/off per registry optimization
    // (all on by default = the normal optimized build). Flipping one inserts a
    // `## No <X>` decorator and recompiles the Rust live, in the browser.
    let mut opt_toggles =
        use_signal(|| vec![true; logicaffeine_compile::optimization::REGISTRY.len()]);
    // Live-compile state for the toggle showcase. `live_rust` holds the last
    // in-browser compile (None = show the pre-baked cached Rust), `compiling`
    // drives the spinner, and `compile_gen` lets a newer toggle supersede an
    // in-flight compile.
    let mut live_rust = use_signal(|| Option::<String>::None);
    // Which tier's output the right-hand panel shows. Rust, C and the VM's bytecode rebuild in the browser when an
    // optimization is off; the bytecode C and the JIT's machine code are the measured build's, every optimization on.
    let mut code_tab = use_signal(|| CodeTab::Rust);
    // The browser-built C or bytecode listing for the current toggles: `(tab, toggles, text)`, so a stale build for
    // another tab or another toggle state is never shown.
    let mut live_alt = use_signal(|| Option::<(CodeTab, Vec<bool>, String)>::None);
    let mut alt_gen = use_signal(|| 0u64);
    // The size the main chart shows; `None` is the benchmark's reference size.
    let mut chosen_size = use_signal(|| Option::<String>::None);
    let mut compiling = use_signal(|| false);
    let mut compile_gen = use_signal(|| 0u64);
    // The all-on optimization graph for the current benchmark — the STABLE tree
    // structure. `base_fired` (what fires), `base_preempted` (blockers), and
    // `base_dependencies` (emergent per-program dependencies) are seeded instantly
    // from the baked benchmark data so switching benchmarks pops the tree in with no
    // compile; `fired_now`/`preempted_now` reflect the CURRENT toggle state (what is
    // firing right now), updated by the live re-trace when an optimization is off.
    let mut base_fired = use_signal(Vec::<&'static str>::new);
    let mut fired_now = use_signal(Vec::<&'static str>::new);
    let mut preempted_now = use_signal(Vec::<(&'static str, &'static str)>::new);
    let mut base_preempted = use_signal(Vec::<(&'static str, &'static str)>::new);
    let mut base_dependencies = use_signal(Vec::<(&'static str, &'static str)>::new);
    // Tree expand/collapse state, one bool per registry optimization (default
    // expanded). A collapsed parent hides its requires-descendants.
    let mut expanded =
        use_signal(|| vec![true; logicaffeine_compile::optimization::REGISTRY.len()]);
    let mut stats_open = use_signal(|| false);
    let mut compile_detail_open = use_signal(|| false);
    let mut methodology_open = use_signal(|| false);
    let mut source_open: Signal<[bool; 10]> = use_signal(|| [false; 10]);

    // Seed the stable all-on graph from the BAKED benchmark data and re-trace the
    // showcase Rust on toggle changes. The DEFAULT all-on view never compiles in the
    // browser — the optimization graph is embedded in `latest.json` (baked by the
    // benchmark run / `scripts/bench/bake-opt-graph.sh`, exactly like the timing results)
    // and the Rust is the pre-baked `generated_rust`, so switching benchmarks is
    // instant. ONLY turning an optimization OFF triggers a browser compile, and only
    // to show that toggled state's Rust + which opts then fire — the one thing that
    // cannot be pre-computed (it is combinatorial). Tracks `active_bench` +
    // `opt_toggles`; `compile_gen` is read via peek so the effect never re-triggers
    // itself.
    use_effect(move || {
        let idx = active_bench();
        let toggles = opt_toggles();
        let b = &data.benchmarks[idx];
        let kw =
            |s: &str| logicaffeine_compile::optimization::by_keyword(s).map(|o| o.meta().keyword);
        // The baked all-on graph (keywords → interned &'static str). Always seeds the
        // stable tree structure — no analysis in the browser.
        let baked_fired: Vec<&'static str> = b.fired.iter().filter_map(|s| kw(s)).collect();
        let baked_blockers: Vec<(&'static str, &'static str)> = b
            .blockers
            .iter()
            .filter_map(|(w, l)| Some((kw(w)?, kw(l)?)))
            .collect();
        let baked_deps: Vec<(&'static str, &'static str)> = b
            .dependencies
            .iter()
            .filter_map(|(d, x)| Some((kw(d)?, kw(x)?)))
            .collect();
        base_fired.set(baked_fired.clone());
        base_preempted.set(baked_blockers.clone());
        base_dependencies.set(baked_deps);

        let disabled: Vec<&'static str> = logicaffeine_compile::optimization::REGISTRY
            .iter()
            .enumerate()
            .filter(|(i, _)| !toggles[*i])
            .map(|(_, m)| m.keyword)
            .collect();
        let cache_present = !b.generated_rust.trim().is_empty();

        // Default view (every optimization on): fully served from baked data — the
        // baked fired set, the baked blockers, and the cached Rust. No compile, ever.
        if disabled.is_empty() {
            fired_now.set(baked_fired);
            preempted_now.set(baked_blockers);
            live_rust.set(if cache_present {
                None
            } else {
                Some(b.generated_rust.clone())
            });
            compiling.set(false);
            return;
        }

        // An optimization is OFF — compile just this toggled state to show its Rust
        // and which optimizations now fire.
        let decorated =
            logicaffeine_compile::optimization::decorate_source(&b.logos_source, &disabled);
        let gen = *compile_gen.peek() + 1;
        compile_gen.set(gen);
        compiling.set(true);
        spawn(async move {
            gloo_timers::future::TimeoutFuture::new(30).await;
            if *compile_gen.peek() != gen {
                return;
            }
            let (rust, fired, preempted) = logicaffeine_compile::compile::compile_to_rust_traced(
                &decorated,
            )
            .unwrap_or_else(|e| (format!("// compile error: {e:?}"), Vec::new(), Vec::new()));
            if *compile_gen.peek() != gen {
                return;
            }
            fired_now.set(fired);
            preempted_now.set(preempted);
            live_rust.set(Some(rust));
            compiling.set(false);
        });
    });

    // The JIT's machine code is fetched the first time its tab opens.
    #[cfg(target_arch = "wasm32")]
    let jit_loaded = use_resource(move || async move {
        if code_tab() == CodeTab::Jit {
            load_jit_code().await;
        }
        code_tab()
    });
    #[cfg(target_arch = "wasm32")]
    let _ = jit_loaded.read();

    // Build the C or the bytecode listing for the current toggles when that tab is open. The bytecode listing has no
    // baked form, so it is built for the all-on state too; the C's all-on form is the measured run's.
    use_effect(move || {
        let tab = code_tab();
        let toggles = opt_toggles();
        let idx = active_bench();
        if !matches!(tab, CodeTab::C | CodeTab::Vm) {
            return;
        }
        if tab == CodeTab::C && toggles.iter().all(|on| *on) {
            return;
        }
        let disabled: Vec<&'static str> = logicaffeine_compile::optimization::REGISTRY
            .iter()
            .enumerate()
            .filter(|(i, _)| !toggles[*i])
            .map(|(_, m)| m.keyword)
            .collect();
        let decorated = logicaffeine_compile::optimization::decorate_source(
            &data.benchmarks[idx].logos_source,
            &disabled,
        );
        let gen = *alt_gen.peek() + 1;
        alt_gen.set(gen);
        spawn(async move {
            gloo_timers::future::TimeoutFuture::new(30).await;
            if *alt_gen.peek() != gen {
                return;
            }
            let text = match tab {
                CodeTab::C => logicaffeine_compile::compile::compile_to_c(&decorated)
                    .map(|c| c_program_part(&c))
                    .unwrap_or_else(|e| format!("// the C generator refused this program: {e:?}")),
                _ => logicaffeine_compile::ui_bridge::vm_listing(&decorated)
                    .unwrap_or_else(|why| format!("; {why}")),
            };
            if *alt_gen.peek() != gen {
                return;
            }
            live_alt.set(Some((tab, toggles, text)));
        });
    });

    let breadcrumbs = vec![
        BreadcrumbItem {
            name: "Home",
            path: "/",
        },
        BreadcrumbItem {
            name: "Benchmarks",
            path: "/benchmarks",
        },
    ];
    let schemas = vec![
        organization_schema(),
        webpage_schema(
            "LOGOS Benchmarks",
            seo_pages::BENCHMARKS.description,
            "/benchmarks",
        ),
        breadcrumb_schema(&breadcrumbs),
    ];

    let interp = interp_data().expect("gated: the Benchmarks shell loads the bundle first");
    // Headline numbers, all framed as "x the speed of <baseline>" (higher = faster).
    let collapsed: std::collections::HashSet<&str> = data
        .benchmarks
        .iter()
        .filter(|b| b.collapse_note().is_some())
        .map(|b| b.id.as_str())
        .collect();
    let interp_speed = interp_speed_vs_v8(interp, &collapsed);
    let aot_speed = aot_speed_vs_v8(interp);
    let collapse_count = data
        .benchmarks
        .iter()
        .filter(|b| b.collapse_note().is_some())
        .count();

    let bench = &data.benchmarks[active_bench()];
    let bench_sources = bench_sources_for(&bench.id)
        .expect("sources_cover_every_benchmark locks every benchmark id to a sources entry");

    // Toggle showcase — cheap, render-safe derived state only. The decorated
    // LOGOS source is a string op; the expensive Rust compile is NOT run here
    // (doing it on every render froze the page). With every optimization on we
    // show the pre-baked cached Rust (`bench.generated_rust`) for an instant,
    // no-compile view; any optimization off shows the live-compiled `live_rust`
    // produced asynchronously by the use_effect above, with a spinner in between.
    let opt_tog = opt_toggles();
    let opt_disabled: Vec<&'static str> = logicaffeine_compile::optimization::REGISTRY
        .iter()
        .enumerate()
        .filter(|(i, _)| !opt_tog[*i])
        .map(|(_, m)| m.keyword)
        .collect();
    let opt_decorated =
        logicaffeine_compile::optimization::decorate_source(&bench.logos_source, &opt_disabled);
    let all_opts_on = opt_disabled.is_empty();

    // The program's optimization chain as a tree, derived by the crate's
    // `relationship_tree` from the baked all-on graph: the fired opts and the
    // `requires` enablers they hang under (so `coins`' fired `unchecked`/
    // `oraclehints`/`elemtype` show under `oracle`), the BLOCKERS they skipped, and
    // the per-program DEPENDENCIES that nest one opt under another it only fired
    // because of (dead-code under scalarization, symmetry under partial eval). Map
    // the keyword signals back to `Opt` and hand them to the deterministic O(n²)
    // derivation — the single source of truth for the menu-tree.
    let opt_tree: Vec<logicaffeine_compile::optimization::OptNode> = {
        use logicaffeine_compile::optimization::by_keyword;
        let fired: Vec<_> = base_fired()
            .iter()
            .filter_map(|kw| by_keyword(kw))
            .collect();
        let preempted: Vec<_> = base_preempted()
            .iter()
            .filter_map(|(w, l)| Some((by_keyword(w)?, by_keyword(l)?)))
            .collect();
        let dependencies: Vec<_> = base_dependencies()
            .iter()
            .filter_map(|(d, x)| Some((by_keyword(d)?, by_keyword(x)?)))
            .collect();
        logicaffeine_compile::optimization::relationship_tree(&fired, &preempted, &dependencies)
    };

    // Collapse visibility, parallel to `opt_tree`: walking the pre-order DFS, once
    // a collapsed parent is seen every deeper node is hidden until depth returns to
    // the parent's level. A single threshold suffices because the order is DFS.
    let tree_visible: Vec<bool> = {
        let exp = expanded();
        let mut vis = Vec::with_capacity(opt_tree.len());
        let mut hide_from: Option<usize> = None;
        for node in &opt_tree {
            if let Some(d) = hide_from {
                if node.depth <= d {
                    hide_from = None;
                }
            }
            let visible = hide_from.is_none();
            if visible && node.has_children && !exp.get(node.opt as usize).copied().unwrap_or(true)
            {
                hide_from = Some(node.depth);
            }
            vis.push(visible);
        }
        vis
    };

    let cache_ok = !bench.generated_rust.trim().is_empty();
    let rust_loading = (!all_opts_on || !cache_ok) && (compiling() || live_rust().is_none());
    let rust_text: String = if all_opts_on && cache_ok {
        bench.generated_rust.clone()
    } else {
        live_rust().unwrap_or_default()
    };
    // The output panel: one tab per LOGOS tier, labelled as its bar is.
    let code_tabs: Vec<(CodeTab, String)> = CodeTab::all()
        .into_iter()
        .map(|t| {
            let label = data
                .languages
                .iter()
                .find(|l| l.id == t.tier())
                .map(|l| l.label.clone());
            (t, label.unwrap_or_else(|| t.fallback_label().to_string()))
        })
        .collect();
    let tab = code_tab();
    let stage_trace = match tab {
        CodeTab::Rust => &bench.optimization_stages.compiled,
        CodeTab::C => &bench.optimization_stages.c,
        CodeTab::CBytecode => &bench.optimization_stages.c_bytecode,
        CodeTab::Jit => &bench.optimization_stages.jit,
        CodeTab::Vm => &bench.optimization_stages.vm,
        CodeTab::TreeWalker => &bench.optimization_stages.run,
    };
    let optimizer_trace = stage_trace.optimizer.join(", ");
    let lowering_trace = stage_trace.lowering.join(", ");
    let decline_trace = stage_trace.declines.join("; ");
    let fallback_trace = stage_trace.fallbacks.join("; ");
    let native_jit_trace = if tab == CodeTab::Jit {
        bench
            .jit
            .as_ref()
            .map(|jit| jit.optimizations.join(", "))
            .unwrap_or_default()
    } else {
        String::new()
    };
    let (native_jit_compiled, native_jit_declines, native_jit_execution) = if tab == CodeTab::Jit {
        bench
            .jit
            .as_ref()
            .map(|jit| {
                (
                    jit.compiled.clone().unwrap_or_default(),
                    jit.declines.clone(),
                    jit.execution.clone(),
                )
            })
            .unwrap_or_default()
    } else {
        (String::new(), String::new(), String::new())
    };
    let alt_text: Option<String> = live_alt()
        .filter(|(t, toggles, _)| *t == tab && *toggles == opt_tog)
        .map(|(_, _, text)| text);
    let loading = rsx! {
        div { class: "bench-source-code", span { class: "bench-compiling", "Compiling\u{2026}" } }
    };
    let as_measured = if all_opts_on {
        ", every optimization on"
    } else {
        ", with the optimizations above switched off"
    };
    let code_body: Element = match tab {
        CodeTab::Rust if rust_loading => loading,
        CodeTab::Rust => rsx! {
            div { class: "bench-source-note",
                "The Rust " code { "largo build --release" } " hands to cargo{as_measured}."
            }
            div { class: "bench-source-code", "{rust_text}" }
        },
        CodeTab::C => {
            let text = if all_opts_on {
                Some(bench.generated_c.clone()).filter(|c| !c.trim().is_empty())
            } else {
                alt_text
            };
            match text {
                Some(text) => rsx! {
                    div { class: "bench-source-note",
                        "The C " code { "largo build --release --emit c" } " generates from the program{as_measured}. "
                        "The runtime it links against is the same for every benchmark and is not shown."
                    }
                    div { class: "bench-source-code", "{text}" }
                },
                None if all_opts_on => {
                    rsx! { div { class: "bench-source-code", "This run did not emit C for this benchmark." } }
                }
                None => loading,
            }
        }
        CodeTab::CBytecode if bench.generated_c_bytecode.trim().is_empty() => rsx! {
            div { class: "bench-source-code", "This run did not emit C from the bytecode for this benchmark." }
        },
        CodeTab::CBytecode => rsx! {
            div { class: "bench-source-note",
                "The C " code { "largo build --release --emit c-bytecode" } " generates from the program's bytecode, "
                "every optimization on: the build the run measured"
                {if all_opts_on { "." } else { " (the browser does not rebuild this form, so the toggles above do not change it)." }}
            }
            div { class: "bench-source-code", "{bench.generated_c_bytecode}" }
        },
        CodeTab::Jit => match jit_code_data()
            .and_then(|d| d.benchmarks.get(&bench.id))
            .filter(|r| !r.is_empty())
        {
            Some(regions) => rsx! {
                div { class: "bench-source-note",
                    "The x86-64 machine code the JIT emitted while the run verified this benchmark, every optimization "
                    "on: each function and loop that got hot, disassembled at the address it ran from."
                }
                div { class: "bench-source-code bench-asm",
                    for region in regions.iter() {
                        div { class: "bench-asm-region", key: "{region.start}",
                            div { class: "bench-asm-title", "{region.title()} \u{00b7} {region.bytes} bytes at {region.start}" }
                            "{region.asm}"
                        }
                    }
                }
            },
            None if !jit_code_settled() => loading,
            None => rsx! {
                div { class: "bench-source-code",
                    "This run recorded no machine code for this benchmark: nothing in it got hot enough to compile, or the "
                    "run was assembled without the JIT's code."
                }
            },
        },
        CodeTab::Vm => match alt_text {
            Some(text) => rsx! {
                div { class: "bench-source-note",
                    "The bytecode " code { "largo run --interpret --engine vm" } " runs, one section per function, "
                    "built in your browser{as_measured}."
                }
                div { class: "bench-source-code", "{text}" }
            },
            None => loading,
        },
        CodeTab::TreeWalker => rsx! {
            div { class: "bench-source-note",
                code { "largo run --interpret --engine tree" } " interprets the optimized syntax tree directly; there is "
                "no compiled form. The program it parses:"
            }
            div { class: "bench-source-code", "{opt_decorated}" }
        },
    };

    // Use reference_size if it has data, otherwise fall back to the largest benchmarked size
    let ref_size = if bench.scaling.contains_key(bench.reference_size.as_str()) {
        bench.reference_size.clone()
    } else {
        bench
            .sizes
            .last()
            .cloned()
            .unwrap_or_else(|| bench.reference_size.clone())
    };
    // Every size this run timed, ascending. The chart and the statistics show the chosen one, the reference size until
    // the reader picks another.
    let mut timed_sizes: Vec<String> = bench.scaling.keys().cloned().collect();
    timed_sizes.sort_by_key(|n| n.parse::<u64>().unwrap_or(u64::MAX));
    let shown_size = chosen_size()
        .filter(|n| bench.scaling.contains_key(n.as_str()))
        .unwrap_or_else(|| ref_size.clone());
    let ref_timings = bench.scaling.get(shown_size.as_str());

    // Stats table entries (all fields, sorted by median; timed-out langs appended at end)
    let mut stats_entries: Vec<(&str, &str, Option<&TimingResult>)> = Vec::new();
    let ref_timed_out = bench.timed_out.get(shown_size.as_str());
    if let Some(timings) = ref_timings {
        for lang in &data.languages {
            if let Some(t) = timings.get(&lang.id) {
                stats_entries.push((&lang.label, &lang.id, Some(t)));
            } else if ref_timed_out.is_some_and(|m| m.contains_key(&lang.id)) {
                stats_entries.push((&lang.label, &lang.id, None));
            }
        }
    }
    stats_entries.sort_by(|a, b| match (a.2, b.2) {
        (Some(ta), Some(tb)) => ta
            .median_ms
            .partial_cmp(&tb.median_ms)
            .unwrap_or(std::cmp::Ordering::Equal),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });

    // Compilation entries sorted by mean_ms
    let mut compile_entries: Vec<(&str, f64, f64, bool)> = bench
        .compilation
        .iter()
        .map(|(name, r)| {
            (
                name.as_str(),
                r.mean_ms,
                r.stddev_ms,
                name.starts_with("logos"),
            )
        })
        .collect();
    compile_entries.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

    let compile_max = compile_entries.last().map(|e| e.1).unwrap_or(1.0);

    // Interpreter vs V8, for the active benchmark (at its own calibrated N)
    let interp_bench = interp.benchmarks.iter().find(|ib| ib.id == bench.id);
    // (label, bar class, median_ms, is_logos, node_floored)
    let mut interp_bars: Vec<(&'static str, &'static str, f64, bool, bool)> = Vec::new();
    let mut interp_n = String::new();
    let mut interp_engine = String::new();
    let mut interp_active_speed: Option<f64> = None;
    if let Some(ib) = interp_bench {
        interp_n = ib.reference_size.clone();
        interp_engine = ib.interpreter_engine.clone();
        if let Some(t) = interp_ref(ib) {
            // The LOGOS tier ladder (eager VM → tiered VM+JIT → AOT-native, HOTSWAP
            // §12/§Axis-3) then the peer runtimes. Data-driven: a tier renders only
            // when its row is present, so tiered/AOT appear once the run emits them.
            for id in ["logos_interp", "logos_tiered", "logos_aot", "js"] {
                if let Some(tr) = t.get(id) {
                    let lbl = if id == "js" {
                        "Node / V8"
                    } else {
                        lang_label(id)
                    };
                    let col = bar_class(id);
                    let is_logos = id.starts_with("logos");
                    interp_bars.push((
                        lbl,
                        col,
                        tr.median_ms,
                        is_logos,
                        id == "js" && node_floored(tr),
                    ));
                }
            }
            if let (Some(j), Some(l)) = (t.get("js"), t.get("logos_interp")) {
                if l.median_ms > 0.0 {
                    interp_active_speed = Some(j.median_ms / l.median_ms);
                }
            }
        }
    }
    interp_bars.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));
    let interp_max = interp_bars
        .iter()
        .map(|e| e.2)
        .fold(0.0_f64, f64::max)
        .max(1e-9);
    if interp_engine.is_empty() {
        interp_engine = "\u{2014}".to_string();
    }
    let interp_offfloor = interp
        .benchmarks
        .iter()
        .filter(|ib| interp_counts(ib, &collapsed))
        .count();
    let interp_node_ver = if interp.metadata.node.is_empty() {
        "Node".to_string()
    } else {
        format!("Node {}", interp.metadata.node)
    };

    // Cold-start floor (serverless / CLI): time to launch the engine and run a
    // trivial program. Smaller is faster. (label, bar class, mean_ms, is_logos)
    let mut startup_bars: Vec<(&'static str, &'static str, f64, bool)> = Vec::new();
    for id in ["logos_interp", "js"] {
        if let Some(t) = interp.startup.engines.get(id) {
            if t.mean_ms > 0.0 {
                let lbl = match id {
                    "logos_interp" => "LOGOS interp",
                    "js" => "Node / V8",
                    _ => id,
                };
                let col = bar_class(id);
                startup_bars.push((lbl, col, t.mean_ms, id == "logos_interp"));
            }
        }
    }
    startup_bars.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));
    let startup_max = startup_bars
        .iter()
        .map(|e| e.2)
        .fold(0.0_f64, f64::max)
        .max(1e-9);
    let startup_logos = interp
        .startup
        .engines
        .get("logos_interp")
        .map(|t| t.mean_ms)
        .unwrap_or(0.0);
    let startup_node = interp
        .startup
        .engines
        .get("js")
        .map(|t| t.mean_ms)
        .unwrap_or(0.0);
    let startup_vs_v8 = if startup_logos > 0.0 {
        startup_node / startup_logos
    } else {
        0.0
    };

    // Engine footprint — what you ship to run a program. (label, bar class, as_built, stripped, is_logos)
    let mut engine_size_bars: Vec<(&'static str, &'static str, f64, Option<f64>, bool)> =
        Vec::new();
    let mut wasm_bundle_bytes = 0.0_f64;
    if let Some(es) = &interp.interpreter_sizes {
        for id in ["logos", "node", "deno", "bun"] {
            if let Some(s) = es.engines.get(id) {
                if s.as_built > 0.0 {
                    let lbl = match id {
                        "logos" => "largo (LOGOS VM+JIT)",
                        "node" => "Node / V8",
                        "deno" => "Deno",
                        "bun" => "Bun",
                        _ => id,
                    };
                    let col = if id == "logos" {
                        bar_class("logos_jit")
                    } else {
                        bar_class(id)
                    };
                    engine_size_bars.push((lbl, col, s.as_built, s.stripped, id == "logos"));
                }
            }
        }
        wasm_bundle_bytes = es.wasm_bundle_bytes.unwrap_or(0.0);
    }
    engine_size_bars.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));
    let engine_size_max = engine_size_bars
        .iter()
        .map(|e| e.2)
        .fold(0.0_f64, f64::max)
        .max(1.0);
    let wasm_bundle_str = if wasm_bundle_bytes > 0.0 {
        format_bytes(wasm_bundle_bytes)
    } else {
        String::new()
    };

    // Source code languages to show (not LOGOS — that's always visible)
    let source_langs = [
        "c", "cpp", "rust", "zig", "go", "java", "js", "python", "ruby", "nim",
    ];

    let commit_url = format!("{}/commit/{}", GITHUB_REPO, data.metadata.commit);
    let release_url = format!(
        "{}/releases/tag/v{}",
        GITHUB_REPO, data.metadata.logos_version
    );
    let raw_json_url = format!("{}/blob/main/benchmarks/results/latest.json", GITHUB_REPO);
    // Per-tab hero metadata: each section has its own baked measurement provenance.
    let solver_meta = &solver_data()
        .expect("gated: the Benchmarks shell loads the bundle first")
        .metadata;
    let codec_meta = &codec_data()
        .expect("gated: the Benchmarks shell loads the bundle first")
        .metadata;
    let solver_json_url = format!("{}/blob/main/benchmarks/results/solvers.json", GITHUB_REPO);
    let codec_json_url = format!(
        "{}/blob/main/benchmarks/results/latest-codec.json",
        GITHUB_REPO
    );
    let solver_date = solver_meta
        .date
        .get(..10)
        .unwrap_or(solver_meta.date.as_str());
    let codec_date = codec_meta
        .date
        .get(..10)
        .unwrap_or(codec_meta.date.as_str());
    let history_url = format!("{}/tree/main/benchmarks/results/history", GITHUB_REPO);
    let runsh_url = format!("{}/blob/main/benchmarks/run.sh", GITHUB_REPO);
    let bench_dir_url = format!("{}/tree/main/benchmarks", GITHUB_REPO);

    rsx! {
        PageHead {
            title: seo_pages::BENCHMARKS.title,
            description: seo_pages::BENCHMARKS.description,
            canonical_path: seo_pages::BENCHMARKS.canonical_path,
        }
        style { "{BENCHMARKS_STYLE}" }
        JsonLdMultiple { schemas }

        div { class: "bench-container",
            MainNav { active: ActivePage::Benchmarks, subtitle: Some("Performance") }

            // Hero + Overview
            section { class: "bench-hero", id: "overview",
                h1 { "Benchmarks" }
                if bench_tab() == BenchTab::Language {
                    p {
                        "LOGOS runs six ways: compiled through Rust, compiled to C from the program and from its "
                        "bytecode, on our x86-64 JIT, on the bytecode VM, and on the tree-walking interpreter. Here is "
                        "each one against the languages people pick when speed matters, from programs with "
                        "English-level readability."
                    }
                    div { class: "bench-pills",
                        a { class: "bench-pill", href: "{release_url}", target: "_blank",
                            strong { "v{data.metadata.logos_version}" }
                        }
                        a { class: "bench-pill", href: "{commit_url}", target: "_blank",
                            "{data.metadata.commit}"
                        }
                        span { class: "bench-pill", "{data.metadata.cpu}" }
                        span { class: "bench-pill", "{data.metadata.os}" }
                        span { class: "bench-pill", "{data.metadata.day()}" }
                        if let Some(env) = &data.metadata.environment {
                            if !env.pinning.cpus.is_empty() {
                                span { class: "bench-pill", title: "Every timed command ran on these CPUs ({env.pinning.source}).",
                                    "Pinned to CPUs {env.pinning.cpus}"
                                }
                            }
                            if env.noisy() {
                                span { class: "bench-pill warn",
                                    title: "This run skipped the quiet-machine check, so another workload may have shared the CPUs.",
                                    "Noisy run"
                                }
                            }
                        }
                        a { class: "bench-pill link", href: "{raw_json_url}", target: "_blank",
                            "Raw JSON"
                        }
                        a { class: "bench-pill link", href: "/data/flamegraphs.pdf",
                            download: "logos-flamegraphs.pdf",
                            "\u{2b07} Flamegraph PDF"
                        }
                    }
                }
                if bench_tab() == BenchTab::Solver {
                    p { "Certified proofs in milliseconds \u{2014} the same pure-Rust prover that runs in your browser, with no Z3." }
                    div { class: "bench-pills",
                        if !solver_meta.cpu.is_empty() {
                            span { class: "bench-pill", "{solver_meta.cpu}" }
                        }
                        if !solver_meta.kissat.is_empty() {
                            span { class: "bench-pill", "Kissat {solver_meta.kissat}" }
                        }
                        if !solver_meta.date.is_empty() {
                            span { class: "bench-pill", "{solver_date}" }
                        }
                        a { class: "bench-pill link", href: "{solver_json_url}", target: "_blank",
                            "Raw JSON"
                        }
                    }
                }
                if bench_tab() == BenchTab::Codec {
                    p { "Smallest bytes on the wire \u{2014} the LOGOS codec, head-to-head against the industry serializers." }
                    div { class: "bench-pills",
                        if !codec_meta.cpu.is_empty() {
                            span { class: "bench-pill", "{codec_meta.cpu}" }
                        }
                        if !codec_meta.os.is_empty() {
                            span { class: "bench-pill", "{codec_meta.os}" }
                        }
                        if !codec_meta.date.is_empty() {
                            span { class: "bench-pill", "{codec_date}" }
                        }
                        a { class: "bench-pill link", href: "{codec_json_url}", target: "_blank",
                            "Raw JSON"
                        }
                    }
                }
            }

            // Top-level tabs: the three benchmark stories.
            nav { class: "bench-tabbar",
                for t in BenchTab::all() {
                    button {
                        key: "{t.label()}",
                        class: if bench_tab() == t { "bench-tabbar-btn active" } else { "bench-tabbar-btn" },
                        onclick: move |_| bench_tab.set(t),
                        "{t.label()}"
                    }
                }
            }

            // In-page anchor nav for the programming-language sub-sections.
            if bench_tab() == BenchTab::Language {
                nav { class: "bench-section-nav",
                    a { href: "#overview", "Overview" }
                    a { href: "#performance", "Performance" }
                    a { href: "#interpreter", "Interpreter" }
                    a { href: "#source", "Source Code" }
                    a { href: "#compilation", "Compilation" }
                    a { href: "#summary", "Summary" }
                    a { href: "#methodology", "Methodology" }
                }
            }

            if bench_tab() == BenchTab::Language {
                div { class: "bench-content",
                    // One card per LOGOS tier: each tier's own geometric mean against C, never one number
                    // picked across them.
                    div { class: "bench-summary bench-summary-tiers",
                        for card in tier_headlines(data).iter() {
                            div { class: "bench-summary-card bench-summary-{card.id}",
                                div { class: "bench-summary-eyebrow", "{card.label}" }
                                div { class: "bench-summary-value",
                                    {card.vs_c.map(|v| format!("{v:.2}×")).unwrap_or_else(|| "—".to_string())}
                                }
                                div { class: "bench-summary-label", "the speed of C, geometric mean" }
                                if let Some(same) = card.same_algorithm {
                                    div { class: "bench-summary-sub", "{same:.2}× where every language runs the same algorithm" }
                                }
                                if let Some((k, n)) = card.coverage {
                                    div { class: "bench-summary-sub", "{k} of {n} benchmarks" }
                                }
                            }
                        }
                    }

                    div { class: "bench-intro",
                        p {
                            "Benchmarks are easy to rig, so we've tried to make ours easy to check. Every program on this "
                            "page computes the same answer in every language, byte for byte, at every size we time, and "
                            "every number comes out of one script you can run on your own machine. LOGOS shows up once for "
                            "every way it runs: compiled through Rust, compiled to C from the program and from its "
                            "bytecode, on our x86-64 JIT, on the bytecode VM alone, and on the tree-walking interpreter. "
                            "Each one's output is in the panel under the source, so you can read what actually ran."
                        }
                        p {
                            "Sometimes the compiler wins by doing less work, like folding a recursion into a closed form. "
                            "Those benchmarks are marked \u{26a1}, the second number on each card leaves them out, and the "
                            "generated code is right there to read. A faster algorithm and faster machine code are two "
                            "different claims, and we'd rather you didn't have to take our word for either."
                        }
                    }

                    // Benchmark tabs (shared across performance, source, compilation)
                    div { class: "bench-tabs",
                        for &i in display_order.iter() {
                            button {
                                key: "{i}",
                                class: if active_bench() == i { "bench-tab active" } else { "bench-tab" },
                                onclick: move |_| {
                                    active_bench.set(i);
                                    stats_open.set(false);
                                    compile_detail_open.set(false);
                                    source_open.set([false; 10]);
                                    // Reset optimizations to all-on so the new benchmark
                                    // shows its cached Rust instantly (no compile on switch).
                                    opt_toggles.set(vec![true; logicaffeine_compile::optimization::REGISTRY.len()]);
                                    live_rust.set(None);
                                    live_alt.set(None);
                                    chosen_size.set(None);
                                    compiling.set(false);
                                    base_fired.set(Vec::new());
                                    fired_now.set(Vec::new());
                                    preempted_now.set(Vec::new());
                                    base_preempted.set(Vec::new());
                                    base_dependencies.set(Vec::new());
                                    expanded.set(vec![true; logicaffeine_compile::optimization::REGISTRY.len()]);
                                },
                                "{data.benchmarks[i].name}"
                                if data.benchmarks[i].collapse_note().is_some() {
                                    span { class: "bench-tab-badge", title: "Algorithm collapsed by the LOGOS compiler", "\u{26a1}" }
                                }
                            }
                        }
                    }

                    // =============== PERFORMANCE ===============
                    div { class: "bench-section", id: "performance",
                        div { class: "bench-section-title", "{bench.name}" }
                        div { class: "bench-section-desc",
                            "{bench.description} (n = {shown_size})"
                        }

                        if let Some(note) = bench.collapse_note() {
                            div { class: "bench-callout",
                                span { class: "bench-callout-icon", "\u{26a1}" }
                                span { "{note}" }
                            }
                        }

                        if timed_sizes.len() > 1 {
                            div { class: "bench-size-pills", role: "tablist", "aria-label": "Problem size",
                                for n in timed_sizes.iter().cloned() {
                                    button {
                                        key: "{n}",
                                        class: if n == shown_size { "bench-size-pill active" } else { "bench-size-pill" },
                                        role: "tab",
                                        "aria-selected": "{n == shown_size}",
                                        onclick: {
                                            let n = n.clone();
                                            move |_| chosen_size.set(Some(n.clone()))
                                        },
                                        "n = {n}"
                                        if n == ref_size {
                                            span { class: "bench-size-ref", " reference" }
                                        }
                                    }
                                }
                            }
                        }
                        div { class: "bench-chart-hint", "Wall-clock time at n = {shown_size} \u{2014} shorter bar is faster." }

                        BarChart {
                            groups: chart_groups(
                                &data.languages,
                                ref_timings,
                                bench.timed_out.get(shown_size.as_str()),
                                bench.limits.get(shown_size.as_str()),
                            ),
                            unit: BarUnit::Millis,
                        }

                        // Scaling curve — time vs problem size across the benchmarked sizes
                        div {
                            class: "bench-section-desc bench-subhead",
                            "Scaling \u{2014} time vs problem size (log\u{2013}log)"
                        }
                        {scaling_chart(bench, &data.languages)}

                        {complexity_panel(bench, &data.languages)}

                        // Peak memory — same style as the runtime bars (lights up after a MEASURE_MEM run)
                        if bench.memory.is_some() {
                            div {
                                class: "bench-section-desc bench-subhead",
                                "Memory \u{2014} peak resident set size"
                            }
                            {memory_bar_chart(bench, &data.languages)}
                        }

                        // Binary size — compiled-artifact footprint (lights up after a size run)
                        if bench.binary_sizes.is_some() {
                            div {
                                class: "bench-section-desc bench-subhead",
                                "Binary size \u{2014} compiled-artifact footprint"
                            }
                            {binary_size_bar_chart(bench, &data.languages)}
                            div { class: "bench-note", style: "margin-top:12px;",
                                "The size of the program you actually ship. C and C++ stay tiny because the runtime lives in the system libc; Rust and Go statically link their runtimes; LOGOS compiles to a compact self-contained binary in between. "
                                "Java\u{2019}s figure is its bytecode alone \u{2014} it still needs the JVM to run \u{2014} and JavaScript has no compiled artifact at all (its footprint is the V8 engine, in the Interpreter section). "
                                "As-built is the real shipped file; stripped removes debug symbols for a code-only comparison."
                            }
                        }

                        // Collapsible: Detailed Statistics
                        button {
                            class: "bench-collapsible-btn",
                            onclick: move |_| stats_open.set(!stats_open()),
                            span {
                                class: if stats_open() { "bench-collapsible-chevron open" } else { "bench-collapsible-chevron" },
                                "\u{25b6}"
                            }
                            "Detailed Statistics"
                        }
                        div {
                            class: if stats_open() { "bench-collapsible-body open" } else { "bench-collapsible-body" },
                            table { class: "bench-stats-table",
                                thead {
                                    tr {
                                        th { "Language" }
                                        th { "Mean" }
                                        th { "Median" }
                                        th { "StdDev" }
                                        th { "Min" }
                                        th { "Max" }
                                        th { "User" }
                                        th { "System" }
                                        th { "CV" }
                                        th { "Runs" }
                                    }
                                }
                                tbody {
                                    for (label, lid, maybe_t) in stats_entries.iter() {
                                        if let Some(t) = maybe_t {
                                            tr {
                                                class: if lid.starts_with("logos_") { "highlight" } else { "" },
                                                td { "{label}" }
                                                td { "{format_time(t.mean_ms)}" }
                                                td { "{format_time(t.median_ms)}" }
                                                td { "\u{00b1}{format_time(t.stddev_ms)}" }
                                                td { "{format_time(t.min_ms)}" }
                                                td { "{format_time(t.max_ms)}" }
                                                td { "{t.user_ms.map(|v| format_time(v)).unwrap_or_else(|| \"\u{2014}\".to_string())}" }
                                                td { "{t.system_ms.map(|v| format_time(v)).unwrap_or_else(|| \"\u{2014}\".to_string())}" }
                                                td { "{t.cv:.3}" }
                                                td { "{t.runs}" }
                                            }
                                        } else {
                                            {
                                                let timeout_str = ref_timed_out
                                                    .and_then(|m| m.get(*lid))
                                                    .map(|t| format_timeout(*t))
                                                    .unwrap_or_else(|| ">timeout".to_string());
                                                rsx! {
                                                    tr {
                                                        class: if lid.starts_with("logos_") { "highlight" } else { "" },
                                                        td { "{label}" }
                                                        td { "{timeout_str}" }
                                                        td { "{timeout_str}" }
                                                        td { "\u{2014}" }
                                                        td { "\u{2014}" }
                                                        td { "\u{2014}" }
                                                        td { "\u{2014}" }
                                                        td { "\u{2014}" }
                                                        td { "\u{2014}" }
                                                        td { "\u{2014}" }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // =============== INTERPRETER vs V8 ===============
                    div { class: "bench-section", id: "interpreter",
                        div { class: "bench-section-title", "LOGOS vs JavaScript / V8" }
                        div { class: "bench-section-desc",
                            "The LOGOS engine ladder \u{2014} bytecode VM, copy-and-patch JIT, and the warm AOT-native tier \u{2014} against {interp_node_ver} / V8."
                        }

                        div { class: "bench-summary", style: "margin-bottom: 20px;",
                            div { class: "bench-summary-card",
                                div { class: "bench-summary-value cyan", "{startup_vs_v8:.1}x" }
                                div { class: "bench-summary-label", "faster cold start than V8" }
                            }
                            div { class: "bench-summary-card",
                                div { class: "bench-summary-value cyan", "{startup_logos:.1}ms" }
                                div { class: "bench-summary-label", "LOGOS interpreter cold start" }
                            }
                            div { class: "bench-summary-card",
                                div { class: "bench-summary-value purple", "{interp_speed:.2}x" }
                                div { class: "bench-summary-label", "the speed of V8 on compute (geomean)" }
                            }
                            if aot_speed > 0.0 {
                                div { class: "bench-summary-card",
                                    div { class: "bench-summary-value cyan", "{aot_speed:.2}x" }
                                    div { class: "bench-summary-label", "the speed of V8, AOT-native warm (geomean)" }
                                }
                            }
                        }

                        // Cold-start chart (serverless / CLI win)
                        if !startup_bars.is_empty() {
                            div { class: "bench-chart-hint bench-chart-hint-strong", style: "margin-top: 4px;",
                                "Cold start \u{2014} launch the engine and run a trivial program ({interp.startup.runs} runs, shorter is faster)"
                            }
                            div { class: "bench-chart",
                                for (label, bar_class, mean, _is_logos) in startup_bars.iter() {
                                    {
                                        let pct = (*mean / startup_max * 100.0).min(100.0);
                                        let time_str = format_time(*mean);
                                        let show_inside = pct > 18.0;
                                        rsx! {
                                            div { class: "bench-bar-row",
                                                div { class: "bench-bar-label", "{label}" }
                                                div { class: "bench-bar-track",
                                                    div { class: "{bar_class}", style: "width: {pct:.1}%;",
                                                        if show_inside { span { class: "bench-bar-time", "{time_str}" } }
                                                    }
                                                }
                                                if !show_inside { span { class: "bench-bar-time-outside", "{time_str}" } }
                                            }
                                        }
                                    }
                                }
                            }
                            div { class: "bench-note", style: "margin-top:14px;margin-bottom:0;",
                                "A native binary has no VM to warm up, so the LOGOS interpreter reaches first output in "
                                strong { "{startup_logos:.1}ms" }
                                " versus V8\u{2019}s {startup_node:.0}ms, about "
                                strong { "{startup_vs_v8:.1}x quicker" }
                                ", which is what matters for short-lived work like cloud functions, CLI tools, and scripts. "
                                "On long-running loops V8\u{2019}s optimizing JIT pulls ahead: the interpreter is competitive on "
                                "memory-bound work and behind on heavy compute, a geometric mean of {interp_speed:.2}x the "
                                "speed of V8 across {interp_offfloor} benchmarks."
                            }
                        }

                        // Engine size — what you ship to run a program (benchmark-independent, like cold start)
                        if !engine_size_bars.is_empty() {
                            div { class: "bench-chart-hint bench-chart-hint-strong", style: "margin-top: 22px;",
                                "Engine size \u{2014} the runtime you ship to execute a program (shorter ships less; stripped, code-only size in parentheses)"
                            }
                            div { class: "bench-chart",
                                for (label, bar_class, bytes, stripped, _is_logos) in engine_size_bars.iter() {
                                    {
                                        let pct = (*bytes / engine_size_max * 100.0).min(100.0);
                                        let s = footprint_label(*bytes, *stripped);
                                        let show_inside = pct > 32.0;
                                        rsx! {
                                            div { class: "bench-bar-row",
                                                div { class: "bench-bar-label", "{label}" }
                                                div { class: "bench-bar-track",
                                                    div { class: "{bar_class}", style: "width: {pct:.1}%;",
                                                        if show_inside { span { class: "bench-bar-time", "{s}" } }
                                                    }
                                                }
                                                if !show_inside { span { class: "bench-bar-time-outside", "{s}" } }
                                            }
                                        }
                                    }
                                }
                            }
                            div { class: "bench-note", style: "margin-top:14px;",
                                if !wasm_bundle_str.is_empty() {
                                    "In the browser the whole LOGOS engine ships as a "
                                    strong { "{wasm_bundle_str}" }
                                    " WebAssembly bundle \u{2014} the same VM+JIT, no native install. "
                                }
                                "Node\u{2019}s binary bundles V8, libuv, and ICU; largo bundles the transpiler, bytecode VM, and copy-and-patch JIT \u{2014} each is the whole engine you ship to run a program. As-built is the real file; stripped removes debug symbols."
                            }
                        }

                        div { class: "bench-chart-hint bench-chart-hint-strong", style: "margin-top: 22px;",
                            "{bench.name} (engine: {interp_engine})"
                        }

                        if bench.id == "ackermann" {
                            div { class: "bench-callout",
                                span { class: "bench-callout-icon", "\u{26a1}" }
                                span { "Interpreted recursion is capped at the shared MAX_CALL_DEPTH (2,500 frames); ackermann\u{2019}s deep self-recursion blows past it, so the interpreter runs it at a reduced m. Deep recursion only completes in compiled mode, where the optimizer collapses it." }
                            }
                        }

                        if interp_bars.is_empty() {
                            div { class: "bench-chart-hint", "No interpreter result for {bench.name} (skipped, or not yet supported by the interpreter)." }
                        } else {
                            div { class: "bench-chart-hint",
                                "Wall-clock time at n = {interp_n} \u{2014} shorter is faster."
                                if let Some(s) = interp_active_speed {
                                    " The interpreter runs at {s:.2}x the speed of V8 here."
                                }
                            }
                            div { class: "bench-chart",
                                for (label, bar_class, median, _is_logos, _floored) in interp_bars.iter() {
                                    {
                                        let pct = (*median / interp_max * 100.0).min(100.0);
                                        let time_str = format_time(*median);
                                        let show_inside = pct > 15.0;
                                        rsx! {
                                            div { class: "bench-bar-row",
                                                div { class: "bench-bar-label", "{label}" }
                                                div { class: "bench-bar-track",
                                                    div {
                                                        class: "{bar_class}",
                                                        style: "width: {pct:.1}%;",
                                                        if show_inside {
                                                            span { class: "bench-bar-time", "{time_str}" }
                                                        }
                                                    }
                                                }
                                                if !show_inside {
                                                    span { class: "bench-bar-time-outside", "{time_str}" }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // =============== SOURCE CODE ===============
                    div { class: "bench-section", id: "source",
                        div { class: "bench-section-title", "Source Code" }
                        div { class: "bench-section-desc",
                            "The LOGOS source and the Rust it compiles to. Switch an optimization off and watch its \u{201c}## No <X>\u{201d} decorator appear on the LOGOS source — and the generated Rust recompile live in your browser. With every optimization on you see the cached release build; disabling them all yields plain, boring Rust."
                        }

                        // Master switch: all optimizations on (cached release Rust) ↔ all
                        // off (plain, un-optimized Rust). Both are instant.
                        div { class: "bench-opt-master",
                            label {
                                class: if all_opts_on { "bench-opt-toggle on" } else { "bench-opt-toggle off" },
                                input {
                                    r#type: "checkbox",
                                    checked: all_opts_on,
                                    onchange: move |_| {
                                        let n = logicaffeine_compile::optimization::REGISTRY.len();
                                        let v = if opt_toggles().iter().all(|&b| b) { vec![false; n] } else { vec![true; n] };
                                        opt_toggles.set(v);
                                        live_rust.set(None);
                                    },
                                }
                                span { "All Optimizations" }
                            }
                            span { class: "bench-opt-hint",
                                if base_fired().is_empty() {
                                    "analyzing which optimizations this program uses\u{2026}"
                                } else {
                                    "{base_fired().len()} of {logicaffeine_compile::optimization::REGISTRY.len()} optimizations fire for this program"
                                }
                            }
                        }

                        if all_opts_on {
                            div { class: "bench-backend-trace",
                                div { class: "bench-backend-trace-title",
                                    "Trace for {tab.fallback_label()}"
                                }
                                if optimizer_trace.is_empty()
                                    && lowering_trace.is_empty()
                                    && native_jit_trace.is_empty()
                                    && native_jit_compiled.is_empty()
                                    && native_jit_declines.is_empty()
                                    && native_jit_execution.is_empty()
                                    && decline_trace.is_empty()
                                    && fallback_trace.is_empty()
                                {
                                    div { class: "bench-backend-trace-empty",
                                        "No staged trace is baked for this result yet. The next benchmark run will populate it."
                                    }
                                } else {
                                    div { class: "bench-backend-trace-row",
                                        span { class: "bench-backend-trace-stage", "optimizer produced" }
                                        span { class: "bench-backend-trace-value",
                                            if optimizer_trace.is_empty() { "none" } else { "{optimizer_trace}" }
                                        }
                                    }
                                    div { class: "bench-backend-trace-row",
                                        span { class: "bench-backend-trace-stage", "backend lowered" }
                                        span { class: "bench-backend-trace-value",
                                            if lowering_trace.is_empty() { "none recorded" } else { "{lowering_trace}" }
                                        }
                                    }
                                    if !native_jit_compiled.is_empty() {
                                        div { class: "bench-backend-trace-row",
                                            span { class: "bench-backend-trace-stage", "JIT coverage" }
                                            span { class: "bench-backend-trace-value", "{native_jit_compiled}" }
                                        }
                                    }
                                    if !native_jit_trace.is_empty() {
                                        div { class: "bench-backend-trace-row",
                                            span { class: "bench-backend-trace-stage", "JIT native" }
                                            span { class: "bench-backend-trace-value", "{native_jit_trace}" }
                                        }
                                    }
                                    if !native_jit_execution.is_empty() {
                                        div { class: "bench-backend-trace-row",
                                            span { class: "bench-backend-trace-stage", "JIT executed" }
                                            span { class: "bench-backend-trace-value", "{native_jit_execution}" }
                                        }
                                    }
                                    if !native_jit_declines.is_empty() {
                                        div { class: "bench-backend-trace-row fallback",
                                            span { class: "bench-backend-trace-stage", "JIT declined" }
                                            span { class: "bench-backend-trace-value", "{native_jit_declines}" }
                                        }
                                    }
                                    if !decline_trace.is_empty() {
                                        div { class: "bench-backend-trace-row fallback",
                                            span { class: "bench-backend-trace-stage", "backend declined" }
                                            span { class: "bench-backend-trace-value", "{decline_trace}" }
                                        }
                                    }
                                    if !fallback_trace.is_empty() {
                                        div { class: "bench-backend-trace-row fallback",
                                            span { class: "bench-backend-trace-stage", "fallback" }
                                            span { class: "bench-backend-trace-value", "{fallback_trace}" }
                                        }
                                    }
                                }
                            }
                        }

                        // The optimization graph for this program, as a collapsible tree:
                        // every opt that fires, the `requires`-parents they depend on
                        // (an "enabler" that does not itself fire, dashed), and the opts
                        // that were SKIPPED because a higher-precedence one claimed them
                        // (a greyed "preempted" node — it fires if its winner is turned
                        // off). Nested by `requires` depth; turning a parent off cascades
                        // its children off, turning a child on pulls its parents on (the
                        // registry's own rule, via the compiler's `normalize`). Cyan =
                        // firing right now.
                        div { class: "bench-opt-toggles",
                            for (node, visible) in opt_tree.iter().zip(tree_visible.iter().copied()) {
                                if visible {
                                    {
                                        use logicaffeine_compile::optimization::OptRole;
                                        let opt = node.opt;
                                        let ri = opt as usize;
                                        let m = &logicaffeine_compile::optimization::REGISTRY[ri];
                                        let firing = fired_now().contains(&m.keyword);
                                        let needs = node.requires.iter().map(|o| o.meta().keyword).collect::<Vec<_>>().join(", ");
                                        let depends = node.depends_on.iter().map(|o| o.meta().keyword).collect::<Vec<_>>().join(", ");
                                        let blocks = node.preempts.iter().map(|o| o.meta().keyword).collect::<Vec<_>>().join(", ");
                                        let blocked_by = node.preempted_by.iter().map(|o| o.meta().keyword).collect::<Vec<_>>().join(", ");
                                        let blocks_now = preempted_now().iter()
                                            .filter(|(w, _)| *w == m.keyword)
                                            .map(|(_, l)| *l)
                                            .collect::<Vec<_>>()
                                            .join(", ");
                                        let cls = if !opt_tog[ri] { "bench-opt-toggle off" }
                                                  else if firing { "bench-opt-toggle on firing" }
                                                  else { match node.role {
                                                      OptRole::Preempted => "bench-opt-toggle on preempted",
                                                      OptRole::Enabler => "bench-opt-toggle on enabling",
                                                      OptRole::Fired => "bench-opt-toggle on",
                                                  } };
                                        let indent = node.depth * 22;
                                        let has_children = node.has_children;
                                        let is_expanded = expanded().get(ri).copied().unwrap_or(true);
                                        let chevron_cls = if is_expanded { "bench-tree-chevron open" } else { "bench-tree-chevron" };
                                        rsx! {
                                            div { class: "bench-tree-row", style: "padding-left: {indent}px;",
                                                if has_children {
                                                    span {
                                                        class: "{chevron_cls}",
                                                        onclick: move |_| {
                                                            let mut e = expanded();
                                                            if ri < e.len() { e[ri] = !e[ri]; }
                                                            expanded.set(e);
                                                        },
                                                        "\u{25b6}"
                                                    }
                                                } else {
                                                    span { class: "bench-tree-spacer" }
                                                }
                                                label { class: "{cls}", title: "{m.group}",
                                                    input { r#type: "checkbox", checked: opt_tog[ri],
                                                        onchange: move |_| {
                                                            let toggles = opt_toggles();
                                                            let turning_on = !toggles.get(ri).copied().unwrap_or(false);
                                                            let mut cfg = config_from_toggles(&toggles);
                                                            if turning_on {
                                                                cfg.enable_with_requires(opt);
                                                            } else {
                                                                cfg.set(opt, false);
                                                            }
                                                            cfg.normalize();
                                                            opt_toggles.set(toggles_from_config(&cfg));
                                                            live_rust.set(None);
                                                        },
                                                    }
                                                    span { "{m.label}" }
                                                    if node.role == OptRole::Enabler {
                                                        span { class: "bench-opt-rel enables", "enabler" }
                                                    }
                                                    if node.role == OptRole::Preempted && !blocked_by.is_empty() {
                                                        span { class: "bench-opt-rel beaten", "blocked by {blocked_by} \u{00b7} fires if off" }
                                                    }
                                                    if !needs.is_empty() {
                                                        span { class: "bench-opt-rel needs", "needs {needs}" }
                                                    }
                                                    if !depends.is_empty() {
                                                        span { class: "bench-opt-rel depends", "depends on {depends}" }
                                                    }
                                                    if !blocks.is_empty() {
                                                        span { class: "bench-opt-rel beats", "blocks {blocks}" }
                                                    }
                                                    if !blocks_now.is_empty() {
                                                        span { class: "bench-opt-rel beats-now", "blocks {blocks_now} now" }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // LOGOS (with the toggles applied) beside what each tier makes of it.
                        div { class: "bench-source",
                            div { class: "bench-source-panel",
                                div { class: "bench-source-header logos", "LOGOS" }
                                div { class: "bench-source-code", "{opt_decorated}" }
                            }
                            div { class: "bench-source-panel",
                                div { class: "bench-source-header rust bench-source-tabs", role: "tablist",
                                    for (t, label) in code_tabs.iter().cloned() {
                                        button {
                                            key: "{label}",
                                            class: if code_tab() == t { "bench-source-switch active" } else { "bench-source-switch" },
                                            role: "tab",
                                            "aria-selected": "{code_tab() == t}",
                                            onclick: move |_| code_tab.set(t),
                                            "{label}"
                                        }
                                    }
                                }
                                {code_body}
                            }
                        }

                        // Collapsible language source sections
                        for (idx, lang_id) in source_langs.iter().enumerate() {
                            {
                                let src = get_source(bench_sources, lang_id);
                                let label = lang_label(lang_id);
                                let version = data.metadata.versions.get(*lang_id).map(|s| s.as_str()).unwrap_or("");
                                let ext = lang_ext(lang_id);
                                let file_url = format!("{}/blob/main/benchmarks/programs/{}/{}", GITHUB_REPO, bench.id, ext);
                                let is_open = source_open()[idx];
                                rsx! {
                                    div { class: "bench-lang-collapsible",
                                        div {
                                            class: "bench-lang-header",
                                            onclick: move |_| {
                                                let mut arr = source_open();
                                                arr[idx] = !arr[idx];
                                                source_open.set(arr);
                                            },
                                            span { class: swatch_class(lang_id) }
                                            span { class: "bench-lang-name", "{label}" }
                                            span { class: "bench-lang-version", "{version}" }
                                            a {
                                                class: "bench-lang-link",
                                                href: "{file_url}",
                                                target: "_blank",
                                                onclick: move |e: Event<MouseData>| e.stop_propagation(),
                                                "View on GitHub \u{2192}"
                                            }
                                            span {
                                                class: if is_open { "bench-collapsible-chevron open" } else { "bench-collapsible-chevron" },
                                                "\u{25b6}"
                                            }
                                        }
                                        div {
                                            class: if is_open { "bench-lang-code open" } else { "bench-lang-code" },
                                            "{src}"
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // =============== COMPILATION ===============
                    div { class: "bench-section", id: "compilation",
                        div { class: "bench-section-title", "Compilation Times" }
                        div { class: "bench-section-desc",
                            "Time to compile each benchmark from source, with the same flags the timed builds use. "
                            "LOGOS compiles through Rust or through C, so its bars include rustc or the C compiler at full optimization."
                        }

                        div { class: "bench-chart",
                            for (name, mean, _stddev, is_largo) in compile_entries.iter() {
                                {
                                    let pct = (*mean / compile_max * 100.0).min(100.0);
                                    let time_str = format_time(*mean);
                                    let show_inside = pct > 15.0;
                                    let bar_class = match *name {
                                        "logos_release" => "bench-bar-fill bench-bar-logos-rust",
                                        "logos_c" => "bench-bar-fill bench-bar-logos-c",
                                        "logos_cbc" => "bench-bar-fill bench-bar-logos-cbc",
                                        _ if *is_largo => "bench-bar-fill bench-bar-logos-jit",
                                        _ => "bench-bar-fill bench-bar-other",
                                    };
                                    let display_name = data.metadata.compiler_label(name);
                                    rsx! {
                                        div { class: "bench-bar-row",
                                            div { class: "bench-bar-label", "{display_name}" }
                                            div { class: "bench-bar-track",
                                                div {
                                                    class: "{bar_class}",
                                                    style: "width: {pct:.1}%;",
                                                    if show_inside {
                                                        span { class: "bench-bar-time", "{time_str}" }
                                                    }
                                                }
                                            }
                                            if !show_inside {
                                                span { class: "bench-bar-time-outside", "{time_str}" }
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // Collapsible: Detailed Compilation Data
                        button {
                            class: "bench-collapsible-btn",
                            onclick: move |_| compile_detail_open.set(!compile_detail_open()),
                            span {
                                class: if compile_detail_open() { "bench-collapsible-chevron open" } else { "bench-collapsible-chevron" },
                                "\u{25b6}"
                            }
                            "Detailed Compilation Data"
                        }
                        div {
                            class: if compile_detail_open() { "bench-collapsible-body open" } else { "bench-collapsible-body" },
                            table { class: "bench-compile-table",
                                thead {
                                    tr {
                                        th { "Compiler" }
                                        th { "Mean" }
                                        th { "StdDev" }
                                    }
                                }
                                tbody {
                                    for (name, mean, stddev, is_largo) in compile_entries.iter() {
                                        tr {
                                            class: if *is_largo { "highlight" } else { "" },
                                            td { class: "compiler-name", "{data.metadata.compiler_label(name)}" }
                                            td { "{format_time(*mean)}" }
                                            td { "\u{00b1}{format_time(*stddev)}" }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // =============== SUMMARY ===============
                    div { class: "bench-section", id: "summary",
                        div { class: "bench-section-title", "Cross-Benchmark Summary" }
                        div { class: "bench-section-desc", "Geometric-mean speed vs C across all {data.benchmarks.len()} benchmarks (bars proportional to speed; higher is faster)." }

                        BarChart { groups: summary_groups(data), unit: BarUnit::SpeedOfC }
                    }

                    // =============== METHODOLOGY ===============
                    div { class: "bench-section", id: "methodology",
                        button {
                            class: "bench-collapsible-btn bench-methodology-toggle",
                            onclick: move |_| methodology_open.set(!methodology_open()),
                            span {
                                class: if methodology_open() { "bench-collapsible-chevron open" } else { "bench-collapsible-chevron" },
                                "\u{25b6}"
                            }
                            span { class: "bench-methodology-title", "Methodology" }
                        }
                        div {
                            class: if methodology_open() { "bench-collapsible-body open" } else { "bench-collapsible-body" },
                            div { class: "bench-methodology",
                                ul {
                                    li { "Each benchmark measured with hyperfine ({data.metadata.runs.unwrap_or(20)} runs, {data.metadata.warmup.unwrap_or(5)} warmup), pinned to the performance cores on a machine checked quiet first; the bars show the median, and every ratio and geometric mean uses the mean." }
                                    li { "CPU: {data.metadata.cpu}." }
                                    li { "OS: {data.metadata.os}." }
                                    li { "Every implementation runs the same algorithm, with its sorts and hash tables hand-rolled in every language, and produces identical output, verified byte for byte before anything is timed." }
                                    li { "Every compiled language is built at its fastest release setting for this CPU, with the commands below; C, C++ and Nim compile with floating-point contraction off, as every other language rounds each operation." }
                                    li { "With the algorithm unchanged, the LOGOS compiler also chooses storage the other programs spell out: an Int it proves fits in 32 bits is stored in 32 bits, a local Seq of Bool is held one bit per item, and the remainder of a value proven below twice its divisor is one subtraction. These count as the same algorithm." }
                                    li { "Two geometric means are reported vs C: the headline keeps all {data.benchmarks.len()} benchmarks; the apples-to-apples figure removes the {collapse_count} where the LOGOS compiler collapses the algorithm (\u{26a1}). Every collapse is auditable in the generated Rust per benchmark." }
                                    li { "The Interpreter-vs-V8 section is a separate peer comparison (LOGOS bytecode VM + JIT against Node/V8) at interpreter-calibrated sizes, so its n differs from the compiled section." }
                                }

                                h3 { "Compiler Versions" }
                                table { class: "bench-version-table",
                                    thead {
                                        tr {
                                            th { "Language" }
                                            th { "Version" }
                                        }
                                    }
                                    tbody {
                                        for (lang_id, version) in data.metadata.versions.iter() {
                                            tr {
                                                td { "{lang_label(lang_id)}" }
                                                td { "{version}" }
                                            }
                                        }
                                    }
                                }

                                h3 { "Build commands" }
                                table { class: "bench-version-table",
                                    thead {
                                        tr {
                                            th { "Language" }
                                            th { "Built and run with" }
                                        }
                                    }
                                    tbody {
                                        for lang in data.languages.iter() {
                                            if let Some(command) = data.metadata.build_commands.get(&lang.id) {
                                                tr {
                                                    td { "{lang.label}" }
                                                    td { code { "{command}" } }
                                                }
                                            }
                                        }
                                    }
                                }

                                h3 { "Links" }
                                ul {
                                    li {
                                        a { href: "{runsh_url}", target: "_blank", "benchmarks/run.sh" }
                                        " \u{2014} benchmark runner script"
                                    }
                                    li {
                                        a { href: "{raw_json_url}", target: "_blank", "results/latest.json" }
                                        " \u{2014} raw benchmark data"
                                    }
                                    li {
                                        a { href: "{history_url}", target: "_blank", "results/history/" }
                                        " \u{2014} historical results by version"
                                    }
                                    li {
                                        a { href: "{bench_dir_url}", target: "_blank", "benchmarks/" }
                                        " \u{2014} all benchmark source code"
                                    }
                                    li {
                                        a { href: "/data/flamegraphs.pdf", download: "logos-flamegraphs.pdf", "profile/flamegraphs.pdf" }
                                        " \u{2014} CPU-profile flamegraph breakdowns ({data.benchmarks.len()} benchmarks, LOGOS vs C)"
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if bench_tab() == BenchTab::Solver {
                div { class: "bench-content",
                    SolverSection {}
                    EdaSection {}
                }
            }
            if bench_tab() == BenchTab::Codec {
                div { class: "bench-content",
                    CodecSection {}
                }
            }

            Footer {}
        }
    }
}
