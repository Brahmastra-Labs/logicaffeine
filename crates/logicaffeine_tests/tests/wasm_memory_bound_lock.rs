//! ★ LOCK — a page survives being USED. ★
//!
//! Every repaint allocates: the text of each hole, each attribute value, the composed class list,
//! the scoped stylesheet. The browser tier allocates them in a bump-pointer heap that never gives
//! anything back, so the question this file asks is whether a page's memory stays bounded enough to
//! be left open.
//!
//! ★★★ THIS FILE ONCE RECORDED A CATASTROPHE THAT DID NOT EXIST, and the correction is worth more
//! than the tests. It said a page traps with `MemoryOutOfBounds` after TWELVE clicks, measured, with
//! a doubling curve to prove it. Every number was real. The cause was one line — IN THIS FILE:
//!
//!     l.func_wrap("env", "fmt_i64_into", |_c, b: i32, _v: i64| -> i32 { b })
//!
//! `fmt_i64_into(buf, val) -> i32` returns THE BYTE LENGTH WRITTEN. That stub returned `buf`, the
//! POINTER. The pointer grows with the heap; it was stored as the Text's `len`; the next concat
//! allocated `len_a + len_b` — a buffer the size of the whole heap. Hence exact doubling, hence
//! twelve clicks. The product was never broken.
//!
//! ★ WHAT IT COST, which is the reason this is written down: a false diagnosis is not a neutral
//! absence of knowledge, it is a confident wrong belief that spends real work. It produced a task
//! (T5.3), a conclusion that "the obvious fix is unsound" (the mark/release experiment corrupted
//! memory because it was fighting a phantom), and a design for a two-region scratch/persistent
//! allocator that was never needed. It also sat in this header as settled fact for other readers.
//!
//! ★ AND THE SYSTEMIC FIX IS NOT "BE MORE CAREFUL". Every wasm test used to hand-write its own host
//! — twenty-odd imports each, independently, with nothing checking any of them against the real
//! `runtime.js` or against the ABI's own documentation. A host that is merely PLAUSIBLE produces
//! results that are merely plausible, and a test cannot tell you its oracle is lying. The hosts are
//! shared now (`tests/common/wasm_host.rs`), so an import is implemented ONCE and a mistake in one
//! is a mistake every test sees at once — which is how it gets found.
//!
//! The floor here is deliberately far above any plausible session: a page a person uses for a
//! minute is thousands of turns.

#![cfg(feature = "wasm-jit")]

use logicaffeine_compile::compile::compile_to_wasm;

#[path = "compile_common/mod.rs"]
mod common;
use common::wasm_host::{mount_interactive, mount_interactive_checked};

/// A mounted page in a real module, with everything a turn needs.
///
/// ★ EXTRACTED SO THE TWO QUESTIONS SHARE ONE SETUP. "How many turns does it survive" and "how many
/// bytes does a turn cost" are different questions about the same page, and two copies of a
/// twenty-import host is exactly the shape this file's own header warns about.
struct Mounted {
    store: wasmi::Store<std::rc::Rc<std::cell::RefCell<Option<i64>>>>,
    inst: wasmi::Instance,
}

impl Mounted {
    /// Run one turn — a click on `#b`, the element the page's paint registered for a click last,
    /// handed to the node dispatcher as `runtime.js` hands it. `Err` is the failure the page died
    /// with.
    fn turn(&mut self) -> Result<(), wasmi::core::Trap> {
        let button = self
            .store
            .data()
            .borrow()
            .expect("the page listens for a click");
        crate::common::wasm_host::dispatch_node(&mut self.store, &self.inst, button, "click", "")
            .map(|_| ())
    }

    /// The GROSS heap — every byte the allocator has ever claimed from linear memory, live or
    /// waiting on a free list, through the module's own `__logos_heap_allocated` export rather
    /// than a raw read of its internal bump pointer.
    ///
    /// ★ EXPORTED EXPRESSLY SO THIS COULD BE HONEST, AND READ BY NOTHING. A ratchet that counts
    /// TURNS SURVIVED cannot tell a page that allocates a bounded amount per turn from one that
    /// leaks a kilobyte each time: both survive the floor. The bytes are the measurement; the turn
    /// count is a proxy for it.
    fn heap(&mut self) -> u32 {
        self.inst
            .get_typed_func::<(), i32>(&self.store, "__logos_heap_allocated")
            .expect("an interactive page that uses the heap exports __logos_heap_allocated")
            .call(&mut self.store, ())
            .expect("__logos_heap_allocated does not trap") as u32
    }
}

/// Compile `src`, instantiate it, run `main`, and find the element it listens for a click on.
fn mount(src: &str) -> Mounted {
    let module = compile_to_wasm(src).unwrap_or_else(|e| panic!("must compile: {e:?}\n{src}"));
    let engine = wasmi::Engine::default();
    let m = wasmi::Module::new(&engine, &module[..]).unwrap_or_else(|e| panic!("valid wasm: {e}"));
    // The host records the one thing this test needs: the element the page listens for a click on.
    type D = std::rc::Rc<std::cell::RefCell<Option<i64>>>;
    let seen: D = std::rc::Rc::new(std::cell::RefCell::new(None));
    let mut l = wasmi::Linker::<D>::new(&engine);
    l.func_wrap(
        "env",
        "dom_get_text",
        |_c: wasmi::Caller<'_, D>, _e: i64| -> i32 { 0 },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_get_value",
        |_c: wasmi::Caller<'_, D>, _e: i64| -> i32 { 0 },
    )
    .unwrap();
    l.func_wrap("env", "print_i64", |_c: wasmi::Caller<'_, D>, _v: i64| {})
        .unwrap();
    l.func_wrap("env", "print_text", |_c: wasmi::Caller<'_, D>, _h: i32| {})
        .unwrap();
    l.func_wrap("env", "print_bool", |_c: wasmi::Caller<'_, D>, _v: i32| {})
        .unwrap();
    // The DOM surface a mounted page imports. Inert on purpose: this file measures what the
    // MODULE allocates, so a host that builds a real tree would be measuring the host.
    // `dom_listen_flags` and a paint's `dom_apply_words` stream are the exception — each can carry
    // the element the page listens for a click on, which is where a turn is driven.
    l.func_wrap("env", "dom_body", |_c: wasmi::Caller<'_, D>| -> i64 { 0 })
        .unwrap();
    // Inert too, for the same reason: `the journal`/`Replay` are a `## Test`'s own concept
    // (verified elsewhere, through `common::wasm_host::InteractivePage::turns()`), not something
    // this file's own byte count cares about — but every lowered `__act_`/event/bind now imports
    // it (R3), so any interactive module this host instantiates needs
    // it satisfied to link at all.
    l.func_wrap(
        "env",
        "dom_note_turn",
        |_c: wasmi::Caller<'_, D>, _name: i32, _key: i32, _changes: i32, _failure: i32| {},
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_journal_detail",
        |_c: wasmi::Caller<'_, D>| -> i64 { 0 },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_wake_after",
        |_c: wasmi::Caller<'_, D>, _ms: i64| {},
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_create_element",
        |_c: wasmi::Caller<'_, D>, _t: i32| -> i64 { 1 },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_create_text_node",
        |_c: wasmi::Caller<'_, D>, _t: i32| -> i64 { 2 },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_set_text",
        |_c: wasmi::Caller<'_, D>, _n: i64, _t: i32| {},
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_clear_children",
        |_c: wasmi::Caller<'_, D>, _n: i64| {},
    )
    .unwrap();
    crate::common::wasm_host::link_no_attributes(&mut l);
    l.func_wrap(
        "env",
        "dom_set_attribute",
        |_c: wasmi::Caller<'_, D>, _n: i64, _k: i32, _v: i32| {},
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_append_child",
        |_c: wasmi::Caller<'_, D>, _p: i64, _c2: i64| {},
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_listen_flags",
        |c: wasmi::Caller<'_, D>, node: i64, event: i32, _flags: i64| {
            if crate::common::wasm_host::read_text(&c, event) == "click" {
                *c.data().borrow_mut() = Some(node);
            }
        },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "dom_apply_words",
        |c: wasmi::Caller<'_, D>,
         words: i32,
         word_count: i64,
         strings: i32,
         string_count: i64|
         -> i64 {
            use logicaffeine_compile::semantics::dom::DomOp;
            for op in crate::common::wasm_host::applied_words(
                &c,
                words,
                word_count,
                strings,
                string_count,
            ) {
                if let DomOp::Listen { node, event, .. } = op {
                    if event == "click" {
                        *c.data().borrow_mut() = Some(i64::from(node));
                    }
                }
            }
            0
        },
    )
    .unwrap();
    // The formatters are not this test's decision — one implementation, shared, derived from the
    // product's own display code. This line is the fix for the bug in this file's header.
    crate::common::wasm_host::link_failure_hooks::<D>(&mut l);
    crate::common::wasm_host::link_value_imports::<D>(&mut l);

    let mut store = wasmi::Store::new(&engine, seen.clone());
    crate::common::wasm_host::link_unmodelled_imports(&mut l);
    let inst = l
        .instantiate(&mut store, &m)
        .unwrap_or_else(|e| {
            panic!("a page must instantiate — a missing host import is not a memory result: {e}")
        })
        .start(&mut store)
        .unwrap();
    inst.get_typed_func::<(), ()>(&store, "main")
        .unwrap()
        .call(&mut store, ())
        .expect("mount runs");
    assert!(
        seen.borrow().is_some(),
        "the page must register a click listener"
    );
    Mounted { store, inst }
}

/// Mount `src` and click its `#b` button until it fails or `limit` is reached.
/// Returns how many turns it survived.
fn turns_survived(src: &str, limit: usize) -> usize {
    let mut page = mount(src);
    for i in 0..limit {
        if let Err(e) = page.turn() {
            if std::env::var_os("LOGOS_MEM_TRACE").is_some() {
                eprintln!("[mem] failed at turn {i}: {e}");
            }
            return i;
        }
    }
    limit
}

/// Heap bytes a turn costs, measured over `windows` consecutive windows of `per` turns each.
///
/// The first window is discarded: a page's FIRST turns allocate things it will not allocate again
/// (the scoped stylesheet, the initial class list), and charging those to the steady-state rate
/// would report a leak that is really a warm-up.
fn bytes_per_turn(src: &str, per: usize, windows: usize) -> Vec<f64> {
    let mut page = mount(src);
    for _ in 0..per {
        page.turn().expect("the warm-up window must run");
    }
    let mut out = Vec::new();
    let mut last = page.heap();
    for w in 0..windows {
        for _ in 0..per {
            page.turn()
                .unwrap_or_else(|e| panic!("window {w} must run: {e}"));
        }
        let now = page.heap();
        out.push((now.saturating_sub(last)) as f64 / per as f64);
        last = now;
    }
    if std::env::var_os("LOGOS_MEM_TRACE").is_some() {
        eprintln!("[mem] bytes/turn per window: {out:?}");
    }
    out
}

/// A counter whose click is shown — so every turn really repaints.
const REPAINTING: &str = "## Component App\n\
   \n\
   State:\n\
  \x20   Let n be 0.\n\
   \n\
   Actions:\n\
  \x20   To bump:\n\
  \x20       Set n to n + 1.\n\
   \n\
   View:\n\
  \x20   <main><p id=\"p\">{n}</p><button id=\"b\" on-click={bump}>x</button></main>\n\
   \n\
   ## Main\n\
   Mount App.\n";

#[test]
fn a_repainting_page_survives_ten_thousand_turns() {
    // ★ THE HEADLINE, and the number a person would notice. Ten thousand clicks is a page someone
    // used for a while; twelve is a page that breaks before they finish reading it.
    let survived = turns_survived(REPAINTING, 10_000);
    assert_eq!(
        survived, 10_000,
        "the page ran out of memory after {survived} repaints — a repaint's allocations are all \
         handed to the host and dead when it returns, so they must not accumulate"
    );
}

#[test]
fn a_page_with_a_stylesheet_survives_too() {
    // A `Style:` block re-emits its scoped CSS on every repaint, which is the largest single string
    // a paint produces. If anything accumulates, this is where it shows first.
    let src = REPAINTING.replace(
        "\n## Main\n",
        "\nStyle:\n    p { color: red; }\n\n## Main\n",
    );
    let survived = turns_survived(&src, 10_000);
    assert_eq!(
        survived, 10_000,
        "a styled page ran out after {survived} repaints"
    );
}

#[test]
fn a_page_rendering_a_growing_list_survives() {
    // The shape a real app has: a loop whose length changes, so each repaint allocates more than
    // the last. Bounded memory has to mean bounded PER TURN, not merely constant.
    let src = "## Component App\n\
       \n\
       State:\n\
      \x20   Let items be [1].\n\
       \n\
       Actions:\n\
      \x20   To add:\n\
      \x20       Set items to [1, 2, 3].\n\
       \n\
       View:\n\
      \x20   <main><ul>{for each i in items:}<li>{i}</li>{end}</ul>\
       <button id=\"b\" on-click={add}>x</button></main>\n\
       \n\
       ## Main\n\
       Mount App.\n";
    let survived = turns_survived(src, 5_000);
    assert_eq!(
        survived, 5_000,
        "a list page ran out after {survived} repaints"
    );
}

/// A keyed list of CHILD COMPONENTS (not plain elements) that empties and refills every turn — the
/// per-instance registries `lgx_lower.rs`'s `sweep_gone` prunes (`__SEEN_`, `__M_`, `__S_<C>_
/// <field>`, `__SITE_`, `__P_`, `__DUE_<C>_<i>`) and its per-instance `__LISTENER_SITES_<C>` ledger
/// have something to prune EVERY turn, not just once: three `Row` instances (each with its OWN
/// click listener) are created, then destroyed, then re-created, forever. If registry churn ever
/// stopped being swept, these tables would grow WITHOUT BOUND turn over turn — the exact class the
/// coordinator asked this lane to verify on wasm once lang2 landed the pruning.
const CHURNING_KEYED_COMPONENTS: &str = "## Component Row (label: Text)\nState:\n    Let clicks be 0.\nActions:\n    To bump:\n        Set clicks to clicks + 1.\nView:\n    <li><button class=\"rb\" on-click={bump}>{label}</button></li>\n\n## Component App\nState:\n    Let full be true.\n    Let items be [\"a\", \"b\", \"c\"].\nActions:\n    To churn:\n        If full:\n            Set full to false.\n            Set items to a new Seq of Text.\n        Otherwise:\n            Set full to true.\n            Set items to [\"a\", \"b\", \"c\"].\nView:\n    <main><ul>{for each x in items, keyed by x:}<Row label={x}/>{end}</ul><button id=\"b\" on-click={churn}>x</button></main>\n\n## Main\nMount App.\n";

#[test]
fn a_keyed_component_list_that_empties_and_refills_survives() {
    let survived = turns_survived(CHURNING_KEYED_COMPONENTS, 5_000);
    assert_eq!(
        survived, 5_000,
        "a page churning three component instances in and out of a keyed list ran out after \
         {survived} turns — each churn creates and destroys three listener-bearing instances, so \
         an unswept per-instance registry exhausts memory fast"
    );
}

#[test]
fn the_cost_of_churning_keyed_components_does_not_grow() {
    // The stronger claim `the_cost_of_a_turn_does_not_grow` makes for a plain repaint, here for
    // registry churn specifically: `sweep_gone`'s tables (`__SEEN_`/`__M_`/`__S_<C>_<field>`/
    // `__SITE_`/`__P_`/`__DUE_<C>_<i>`) and `__LISTENER_SITES_<C>` must be reclaimed EVERY churn,
    // not merely bounded by a survival floor — a slow per-turn leak in any one of them would still
    // pass a survival count while failing this.
    let windows = bytes_per_turn(CHURNING_KEYED_COMPONENTS, 200, 4);
    let first = windows[0];
    let last = *windows.last().unwrap();
    assert!(
        last <= first * 2.0 + 64.0,
        "churning three keyed component instances cost {first:.0} bytes/turn early and \
         {last:.0} bytes/turn later (windows {windows:?}) — a registry the pruning does not \
         reach grows every churn, which a survival count alone would not catch in time"
    );
}

#[test]
fn a_turn_that_repaints_nothing_was_never_the_problem() {
    // ★ THE CONTROL THAT LOCATES THE LEAK. An action writing a field no site displays emits no
    // paint at all, and it already survives indefinitely. Keeping it here means a future "fix" that
    // merely made repaints rarer — rather than making their memory reclaimable — cannot pass the
    // tests above by accident while this one silently stops being the distinguishing case.
    let src = "## Component App\n\
       \n\
       State:\n\
      \x20   Let n be 0.\n\
      \x20   Let hidden be 0.\n\
       \n\
       Actions:\n\
      \x20   To bump:\n\
      \x20       Set hidden to hidden + 1.\n\
       \n\
       View:\n\
      \x20   <main><p id=\"p\">{n}</p><button id=\"b\" on-click={bump}>x</button></main>\n\
       \n\
       ## Main\n\
       Mount App.\n";
    assert_eq!(
        turns_survived(src, 20_000),
        20_000,
        "a silent turn must never have leaked"
    );
}

// ---------------------------------------------------------------------------------------------
// ★ THE BYTES THEMSELVES. `__logos_heap_allocated` is exported expressly so this file could be
// honest about them, and until now nothing read it — every test here counted TURNS SURVIVED, which cannot tell
// a page that allocates a bounded amount per turn from one that leaks a kilobyte each time. Both
// survive the floor. The turn count is a proxy; the bytes are the measurement.
// ---------------------------------------------------------------------------------------------

#[test]
fn a_turn_costs_a_reported_number_of_bytes() {
    // Not a budget anyone tuned — a ceiling far above any plausible per-repaint cost, whose job is
    // to fail LOUDLY with the real figure when a change makes a turn cost kilobytes.
    let windows = bytes_per_turn(REPAINTING, 200, 3);
    let worst = windows.iter().cloned().fold(0.0_f64, f64::max);
    assert!(
        worst < 4096.0,
        "a repaint of a page with one hole costs {worst:.0} bytes per turn (windows {windows:?}) — \
         far more than the text it produces, so something is allocating per turn that should not"
    );
}

#[test]
fn the_cost_of_a_turn_does_not_grow() {
    // ★★ THE SHAPE OF THE CATASTROPHE THIS FILE'S HEADER DESCRIBES. The false alarm was an exact
    // DOUBLING curve — each turn allocating a buffer the size of the whole heap. A survival count
    // catches that only once it exhausts memory, and a page that doubles slowly enough survives
    // ten thousand turns and is still fatally wrong. Comparing an early window against a late one
    // catches it while the numbers are still small.
    let windows = bytes_per_turn(REPAINTING, 300, 4);
    let first = windows[0];
    let last = *windows.last().unwrap();
    assert!(
        last <= first * 2.0 + 64.0,
        "a turn cost {first:.0} bytes early and {last:.0} bytes later (windows {windows:?}) — the \
         per-turn cost is GROWING, which is the doubling shape this file's header describes and \
         which no survival count would have caught in time"
    );
}

#[test]
fn the_byte_measurement_actually_measures() {
    // ★ ANTI-VACUITY, and this file has earned its caution about oracles. If `__logos_heap_allocated`
    // never moved — a stub, a wrong export, a page that does not really repaint — every assertion above
    // would pass at zero bytes forever, which is exactly the shape of the bug in the header.
    let mut page = mount(REPAINTING);
    let before = page.heap();
    for _ in 0..100 {
        page.turn().expect("a turn must run");
    }
    let after = page.heap();
    assert!(
        after > before,
        "the heap pointer did not move over a hundred repaints ({before} → {after}) — a repaint \
         allocates the text of its hole, so a still pointer means this is measuring nothing"
    );
    // And it is a plausible amount rather than the whole address space.
    assert!(
        after - before < 4096 * 100,
        "a hundred repaints took {} bytes, which is not a measurement of one hole's text",
        after - before
    );
}

#[test]
fn a_page_that_allocates_nothing_per_turn_reports_near_zero() {
    // The other end of the instrument: a turn that changes no rendered text should cost almost
    // nothing, so the number is a real reading rather than a constant.
    let src = "## Component App\n\nState:\n\x20   Let n be 0.\n\nActions:\n\x20   To bump:\n\
              \x20       Set n to n.\n\nView:\n\
              \x20   <main><p id=\"p\">steady</p><button id=\"b\" on-click={bump}>x</button></main>\n\n\
               ## Main\nMount App.\n";
    let windows = bytes_per_turn(src, 200, 2);
    let worst = windows.iter().cloned().fold(0.0_f64, f64::max);
    let repainting = bytes_per_turn(REPAINTING, 200, 2)[0];
    assert!(
        worst <= repainting + 64.0,
        "a turn that renders the same text must not cost more than one that renders new text \
         ({worst:.0} vs {repainting:.0} bytes) — the instrument is not reading the page"
    );
}

/// ★ THE HEAP GIVES BLOCKS BACK. A growing sequence allocates a bigger buffer, copies into it and
/// abandons the old one — and the self-contained allocator used to be a pure bump, so every buffer
/// a list outgrew was gone for the life of the page.
///
/// That block is dead by construction (a data buffer belongs to exactly one header, which has just
/// stopped pointing at it), so it needs no refcount to reclaim — which is why it is the first free
/// the allocator got, ahead of the release protocol that decides when a VALUE dies.
///
/// Measured on a turn that pushes n items: **32 bytes per push before, 16 after** — the payload is
/// 8, so what is left is one allocation's overhead rather than a discarded buffer each doubling.
/// The saving grows with the list: 21% at 4 pushes, 43% at 32.
///
/// Pinned as a SLOPE, not a total, so the page's fixed costs can move without touching this.
#[test]
fn a_growing_sequence_reuses_the_buffer_it_outgrew() {
    let page = |n: usize| {
        let pushes: String = (1..=n)
            .map(|i| format!("        Push {i} to xs.\n"))
            .collect();
        format!(
            "## Component App\nState:\n    Let k be 0.\nActions:\n    To add:\n\
             \x20       Let xs be [0].\n{pushes}        Set k to length of xs.\n\
             View:\n    <main><p>{{k}}</p><button id=\"b\" on-click={{add}}>x</button></main>\n\n\
             ## Main\nMount App.\n"
        )
    };
    let small = bytes_per_turn(&page(8), 60, 2)[0];
    let big = bytes_per_turn(&page(32), 60, 2)[0];
    let per_push = (big - small) / 24.0;
    assert!(
        per_push < 24.0,
        "a push costs {per_push:.0} bytes of heap ({small:.0} at 8 pushes, {big:.0} at 32). \
         An 8-byte element that costs more than a whole spare slot means the buffer a growth \
         abandoned is not going back on the free list — see `emit_free` in the seq-growth path."
    );
}

/// ★ A MAP GROWS THE SAME WAY A SEQUENCE DOES — one entry at a time, no cap — so an insert used to
/// realloc to EXACTLY `num + 1` entries and abandon the old buffer, unfreed: building an n-entry map
/// from empty cost `Σ(1..n) * 16` bytes rather than `n * 16`, quadratic in the map built each turn.
///
/// Pinned as a SLOPE per entry, not a total, so the page's fixed costs can move without touching it.
#[test]
fn a_growing_map_reuses_the_buffer_it_outgrew() {
    let page = |n: usize| {
        let inserts: String = (1..=n)
            .map(|i| format!("        Set item \"k{i}\" of m to {i}.\n"))
            .collect();
        format!(
            "## Component App\nState:\n    Let k be 0.\nActions:\n    To add:\n\
             \x20       Let m be a new Map of Text to Int.\n{inserts}        Set k to length of m.\n\
             View:\n    <main><p>{{k}}</p><button id=\"b\" on-click={{add}}>x</button></main>\n\n\
             ## Main\nMount App.\n"
        )
    };
    let small = bytes_per_turn(&page(8), 60, 2)[0];
    let big = bytes_per_turn(&page(32), 60, 2)[0];
    let per_insert = (big - small) / 24.0;
    assert!(
        per_insert < 48.0,
        "a map insert costs {per_insert:.0} bytes of heap ({small:.0} at 8 entries, {big:.0} at 32). \
         A 16-byte entry that costs multiples of a whole entry means the buffer a growth abandoned \
         is not going back on the free list — see `lower_map_insert`'s growth in `vm/wasm/module.rs`."
    );
}

/// ★ A SET GROWS THE SAME WAY. `Add x to s` used to realloc to exactly `num + 1` elements on every
/// add and abandon the old buffer, unfreed — the same quadratic shape a growing sequence had before
/// its push path doubled and freed.
///
/// Pinned as a SLOPE per element, not a total, so the page's fixed costs can move without touching it.
#[test]
fn a_growing_set_reuses_the_buffer_it_outgrew() {
    let page = |n: usize| {
        let adds: String = (1..=n)
            .map(|i| format!("        Add {i} to s.\n"))
            .collect();
        format!(
            "## Component App\nState:\n    Let k be 0.\nActions:\n    To add:\n\
             \x20       Let s be a new Set of Int.\n{adds}        Set k to length of s.\n\
             View:\n    <main><p>{{k}}</p><button id=\"b\" on-click={{add}}>x</button></main>\n\n\
             ## Main\nMount App.\n"
        )
    };
    let small = bytes_per_turn(&page(8), 60, 2)[0];
    let big = bytes_per_turn(&page(32), 60, 2)[0];
    let per_add = (big - small) / 24.0;
    assert!(
        per_add < 24.0,
        "a set add costs {per_add:.0} bytes of heap ({small:.0} at 8 elements, {big:.0} at 32). \
         An 8-byte element that costs more than a whole spare slot means the buffer a growth \
         abandoned is not going back on the free list — see `emit_set_add_elem` in `vm/wasm/module.rs`."
    );
}

/// ★ REUSE DOES NOT STOP AT 1 KiB. The free list is exact only up to 1024 bytes (128 classes of 8
/// bytes); a block bigger than that used to bump and never come back, so a set or map big enough to
/// double PAST that boundary went right back to leaking — the same shape `a_growing_set/map_reuses…`
/// catch, just at a scale those tests are too small to reach. 600 elements doubles a `Set of Int`'s
/// buffer through 2048 and 4096 bytes on the way to its final size, so the slope stays the SAME
/// small per-element cost only if those large abandoned buffers go back on a list too.
#[test]
fn a_set_growing_past_one_kib_still_reuses_its_buffers() {
    let page = |n: usize| {
        let adds: String = (1..=n)
            .map(|i| format!("        Add {i} to s.\n"))
            .collect();
        format!(
            "## Component App\nState:\n    Let k be 0.\nActions:\n    To add:\n\
             \x20       Let s be a new Set of Int.\n{adds}        Set k to length of s.\n\
             View:\n    <main><p>{{k}}</p><button id=\"b\" on-click={{add}}>x</button></main>\n\n\
             ## Main\nMount App.\n"
        )
    };
    let small = bytes_per_turn(&page(300), 20, 2)[0];
    let big = bytes_per_turn(&page(600), 20, 2)[0];
    let per_add = (big - small) / 300.0;
    assert!(
        per_add < 24.0,
        "a set add above 1 KiB costs {per_add:.0} bytes of heap ({small:.0} at 300 elements, \
         {big:.0} at 600) — the same slope the small-scale test pins below 1 KiB. A block over 1 KiB \
         that bumps and is never reclaimed means the allocator's free list stops at 1024 bytes — see \
         the power-of-two tier `emit_alloc_helper`/`emit_free_helper` should carry above it."
    );
}

/// The `Map of Text to Int` counterpart: 40 entries × 16 bytes crosses 1024 and 2048 bytes on the
/// way to 640 entries' final buffer.
#[test]
fn a_map_growing_past_one_kib_still_reuses_its_buffers() {
    let page = |n: usize| {
        let inserts: String = (1..=n)
            .map(|i| format!("        Set item \"k{i}\" of m to {i}.\n"))
            .collect();
        format!(
            "## Component App\nState:\n    Let k be 0.\nActions:\n    To add:\n\
             \x20       Let m be a new Map of Text to Int.\n{inserts}        Set k to length of m.\n\
             View:\n    <main><p>{{k}}</p><button id=\"b\" on-click={{add}}>x</button></main>\n\n\
             ## Main\nMount App.\n"
        )
    };
    let small = bytes_per_turn(&page(300), 20, 2)[0];
    let big = bytes_per_turn(&page(600), 20, 2)[0];
    let per_insert = (big - small) / 300.0;
    assert!(
        per_insert < 48.0,
        "a map insert above 1 KiB costs {per_insert:.0} bytes of heap ({small:.0} at 300 entries, \
         {big:.0} at 600) — the same slope the small-scale test pins below 1 KiB. A block over 1 KiB \
         that bumps and is never reclaimed means the allocator's free list stops at 1024 bytes — see \
         the power-of-two tier `emit_alloc_helper`/`emit_free_helper` should carry above it."
    );
}

/// ★ A CRDT COUNTER MERGE GROWS THE SAME WAY A SET/MAP DOES. `Merge r into local` appends a fresh
/// per-replica record to `local`'s counter object when `r`'s replica is new, doubling the record
/// buffer on the way — but the doubling path never freed the buffer it outgrew, unlike every other
/// growth path in this file. Each turn rebuilds `local` and every `r{i}` fresh (`NewCrdt` bump-
/// allocates a distinct header, so every replica id is unique turn to turn), so the only thing a fix
/// can carry across turns is the abandoned RECORD buffers going back on the free list.
#[test]
fn a_merging_counter_reuses_the_buffer_its_growth_outgrew() {
    let page = |n: usize| {
        let merges: String = (1..=n)
            .map(|i| {
                format!(
                    "\x20       Let r{i} be a new Stats.\n\
                     \x20       Increase r{i}'s views by {i}.\n\
                     \x20       Merge r{i} into local.\n"
                )
            })
            .collect();
        format!(
            "## A Stats is Shared and has:\n    a views, which is ConvergentCount.\n\n\
             ## Component App\nState:\n    Let k be 0.\nActions:\n    To add:\n\
             \x20       Let local be a new Stats.\n{merges}\
             \x20       Set k to local's views.\n\
             View:\n    <main><p>{{k}}</p><button id=\"b\" on-click={{add}}>x</button></main>\n\n\
             ## Main\nMount App.\n"
        )
    };
    let small = bytes_per_turn(&page(8), 20, 2)[0];
    let big = bytes_per_turn(&page(32), 20, 2)[0];
    let per_merge = (big - small) / 24.0;
    // Measured 181 B/merge with the free call removed, 133 with it present (each fresh replica's own
    // header + record is genuinely new data every merge; only the abandoned growth buffer is not).
    // The ceiling sits between the two, close to the fixed number, so a regression that drops the
    // free again fails loudly rather than hiding in a wide margin.
    assert!(
        per_merge < 150.0,
        "an extra merge costs {per_merge:.0} bytes of heap ({small:.0} at 8 merges, {big:.0} at 32) — \
         see `emit_counter_append` in `vm/wasm/module.rs`; a per-replica record buffer a merge's \
         growth abandons is not going back on the free list."
    );
}

/// ★ A STATIC STRING MUST NOT ALLOCATE. `Constant::Text` used to build "a fresh Text object in
/// linear memory" on every evaluation — a 16-byte header plus an 8-aligned buffer — so a literal in
/// a view minted a new Text on every paint. Measured at 24 bytes of heap per static text child per
/// row per turn, and it was invisible: the page rendered correctly and simply consumed its heap
/// faster. Literals are interned into the module's data section now, so their handles are constants.
///
/// The property, not the number: a row carrying a literal must cost no more heap per turn than an
/// empty element does. (It also caught the element TAG name, which was itself a literal being
/// rebuilt every render — an empty `<li>` fell from 40 bytes a row to 16.)
#[test]
fn a_static_string_costs_no_heap_to_render() {
    let page = |n: usize, row: &str| {
        let items: Vec<String> = (1..=n).map(|i| i.to_string()).collect();
        format!(
            "## Component App\nState:\n    Let items be [{}].\nActions:\n    To add:\n\
             \x20       Set items to [{}].\nView:\n    <main><ul>{{for each i in items:}}{row}{{end}}\
             </ul><button id=\"b\" on-click={{add}}>x</button></main>\n\n## Main\nMount App.\n",
            items.join(", "),
            items.join(", ")
        )
    };
    let per_row = |row: &str| {
        let a = bytes_per_turn(&page(2, row), 40, 2)[0];
        let b = bytes_per_turn(&page(10, row), 40, 2)[0];
        (b - a) / 8.0
    };
    let empty = per_row("<li></li>");
    let literal = per_row("<li>hello there</li>");
    assert!(
        literal <= empty + 8.0,
        "a row with a static string costs {literal:.0} bytes of heap a turn where an empty element \
         costs {empty:.0}. A literal cannot change, so it belongs in the data section with a \
         constant handle — see `TextPool` in the wasm emitter."
    );
}

// ---- S3: a repainting element's OLD listener-table entries do not outlive it ----
//
// `__LISTENER_FN`/`__LISTENER_KEY` are keyed by `node*256+code` (`lgx_lower.rs::register_listener_
// with`), and a repaint hands out FRESH node ids every time (`clearChildren` destroys the old
// elements; the render that follows creates new ones from zero). Before S3, nothing ever removed
// the entry a PREVIOUS paint's node registered — a component that re-registers a listener on every
// repaint (any stateful component with `on-click`/`bind` inside its own re-rendered markup) grew
// these two maps by one dead row per repaint, for the life of the page. S3
// (`lower_component_render`'s prune-then-write-back, `__LISTENER_SITES_<C>`) retires the previous
// paint's keys before the new paint registers anything. Calibrated against the pre-S3 lowering in a
// private mutant copy (never the shared tree): the SAME
// fixture, run the SAME way, costs a rate that is no longer flat between the two window sizes below
// (RED there; GREEN here).

/// A page whose button re-registers ONE listener every repaint, and whose state — an `Int` that only
/// ever counts up — never grows a collection, so a repaint's only NON-CONSTANT heap cost, once the S2
/// map/set doubling fixes have amortized the rest to zero, is exactly what S3 governs: the listener
/// table.
///
/// ★ THREE SHAPES A FIXTURE FOR THIS LOCK MUST AVOID, all three found the hard way (GREEN for the
/// wrong reason, on both sides of the mutation, while writing this file):
///
/// 1. **The button must not be the component's own single root.** A component whose view is exactly
///    one top-level element REUSES that element across a repaint (`lgx_pending_reuse`/`repaint_def`'s
///    own root-reuse trick) rather than recreating it — so a button standing alone as `<button
///    on-click={bump}>x</button>` keeps the SAME node id forever, `register_listener_with`'s key
///    never changes, and `SetIndex` on an unchanging key overwrites in place: this shape could never
///    exercise the leak S3 fixes, with or without the fix.
/// 2. **`n` must be DISPLAYED somewhere.** This file's own
///    `a_turn_that_repaints_nothing_was_never_the_problem` (below) documents the mechanism this fixture must
///    NOT accidentally hit: an action that writes a field no site displays triggers no repaint at
///    all, by design — the compiled page has no reason to redraw anything nothing depends on.
/// 3. **Displaying `n` in a plain text hole is not enough either.** `repaint_def`'s own header
///    explains why: a turn PATCHES when it can (`Stage B` — a bound text hole rewritten in place via
///    `setText`, no element touched at all) and only falls back to a full clear-and-rebuild repaint
///    when the turn is NOT patchable. A `<p>{n}</p>` next to the button is exactly the patchable case
///    — measured directly: it produced the IDENTICAL bytes/turn with S3's pruning mutated OUT as with
///    it in, because neither run ever called `createElement`/`register_listener_with` a second time
///    at all. The button needs a truly UNPATCHABLE sibling change forcing `repaint_def`'s full path —
///    an `{if}` whose TAKEN BRANCH flips every turn (a structural change, never patchable) — so the
///    whole subtree, button included, is genuinely torn down and rebuilt from nothing each time.
const REPAINTS_WITH_ONE_LISTENER: &str = "## Component App\n\
   \n\
   State:\n\
  \x20   Let flag be true.\n\
   \n\
   Actions:\n\
  \x20   To bump:\n\
  \x20       Set flag to not flag.\n\
   \n\
   View:\n\
  \x20   <div>{if flag:}<p>even</p>{else:}<p>odd</p>{end}\
<button id=\"b\" on-click={bump}>x</button></div>\n\
   \n\
   ## Main\n\
   Mount App.\n";

/// A mounted page whose host hands out a FRESH node id per `createElement` call, exactly as a real
/// DOM does and as `runtime.js` does — the one detail this file's own `Mounted` does NOT reproduce
/// (its `dom_create_element` always answers the constant `1`, which is right for measuring aggregate
/// bytes but wrong here: a fixed node id makes `node*256+code` the SAME key on every repaint, so the
/// very growth this lock exists to catch could never appear at all).
struct GrowingIds {
    store: wasmi::Store<std::rc::Rc<Host>>,
    inst: wasmi::Instance,
}

struct Host {
    next_node: std::cell::Cell<i64>,
    clicked: std::cell::Cell<Option<i64>>,
}

impl GrowingIds {
    fn mount(src: &str) -> Self {
        let module = compile_to_wasm(src).unwrap_or_else(|e| panic!("must compile: {e:?}\n{src}"));
        let engine = wasmi::Engine::default();
        let m =
            wasmi::Module::new(&engine, &module[..]).unwrap_or_else(|e| panic!("valid wasm: {e}"));
        let host = std::rc::Rc::new(Host {
            next_node: std::cell::Cell::new(1),
            clicked: std::cell::Cell::new(None),
        });
        let mut l = wasmi::Linker::<std::rc::Rc<Host>>::new(&engine);
        l.func_wrap(
            "env",
            "dom_get_text",
            |_c: wasmi::Caller<'_, std::rc::Rc<Host>>, _e: i64| -> i32 { 0 },
        )
        .unwrap();
        l.func_wrap(
            "env",
            "dom_get_value",
            |_c: wasmi::Caller<'_, std::rc::Rc<Host>>, _e: i64| -> i32 { 0 },
        )
        .unwrap();
        l.func_wrap(
            "env",
            "dom_body",
            |_c: wasmi::Caller<'_, std::rc::Rc<Host>>| -> i64 { 0 },
        )
        .unwrap();
        l.func_wrap(
            "env",
            "dom_note_turn",
            |_c: wasmi::Caller<'_, std::rc::Rc<Host>>,
             _name: i32,
             _key: i32,
             _changes: i32,
             _failure: i32| {},
        )
        .unwrap();
        l.func_wrap(
            "env",
            "dom_journal_detail",
            |_c: wasmi::Caller<'_, std::rc::Rc<Host>>| -> i64 { 0 },
        )
        .unwrap();
        l.func_wrap(
            "env",
            "dom_wake_after",
            |_c: wasmi::Caller<'_, std::rc::Rc<Host>>, _ms: i64| {},
        )
        .unwrap();
        l.func_wrap(
            "env",
            "dom_create_element",
            |c: wasmi::Caller<'_, std::rc::Rc<Host>>, _t: i32| -> i64 {
                let id = c.data().next_node.get();
                c.data().next_node.set(id + 1);
                id
            },
        )
        .unwrap();
        l.func_wrap(
            "env",
            "dom_create_text_node",
            |c: wasmi::Caller<'_, std::rc::Rc<Host>>, _t: i32| -> i64 {
                let id = c.data().next_node.get();
                c.data().next_node.set(id + 1);
                id
            },
        )
        .unwrap();
        l.func_wrap(
            "env",
            "dom_set_text",
            |_c: wasmi::Caller<'_, std::rc::Rc<Host>>, _n: i64, _t: i32| {},
        )
        .unwrap();
        l.func_wrap(
            "env",
            "dom_clear_children",
            |_c: wasmi::Caller<'_, std::rc::Rc<Host>>, _n: i64| {},
        )
        .unwrap();
        crate::common::wasm_host::link_no_attributes(&mut l);
        l.func_wrap(
            "env",
            "dom_set_attribute",
            |_c: wasmi::Caller<'_, std::rc::Rc<Host>>, _n: i64, _k: i32, _v: i32| {},
        )
        .unwrap();
        l.func_wrap(
            "env",
            "dom_append_child",
            |_c: wasmi::Caller<'_, std::rc::Rc<Host>>, _p: i64, _c2: i64| {},
        )
        .unwrap();
        l.func_wrap(
            "env",
            "dom_listen_flags",
            |c: wasmi::Caller<'_, std::rc::Rc<Host>>, node: i64, event: i32, _flags: i64| {
                if crate::common::wasm_host::read_text(&c, event) == "click" {
                    c.data().clicked.set(Some(node));
                }
            },
        )
        .unwrap();
        l.func_wrap(
            "env",
            "dom_apply_words",
            |c: wasmi::Caller<'_, std::rc::Rc<Host>>,
             words: i32,
             word_count: i64,
             strings: i32,
             string_count: i64|
             -> i64 {
                use logicaffeine_compile::semantics::dom::DomOp;
                for op in crate::common::wasm_host::applied_words(
                    &c,
                    words,
                    word_count,
                    strings,
                    string_count,
                ) {
                    if let DomOp::Listen { node, event, .. } = op {
                        if event == "click" {
                            c.data().clicked.set(Some(i64::from(node)));
                        }
                    }
                }
                0
            },
        )
        .unwrap();
        crate::common::wasm_host::link_failure_hooks::<std::rc::Rc<Host>>(&mut l);
        crate::common::wasm_host::link_value_imports::<std::rc::Rc<Host>>(&mut l);
        let mut store = wasmi::Store::new(&engine, host);
        crate::common::wasm_host::link_unmodelled_imports(&mut l);
        let inst = l
            .instantiate(&mut store, &m)
            .unwrap_or_else(|e| panic!("a page must instantiate — a missing host import: {e}"))
            .start(&mut store)
            .unwrap_or_else(|e| panic!("main must not trap: {e}"));
        inst.get_typed_func::<(), ()>(&store, "main")
            .unwrap()
            .call(&mut store, ())
            .expect("mount runs");
        GrowingIds { store, inst }
    }

    /// Click the button: the node the LATEST paint registered for a click — a full repaint rebuilds
    /// the button under a fresh id, and the node dispatcher answers the element as it now stands,
    /// exactly as `runtime.js` hands a browser's click over.
    fn turn(&mut self) {
        let button = self
            .store
            .data()
            .clicked
            .get()
            .expect("a listener was registered");
        crate::common::wasm_host::dispatch_node(&mut self.store, &self.inst, button, "click", "")
            .expect("a turn must not trap");
    }

    fn heap(&mut self) -> u32 {
        self.inst
            .get_typed_func::<(), i32>(&self.store, "__logos_heap_allocated")
            .expect("an interactive heap module exports __logos_heap_allocated")
            .call(&mut self.store, ())
            .expect("__logos_heap_allocated does not trap") as u32
    }
}

/// Steady-state bytes per turn, discarding a `warmup`-turn window first (the page's first paints
/// allocate things — the initial class list, the first stringified `n` — a steady-state page never
/// allocates again; charging those to the rate would report a leak that is really a warm-up, exactly
/// as `bytes_per_turn`'s own header (above) explains).
fn steady_state_bytes_per_turn(src: &str, warmup: usize, window: usize) -> f64 {
    let mut page = GrowingIds::mount(src);
    for _ in 0..warmup {
        page.turn();
    }
    let before = page.heap();
    for _ in 0..window {
        page.turn();
    }
    let after = page.heap();
    (after.saturating_sub(before)) as f64 / window as f64
}

/// The steady-state cost of this fixture once its listener table stops growing: nothing. Every block
/// a turn allocates — `lower_component_render`'s `__lgx_paint_sites` accumulator (S3's local, a new
/// empty `Seq of Int` at the top of every render), the `{if flag:}`/`{else:}` branch's rebuilt `<p>`
/// and its text, the event's name and value the host writes into the heap for the node dispatcher —
/// is credited to the turn and freed at the safepoint that ends it (`runtime.js`'s `guarded` at depth
/// 0, `dispatch_node` here), and the next turn's allocations take those blocks back off the free
/// lists, so the gross heap stops moving. `__LISTENER_FN`/`__LISTENER_KEY`/the
/// `__ARG_<C>_<action>_<i>` maps add nothing once S3 has retired their previous entry — nor does
/// writing them: a write takes the map out of its global and puts it back, which moves the global's
/// hold and copies nothing. A table still growing costs bytes at 20,000 turns as at 2,000 (this
/// file's second test), and so does anything a turn leaves held.
const STEADY_STATE_BYTES_PER_TURN: f64 = 0.0;

/// ★ THE LOCK. A component whose only per-turn allocation is its own re-registered listener costs no
/// steady-state bytes at all once S3's per-instance ledger has retired the
/// previous paint's entries — the listener table's own backing buffer stabilizes at its first real
/// size and never grows again, because `num` never exceeds the peak it reached on the SECOND paint
/// (prune-then-register keeps it at exactly one live entry, forever). Calibrated against the pre-S3
/// lowering in a private mutant copy (never the shared tree): the
/// identical fixture, run the identical way, costs a rate that keeps GROWING between the
/// two window sizes below instead of staying at this one constant (the map's own amortized
/// per-insert growth, for an entry count that climbs by one every single turn instead of staying at
/// one) — RED there, GREEN here.
#[test]
fn a_repainting_listener_costs_a_bounded_constant_once_its_table_stabilizes() {
    let rate = steady_state_bytes_per_turn(REPAINTS_WITH_ONE_LISTENER, 200, 2_000);
    assert_eq!(
        rate, STEADY_STATE_BYTES_PER_TURN,
        "a component that re-registers one listener every repaint must cost nothing per turn once \
         its listener table has stabilized: everything a turn builds is freed at the safepoint that \
         ends it — {rate} bytes/turn means something a turn made is still held, or \
         `__LISTENER_FN`/`__LISTENER_KEY`/an `__ARG_<C>_<action>_<i>` map is still growing every turn"
    );
}

/// The same measurement, ten times further into the run, MUST equal the first window's rate exactly
/// — proving the table's size (and so its own contribution to bytes/turn) is the SAME at turn 2,200
/// as at turn 20,200, not merely small at the smaller scale. A table that still grows, even slowly,
/// costs MORE per turn at the larger scale: each of its own reallocations grows proportionally with
/// its current size, so a genuinely unbounded map's amortized rate at 10x the turns is measurably
/// different from its rate at 1x — flat-to-the-byte at both scales is the signature only a bounded
/// table produces.
#[test]
fn the_table_costs_the_same_constant_ten_times_further_into_the_run() {
    let rate = steady_state_bytes_per_turn(REPAINTS_WITH_ONE_LISTENER, 200, 20_000);
    assert_eq!(
        rate, STEADY_STATE_BYTES_PER_TURN,
        "a rate of {rate} bytes/turn at 20,000 turns, rather than the same {STEADY_STATE_BYTES_PER_TURN} \
         bytes/turn measured at 2,000, means SOMETHING is still growing over the page's real life, even \
         if it looked bounded in a short run"
    );
}

/// S3, proven on the PLAIN BYTECODE VM — the tier `largo test --tiers all`'s own VM replay and a
/// real page's non-WASM fallback actually run, not only the WASM AOT backend the rate-based tests
/// above measure. `__LISTENER_FN`/`__LISTENER_KEY` are ordinary LOGOS-level global maps
/// (`register_listener_with`, `lgx_lower/naming.rs`), compiled identically for either backend, so
/// the VM's own copy is directly readable — [`logicaffeine_compile::vm::session::Session::
/// global_len`] counts its entries from the SAME `Show`-style display the VM already renders a
/// global with, bracket-depth-aware over an `Int`-keyed/`Int`-valued map, exactly this table's
/// shape. Mounts `REPAINTS_WITH_ONE_LISTENER` through [`LoweredPage`] (`page_host`'s own driver, the
/// one `largo test`'s cross-tier check drives every page through) and clicks its button `clicks`
/// times, in-process — no `wasmi`/host-import scaffolding needed, unlike the WASM measurement above.
fn listener_table_len_after(src: &str, clicks: usize) -> usize {
    use logicaffeine_compile::page_host::{Drive, LoweredPage};
    use logicaffeine_compile::vm::session::Session;
    use logicaffeine_compile::vm::{Compiler, Vm};
    logicaffeine_compile::semantics::dom::reset();
    logicaffeine_compile::ui_bridge::with_parsed_program(src, |parsed, interner| {
        let (stmts, types, policies) = parsed.unwrap_or_else(|e| panic!("must parse: {e}\n{src}"));
        let program = Compiler::compile_with_types(stmts, interner, Some(types))
            .unwrap_or_else(|e| panic!("must compile for the VM: {e}\n{src}"));
        let vm = Vm::new(&program).with_policy_ctx(&policies, interner);
        let mut page = LoweredPage::new(Session::new(vm, interner));
        page.start(None, |s| s.run_main())
            .unwrap_or_else(|e| panic!("main must not trap: {e:?}"));
        for _ in 0..clicks {
            page.drive(&Drive::Click {
                selector: "#b".to_string(),
            })
            .unwrap_or_else(|e| panic!("a click must not trap: {e:?}"));
        }
        page.calls().global_len("__LISTENER_FN").unwrap_or_else(|| {
            panic!(
                "the compiled program must declare `__LISTENER_FN` — this fixture registers an \
                 `on-click` listener every repaint"
            )
        })
    })
}

/// ★ THE VM-TIER LOCK. Mirrors `a_repainting_listener_costs_a_bounded_constant_once_its_table_
/// stabilizes`'s own claim, on the tier that test cannot reach: `__LISTENER_FN`'s entry count after
/// 200 turns must equal its count after 2,000 — S3's prune-then-register retiring the previous
/// paint's key on EVERY repaint, not merely amortizing a slow leak away, on the bytecode VM exactly
/// as on the WASM AOT backend (the SAME lowering compiles both).
#[test]
fn a_repainting_listener_table_is_bounded_on_the_lowered_vm() {
    let small = listener_table_len_after(REPAINTS_WITH_ONE_LISTENER, 200);
    let big = listener_table_len_after(REPAINTS_WITH_ONE_LISTENER, 2_000);
    assert_eq!(
        small, big,
        "`__LISTENER_FN` held {small} entries after 200 turns but {big} after 2,000 — its own \
         previous paint's key must be retired every repaint, not accumulated"
    );
}

/// The same proof, ten times further into the run (2,000 vs 20,000) — the same widening this file's
/// WASM-side `the_table_costs_the_same_constant_ten_times_further_into_the_run` uses to catch a
/// slow leak too small to move the smaller window's count.
#[test]
fn the_lowered_vm_table_stays_the_same_size_ten_times_further_into_the_run() {
    let small = listener_table_len_after(REPAINTS_WITH_ONE_LISTENER, 2_000);
    let big = listener_table_len_after(REPAINTS_WITH_ONE_LISTENER, 20_000);
    assert_eq!(
        small, big,
        "`__LISTENER_FN` held {small} entries after 2,000 turns but {big} after 20,000 — a slow \
         per-turn leak too small to see at the smaller scale would still show up here"
    );
}

// ============================================================================================
// ★ S8 — a host-held closure handle's lifetime equals its holder count. ★
//
// `runtime.js` calls `inst.exports.__logos_release(h)` once per registration it drops (a replaced
// listener, a finished fetch, a superseded route handler) — the Wave 6 plan's own "the host calls
// `__logos_release(h)` once per dropped handle". Before this landed the export did not exist (the
// call always silently no-opped), so a closure was permanently immortal — safe, but it meant
// NOTHING ever checked a closure's lifetime against its holder count at all. `dom_on_window_event`
// (the closure a paint registers for a window sentence) exercises the identical "registering import
// retains, `__logos_release` decrements" path `dom_fetch_start`/`_send` do (both are `CALLBACK`-marked
// in `host_fns!`, both go through `lower_dom_call`'s ABI-driven `host.retained_params()` retain loop)
// — these tests use it because it needs no fetch/decode machinery to drive to completion, only
// `dom_on_window_event` (recorded by `InteractivePage`) and the exported `__logos_release`/
// `call_closure`. An element's event is no closure at all (it is a listener-table entry the node
// dispatcher reads), so it has no handle whose lifetime to check.
//
// What THIS section proves: a closure's true count equals its holder count exactly — one
// registration, one release, freed exactly then — and two closures are counted independently.
//
// All tests here mount the check-mode module (`mount_interactive_checked`): the page as shipped, with
// the census exported.
// ============================================================================================

/// Two window sentences doing the same thing — two registrations, so two DIFFERENT closure handles:
/// "two independent closures, independently tracked".
const TWO_WINDOW_SENTENCES: &str = "\
## Component App
State:
    Let n be 0.
Actions:
    When the window resizes:
        Set n to n + 1.
    When the app goes offline:
        Set n to n + 1.
View:
    <p>{n}</p>

## Main
Mount App.
";

/// The property itself, checked over the two INDEPENDENT closures `TWO_WINDOW_SENTENCES` builds:
/// releasing ONE closure's own registration must not disturb the OTHER, unrelated closure — each is
/// freed exactly at its own count reaching 0, never early, never coupled to a sibling.
#[test]
fn releasing_one_closures_registration_leaves_an_unrelated_closure_callable() {
    let mut page = mount_interactive_checked(TWO_WINDOW_SENTENCES);
    let handlers = page.handlers();
    assert_eq!(
        handlers.len(),
        2,
        "two window sentences must register two listeners: {handlers:?}"
    );
    let (a, b) = (handlers[0], handlers[1]);
    assert_ne!(
        a, b,
        "this fixture's premise: two window sentences register two DIFFERENT closures"
    );
    page.safepoint();
    assert_eq!(
        page.census(),
        0,
        "both independent registrations own exactly one reference"
    );
    let both_live = page.heap_used();
    // Release `a` only.
    page.release_handle(a);
    page.safepoint();
    assert_eq!(page.census(), 0, "released registration owns nothing");
    assert!(
        page.heap_used() < both_live,
        "released closure storage is reclaimed"
    );
    assert!(
        page.call_closure(a, 0).is_err(),
        "calling a released, poisoned closure must trap"
    );
    // `b` is a completely separate object — releasing `a` must not have touched it.
    assert_eq!(
        page.census(),
        0,
        "the unrelated registration retains its exact ownership"
    );
    page.call_closure(b, 0)
        .expect("an unrelated, still-registered closure must remain callable");
}

/// The un-shared control: a closure registered exactly ONCE follows the SAME rule — one release
/// frees it. Proves the mechanism is not "never frees anything", only "frees at the right count".
#[test]
fn a_closure_registered_once_is_freed_by_its_one_release() {
    const ONE_SENTENCE: &str = "\
## Component App
State:
    Let n be 0.
Actions:
    When the window resizes:
        Set n to n + 1.
View:
    <p>{n}</p>

## Main
Mount App.
";
    let mut page = mount_interactive_checked(ONE_SENTENCE);
    let handlers = page.handlers();
    assert_eq!(handlers.len(), 1);
    let handle = handlers[0];
    page.safepoint();
    assert_eq!(
        page.census(),
        0,
        "one registration holds the closure exactly once"
    );
    page.call_closure(handle, 0)
        .expect("a live, once-registered closure is callable");
    page.safepoint();
    assert_eq!(page.census(), 0, "the callback turn has settled");
    let live = page.heap_used();
    page.release_handle(handle);
    assert!(
        page.call_closure(handle, 0).is_err(),
        "a released callback must trap"
    );
    page.safepoint();
    assert_eq!(page.census(), 0, "the released registration owns nothing");
    assert!(
        page.heap_used() < live,
        "the one release reclaims closure storage"
    );
}

/// A closure that is NEVER registered anywhere (only ever `f(args)`-called directly, never handed to
/// a `CALLBACK`/`KEPT` host import) is never retained and never released — it stays exactly as
/// immortal as it was before S8, matching `RuntimeFn::ClosureRetain`'s doc.
#[test]
fn an_unregistered_closure_is_unaffected_by_reclaim() {
    // `sortBy`/`map`-style higher-order calls are not part of this crate's minimal interactive host,
    // so this uses a `Call` through a locally bound closure value instead — still never touching a
    // CALLBACK-marked host import, which is the property under test.
    const NEVER_REGISTERED: &str = "\
## Component App
State:
    Let n be 0.
Actions:
    When the window resizes:
        Let f be (x: Int) -> x + 1.
        Set n to f(n).
View:
    <p>{n}</p>

## Main
Mount App.
";
    let page = mount_interactive_checked(NEVER_REGISTERED);
    // Only the outer window-sentence closure is ever registered; the inner `f` closure is never
    // handed to any CALLBACK/KEPT import, so it is exactly as untouched by S8 as before.
    let handlers = page.handlers();
    assert_eq!(
        handlers.len(),
        1,
        "only the outer window-sentence registration exists"
    );
}

// ============================================================================================
// ★ S8 — the true count is EXACT, not merely plausible (`__logos_heap_census`). ★
//
// `__logos_heap_census` (check-mode only) independently recomputes, from the program's
// own globals, how many references point at every reachable counted object, and compares that
// number to the object's own stored count. `0` means every one agreed; anything else is the
// address of the first disagreement. This is the instrument the whole S8 safety story rests on:
// a stored count nobody re-derives from scratch is a number that LOOKS load-bearing and is not
// checked against anything.
//
// ★ THE CENSUS IS ONLY MEANINGFUL AFTER A SAFEPOINT. A fresh block's count starts at 1, credited
// to THE TURN (not any real, census-visible root) — `main`'s own execution IS a turn, so right
// after mounting, every freshly-created counted object's stored count is exactly one HIGHER than
// its real, root-visible holders until a safepoint reconciles that turn-credit away (S8's own
// design: "at any point where NO turn is in flight — post-`main()`, or after a safepoint has run").
// Every test here calls `page.safepoint()` once after mounting (and again after each click) before
// reading `census()` — checking the census WITHOUT that call is checking the wrong invariant, not
// finding a bug (this section's own first draft did exactly that and had to be corrected).
//
// All tests here mount the check-mode module (`mount_interactive_checked`) — a shipped page carries
// no census.
// ============================================================================================

/// Class 1 — no collections anywhere, only scalar `State:` globals — plus whatever the framework's
/// own bookkeeping adds (a stateful component gets an instance-key registry, `__KEYS_<C>`, which
/// IS a counted `Seq of Text`, per `lgx_lower.rs::keeps_instance_registry`). The census must still
/// agree exactly: this is not "zero counted globals", it is "every counted global — user's or the
/// framework's own — is exactly reconciled".
#[test]
fn a_page_with_only_scalar_globals_has_an_exact_census() {
    let src = "\
## Component App
State:
    Let n be 0.
Actions:
    To bump:
        Set n to n + 1.
View:
    <div><button id=\"a\" on-click={bump}>a</button></div>

## Main
Mount App.
";
    let mut page = mount_interactive_checked(src);
    page.safepoint();
    assert_eq!(page.census(), 0, "a page with only a scalar `State:` field must have an exact census once the mount turn is reconciled");
}

/// Class 2 — a `Seq of Int` (`ScalarHeader`) held directly by a global. One real holder (the
/// global) must equal the stored count of exactly 1.
#[test]
fn a_global_seq_of_int_has_an_exact_census() {
    let src = "\
## Component App
State:
    Let xs be [1, 2, 3].
Actions:
    To bump:
        Push 4 to xs.
View:
    <div><button id=\"a\" on-click={bump}>a</button></div>

## Main
Mount App.
";
    let mut page = mount_interactive_checked(src);
    page.safepoint();
    assert_eq!(
        page.census(),
        0,
        "one global holding one Seq must have a stored count of exactly 1"
    );
}

/// Two DIFFERENT globals holding the SAME Seq (via an assignment, `Let ys be xs.` at the value
/// level — `Set` copies the HANDLE for a `Let … be <existing global>` binding) must show a stored
/// count of 2 — the census recomputing 2 from two roots pointing at one object is the property
/// that would have caught the original "the count never decrements" bug from being silently wrong
/// in the OTHER direction (under-counting, not just over-counting).
#[test]
fn two_globals_holding_the_same_seq_have_an_exact_census() {
    // `ys` is aliased to `xs` from an ACTION, not from a second `State:` declaration — two
    // `Let`s of the same value directly in `State:` hits an unrelated, pre-existing wasm backend
    // limitation ("register reused across incompatible value types"), not anything S8 touches.
    let src = "\
## Component App
State:
    Let xs be [1, 2, 3].
    Let ys be a new Seq of Int.
Actions:
    To bump:
        Set ys to xs.
View:
    <div><button id=\"a\" on-click={bump}>a</button></div>

## Main
Mount App.
";
    let mut page = mount_interactive_checked(src);
    let button = page.listening("click")[0];
    page.dispatch(button, "click", "")
        .expect("the click aliases ys to xs");
    page.safepoint();
    assert_eq!(
        page.census(),
        0,
        "two globals genuinely holding the same Seq must both be counted"
    );
}

/// After a value is REPLACED (the old Seq released, a new one retained), the census must still
/// read exact — proving the release half of GlobalSet's true-count pair, not only the retain half.
/// Two safepoints: one after mount (reconciling `xs`'s own creation), one after the click that
/// replaces it (reconciling the NEW `[9, 9]` literal's own creation and releasing the old one).
#[test]
fn replacing_a_global_seq_keeps_an_exact_census() {
    let src = "\
## Component App
State:
    Let xs be [1, 2, 3].
Actions:
    To bump:
        Set xs to [9, 9].
View:
    <div><button id=\"a\" on-click={bump}>a</button></div>

## Main
Mount App.
";
    let mut page = mount_interactive_checked(src);
    page.safepoint();
    assert_eq!(
        page.census(),
        0,
        "the initial `xs` must have an exact census before it is ever replaced"
    );
    let button = page.listening("click")[0];
    page.dispatch(button, "click", "").expect("the click runs");
    page.safepoint();
    assert_eq!(
        page.census(),
        0,
        "replacing a global's Seq must release the old one and retain the new one exactly"
    );
}

/// A `SeqSeqInt` (a matrix): the outer Seq is `HandleHeader`, each row a `ScalarHeader` Seq the
/// census must recurse ONE level into. Both the outer container and every row must be exactly
/// accounted for.
#[test]
fn a_matrix_global_has_an_exact_census() {
    let src = "\
## Component App
State:
    Let rows be [[1, 2], [3, 4]].
Actions:
    To bump:
        Push [5, 6] to rows.
View:
    <div><button id=\"a\" on-click={bump}>a</button></div>

## Main
Mount App.
";
    let mut page = mount_interactive_checked(src);
    page.safepoint();
    assert_eq!(
        page.census(),
        0,
        "a matrix's outer Seq and every row must have an exact census after mount"
    );
    let button = page.listening("click")[0];
    page.dispatch(button, "click", "").expect("the click runs");
    page.safepoint();
    assert_eq!(
        page.census(),
        0,
        "pushing a new row must retain it exactly once"
    );
}

// ================================================================================================
// FETCH-COMPLETION JOURNAL — a second fetch completing through a named action ALREADY completed
// through must still journal its own turn (heap3.md's queue, routed from safety2.md's "Repro for
// heap3 — bug 1"). `examples/fullstack_todo` has three independent `Post … then loaded` call sites
// (`add`, `toggle`, `remove`); on the wasm backend only the FIRST one's completion ever reached
// `env.dom_note_turn` in the real browser tier — ledgered as `browser:journal @ 6` on
// `fullstack_todo_agrees_on_the_tree_walker_the_vm_and_chrome_or_is_ledgered`'s statement 6 (the
// SECOND `Post … then loaded` the corpus exercises, `add`'s own having already completed).
//
// This drives the WASM AOT backend directly (`mount_interactive`, the shipped module; this bug
// predates and does not depend on S8) through TWO INDEPENDENT actions that both
// `Post … then <the same action>`, mocking `fetchSend`/`responseStatus`/`responseBody` exactly as
// `runtime.js` implements them (`tests/common/wasm_host.rs`'s `dom_fetch_send`/`complete_fetch`) —
// no ChromeLauncher needed to reproduce a wasm-codegen bug.
// ================================================================================================

/// Two buttons, each posting to a DIFFERENT url, both completing through the SAME named action —
/// the exact shape of `fullstack_todo`'s `add`/`toggle` (different Posts, one shared `loaded`).
const TWO_FETCHES_ONE_COMPLETION: &str = "\
## A Note has:
    a text, which is Text.

## Component App
State:
    Let n be 0.
Actions:
    To bumpA:
        Post \"\" to \"/a\" as Seq of Note, then done.
    To bumpB:
        Post \"\" to \"/b\" as Seq of Note, then done.
    To done (r: Result of Seq of Note and HttpError):
        Set n to n + 1.
View:
    <main>
        <p id=\"out\">{n}</p>
        <button id=\"a\" on-click={bumpA}>a</button>
        <button id=\"b\" on-click={bumpB}>b</button>
    </main>

## Main
Mount App.
";

/// Click the page's `which`-th button as it stands NOW — a turn may have repainted it under a new id
/// since the last click — through the node dispatcher, which must answer it.
fn click_button(page: &mut common::wasm_host::InteractivePage, which: usize, action: &str) {
    let button = page.listening("click")[which];
    let answered = page
        .dispatch(button, "click", "")
        .unwrap_or_else(|e| panic!("{action}'s click runs: {e}"));
    assert!(answered, "{action}'s button answers its click");
}

/// The property itself: a SECOND fetch completing through a named action a FIRST fetch already
/// completed through must still journal its own turn. Every lowered `__act_` notes its turn, the
/// click that sends a request included, so the journal reads each click and then its completion,
/// in the order they ran.
#[test]
fn a_second_fetch_completing_through_an_already_used_action_still_journals_its_turn() {
    let mut page = mount_interactive(TWO_FETCHES_ONE_COMPLETION);
    let buttons = page.listening("click");
    assert_eq!(
        buttons.len(),
        2,
        "two `on-click` sites must register two listeners: {buttons:?}"
    );
    click_button(&mut page, 0, "bumpA");
    let id_a = page
        .fetch_request("/a")
        .expect("bumpA must have sent a fetch to /a");
    page.complete_fetch(id_a, 200, "[]")
        .expect("the FIRST completion must not trap");
    assert_eq!(
        page.turns(),
        vec!["bumpA", "done"],
        "the click journals its turn, and so does the first completion"
    );

    click_button(&mut page, 1, "bumpB");
    let id_b = page
        .fetch_request("/b")
        .expect("bumpB must have sent a fetch to /b");
    assert_ne!(
        id_a, id_b,
        "two independent fetches must not share a request id"
    );
    page.complete_fetch(id_b, 200, "[]")
        .expect("the SECOND completion must not trap");
    assert_eq!(
        page.turns(),
        vec!["bumpA", "done", "bumpB", "done"],
        "the second fetch's completion, through the SAME named action as the first, must journal \
         its own turn too — a missing second `done` here is bug 1 (safety2.md's 'Repro for heap3')"
    );
}

/// The narrower control: does the SAME action, called via TWO fetches BOTH already in flight
/// BEFORE either completes, still journal both? Distinguishes "sharing only shows up once a prior
/// completion has fully settled" from "sharing is present from the moment both are registered".
#[test]
fn two_concurrently_pending_fetches_to_the_same_completion_both_journal_when_answered() {
    let mut page = mount_interactive(TWO_FETCHES_ONE_COMPLETION);
    click_button(&mut page, 0, "bumpA");
    click_button(&mut page, 1, "bumpB");
    let ids = page.pending_fetch_ids();
    assert_eq!(
        ids.len(),
        2,
        "both fetches must be pending at once: {ids:?}"
    );
    assert_eq!(
        page.turns(),
        vec!["bumpA", "bumpB"],
        "each click journals its own turn"
    );
    page.complete_fetch(ids[0], 200, "[]")
        .expect("the first-sent completion must not trap");
    page.complete_fetch(ids[1], 200, "[]")
        .expect("the second-sent completion must not trap");
    assert_eq!(
        page.turns(),
        vec!["bumpA", "bumpB", "done", "done"],
        "both concurrently-pending fetches must journal their own turn once answered"
    );
}
