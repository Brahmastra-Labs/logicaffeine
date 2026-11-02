//! ★ LOCK — a RECORD READ OUT OF A LIST is readable on wasm. ★
//!
//! `Repeat for t in todos:` and `Show t's text.` where `todos` is a `Seq of Todo` is ordinary
//! LOGOS — it is how every program that holds a table of anything reads one row. It ran on the
//! tree-walker and the VM and was REFUSED by the WebAssembly backend with "field access with no
//! static slot", a message about the backend's internals rather than about anything the author
//! wrote.
//!
//! WHY IT HAPPENED. This backend resolves a field to a static SLOT by tracing where the record
//! was BUILT, following `NewStruct`, `StructInsert`, `GetField` and aliasing `Move`s. A loop
//! variable arrives through `IterNext`, which that walk did not follow — so the element's struct
//! identity was lost at the loop boundary and the field had nowhere to resolve to. The machinery
//! to fix it already existed on both sides: `iter_pairing` already pairs an `IterNext` with its
//! iterable to give the loop variable its KIND, and a per-register `FieldLayout` is already
//! carried for call results and parameters. What was missing was the element LAYOUT of a list.
//!
//! WHAT IT COST, which is why this file is plain LOGOS rather than a frontend test: a page
//! showing a list of records — the single most common thing a page does — could not be built for
//! a browser at all. A gap in the general language surfaced as "the todo app does not compile",
//! and a lock on the page would have pinned the symptom while leaving the cause reachable from
//! any ordinary function.

#![cfg(feature = "wasm-jit")]

use logicaffeine_compile::compile::{compile_to_wasm, tw_outcome, vm_outcome};

#[path = "compile_common/mod.rs"]
mod common;

/// Compile, instantiate, and run — a module that compiles but does not VALIDATE is not a working
/// program, and the difference has bitten this campaign before.
fn wasm_runs(src: &str) -> String {
    let module =
        compile_to_wasm(src).unwrap_or_else(|e| panic!("must compile to wasm: {e:?}\n{src}"));
    let engine = wasmi::Engine::default();
    let m = wasmi::Module::new(&engine, &module[..]).unwrap_or_else(|e| panic!("valid wasm: {e}"));
    let out = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
    let mut linker = wasmi::Linker::<std::rc::Rc<std::cell::RefCell<Vec<String>>>>::new(&engine);
    common::wasm_host::link_value_imports(&mut linker);
    common::wasm_host::link_exact_show(&mut linker, |out, line| out.borrow_mut().push(line));
    // Every program that CAN fail declares `logos_rt_fail`, and a module importing a
    // function its host does not define FAILS TO INSTANTIATE — silently, with no
    // compile error anywhere. This host does not care why; it must simply be here.
    linker
        .func_wrap(
            "env",
            "logos_rt_fail",
            |_c: wasmi::Caller<'_, _>, _h: i32| {},
        )
        .unwrap();
    linker
        .func_wrap(
            "env",
            "logos_rt_fail_at",
            |_c: wasmi::Caller<'_, _>, _code: i32, _v: i64| {},
        )
        .unwrap();
    linker
        .func_wrap(
            "env",
            "print_i64",
            |c: wasmi::Caller<'_, std::rc::Rc<std::cell::RefCell<Vec<String>>>>, v: i64| {
                c.data().borrow_mut().push(v.to_string());
            },
        )
        .unwrap();
    // A Bool prints through its own import — a harness that wires only ints and text fails to
    // INSTANTIATE the moment a program shows one, which reads like a compiler refusal and is not.
    linker
        .func_wrap(
            "env",
            "print_bool",
            |c: wasmi::Caller<'_, std::rc::Rc<std::cell::RefCell<Vec<String>>>>, v: i32| {
                c.data().borrow_mut().push(if v != 0 {
                    "true".to_string()
                } else {
                    "false".to_string()
                });
            },
        )
        .unwrap();
    linker
        .func_wrap(
            "env",
            "print_text",
            |c: wasmi::Caller<'_, std::rc::Rc<std::cell::RefCell<Vec<String>>>>, h: i32| {
                let mem = c.get_export("memory").unwrap().into_memory().unwrap();
                let data = mem.data(&c);
                let len = i32::from_le_bytes(data[h as usize..h as usize + 4].try_into().unwrap())
                    as usize;
                let ptr =
                    i32::from_le_bytes(data[h as usize + 8..h as usize + 12].try_into().unwrap())
                        as usize;
                let s = String::from_utf8_lossy(&data[ptr..ptr + len]).into_owned();
                c.data().borrow_mut().push(s);
            },
        )
        .unwrap();
    let mut store = wasmi::Store::new(&engine, out.clone());
    crate::common::wasm_host::link_unmodelled_imports(&mut linker);
    let inst = linker
        .instantiate(&mut store, &m)
        .unwrap()
        .start(&mut store)
        .unwrap();
    inst.get_typed_func::<(), ()>(&store, "main")
        .unwrap()
        .call(&mut store, ())
        .unwrap_or_else(|e| panic!("main runs: {e}\n{src}"));
    let out = out.borrow().join("\n");
    out
}

/// Every tier, and they must agree — the wasm tier is the one that was refusing, and the other
/// two are what "correct" means.
fn all_tiers(src: &str) -> String {
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "tree-walker:\n{src}");
    let vm = vm_outcome(src);
    assert_eq!(vm.error, None, "VM:\n{src}");
    assert_eq!(
        tw.output.trim_end(),
        vm.output.trim_end(),
        "tree-walker vs VM:\n{src}"
    );
    let wasm = wasm_runs(src);
    assert_eq!(
        tw.output.trim_end(),
        wasm.trim_end(),
        "tree-walker vs wasm:\n{src}"
    );
    wasm
}

const TODO: &str = "## A Todo has:\n    an id, which is Int.\n    a text, which is Text.\n    a done, which is Bool.\n\n";

#[test]
fn a_field_of_a_loop_variable_reads_on_every_tier() {
    // ★ THE HEADLINE. Nothing exotic: a list of records, a loop, a field.
    let src = format!(
        "{TODO}## Main\n\
         Let todos be [a new Todo with id 1 and text \"milk\" and done false].\n\
         Repeat for t in todos:\n\
        \x20   Show t's text.\n"
    );
    assert_eq!(all_tiers(&src), "milk");
}

#[test]
fn every_field_of_the_row_reads_not_only_the_first() {
    // A layout resolved for slot 0 and nothing else would pass the test above and fail on the
    // second column — which is the shape of a table.
    let src = format!(
        "{TODO}## Main\n\
         Let todos be [a new Todo with id 7 and text \"milk\" and done true].\n\
         Repeat for t in todos:\n\
        \x20   Show t's id.\n\
        \x20   Show t's text.\n\
        \x20   Show t's done.\n"
    );
    assert_eq!(all_tiers(&src), "7\nmilk\ntrue");
}

#[test]
fn several_rows_each_read_their_own_values() {
    // One row cannot prove the loop reads the ELEMENT rather than a remembered register.
    let src = format!(
        "{TODO}## Main\n\
         Let todos be [a new Todo with id 1 and text \"a\" and done false, \
         a new Todo with id 2 and text \"b\" and done true].\n\
         Repeat for t in todos:\n\
        \x20   Show t's id.\n\
        \x20   Show t's text.\n"
    );
    assert_eq!(all_tiers(&src), "1\na\n2\nb");
}

#[test]
fn a_loop_inside_a_function_reads_the_field_too() {
    // The failing case in the real program was a FUNCTION counting a list — a different region
    // from Main, which is exactly where the global-layout gap lived too.
    let src = format!(
        "{TODO}## To countUndone (todos: Seq of Todo) -> Int:\n\
        \x20   Let mutable n be 0.\n\
        \x20   Repeat for t in todos:\n\
        \x20       If t's done is equal to false:\n\
        \x20           Set n to n + 1.\n\
        \x20   Return n.\n\
         \n\
         ## Main\n\
         Let todos be [a new Todo with id 1 and text \"a\" and done false, \
         a new Todo with id 2 and text \"b\" and done true].\n\
         Show countUndone(todos).\n"
    );
    assert_eq!(all_tiers(&src), "1");
}

#[test]
fn a_field_read_builds_a_new_record_of_the_same_shape() {
    // The `toggle` shape every list-editing program has: read a row's fields, build a new row
    // from them. It reads AND writes the layout, so a half-resolved one shows up here.
    let src = format!(
        "{TODO}## Main\n\
         Let todos be [a new Todo with id 1 and text \"a\" and done false].\n\
         Let mutable out be a new Seq of Todo.\n\
         Repeat for t in todos:\n\
        \x20   Push (a new Todo with id t's id and text t's text and done (not t's done)) to out.\n\
         Repeat for u in out:\n\
        \x20   Show u's done.\n"
    );
    assert_eq!(all_tiers(&src), "true");
}

#[test]
fn an_empty_list_still_compiles_and_runs() {
    // The loop body never executes, so the field access is unreachable — a backend that resolved
    // the layout only from an executed iteration would refuse a program that is perfectly fine.
    let src = format!(
        "{TODO}## Main\n\
         Let todos be a new Seq of Todo.\n\
         Repeat for t in todos:\n\
        \x20   Show t's text.\n\
         Show \"done\".\n"
    );
    assert_eq!(all_tiers(&src), "done");
}

#[test]
fn a_list_grown_by_push_reads_its_rows_too() {
    // A list built by `Push` rather than by a literal is the other way every real program makes
    // one, and the element layout has to come from there as well.
    let src = format!(
        "{TODO}## Main\n\
         Let mutable todos be a new Seq of Todo.\n\
         Push (a new Todo with id 3 and text \"c\" and done false) to todos.\n\
         Repeat for t in todos:\n\
        \x20   Show t's text.\n"
    );
    assert_eq!(all_tiers(&src), "c");
}

#[test]
fn a_plain_list_of_text_still_works() {
    // The change must not disturb the case that already worked — a loop over scalars resolves no
    // layout at all and must keep resolving none.
    let src = "## Main\nLet xs be [\"a\", \"b\"].\nRepeat for x in xs:\n    Show x.\n";
    assert_eq!(all_tiers(src), "a\nb");
}

// ---------------------------------------------------------------------------------------------
// ★ THE NEXT GAP, written down where it is reachable: a RECORD carried by a `Result`.
// ---------------------------------------------------------------------------------------------

/// A field of a record bound out of a `Result`'s payload does not resolve on wasm.
///
/// ★ REPRODUCED IN PLAIN LOGOS, with no frontend anywhere near it — which is why it lives here.
/// It was found through the todo page's `When Err (e): Set error to e's message.`, and the page
/// was a red herring: any function taking a `Result of T and <record>` and reading a field of the
/// bound payload is refused with "field access with no static slot".
///
/// ★ THE CAUSE, located precisely and since CLOSED: `BoundaryType` — the bytecode's model of a
/// declared type at a region boundary — had no `Result` variant at all. So a `Result` parameter
/// carried NO shape, the `BindArm` that binds its payload resolved no layout, and the field had
/// nowhere to land. Both halves landed: `Result` is a boundary type carrying its Ok and Err types,
/// and an enum parameter's shape carries each variant's declared payload TYPE rather than a name
/// to look up or a projected kind (a record payload needs field slots, not just "it is a struct").
/// The full treatment, with the edge cases, is `wasm_result_payload_lock`; this stays as the
/// marker for where the gap was found.
///
/// ★ IT ONLY BITES WHEN THE FUNCTION IS CALLED. An uncalled action is dead code and never
/// lowered, which is exactly why an isolated reproduction of the page's `loaded` action compiled
/// happily and the page did not — the fetch that calls it is what drags it into the lowering.
/// Worth remembering: "it compiles in isolation" can mean "it was never compiled".
///
/// It was `#[ignore]`d with its reason in the open rather than deleted or quietly passing, and it
/// un-ignored itself the day the boundary type learned to carry a Result — which is what an honest
/// not-yet gap is for.
#[test]
fn a_record_carried_by_a_result_reads_its_fields() {
    let src = "## An E has:\n\
      \x20   a message, which is Text.\n\
       \n\
       ## To describe (r: Result of Int and E) -> Text:\n\
      \x20   Let mutable out be \"?\".\n\
      \x20   Inspect r:\n\
      \x20       When Ok (v):\n\
      \x20           Set out to \"ok\".\n\
      \x20       When Err (e):\n\
      \x20           Set out to e's message.\n\
      \x20   Return out.\n\
       \n\
       ## Main\n\
       Show describe(a new Err with error (a new E with message \"bad\")).\n";
    assert_eq!(all_tiers(src), "bad");
}
