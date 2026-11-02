//! ★ LOCK — a record held in a module-level binding is readable from a FUNCTION on wasm. ★
//!
//! `Let counter be a new Counter with n 1.` at the top level and `Show counter's n.` inside a
//! function is ordinary LOGOS — a shared record a routine touches. It compiled everywhere and was
//! REFUSED by the WebAssembly backend with "field access with no static slot", which is a message
//! about the backend's internals rather than anything the author wrote.
//!
//! WHY IT HAPPENED, and why the fix belongs where it is: this backend resolves a field to a static
//! SLOT by tracing where the record was built. Main builds it; a function only READS the global,
//! so that region has no defining register and no layout — the exact position a struct PARAMETER
//! is in, which the backend already solves by threading the DECLARED type in. Globals now get the
//! same treatment, from the same whole-program pass that already types them.
//!
//! WHAT IT COST, which is why this file exists in plain LOGOS rather than beside the frontend: a
//! `## Store` IS a record global read by every action, so NO STORE OF ANY KIND reached a browser.
//! A gap in the general language showed up as a whole frontend feature missing, and a lock on the
//! store would have pinned the symptom while leaving the cause reachable from ordinary code.

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

#[test]
fn a_function_reads_a_record_global() {
    let out = all_tiers(
        "## A Counter has:\n\
        \x20   a n, which is Int.\n\
         \n\
         ## To report:\n\
        \x20   Show counter's n.\n\
         \n\
         ## Main\n\
         Let mutable counter be a new Counter with n 7.\n\
         Call report.\n",
    );
    assert_eq!(out.trim(), "7");
}

#[test]
fn a_function_writes_a_record_global_and_main_sees_it() {
    // The write half. A counter a routine bumps is the most ordinary shape there is, and it is
    // also what a `## Store` action does to its store.
    let out = all_tiers(
        "## A Counter has:\n\
        \x20   a n, which is Int.\n\
         \n\
         ## To bump:\n\
        \x20   Set counter's n to counter's n + 1.\n\
         \n\
         ## Main\n\
         Let mutable counter be a new Counter with n 1.\n\
         Call bump.\n\
         Call bump.\n\
         Show counter's n.\n",
    );
    assert_eq!(out.trim(), "3");
}

#[test]
fn every_field_kind_resolves_not_just_the_first() {
    // A slot is a POSITION, so resolving one field and not another is a real failure mode — and a
    // fixture with one field cannot see it. Text sits after Int here deliberately: the two load at
    // different widths, so a layout that resolved positions but not KINDS would read one wrong.
    let out = all_tiers(
        "## A Session has:\n\
        \x20   a id, which is Int.\n\
        \x20   a name, which is Text.\n\
        \x20   a hits, which is Int.\n\
         \n\
         ## To describe:\n\
        \x20   Show session's id.\n\
        \x20   Show session's name.\n\
        \x20   Show session's hits.\n\
         \n\
         ## Main\n\
         Let mutable session be a new Session with id 3 and name \"ada\" and hits 12.\n\
         Call describe.\n",
    );
    assert_eq!(out.trim(), "3\nada\n12");
}

#[test]
fn a_write_in_one_function_is_seen_by_another() {
    // The global is ONE record, not a copy per function. Two functions sharing it is the whole
    // reason a program reaches for a module-level binding.
    let out = all_tiers(
        "## A Counter has:\n\
        \x20   a n, which is Int.\n\
         \n\
         ## To bump:\n\
        \x20   Set counter's n to counter's n + 1.\n\
         \n\
         ## To report:\n\
        \x20   Show counter's n.\n\
         \n\
         ## Main\n\
         Let mutable counter be a new Counter with n 0.\n\
         Call bump.\n\
         Call report.\n",
    );
    assert_eq!(out.trim(), "1");
}

#[test]
fn a_nested_record_global_resolves_through_both_levels() {
    // A field that is itself a record: the outer access must carry the inner layout, or the second
    // `'s` has nothing to resolve against. This is the case a declared-type resolution gets right
    // and a value-register trace cannot.
    let out = all_tiers(
        "## A Point has:\n\
        \x20   a x, which is Int.\n\
         \n\
         ## A Box has:\n\
        \x20   a corner, which is Point.\n\
         \n\
         ## To report:\n\
        \x20   Show boxed's corner's x.\n\
         \n\
         ## Main\n\
         Let mutable boxed be a new Box with corner (a new Point with x 5).\n\
         Call report.\n",
    );
    assert_eq!(out.trim(), "5");
}

#[test]
fn a_record_global_read_inside_a_loop_in_a_function_resolves() {
    // Inside a loop the read sits in a different basic block from the function's entry, which is
    // where a resolution that only worked on straight-line code would come apart.
    let out = all_tiers(
        "## A Counter has:\n\
        \x20   a step, which is Int.\n\
         \n\
         ## To run:\n\
        \x20   Let mutable total be 0.\n\
        \x20   Let mutable i be 0.\n\
        \x20   While i is less than 4:\n\
        \x20       Set total to total + counter's step.\n\
        \x20       Set i to i + 1.\n\
        \x20   Show total.\n\
         \n\
         ## Main\n\
         Let mutable counter be a new Counter with step 3.\n\
         Call run.\n",
    );
    assert_eq!(out.trim(), "12");
}

#[test]
fn a_record_global_passed_to_another_function_still_resolves() {
    // Reading the global and handing it on as an ARGUMENT: the callee resolves it as a parameter
    // (the path that always worked), and the two resolutions must agree about the layout.
    let out = all_tiers(
        "## A Counter has:\n\
        \x20   a n, which is Int.\n\
         \n\
         ## To display (c: Counter):\n\
        \x20   Show c's n.\n\
         \n\
         ## To report:\n\
        \x20   Call display with counter.\n\
         \n\
         ## Main\n\
         Let mutable counter be a new Counter with n 9.\n\
         Call report.\n",
    );
    assert_eq!(out.trim(), "9");
}

#[test]
fn the_top_level_read_still_works() {
    // The path that was never broken, kept beside the fix: resolving a global's layout must not
    // disturb the local resolution Main already had.
    let out = all_tiers(
        "## A Counter has:\n\
        \x20   a n, which is Int.\n\
         \n\
         ## Main\n\
         Let mutable counter be a new Counter with n 4.\n\
         Show counter's n.\n",
    );
    assert_eq!(out.trim(), "4");
}
