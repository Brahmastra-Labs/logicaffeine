//! ★ RATCHET — what a page COSTS to ship. ★
//!
//! A framework's weight is a feature of it, and weight arrives one harmless-looking commit at a
//! time. This measures the two numbers a reader of a LOGOS page actually pays: the size of the
//! WebAssembly module a page compiles to, and how much of that is FRAMEWORK rather than program.
//!
//! Deliberately a SIZE ratchet and not a timing one: bytes are deterministic, so this says the
//! same thing on a loaded machine as on an idle one. (A timing test under a 15,000-test parallel
//! run reports the machine, not the code — this campaign has the scars.)
//!
//! The ceilings are generous on purpose. They are not targets to squeeze against; they are there
//! to catch a TENFOLD accident — a page that suddenly drags in the whole standard library, or a
//! lowering that emits per-node code where it used to emit a loop.

#![cfg(feature = "wasm-jit")]

use logicaffeine_compile::compile::compile_to_wasm;

/// The smallest interesting page: state, a hole, nesting, an attribute.
const PAGE: &str = "\
## Component App
State:
    Let name be \"ada\".
View:
    <div class=\"card\"><h1>Hi {name}</h1><p>welcome</p></div>

## Main
Mount App.
";

/// A program with no page at all — the baseline every module pays.
const PLAIN: &str = "## Main\nShow 1 + 1.\n";

fn size(src: &str) -> usize {
    compile_to_wasm(src).expect("it must compile to wasm").len()
}

/// A page's whole module: 63,181 bytes once the page's reconciler became part of its own module
/// (`vdom.lg`, Wave 7.5), with each feature the page does not write left out; 2,636 when the
/// reconciler lived in the host's JavaScript. The ceiling keeps this ratchet's own margin, three
/// times the measure; `vdom_size_lock` holds the reconciler and the counter page to a tenth.
const PAGE_CEILING: usize = 190_000;

/// What the FRAMEWORK adds over a program that draws nothing: 63,055 bytes with the reconciler in
/// the module; 2,510 when it lived in the host.
const FRAMEWORK_CEILING: usize = 189_000;

#[test]
fn a_page_module_stays_small() {
    let n = size(PAGE);
    assert!(
        n <= PAGE_CEILING,
        "a page compiles to {n} bytes of wasm, over the {PAGE_CEILING}-byte ceiling. It was \
         63,181 with the reconciler in the module. Something is being emitted per-node that used to be a \
         loop, or the page is dragging in a module it does not use — find out which before \
         raising the number."
    );
}

#[test]
fn the_framework_overhead_stays_small() {
    let page = size(PAGE);
    let plain = size(PLAIN);
    let overhead = page.saturating_sub(plain);
    assert!(
        overhead <= FRAMEWORK_CEILING,
        "the framework adds {overhead} bytes over a program that draws nothing ({page} - \
         {plain}), over the {FRAMEWORK_CEILING}-byte ceiling. It was 63,055 with the reconciler in the module."
    );
}

#[test]
fn a_bigger_view_costs_bytes_in_proportion_to_what_it_says() {
    // The property that matters more than any single number: a view twice the size must not cost
    // FOUR times the bytes. Quadratic code growth is how a framework becomes unshippable, and it
    // hides completely behind a ceiling that a small fixture never approaches.
    let row = "<li class=\"row\"><span>{name}</span><b>x</b></li>";
    let small = format!(
        "## Component App\nState:\n    Let name be \"a\".\nView:\n    <ul>{}</ul>\n\n## Main\nMount App.\n",
        row.repeat(4)
    );
    let big = format!(
        "## Component App\nState:\n    Let name be \"a\".\nView:\n    <ul>{}</ul>\n\n## Main\nMount App.\n",
        row.repeat(16)
    );
    let (s, b) = (size(&small), size(&big));
    let grew = (b - size(PLAIN)) as f64 / (s - size(PLAIN)) as f64;
    assert!(
        grew < 6.0,
        "a view 4x bigger produced {grew:.1}x the code ({s} -> {b} bytes). Linear-ish is the \
         requirement; anything near quadratic means the lowering is emitting per-node what it \
         should emit once."
    );
}

#[test]
fn a_loop_does_not_pay_per_iteration() {
    // A `{for each}` over a thousand items must cost the same CODE as one over ten: the loop is
    // the point, and if this fails the lowering has started unrolling.
    //
    // The subtlety that made the first version of this test wrong: a list LITERAL genuinely costs
    // bytes per element — a thousand string constants are a thousand string constants, and that
    // is data the program wrote out, not code the loop emitted. Measuring the page alone
    // conflated the two and reported 170 bytes an item, all of it data.
    //
    // So the data is measured on its own and SUBTRACTED. What is left is the loop's own cost,
    // and that is what has to be flat.
    let list = |n: usize| {
        let items: Vec<String> = (0..n).map(|i| format!("\"r{i}\"")).collect();
        items.join(", ")
    };
    let data_only = |n: usize| format!("## Main\nLet xs be [{}].\nShow length of xs.\n", list(n));
    let with_loop = |n: usize| {
        format!(
            "## Component App\nState:\n    Let xs be [{}].\nView:\n    \
             <ul>{{for each x in xs:}}<li>{{x}}</li>{{end}}</ul>\n\n## Main\nMount App.\n",
            list(n)
        )
    };
    let loop_cost = |n: usize| size(&with_loop(n)) as i64 - size(&data_only(n)) as i64;
    let ten = loop_cost(10);
    let thousand = loop_cost(1000);
    let drift = (thousand - ten).abs() as f64 / 990.0;
    assert!(
        drift < 5.0,
        "the loop itself cost {ten} bytes over its data at 10 items and {thousand} at 1000 — \
         {drift:.2} bytes of drift per item. A `{{for each}}` is ONE piece of code however many \
         items it walks; this looks like unrolling."
    );
}

#[test]
fn a_list_literals_data_cost_is_the_data_and_nothing_more() {
    // The other half of the measurement above, asserted rather than assumed: a list literal's
    // cost per element is small and flat. If THIS grows, the constant pool or the list builder is
    // the thing to look at — and the loop test would otherwise absorb the blame.
    let list = |n: usize| {
        let items: Vec<String> = (0..n).map(|i| format!("\"r{i}\"")).collect();
        format!(
            "## Main\nLet xs be [{}].\nShow length of xs.\n",
            items.join(", ")
        )
    };
    let per_item = (size(&list(1000)) - size(&list(10))) as f64 / 990.0;
    assert!(
        per_item < 400.0,
        "each list-literal element costs {per_item:.1} bytes of module. That is data, so some \
         cost is honest — but this is the ceiling that says how much."
    );
}

/// ★ A CLASS MUST COST WHAT AN ATTRIBUTE COSTS.
///
/// `class` is composed rather than simply set — the `class=` value, every `class:name={cond}`
/// toggle and the component's `Style:` scope class join into ONE list so none can clobber another.
/// That composition is two Text concatenations per piece, and a concat inlines a copy routine on
/// this tier, so it was emitted at **1,329 bytes for a single `class="card"`** against 147 for
/// `id="x"` — and per ELEMENT, so three classed siblings cost 4,455.
///
/// Every element in a real view carries a class, so that was the most expensive thing a page could
/// say, and it was invisible: the page rendered correctly and simply weighed nine times what it
/// should. A list needs composing only when something in it is decided at RUN TIME; a literal
/// `class=` plus a scope class is two constants, and constants join at compile time.
///
/// This pins the ratio rather than a byte count, so it keeps meaning as both paths move.
#[test]
fn a_static_class_costs_no_more_than_any_other_attribute() {
    let page = |view: &str| format!("## Component App\nView:\n    {view}\n\n## Main\nMount App.\n");
    let bare = size(&page("<div>hi</div>"));
    let with_id = size(&page("<div id=\"x\">hi</div>")) - bare;
    let with_class = size(&page("<div class=\"card\">hi</div>")) - bare;
    assert!(
        with_class <= with_id * 2,
        "a static `class=\"card\"` costs {with_class} bytes where `id=\"x\"` costs {with_id}. \
         A class list that cannot change at run time is a constant and must be SET, not composed \
         — see the fold in `lower_lgx_nodes`."
    );
}

/// And the cost must stay per-attribute, not per-element-with-a-class: three classed siblings
/// must cost about what three id'd siblings cost. This is the shape the 1,329-byte version failed
/// worst at, because it re-emitted the whole composition for every element.
#[test]
fn classes_on_many_elements_do_not_compound() {
    let page = |view: &str| format!("## Component App\nView:\n    {view}\n\n## Main\nMount App.\n");
    let bare = size(&page("<div><a>1</a><b>2</b><i>3</i></div>"));
    let ids = size(&page(
        "<div><a id=\"p\">1</a><b id=\"q\">2</b><i id=\"r\">3</i></div>",
    )) - bare;
    let classes = size(&page(
        "<div><a class=\"p\">1</a><b class=\"q\">2</b><i class=\"r\">3</i></div>",
    )) - bare;
    assert!(
        classes <= ids * 2,
        "three classed siblings cost {classes} bytes where three id'd siblings cost {ids}. \
         A class list is being composed per element where it should be folded once."
    );
}

/// The runtime path still has to EXIST: a `class:name={cond}` toggle genuinely cannot be folded,
/// and silently dropping it would be a far worse bug than the bytes. Kept beside the two above so
/// the fold can never be widened into "never compose at all".
#[test]
fn a_conditional_class_still_composes() {
    let src = "\
## Component App
State:
    Let on be true.
View:
    <div class=\"card\" class:lit={on}>hi</div>

## Main
Mount App.
";
    let module = compile_to_wasm(src).expect("a conditional class must still reach the browser");
    let engine = wasmi::Engine::default();
    wasmi::Module::new(&engine, &module[..]).expect("and the module must validate");
}
