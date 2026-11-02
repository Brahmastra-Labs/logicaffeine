//! ★ LOCK — a `Result` carries a REAL PAYLOAD across a boundary on wasm. ★
//!
//! `Result of T and E` is how a LOGOS function says "this may not have worked", and reading the
//! thing it carried — `When Err (e): … e's message` — is the entire point of answering one. On the
//! WebAssembly backend that read was refused with "field access with no static slot": a message
//! about the backend's internals rather than about anything the author wrote.
//!
//! WHY IT HAPPENED, and why the fix is a MODELLING fix rather than a patch. `BoundaryType` is the
//! bytecode's model of a declared type where a value crosses a region boundary (a parameter, a
//! return, a field). It could describe an `Int`, a `Seq`, a `Map`, a struct and a NAMED enum — and
//! had nothing at all to say about a `Result`. A parameter typed `Result of Int and E` therefore
//! carried NO shape, the `BindArm` binding its payload resolved no layout, and the field had
//! nowhere to land. A named enum with the identical shape (`## An Answer is one of: A Bad (p: P).`)
//! worked perfectly, which is what said the machinery was fine and the MODEL was short.
//!
//! ★ THE ANONYMITY IS THE WHOLE DIFFICULTY. A named enum's payload layout is recovered by looking
//! its name up in the program's `enum_types` table. `Result of Int and E` has no name — no
//! `## An X is one of:` ever declared it, and two instantiations in one program (`Result of Int and
//! E` and `Result of Text and F`) are different types that would collide under any single name. So
//! the shape is carried STRUCTURALLY: the boundary type holds its Ok and Err types, and the enum
//! shape a parameter seeds carries each variant's payload BOUNDARY TYPES rather than a name to look
//! up or a projected `Kind`. A `Kind` alone cannot do it — a record payload needs FIELD SLOTS, and
//! `Kind::Struct` says only "it is a struct".
//!
//! WHAT IT COST: every typed fetch. `Get "/api/todos" as Seq of Todo, then loaded.` hands its
//! action a `Result of Seq of Todo and HttpError`, so a page that loads its own data could not be
//! compiled for a browser. As with the list gap this file's neighbour holds, the page was the
//! symptom and ordinary LOGOS was the cause — so the lock is written in plain LOGOS, with no
//! frontend anywhere near it.

#![cfg(feature = "wasm-jit")]

use logicaffeine_compile::compile::{compile_to_wasm, tw_outcome, vm_outcome};

#[path = "compile_common/mod.rs"]
mod common;

/// Compile, instantiate, and RUN. A module that compiles and does not validate is not a working
/// program, and a module that validates and computes the wrong answer is worse than one that
/// refuses — a payload read from the wrong slot is silent.
fn wasm_runs(src: &str) -> String {
    let module =
        compile_to_wasm(src).unwrap_or_else(|e| panic!("must compile to wasm: {e:?}\n{src}"));
    let engine = wasmi::Engine::default();
    let m = wasmi::Module::new(&engine, &module[..]).unwrap_or_else(|e| panic!("valid wasm: {e}"));
    let out = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
    let mut linker = wasmi::Linker::<std::rc::Rc<std::cell::RefCell<Vec<String>>>>::new(&engine);
    crate::common::wasm_host::link_value_imports(&mut linker);
    crate::common::wasm_host::link_exact_show(&mut linker, |out, line| out.borrow_mut().push(line));
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

/// Every tier, and they must agree. The wasm tier is the one that was refusing; the other two are
/// what "correct" means.
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

/// One record, one field — the smallest payload there is.
const E: &str = "## An E has:\n    a message, which is Text.\n\n";

// -------------------------------------------------------------------------------------------
// The headline: a record payload, read through each side of the Result.
// -------------------------------------------------------------------------------------------

#[test]
fn a_record_in_the_err_payload_reads_its_field() {
    // ★ THE ONE THAT WAS REFUSED. `When Err (e): … e's message` is what every handler of a failed
    // answer is written to do.
    let src = format!(
        "{E}## To describe (r: Result of Int and E) -> Text:\n\
        \x20   Let mutable out be \"?\".\n\
        \x20   Inspect r:\n\
        \x20       When Ok (v):\n\
        \x20           Set out to \"ok\".\n\
        \x20       When Err (e):\n\
        \x20           Set out to e's message.\n\
        \x20   Return out.\n\
         \n\
         ## Main\n\
         Show describe(a new Err with error (a new E with message \"bad\")).\n"
    );
    assert_eq!(all_tiers(&src), "bad");
}

#[test]
fn a_record_in_the_ok_payload_reads_its_field() {
    // The Ok side is the same mechanism in the other position, and a fix that reached only the
    // second variant would pass the test above.
    let src = format!(
        "{E}## To describe (r: Result of E and Text) -> Text:\n\
        \x20   Let mutable out be \"?\".\n\
        \x20   Inspect r:\n\
        \x20       When Ok (v):\n\
        \x20           Set out to v's message.\n\
        \x20       When Err (e):\n\
        \x20           Set out to e.\n\
        \x20   Return out.\n\
         \n\
         ## Main\n\
         Show describe(a new Ok with value (a new E with message \"fine\")).\n"
    );
    assert_eq!(all_tiers(&src), "fine");
}

#[test]
fn the_taken_branch_is_the_one_the_value_carries() {
    // Both arms resolve, so the answer must come from the value's own constructor rather than
    // from whichever arm the walk happened to type last.
    let src = format!(
        "{E}## To describe (r: Result of E and E) -> Text:\n\
        \x20   Let mutable out be \"?\".\n\
        \x20   Inspect r:\n\
        \x20       When Ok (v):\n\
        \x20           Set out to \"ok:\" + v's message.\n\
        \x20       When Err (e):\n\
        \x20           Set out to \"err:\" + e's message.\n\
        \x20   Return out.\n\
         \n\
         ## Main\n\
         Show describe(a new Ok with value (a new E with message \"a\")).\n\
         Show describe(a new Err with error (a new E with message \"b\")).\n"
    );
    assert_eq!(all_tiers(&src), "ok:a\nerr:b");
}

// -------------------------------------------------------------------------------------------
// Slots: the failure mode that is worse than a refusal.
// -------------------------------------------------------------------------------------------

#[test]
fn every_field_of_the_payload_reads_not_only_the_first() {
    // ★ A layout resolved for slot 0 and nothing else passes every test above and reads the wrong
    // column of anything with more than one — silently, since a wrong Text is still a Text.
    let src = "## A Row has:\n\
      \x20   an id, which is Int.\n\
      \x20   a name, which is Text.\n\
      \x20   a done, which is Bool.\n\
       \n\
       ## To describe (r: Result of Int and Row) -> Text:\n\
      \x20   Inspect r:\n\
      \x20       When Ok (v):\n\
      \x20           Return \"ok\".\n\
      \x20       When Err (e):\n\
      \x20           Show e's id.\n\
      \x20           Show e's name.\n\
      \x20           Show e's done.\n\
      \x20           Return e's name.\n\
       \n\
       ## Main\n\
       Show describe(a new Err with error (a new Row with id 7 and name \"seven\" and done true)).\n";
    assert_eq!(all_tiers(src), "7\nseven\ntrue\nseven");
}

#[test]
fn two_different_record_types_do_not_borrow_each_others_layout() {
    // ★ THE MISCOMPILE THIS FILE EXISTS TO PREVENT. Ok and Err carry DIFFERENT records whose
    // fields sit at different offsets; a shape that answered one layout for the whole enum would
    // read `code` out of the slot `label` lives in and report a plausible wrong answer.
    let src = "## A Good has:\n\
      \x20   a label, which is Text.\n\
      \x20   a score, which is Int.\n\
       \n\
       ## A Bad has:\n\
      \x20   a code, which is Int.\n\
      \x20   a detail, which is Text.\n\
       \n\
       ## To describe (r: Result of Good and Bad) -> Text:\n\
      \x20   Inspect r:\n\
      \x20       When Ok (v):\n\
      \x20           Show v's score.\n\
      \x20           Return v's label.\n\
      \x20       When Err (e):\n\
      \x20           Show e's code.\n\
      \x20           Return e's detail.\n\
       \n\
       ## Main\n\
       Show describe(a new Ok with value (a new Good with label \"yes\" and score 10)).\n\
       Show describe(a new Err with error (a new Bad with code 404 and detail \"missing\")).\n";
    assert_eq!(all_tiers(src), "10\nyes\n404\nmissing");
}

// -------------------------------------------------------------------------------------------
// The shapes a typed fetch actually produces.
// -------------------------------------------------------------------------------------------

#[test]
fn a_list_of_records_in_the_ok_payload_reads_a_row() {
    // ★ EXACTLY WHAT A TYPED FETCH HANDS ITS ACTION: `Get … as Seq of Todo` answers
    // `Result of Seq of Todo and HttpError`. The payload is a LIST whose element layout has to
    // survive being bound out of the enum and then walked by a loop.
    let src = "## A Todo has:\n\
      \x20   an id, which is Int.\n\
      \x20   a text, which is Text.\n\
       \n\
       ## An HttpError has:\n\
      \x20   a status, which is Int.\n\
      \x20   a message, which is Text.\n\
       \n\
       ## To describe (r: Result of Seq of Todo and HttpError) -> Text:\n\
      \x20   Inspect r:\n\
      \x20       When Ok (list):\n\
      \x20           Repeat for t in list:\n\
      \x20               Show t's text.\n\
      \x20           Return \"loaded\".\n\
      \x20       When Err (e):\n\
      \x20           Show e's status.\n\
      \x20           Return e's message.\n\
       \n\
       ## Main\n\
       Show describe(a new Ok with value [a new Todo with id 1 and text \"milk\", a new Todo with id 2 and text \"eggs\"]).\n\
       Show describe(a new Err with error (a new HttpError with status 404 and message \"not found\")).\n";
    assert_eq!(all_tiers(src), "milk\neggs\nloaded\n404\nnot found");
}

#[test]
fn an_empty_list_payload_is_a_list_and_not_a_missing_answer() {
    // An empty Ok payload has no element to type itself from — the same trap the list lock's
    // "kind is a fallback" note records. It must still be a list the loop can walk zero times.
    let src = "## A Todo has:\n\
      \x20   an id, which is Int.\n\
      \x20   a text, which is Text.\n\
       \n\
       ## To describe (r: Result of Seq of Todo and Text) -> Int:\n\
      \x20   Let mutable n be 0.\n\
      \x20   Inspect r:\n\
      \x20       When Ok (list):\n\
      \x20           Repeat for t in list:\n\
      \x20               Set n to n + 1.\n\
      \x20       When Err (e):\n\
      \x20           Set n to 0 - 1.\n\
      \x20   Return n.\n\
       \n\
       ## Main\n\
       Show describe(a new Ok with value (a new Seq of Todo)).\n";
    assert_eq!(all_tiers(src), "0");
}

// -------------------------------------------------------------------------------------------
// The positions a Result can occupy.
// -------------------------------------------------------------------------------------------

#[test]
fn a_result_returned_by_a_function_carries_its_payload_to_the_caller() {
    // A RETURN type is the other boundary, and it is the one a real program crosses: the answer
    // is built in one function and read in another, so nothing local can be traced.
    let src = format!(
        "{E}## To find (n: Int) -> Result of Int and E:\n\
        \x20   If n > 0:\n\
        \x20       Return a new Ok with value n.\n\
        \x20   Return a new Err with error (a new E with message \"too small\").\n\
         \n\
         ## Main\n\
         Let r be find(0).\n\
         Inspect r:\n\
        \x20   When Ok (v):\n\
        \x20       Show v.\n\
        \x20   When Err (e):\n\
        \x20       Show e's message.\n"
    );
    assert_eq!(all_tiers(&src), "too small");
}

#[test]
fn a_result_passed_on_through_a_second_function_still_reads() {
    // Handing the value along means the shape has to survive a second parameter boundary rather
    // than being a property of the frame it was built in.
    let src = format!(
        "{E}## To inner (r: Result of Int and E) -> Text:\n\
        \x20   Inspect r:\n\
        \x20       When Ok (v):\n\
        \x20           Return \"ok\".\n\
        \x20       When Err (e):\n\
        \x20           Return e's message.\n\
         \n\
         ## To outer (r: Result of Int and E) -> Text:\n\
        \x20   Return inner(r).\n\
         \n\
         ## Main\n\
         Show outer(a new Err with error (a new E with message \"through\")).\n"
    );
    assert_eq!(all_tiers(&src), "through");
}

#[test]
fn a_result_built_and_inspected_in_main_still_reads() {
    // No boundary at all — the local path, which resolved by tracing the construction and must
    // keep doing so. A change that made every Result go through the declared-shape path would
    // break the case that never needed it.
    let src = format!(
        "{E}## Main\n\
         Let r be a new Err with error (a new E with message \"local\").\n\
         Inspect r:\n\
        \x20   When Ok (v):\n\
        \x20       Show \"ok\".\n\
        \x20   When Err (e):\n\
        \x20       Show e's message.\n"
    );
    assert_eq!(all_tiers(&src), "local");
}

#[test]
fn a_record_field_holding_a_result_reads_its_payload() {
    // A `Result` inside a record is a FIELD boundary — a third position, resolved by the same
    // declared-type walk that types a parameter.
    let src = format!(
        "{E}## A Box has:\n\
        \x20   an answer, which is Result of Int and E.\n\
         \n\
         ## To describe (b: Box) -> Text:\n\
        \x20   Inspect b's answer:\n\
        \x20       When Ok (v):\n\
        \x20           Return \"ok\".\n\
        \x20       When Err (e):\n\
        \x20           Return e's message.\n\
         \n\
         ## Main\n\
         Show describe(a new Box with answer (a new Err with error (a new E with message \"boxed\"))).\n"
    );
    assert_eq!(all_tiers(&src), "boxed");
}

// -------------------------------------------------------------------------------------------
// Scalars and the shapes that must keep working.
// -------------------------------------------------------------------------------------------

#[test]
fn a_scalar_payload_still_reads_on_both_sides() {
    // The case that worked before any of this — a Result of two scalars — must not have been
    // disturbed by teaching the boundary about records.
    let src = "## To describe (r: Result of Int and Text) -> Text:\n\
      \x20   Inspect r:\n\
      \x20       When Ok (v):\n\
      \x20           Show v.\n\
      \x20           Return \"got it\".\n\
      \x20       When Err (e):\n\
      \x20           Return \"no: \" + e.\n\
       \n\
       ## Main\n\
       Show describe(a new Ok with value 42).\n\
       Show describe(a new Err with error \"nope\").\n";
    assert_eq!(all_tiers(src), "42\ngot it\nno: nope");
}

#[test]
fn a_named_enum_with_a_record_payload_is_untouched() {
    // The shape that ALREADY worked, and the reason the cause was findable: a named enum with a
    // record payload resolved perfectly while the structurally identical Result did not. It must
    // still resolve through the named path.
    let src = format!(
        "{E}## An Answer is one of:\n\
        \x20   A Good.\n\
        \x20   A Bad (p: E).\n\
         \n\
         ## To describe (a: Answer) -> Text:\n\
        \x20   Inspect a:\n\
        \x20       When Good:\n\
        \x20           Return \"good\".\n\
        \x20       When Bad (p):\n\
        \x20           Return p's message.\n\
         \n\
         ## Main\n\
         Show describe(a new Bad with p (a new E with message \"named\")).\n"
    );
    assert_eq!(all_tiers(&src), "named");
}

#[test]
fn a_nested_result_payload_reads_through_both_levels() {
    // A Result whose payload is itself a Result: the inner shape has to be re-seeded when the
    // outer one is bound, which is the recursive case a named enum reaches through its own
    // declared field types.
    let src = format!(
        "{E}## To describe (r: Result of Int and (Result of Int and E)) -> Text:\n\
        \x20   Inspect r:\n\
        \x20       When Ok (v):\n\
        \x20           Return \"outer ok\".\n\
        \x20       When Err (inner):\n\
        \x20           Inspect inner:\n\
        \x20               When Ok (v):\n\
        \x20                   Return \"inner ok\".\n\
        \x20               When Err (e):\n\
        \x20                   Return e's message.\n\
         \n\
         ## Main\n\
         Show describe(a new Err with error (a new Err with error (a new E with message \"deep\"))).\n"
    );
    assert_eq!(all_tiers(&src), "deep");
}
