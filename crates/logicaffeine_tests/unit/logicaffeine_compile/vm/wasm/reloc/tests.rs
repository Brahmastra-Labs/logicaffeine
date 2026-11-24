use super::*;
use crate as logicaffeine_compile;
#[path = "../../../../../tests/compile_common/wasm_host.rs"]
mod wasm_host;
use crate::vm::wasm::link;
use std::cell::RefCell;
use std::rc::Rc;

/// Instantiate a module and run `main`, capturing the `i64`s it `Show`s through the `print_i64`
/// host — the whole output of a scalar program.
fn run(bytes: &[u8]) -> Vec<i64> {
    let engine = wasmi::Engine::default();
    let module = wasmi::Module::new(&engine, bytes).expect("valid wasm module");
    let out: Rc<RefCell<Vec<i64>>> = Rc::new(RefCell::new(Vec::new()));
    let mut store = wasmi::Store::new(&engine, out.clone());
    let mut linker = wasmi::Linker::new(&engine);
    linker
        .func_wrap(
            "env",
            "print_i64",
            |c: wasmi::Caller<'_, Rc<RefCell<Vec<i64>>>>, v: i64| {
                c.data().borrow_mut().push(v);
            },
        )
        .unwrap();
    // ★ THE FAILURE HOOKS MUST EXIST EVEN WHERE NOTHING FAILS. A module that imports a
    // function its host does not define does not lose a feature — it fails to INSTANTIATE, so
    // the whole corpus stops running with `MissingDefinition`, nowhere near the change that
    // added the import. Every emitted module declares these because `FailWith`, an index and a
    // divisor guard all report through them.
    //
    // They discard here: this corpus is scalar arithmetic that never fails, and what the
    // message SAYS is locked by `runtime_error_message_lock` against
    // `semantics::failcode::message`. (`tests/common/wasm_host.rs::link_failure_hooks` is the
    // shared version; a `src` unit test cannot reach the integration-test crate.)
    linker
        .func_wrap(
            "env",
            "logos_rt_fail",
            |_c: wasmi::Caller<'_, Rc<RefCell<Vec<i64>>>>, _h: i32| {},
        )
        .unwrap();
    linker
        .func_wrap(
            "env",
            "logos_rt_fail_at",
            |_c: wasmi::Caller<'_, Rc<RefCell<Vec<i64>>>>, _code: i32, _v: i64| {},
        )
        .unwrap();
    wasm_host::link_value_imports(&mut linker);
    wasm_host::link_exact_show(&mut linker, |out, value| {
        out.borrow_mut()
            .push(value.parse().expect("this scalar corpus stays within i64"));
    });
    wasm_host::link_unmodelled_imports(&mut linker);
    let instance = linker
        .instantiate(&mut store, &module)
        .unwrap()
        .start(&mut store)
        .unwrap();
    instance
        .get_typed_func::<(), ()>(&store, "main")
        .unwrap()
        .call(&mut store, ())
        .unwrap();
    let r = out.borrow().clone();
    r
}

/// S4 foundation: a real Logos program's self-contained module, converted to a relocatable object
/// and LINKED by `rust-lld`, runs BYTE-IDENTICALLY to the standalone module — the `call`
/// relocations resolve correctly even though the linker renumbers function indices. Exercised over
/// a corpus stressing the decoder (cross-function + recursive calls, `If`/`While` control flow,
/// nested loops, `%`/`*`/`-`): each must convert, link, and match. Skips if the toolchain is absent.
#[test]
fn relocatable_links_and_matches_standalone_over_a_corpus() {
    if !link::toolchain_available() {
        eprintln!(
            "SKIP relocatable_links_and_matches_standalone_over_a_corpus: toolchain unavailable"
        );
        return;
    }
    let corpus: &[(&str, &str)] = &[
        (
            "calls",
            "## To dbl (n: Int) -> Int:\n    Return n + n.\n## Main\n    Show 5.\n    Show 2 + 3.\n    Show dbl(21).\n",
        ),
        (
            "recursion",
            "## No memo\n## To fib (n: Int) -> Int:\n    If n is less than 2:\n        Return n.\n    Return fib(n - 1) + fib(n - 2).\n## Main\n    Show fib(10).\n",
        ),
        (
            "recursion_control_flow",
            "## To collatz (n: Int) -> Int:\n    If n is equal to 1:\n        Return 0.\n    If n % 2 is equal to 0:\n        Return 1 + collatz(n / 2).\n    Return 1 + collatz(3 * n + 1).\n## Main\n    Show collatz(27).\n",
        ),
        (
            "mutual_arith",
            "## To f (n: Int) -> Int:\n    Return n * n - n.\n## To g (n: Int) -> Int:\n    Return f(n) + f(n - 1).\n## Main\n    Show g(10).\n    Show f(7) + g(3).\n",
        ),
        (
            // A mutable Main binding is promoted to a mutable GLOBAL (no loop → no iterator/memory),
            // so this exercises the global.get/set relocations in isolation.
            "mutable_global",
            "## Main\n    Let mutable x be 0.\n    Set x to 10.\n    Set x to x + 32.\n    Show x.\n    Let mutable y be 100.\n    Set y to y - x.\n    Show y.\n",
        ),
    ];
    let dir = std::env::temp_dir().join("logos_reloc_corpus");
    for (name, src) in corpus {
        let standalone = crate::compile::compile_to_wasm(src)
            .unwrap_or_else(|e| panic!("compile {name}: {e:?}"));
        let obj = module_to_relocatable(&standalone)
            .unwrap_or_else(|e| panic!("relocatable {name}: {e:?}"));
        let linked =
            link::link_objects(&[&obj], &dir).unwrap_or_else(|e| panic!("link {name}: {e:?}"));
        let standalone_out = run(&standalone);
        let linked_out = run(&linked);
        assert_eq!(
            linked_out, standalone_out,
            "relocatable-linked output must equal standalone for {name}"
        );
        assert!(!standalone_out.is_empty(), "{name} should Show something");
    }
}

/// A random arithmetic expression over small non-negative ints (`+`/`-`/`*`, parenthesized, kept
/// bounded so it never overflows i64 or goes negative under `-`) — stresses the decoder's constant
/// / arithmetic / call-free instruction handling with shapes the curated corpus can't enumerate.
fn fuzz_expr(state: &mut u64, depth: u32) -> (String, i64) {
    let next = |s: &mut u64| {
        *s ^= *s << 13;
        *s ^= *s >> 7;
        *s ^= *s << 17;
        *s
    };
    if depth == 0 || next(state) % 3 == 0 {
        let v = (next(state) % 20) as i64; // a small leaf constant
        return (v.to_string(), v);
    }
    let (ls, lv) = fuzz_expr(state, depth - 1);
    let (rs, rv) = fuzz_expr(state, depth - 1);
    match next(state) % 3 {
        0 => (format!("({ls} + {rs})"), lv + rv),
        1 => (format!("({ls} * {rs})"), lv * rv),
        // subtraction guarded to stay non-negative so the (unsigned-displayed) Show matches simply.
        _ if lv >= rv => (format!("({ls} - {rs})"), lv - rv),
        _ => (format!("({rs} - {ls})"), rv - lv),
    }
}

/// Robust-to-absurdity net for the transform: 60 RANDOM scalar arithmetic programs, each
/// `compile_to_wasm → module_to_relocatable → link → run` asserted BYTE-IDENTICAL to the standalone
/// module. A decoder that mis-skips ANY instruction's immediate would compute a wrong `call`/global
/// offset and diverge — this catches it. (Values are bounded so no i64 overflow, keeping standalone
/// and linked on the fragment where they must agree.)
#[test]
fn fuzz_relocatable_matches_standalone() {
    if !link::toolchain_available() {
        eprintln!("SKIP fuzz_relocatable_matches_standalone: toolchain unavailable");
        return;
    }
    let dir = std::env::temp_dir().join("logos_reloc_fuzz");
    for seed in 0..60u64 {
        let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0x2545_F491_4F6C_DD1D;
        let (expr, expected) = fuzz_expr(&mut state, 4);
        let src = format!("## Main\n    Show {expr}.\n");
        let standalone = crate::compile::compile_to_wasm(&src)
            .unwrap_or_else(|e| panic!("compile seed {seed}: {e:?}\n{src}"));
        let obj = match module_to_relocatable(&standalone) {
            Ok(o) => o,
            Err(_) => continue, // the transform soundly declined this shape (memory/data/etc.) — fine
        };
        let linked =
            link::link_objects(&[&obj], &dir).unwrap_or_else(|e| panic!("link seed {seed}: {e:?}"));
        let standalone_out = run(&standalone);
        let linked_out = run(&linked);
        assert_eq!(
            linked_out, standalone_out,
            "seed {seed}: linked != standalone for {src}"
        );
        assert_eq!(
            standalone_out,
            vec![expected],
            "seed {seed}: wrong value for {src}"
        );
    }
}

/// A function a module exports under a name other than `main` keeps that name in the object, so an object it links
/// with calls it by name; every other function stays `fnN`, and `main` alone is exported from the link.
#[test]
fn an_exported_function_is_named_by_its_export_in_the_object() {
    fn section(id: u8, body: &[u8], out: &mut Vec<u8>) {
        out.push(id);
        out.push(body.len() as u8);
        out.extend_from_slice(body);
    }
    fn name(text: &str, out: &mut Vec<u8>) {
        out.push(text.len() as u8);
        out.extend_from_slice(text.as_bytes());
    }
    let mut module = b"\0asm\x01\0\0\0".to_vec();
    // () -> i64, (i32, i64) -> i32
    section(1, &[2, 0x60, 0, 1, 0x7e, 0x60, 2, 0x7f, 0x7e, 1, 0x7f], &mut module);
    section(3, &[3, 0, 0, 1], &mut module);
    let mut exports = vec![2];
    name("main", &mut exports);
    exports.extend([0x00, 0]);
    name("logos_program_defines", &mut exports);
    exports.extend([0x00, 2]);
    section(7, &exports, &mut module);
    let bodies: [&[u8]; 3] = [&[0, 0x10, 1, 0x0b], &[0, 0x42, 7, 0x0b], &[0, 0x20, 0, 0x0b]];
    let mut code = vec![3];
    for b in bodies {
        code.push(b.len() as u8);
        code.extend_from_slice(b);
    }
    section(10, &code, &mut module);
    let object = super::module_to_relocatable(&module).expect("relocatable");
    let has = |text: &str| object.windows(text.len()).any(|w| w == text.as_bytes());
    assert!(has("logos_program_defines"), "the exported reader keeps its name");
    assert!(has("fn1"), "an unexported function is fnN");
    assert!(!has("fn2"), "the exported function is not also fnN");
}

/// What a module `Show`ed, every line, through the shared display sinks.
#[derive(Default)]
struct Lines(RefCell<Vec<String>>);

impl wasm_host::ShowSink for Rc<Lines> {
    fn show(&self, line: String) {
        self.0.borrow_mut().push(line);
    }
}

/// Instantiate a module, run `main`, and answer every line it showed and the instance with its store, so a test can
/// read the memory the run left behind.
fn run_lines(bytes: &[u8]) -> (Vec<String>, wasmi::Store<Rc<Lines>>, wasmi::Instance) {
    let engine = wasmi::Engine::default();
    let module = wasmi::Module::new(&engine, bytes).expect("valid wasm module");
    let out: Rc<Lines> = Rc::default();
    let mut store = wasmi::Store::new(&engine, out.clone());
    let mut linker = wasmi::Linker::new(&engine);
    linker
        .func_wrap("env", "logos_rt_fail", |_c: wasmi::Caller<'_, Rc<Lines>>, _h: i32| {})
        .unwrap();
    linker
        .func_wrap("env", "logos_rt_fail_at", |_c: wasmi::Caller<'_, Rc<Lines>>, _code: i32, _v: i64| {})
        .unwrap();
    wasm_host::link_value_imports(&mut linker);
    wasm_host::link_show_sinks(&mut linker);
    wasm_host::link_unmodelled_imports(&mut linker);
    let instance = linker.instantiate(&mut store, &module).unwrap().start(&mut store).unwrap();
    instance.get_typed_func::<(), ()>(&store, "main").unwrap().call(&mut store, ()).unwrap();
    let lines = out.0.borrow().clone();
    (lines, store, instance)
}

/// Programs whose modules own memory: Text literals in a data segment, lists and maps on the bump heap, and a
/// memoized recursion whose cache is a module-level map. Each converts to a relocatable object, links, and shows
/// exactly what its standalone module shows.
#[test]
fn programs_with_static_text_a_heap_and_a_memo_link_and_match_standalone() {
    if !link::toolchain_available() {
        eprintln!("SKIP programs_with_static_text_a_heap_and_a_memo_link_and_match_standalone: toolchain unavailable");
        return;
    }
    let corpus: &[(&str, &str)] = &[
        ("literals", "## Main\n    Show \"hello\".\n    Let t be \"ab\" + \"cd\".\n    Show t.\n    Show length of t.\n"),
        (
            "lists",
            "## Main\n    Let mutable xs be a new Seq of Int.\n    Repeat for i from 1 to 500:\n        Push i * i to xs.\n    Show item 500 of xs.\n    Show length of xs.\n",
        ),
        (
            "texts_in_a_list",
            "## Main\n    Let mutable ws be a new Seq of Text.\n    Push \"alpha\" to ws.\n    Push \"beta\" to ws.\n    Push \"gamma\" + \"!\" to ws.\n    Show ws.\n",
        ),
        (
            "memoized",
            "## To collatz (n: Int) -> Int:\n    If n is equal to 1:\n        Return 0.\n    If n % 2 is equal to 0:\n        Return 1 + collatz(n / 2).\n    Return 1 + collatz(3 * n + 1).\n## Main\n    Show collatz(27).\n    Show collatz(97).\n",
        ),
        (
            "map",
            "## Main\n    Let mutable m be a new Map of Text to Int.\n    Set item \"a\" of m to 1.\n    Set item \"b\" of m to 2.\n    Show item \"b\" of m.\n",
        ),
    ];
    let dir = std::env::temp_dir().join("logos_reloc_heap_corpus");
    for (name, src) in corpus {
        let standalone =
            crate::compile::compile_to_wasm(src).unwrap_or_else(|e| panic!("compile {name}: {e:?}"));
        let obj = module_to_relocatable(&standalone).unwrap_or_else(|e| panic!("relocatable {name}: {e:?}"));
        let linked = link::link_objects(&[&obj], &dir).unwrap_or_else(|e| panic!("link {name}: {e:?}"));
        let (standalone_out, _, _) = run_lines(&standalone);
        let (linked_out, _, _) = run_lines(&linked);
        assert!(!standalone_out.is_empty(), "{name} shows something");
        assert_eq!(linked_out, standalone_out, "relocatable-linked output must equal standalone for {name}");
    }
}

/// `rust-lld --stack-first` puts the shadow stack at the bottom of memory, where a standalone module keeps its free
/// lists and Text pool. A linked program whose static addresses were relocated leaves that stack untouched; one whose
/// addresses kept their standalone values would write its heap over it.
#[test]
fn a_linked_program_leaves_the_shadow_stack_untouched() {
    if !link::toolchain_available() {
        eprintln!("SKIP a_linked_program_leaves_the_shadow_stack_untouched: toolchain unavailable");
        return;
    }
    let src = "## Main\n    Let mutable xs be a new Seq of Text.\n    Repeat for i from 1 to 200:\n        Push \"item\" to xs.\n    Show length of xs.\n    Show \"done\".\n";
    let standalone = crate::compile::compile_to_wasm(src).expect("compiles");
    let obj = module_to_relocatable(&standalone).expect("relocatable");
    let linked = link::link_objects(&[&obj], &std::env::temp_dir().join("logos_reloc_stack")).expect("links");
    let (lines, store, instance) = run_lines(&linked);
    assert_eq!(lines, ["200", "done"]);
    let memory = instance.get_memory(&store, "memory").expect("the link exports its memory");
    let stack = &memory.data(&store)[..1 << 20];
    let written: Vec<usize> = stack.iter().enumerate().filter(|(_, b)| **b != 0).map(|(i, _)| i).take(8).collect();
    assert!(written.is_empty(), "the program wrote inside the 1 MiB shadow stack at {written:?}");
}

/// Another allocator in the same memory (the Rust runtime's, once one is linked) owns the pages its `memory.grow`
/// returned. Here the host plays that allocator: every `Show` grows memory by a page and fills it with a sentinel. The
/// program keeps allocating after each one, and must never hand out a byte of a page it did not grow itself.
#[test]
fn a_linked_programs_heap_never_takes_memory_another_allocator_grew() {
    if !link::toolchain_available() {
        eprintln!("SKIP a_linked_programs_heap_never_takes_memory_another_allocator_grew: toolchain unavailable");
        return;
    }
    const SENTINEL: u8 = 0xA5;
    type Host = Rc<RefCell<(Vec<i64>, Vec<u32>)>>;
    let src = "## Main\n    Let mutable total be 0.\n    Repeat for round from 1 to 6:\n        Let mutable xs be a new Seq of Int.\n        Repeat for i from 1 to 3000:\n            Push i * round to xs.\n        Set total to total + item 3000 of xs.\n        Show total.\n";
    let standalone = crate::compile::compile_to_wasm(src).expect("compiles");
    let obj = module_to_relocatable(&standalone).expect("relocatable");
    let linked = link::link_objects(&[&obj], &std::env::temp_dir().join("logos_reloc_foreign")).expect("links");
    let engine = wasmi::Engine::default();
    let module = wasmi::Module::new(&engine, &linked[..]).expect("valid wasm module");
    let host: Host = Rc::default();
    let mut store = wasmi::Store::new(&engine, host.clone());
    let mut linker = wasmi::Linker::new(&engine);
    linker
        .func_wrap("env", "print_i64", |mut c: wasmi::Caller<'_, Host>, v: i64| {
            let memory = c.get_export("memory").and_then(|e| e.into_memory()).expect("memory");
            let one = wasmi::core::Pages::new(1).expect("one page");
            let page = u32::from(memory.grow(&mut c, one).expect("the host grows a page"));
            let start = page as usize * 65536;
            memory.data_mut(&mut c)[start..start + 65536].fill(SENTINEL);
            let mut h = c.data().borrow_mut();
            h.0.push(v);
            h.1.push(page);
        })
        .unwrap();
    linker
        .func_wrap("env", "logos_rt_fail", |_c: wasmi::Caller<'_, Host>, _h: i32| {})
        .unwrap();
    linker
        .func_wrap("env", "logos_rt_fail_at", |_c: wasmi::Caller<'_, Host>, _code: i32, _v: i64| {})
        .unwrap();
    wasm_host::link_value_imports(&mut linker);
    wasm_host::link_exact_show(&mut linker, |h, value| {
        h.borrow_mut().0.push(value.parse().expect("an i64 total"));
    });
    wasm_host::link_unmodelled_imports(&mut linker);
    let instance = linker.instantiate(&mut store, &module).unwrap().start(&mut store).unwrap();
    instance.get_typed_func::<(), ()>(&store, "main").unwrap().call(&mut store, ()).unwrap();
    let (shown, pages) = host.borrow().clone();
    let expected: Vec<i64> = (1..=6).scan(0i64, |t, r| {
        *t += 3000 * r;
        Some(*t)
    }).collect();
    assert_eq!(shown, expected, "the totals");
    let memory = instance.get_memory(&store, "memory").expect("memory");
    for page in pages {
        let start = page as usize * 65536;
        let bytes = &memory.data(&store)[start..start + 65536];
        let taken = bytes.iter().position(|b| *b != SENTINEL);
        assert!(taken.is_none(), "the program wrote into page {page}, grown by another allocator, at +{taken:?}");
    }
}
