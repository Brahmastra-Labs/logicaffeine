//! ★ RATCHET — what the retained-tree reconciler costs a page to ship (Wave 7, D7). ★
//!
//! Every lowered page draws through `vdom.lg`, compiled into the page's own module: the tape it
//! writes, the keyed diff, the patch stream, hydration. That code is paid once per page, and this
//! measures it three ways, in the bytes a reader downloads — the module raw and gzipped exactly as
//! `largo build` precompresses it (`logicaffeine_cli::gzip_asset`):
//!
//! - the counter page, the canonical first page, whole, and the smallest display page (state, a
//!   hole, nesting, an attribute) — the common path a regression would hide in;
//! - the reconciler alone: a program that draws one element through `vdom.lg` and nothing else,
//!   over a program that draws nothing;
//! - the reconciler is paid ONCE: five components cost their own code over one, never another copy
//!   of the reconciler.
//!
//! The ceilings are the measured numbers with a margin for ordinary growth, and they are the
//! numbers the bundle budget is based on; a change that crosses one has changed what every page
//! costs, and says so here first.

#![cfg(feature = "wasm-jit")]

use logicaffeine_compile::compile::compile_to_wasm;

const COUNTER: &str = r#"## Page /

State:
    Let n be 0.
Actions:
    To bump:
        Set n to n + 1.
View:
    <main>
      <p id="out">Count: {n}</p>
      <button id="go" on-click={bump}>go</button>
    </main>
"#;

/// The smallest page that shows state: a hole, nesting, an attribute, no action.
const DISPLAY: &str = "## Component App\nState:\n    Let name be \"ada\".\nView:\n    <div class=\"card\"><h1>Hi {name}</h1><p>welcome</p></div>\n\n## Main\nMount App.\n";

/// A program that draws nothing: what every module pays.
const PLAIN: &str = "## Main\nShow 1 + 1.\n";

/// One element drawn, committed and applied through `vdom.lg` directly — the reconciler and
/// nothing a page adds to it.
const RECONCILER: &str = "## Main\nLet root be vdNewRoot(0).\nvdOpen(\"p\").\nvdText(\"x\").\nvdClose().\nShow vdCommit(root).\nShow vdApply().\n";

fn module(src: &str) -> Vec<u8> {
    compile_to_wasm(src).unwrap_or_else(|e| panic!("it must compile to wasm: {e:?}\n{src}"))
}

/// `(raw, shipped)` bytes of `src`'s module: the second is what a build serves, its gzipped form, or
/// the module itself when gzip would not save enough to be worth a second file.
fn shipped(src: &str) -> (usize, usize) {
    let raw = module(src);
    let served = logicaffeine_cli::gzip_asset(&raw).map_or(raw.len(), |gz| gz.len());
    (raw.len(), served)
}

/// Measured at exact `Int` semantics: every integer the shared exactness analysis cannot bound is a tagged
/// word, with its tagging, release and narrowing outlined outside loops and batch roots synchronized only
/// where a safepoint can run. The same three modules with no integer exact are 80,058 for the counter,
/// 62,935 for the display page and 57,946 for the reconciler, so exactness is what each pays over that;
/// proving `vdom.lg`'s indices, lengths and counters bounded is what brings them back down. Before
/// exactness, when the tape lowering and hydration landed (7.5+7.6): 80,923 / 29,439, 63,181 / 22,931 and
/// 50,597 / 18,505; before the backend's outlined checked arithmetic and stackified locals: 91,556 /
/// 35,486 for the counter, 58,402 / 23,172 for the reconciler; before the pruning as well: 103,690 /
/// 40,385 and 72,423 / 29,106.
const COUNTER_RAW: usize = 137_782;
const COUNTER_GZIP: usize = 45_211;
const RECONCILER_RAW: usize = 101_743;
const RECONCILER_GZIP: usize = 32_047;
const DISPLAY_RAW: usize = 109_652;
const DISPLAY_GZIP: usize = 34_915;

/// Room for ordinary growth over a measured number: a tenth.
fn ceiling(measured: usize) -> usize {
    measured + measured / 10
}

#[test]
fn the_counter_page_ships_within_its_measured_budget() {
    let (raw, gz) = shipped(COUNTER);
    assert!(
        raw <= ceiling(COUNTER_RAW) && gz <= ceiling(COUNTER_GZIP),
        "the counter page ships {raw} bytes ({gz} gzipped), over the ceiling of {} ({}) — measured \
         {COUNTER_RAW} ({COUNTER_GZIP}). A page's first download grew by more than a tenth: find what \
         it drags in before moving the number.",
        ceiling(COUNTER_RAW),
        ceiling(COUNTER_GZIP)
    );
}

#[test]
fn the_display_page_ships_within_its_measured_budget() {
    let (raw, gz) = shipped(DISPLAY);
    assert!(
        raw <= ceiling(DISPLAY_RAW) && gz <= ceiling(DISPLAY_GZIP),
        "the display page ships {raw} bytes ({gz} gzipped), over the ceiling of {} ({}) — measured \
         {DISPLAY_RAW} ({DISPLAY_GZIP}). The common path grew by more than a tenth.",
        ceiling(DISPLAY_RAW),
        ceiling(DISPLAY_GZIP)
    );
}

#[test]
fn the_reconciler_alone_costs_what_it_measured() {
    let (plain_raw, plain_gz) = shipped(PLAIN);
    let (rec_raw, rec_gz) = shipped(RECONCILER);
    let (raw, gz) = (rec_raw - plain_raw, rec_gz.saturating_sub(plain_gz));
    assert!(
        raw <= ceiling(RECONCILER_RAW) && gz <= ceiling(RECONCILER_GZIP),
        "vdom.lg adds {raw} bytes ({gz} gzipped) to a program that draws nothing, over the ceiling of \
         {} ({}) — measured {RECONCILER_RAW} ({RECONCILER_GZIP}).",
        ceiling(RECONCILER_RAW),
        ceiling(RECONCILER_GZIP)
    );
}

#[test]
fn the_reconciler_is_paid_once_not_per_component() {
    let components = |n: usize| {
        let mut src = String::new();
        let mut placed = String::new();
        for i in 0..n {
            src.push_str(&format!(
                "## Component Part{i}\nState:\n    Let v be {i}.\nActions:\n    To bump:\n        Set v to v + 1.\nView:\n    <p id=\"p{i}\" on-click={{bump}}>{{v}}</p>\n\n"
            ));
            placed.push_str(&format!("<Part{i}/>"));
        }
        src.push_str(&format!("## Page /\nView:\n    <main>{placed}</main>\n"));
        src
    };
    let one = module(&components(1)).len();
    let five = module(&components(5)).len();
    let (rec_raw, plain_raw) = (module(RECONCILER).len(), module(PLAIN).len());
    let reconciler = rec_raw - plain_raw;
    let per_component = (five - one) / 4;
    assert!(
        per_component < reconciler / 4,
        "each component past the first costs {per_component} bytes, a quarter of the reconciler's \
         {reconciler} or more: the page is paying for drawing per component, not once"
    );
}

// ---- A page carries a `vdom.lg` feature only when it writes the feature's records. ----

/// The names of the functions `src`'s module can run for a host with `entries` — the analysis the
/// wasm backend keeps code by (`vm::reach`).
fn reached(
    src: &str,
    entries: &logicaffeine_compile::vm::reach::HostEntries,
) -> std::collections::BTreeSet<String> {
    let concrete = logicaffeine_compile::monomorphize::Options {
        lower_option: false,
    };
    logicaffeine_compile::ui_bridge::with_monomorphic_program_checked(
        src,
        concrete,
        |parsed, interner| {
            let (stmts, types, _) = parsed.unwrap_or_else(|e| panic!("{e:?}\n{src}"));
            let oracle = logicaffeine_compile::optimize::oracle_analyze_with(stmts, interner);
            let mut program = logicaffeine_compile::vm::Compiler::compile_with_oracle_located(
                stmts,
                interner,
                Some(types),
                Some(oracle),
            )?;
            logicaffeine_compile::vm::peephole::peephole(&mut program);
            let layout = logicaffeine_compile::vm::reach::code_layout(&program)
                .expect("a resolvable layout");
            let reach =
                logicaffeine_compile::vm::reach::reach_with(&program, &layout, interner, entries);
            Ok(program
                .functions
                .iter()
                .enumerate()
                .filter(|(fi, _)| reach.functions[*fi])
                .map(|(_, f)| interner.resolve(f.name).to_string())
                .collect())
        },
    )
    .unwrap_or_else(|e| panic!("{e:?}\n{src}"))
}

/// Whether any reached function is named for `prefix`'s feature.
fn carries(reached: &std::collections::BTreeSet<String>, prefix: &str) -> bool {
    reached.iter().any(|n| n.starts_with(prefix))
}

const PLAIN_PAGE: &str = "## Page /\nState:\n    Let n be 0.\nActions:\n    To bump:\n        Set n to n + 1.\nView:\n    <main><p>{n}</p><button on-click={bump}>go</button></main>\n";

const KEYED_PAGE: &str = "## Page /\nState:\n    Let items be [\"a\", \"b\"].\nView:\n    <ul>{for each x in items, keyed by x:}<li>{x}</li>{end}</ul>\n";

const WRAPPER_PAGE: &str = "## Component Card\nView:\n    <section>{children}</section>\n\n## Page /\nView:\n    <main><Card><p>inside</p></Card></main>\n";

#[test]
fn a_page_carries_each_vdom_feature_only_when_it_writes_its_records() {
    use logicaffeine_compile::vm::reach::HostEntries;
    let shipped = HostEntries::default();
    let plain = reached(PLAIN_PAGE, &shipped);
    assert!(
        plain.contains("vdCommit") && plain.contains("vdOpen"),
        "the census reads the page's reach: {plain:?}"
    );
    assert!(
        !carries(&plain, "vdKeys") && !carries(&plain, "vdKeyed"),
        "no key is written, so no key is matched: {plain:?}"
    );
    assert!(
        !carries(&plain, "vdSlots"),
        "no range is written, so none is resolved: {plain:?}"
    );
    assert!(
        carries(&plain, "vdAdopt"),
        "a page its build may pre-render adopts: {plain:?}"
    );

    let counter = reached(COUNTER, &shipped);
    assert!(
        carries(&counter, "vdKeys"),
        "a literal id is a key its element is matched by"
    );
    assert!(
        !carries(&counter, "vdKeyed"),
        "and not a key whose repeat is refused"
    );

    let keyed = reached(KEYED_PAGE, &shipped);
    assert!(
        carries(&keyed, "vdKeyed") && carries(&keyed, "vdKeys"),
        "`keyed by` writes firm keys: {keyed:?}"
    );

    let wrapped = reached(WRAPPER_PAGE, &shipped);
    assert!(
        carries(&wrapped, "vdSlots"),
        "`{{children}}` writes a range and its slot: {wrapped:?}"
    );

    let fresh = reached(PLAIN_PAGE, &HostEntries::default().drawing_fresh());
    assert!(
        !carries(&fresh, "vdAdopt") && !fresh.contains("vdHydrate"),
        "a page served without markup never hydrates: {fresh:?}"
    );
}

#[test]
fn a_program_that_places_no_component_carries_no_unmount_sweep() {
    use logicaffeine_compile::vm::reach::HostEntries;
    let gone = logicaffeine_language::parser::lgx_lower::gone_fn_name();
    let plain = reached(PLAIN_PAGE, &HostEntries::default());
    assert!(
        !plain.contains(gone),
        "nothing is placed, so no instance can leave: {plain:?}"
    );
    let wrapped = reached(WRAPPER_PAGE, &HostEntries::default());
    assert!(
        wrapped.contains(gone),
        "a placed Card can leave, so its sweep ships: {wrapped:?}"
    );
}

#[test]
fn a_page_served_without_markup_ships_a_smaller_module() {
    use logicaffeine_compile::vm::reach::HostEntries;
    let hydrating =
        logicaffeine_compile::compile::compile_to_wasm_with(PLAIN_PAGE, &HostEntries::default())
            .unwrap();
    let fresh = logicaffeine_compile::compile::compile_to_wasm_with(
        PLAIN_PAGE,
        &HostEntries::default().drawing_fresh(),
    )
    .unwrap();
    assert!(
        fresh.len() < hydrating.len(),
        "leaving hydration out shrinks the module: {} vs {}",
        fresh.len(),
        hydrating.len()
    );
}
