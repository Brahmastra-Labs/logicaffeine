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
            "## To fib (n: Int) -> Int:\n    If n is less than 2:\n        Return n.\n    Return fib(n - 1) + fib(n - 2).\n## Main\n    Show fib(10).\n",
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
