//! ★ PIT-OF-SUCCESS LOCK — a test may not invent its own answer to a fixed question. ★
//!
//! A wasm test must supply a host, and for most imports that is the test doing its job: which node
//! id `dom_create_element` returns, whether `dom_set_text` records into an op-log, what a stubbed
//! fetch answers. Those are decisions, and they belong to the test.
//!
//! `fmt_i64_into(buf, val) -> i32` is not a decision. It writes the decimal of `val` into `buf` and
//! returns THE BYTE LENGTH. A test can only get that right or wrong.
//!
//! ★★★ ONE GOT IT WRONG AND IT COST A CAMPAIGN TASK. `wasm_memory_bound_lock` returned `buf` — the
//! POINTER — and since the pointer grows with the heap, it landed in a Text header as `len`, the
//! next concat allocated `len_a + len_b` (a heap-sized buffer), and memory doubled every turn. The
//! file recorded, in good faith and with real measurements, that "every LGX page in a real browser
//! stops responding within a few seconds of use". The product was fine. That false diagnosis spent a
//! task, an unsound mark/release experiment, and a design for a two-region allocator nothing needed.
//!
//! ★ AND "BE MORE CAREFUL" IS NOT THE FIX, because of WHY it survived: the broken stub was in the
//! one test that never checked its output. Everywhere else a formatter returning nonsense produces
//! visibly wrong text and fails at once. A test that asks only "did it survive?" cannot see its own
//! oracle lying — so the protection has to be structural, not attentional.
//!
//! Two halves, and each is useless alone: this file refuses a hand-written copy (below), and
//! `common::wasm_host` is the one implementation, derived from the product's own display code rather
//! than describing it a second time.

use std::collections::BTreeMap;
use std::path::Path;

// ★ THE SCANNER BELOW IS DELIBERATELY UNGATED. It needs no wasm engine, so it runs in every pass
// rather than only the `wasm-jit` one — a guard that only fires in the narrower run is a guard
// somebody adds a violation past on an ordinary afternoon.
#[cfg(feature = "wasm-jit")]
#[path = "compile_common/mod.rs"]
mod common;

/// The imports whose behaviour the ABI fixes completely. Adding one here means adding it to
/// `crate::common::wasm_host::link_value_imports` — the list and the implementation are the same decision.
const SHARED: &[&str] = &[
    "fmt_i64_into",
    "fmt_f64_into",
    "fmt_bool_into",
    "fmt_f64_prec_into",
    "pow_ff",
    "pow_fi",
];

/// The shared host itself, and this file, name these imports legitimately.
const ALLOWED_FILES: &[&str] = &["mod.rs", "wasm_host.rs", "wasm_host_stub_lock.rs"];

/// Drop line comments, so the scan reads what a test DOES rather than what it says about itself.
fn strip_comments(src: &str) -> String {
    src.lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn test_sources() -> Vec<(String, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("the tests directory is readable") {
            let path = entry.expect("a readable entry").path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let name = path.file_name().unwrap().to_string_lossy().into_owned();
                if !ALLOWED_FILES.contains(&name.as_str()) {
                    let body = std::fs::read_to_string(&path).expect("readable");
                    out.push((name, strip_comments(&body)));
                }
            }
        }
    }
    out
}

#[test]
fn no_test_hand_writes_an_import_whose_answer_is_fixed() {
    let mut offenders: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for (name, body) in test_sources() {
        for import in SHARED {
            // ★ CODE, NOT PROSE. A doc comment is where these get EXPLAINED — the header of
            // `wasm_memory_bound_lock` quotes the exact broken registration as the thing that went
            // wrong, and a scanner that flagged its own best evidence would get that comment
            // deleted, which is the opposite of what this file is for.
            if body.contains(&format!("func_wrap(\"env\", \"{import}\""))
                || body.contains(&format!("func_wrap(\"env\",\"{import}\""))
            {
                offenders.entry(import).or_default().push(name.clone());
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "these tests hand-write an import whose behaviour the ABI already fixes:\n{offenders:#?}\n\n\
         Call `crate::common::wasm_host::link_value_imports::<YourData>(&mut linker)` instead. A second \
         implementation of a fixed behaviour is a second thing that can be wrong, and the last one \
         that was wrong reported a memory catastrophe that did not exist."
    );
}

/// Run `src` in a real module whose ONLY value imports are the shared ones, and collect its `Show`
/// output. Anything the shared host gets wrong shows up as wrong text — which is the whole point.
#[cfg(feature = "wasm-jit")]
fn wasm_output(src: &str) -> String {
    use std::cell::RefCell;
    use std::rc::Rc;
    type Out = Rc<RefCell<Vec<String>>>;

    let module = logicaffeine_compile::compile::compile_to_wasm(src)
        .unwrap_or_else(|e| panic!("must compile: {e:?}\n{src}"));
    let engine = wasmi::Engine::default();
    let m = wasmi::Module::new(&engine, &module[..]).expect("valid wasm");
    let out: Out = Rc::new(RefCell::new(Vec::new()));
    let mut l = wasmi::Linker::<Out>::new(&engine);
    crate::common::wasm_host::link_value_imports::<Out>(&mut l);
    // ★ AND THE FAILURE HOOKS. A program that indexes past the end or reads a key a map does not
    // hold imports these, and a module importing a function its host lacks does not lose a feature
    // — it fails to INSTANTIATE. This helper is this file's own, so it has to ask for them like any
    // other host; that they are OPTIONAL rather than enforced is the point made in `wasm_host.rs`.
    crate::common::wasm_host::link_failure_hooks::<Out>(&mut l);
    l.func_wrap("env", "print_i64", |c: wasmi::Caller<'_, Out>, v: i64| {
        c.data().borrow_mut().push(v.to_string());
    })
    .unwrap();
    l.func_wrap("env", "print_bool", |c: wasmi::Caller<'_, Out>, v: i32| {
        c.data().borrow_mut().push(if v != 0 {
            "true".into()
        } else {
            "false".into()
        });
    })
    .unwrap();
    l.func_wrap("env", "print_text", |c: wasmi::Caller<'_, Out>, h: i32| {
        let text = crate::common::wasm_host::read_text(&c, h);
        c.data().borrow_mut().push(text);
    })
    .unwrap();
    let mut store = wasmi::Store::new(&engine, out.clone());
    crate::common::wasm_host::link_unmodelled_imports(&mut l);
    let inst = l
        .instantiate(&mut store, &m)
        .expect("instantiates")
        .start(&mut store)
        .unwrap();
    inst.get_typed_func::<(), ()>(&store, "main")
        .unwrap()
        .call(&mut store, ())
        .expect("runs");
    let joined = out.borrow().join("\n");
    joined
}

/// ★★ THE LOAD-BEARING TEST, and the one that would have caught the original bug on the day it was
/// written. The shared host is only correct if what a compiled module DISPLAYS through it is what
/// LOGOS displays — so it is held against the reference tier, not against a string I typed here.
///
/// A stub returning the buffer POINTER instead of the length fails this instantly and loudly, on
/// every case, because the text comes out as garbage of the wrong length. That is the property
/// `wasm_memory_bound_lock` could not check, being a test that only asked whether the page survived.
#[cfg(feature = "wasm-jit")]
#[test]
fn the_shared_host_displays_exactly_what_the_reference_tier_displays() {
    for src in [
        // An ordinary Int through an interpolation — the exact path that was broken.
        "## Main\nLet n be 42.\nShow \"n is {n}\".\n",
        // ★ NEGATIVE and MULTI-DIGIT: a length-vs-pointer confusion survives a single digit by luck
        // more often than it survives a sign and twenty digits.
        "## Main\nLet n be 0 - 7.\nShow \"{n}\".\n",
        "## Main\nLet n be 9223372036854775807.\nShow \"{n}\".\n",
        // A Float, whose display is shortest-round-trip and NOT Rust's default `{}` for every value.
        "## Main\nLet r be 1.5.\nShow \"{r}\".\n",
        "## Main\nLet r be 2.0.\nShow \"{r}\".\n",
        // A Bool.
        "## Main\nLet b be true.\nShow \"{b}\".\n",
        // ★ MULTI-BYTE text either side of the hole: a byte length and a character length differ
        // here, and a host confusing them truncates or overruns.
        "## Main\nLet n be 3.\nShow \"héllo {n} wörld\".\n",
        // Several holes in one string, so a formatter that clobbered a shared scratch buffer between
        // calls could not pass by rendering one value correctly.
        "## Main\nLet a be 1.\nLet b be 22.\nLet c be 333.\nShow \"{a}-{b}-{c}\".\n",
    ] {
        let reference = logicaffeine_compile::compile::tw_outcome(src);
        assert_eq!(
            reference.error, None,
            "the reference tier must run it:\n{src}"
        );
        assert_eq!(
            wasm_output(src),
            reference.output.trim_end(),
            "the shared host displayed something the reference tier does not:\n{src}"
        );
    }
}

/// A name in `SHARED` with no implementation would refuse every test a stub for an import nothing
/// supplies — turning a protection into a wall. Asserted by BUILDING a module that imports each one
/// and instantiating it, so a definition that exists at the wrong SIGNATURE fails here too.
#[cfg(feature = "wasm-jit")]
#[test]
fn every_shared_name_is_actually_implemented() {
    // Each source below forces the compiler to import the named function.
    for (import, src) in [
        ("fmt_i64_into", "## Main\nLet n be 1.\nShow \"{n}\".\n"),
        ("fmt_f64_into", "## Main\nLet r be 1.5.\nShow \"{r}\".\n"),
        ("fmt_bool_into", "## Main\nLet b be true.\nShow \"{b}\".\n"),
    ] {
        assert!(
            SHARED.contains(&import),
            "`{import}` is exercised here but missing from SHARED, so a test may still hand-write it"
        );
        // Instantiating is the assertion: a missing or wrongly-typed import fails to link.
        let _ = wasm_output(src);
    }
}
