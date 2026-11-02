//! ★ LOCK — every heap allocation NAMES what it is. ★
//!
//! `emit_alloc(code, ctx, dst, class: AllocClass)` packs `class` into the low byte of a fresh 16-byte
//! header's word 12 (`count:24 | class:8` — repurposing the byte `emit_retain`/`emit_cow` already
//! owned for `extra_refs`, so a correctly-classified header costs the allocator nothing new). This
//! file holds two things to it: that every call site in the source actually supplies a known
//! class (Rust already refuses a call with the parameter missing — this is the SOURCE census
//! `heap.md`'s S7 record promises, which additionally pins the COUNT so a new call site nobody
//! classified deliberately cannot slip in unnoticed), and that a representative program's collections
//! carry the RIGHT class at runtime, read back from raw memory.
//!
//! What decides a class is documented and kept private to `vm/wasm/module.rs` (`kind_is_handle`,
//! `AllocClass::of_map`, `empty_collection_class`); this file is the outside view: the census, and
//! the wasm-level proof the stamp lands where the logic says it should.

#![cfg(all(feature = "wasm-jit", not(target_arch = "wasm32")))]

use logicaffeine_compile::compile::compile_to_wasm;

#[path = "compile_common/mod.rs"]
mod common;

const SRC: &str = concat!(
    include_str!("../../logicaffeine_compile/src/vm/wasm/module.rs"),
    "\n",
    include_str!("../../logicaffeine_compile/src/vm/wasm/module/exact.rs"),
    "\n",
    include_str!("../../logicaffeine_compile/src/vm/wasm/module/exact/list.rs"),
    "\n",
    include_str!("../../logicaffeine_compile/src/vm/wasm/module/exact/map.rs"),
);

/// Every `emit_alloc(` call in the source, EXCLUDING its own definition — as the raw argument list
/// text (everything between the matching parens), so a census can read what a call passed rather
/// than trusting that it compiled.
fn emit_alloc_call_sites() -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = SRC;
    while let Some(i) = rest.find("emit_alloc(") {
        let after = &rest[i + "emit_alloc(".len()..];
        // Skip the definition itself: its very next token is a formal parameter name, `code:`.
        if after.trim_start().starts_with("code: &mut Vec<u8>") {
            rest = after;
            continue;
        }
        let mut depth = 1i32;
        let mut end = 0;
        for (k, c) in after.char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = k;
                        break;
                    }
                }
                _ => {}
            }
        }
        assert!(
            end > 0,
            "an emit_alloc( call never closes its parens — the reader is confused, not the source"
        );
        out.push(after[..end].to_string());
        rest = &after[end..];
    }
    out
}

/// ★ THE CENSUS — every `emit_alloc` call names a real `AllocClass`, and the count is pinned so a
/// call site added later without a deliberate class fails here by name, not by silently landing on
/// whatever the last positional argument happens to parse as.
/// The exact count the S7 census found. Pinned, not a floor: a NEW call site changes this number
/// either way, and updating it is the deliberate act of naming that site's class — the same
/// discipline `lgx/html.rs`'s `VOID_ELEMENTS` holds itself to.
/// The 75 audited emitter sites cover closure, sequence, capacity and Text storage. Exact-integer cells
/// share one runtime allocator; their header is ExactIntegerHeader and their buffer is Raw. env.lg's `get`
/// builds its Text as every Text builder does: a Raw buffer the host writes the bytes into, under a TextHeader.
const KNOWN_CALL_SITES: usize = 75;

#[test]
fn every_allocation_site_is_classified() {
    let sites = emit_alloc_call_sites();
    assert_eq!(
        sites.len(),
        KNOWN_CALL_SITES,
        "found {} emit_alloc call sites, expected exactly {KNOWN_CALL_SITES} — a site was added or \
         removed; update KNOWN_CALL_SITES only after confirming every site (old and new) still names \
         a deliberate class",
        sites.len()
    );
    let known = [
        "Raw",
        "ScalarHeader",
        "HandleHeader",
        "MapScalarScalar",
        "MapScalarHandle",
        "MapHandleScalar",
        "MapHandleHandle",
        "TextHeader",
        "RecordHeader",
        "EnumHeader",
        "CounterHeader",
        "ClosureHeader",
        "ExactIntegerHeader",
    ];
    for args in &sites {
        let names_a_class = args.contains("AllocClass::")
            || args.trim_end().ends_with("class")
            || args.trim_end().ends_with("inner_class");
        assert!(
            names_a_class,
            "an emit_alloc call passes no recognisable class argument: `emit_alloc({args})` — every \
             allocation site must say what it is, even if that is `AllocClass::Raw`"
        );
        if let Some(at) = args.rfind("AllocClass::") {
            let name: String = args[at + "AllocClass::".len()..]
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            let is_constructor = name == "of_elements" || name == "of_map";
            assert!(
                is_constructor || known.contains(&name.as_str()),
                "`emit_alloc({args})` names an AllocClass variant `{name}` this lock does not know — the catalogue above is stale"
            );
        }
    }
}

/// `lower_map_insert` — the one site that can classify a Map (its key/value kind is not known at
/// creation) — actually calls `AllocClass::of_map`, not a hand-picked constant that would silently
/// stop tracking a map's real key/value shape.
#[test]
fn map_insert_computes_its_class_rather_than_hardcoding_it() {
    let start = SRC
        .find("fn lower_map_insert(")
        .expect("lower_map_insert exists");
    let body = &SRC[start..start + 2000.min(SRC.len() - start)];
    assert!(
        body.contains("AllocClass::of_map("),
        "lower_map_insert must derive its class from AllocClass::of_map(key, value), not a literal"
    );
}

/// A mounted-once program with no listener, capturing the raw `i32` handle a `Show`/`print_*` sink
/// receives — the same handle the module allocated — so this reads the class byte the ALLOCATOR
/// actually stamped, not a value this test assumes.
fn live_class<D>(caller: &wasmi::Caller<'_, D>, handle: i32) -> u8 {
    let memory = caller
        .get_export("memory")
        .and_then(|e| e.into_memory())
        .expect("memory");
    memory.data(caller)[handle as usize + 12]
}

fn class_byte_of_shown_collection(src: &str) -> u8 {
    let module = compile_to_wasm(src).unwrap_or_else(|e| panic!("must compile: {e:?}\n{src}"));
    let engine = wasmi::Engine::default();
    let m = wasmi::Module::new(&engine, &module[..]).unwrap_or_else(|e| panic!("valid wasm: {e}"));
    type D = std::rc::Rc<std::cell::RefCell<Vec<u8>>>;
    let seen: D = Default::default();
    let mut l = wasmi::Linker::<D>::new(&engine);
    for name in [
        "print_seq_i64",
        "print_seq_bool",
        "print_seq_f64",
        "print_seq_word32",
        "print_seq_word64",
        "print_seq_text",
        "print_set_i64",
        "print_set_text",
    ] {
        l.func_wrap("env", name, move |c: wasmi::Caller<'_, D>, h: i32| {
            c.data().borrow_mut().push(live_class(&c, h));
        })
        .unwrap_or_else(|e| panic!("{name}: {e}"));
    }
    common::wasm_host::link_failure_hooks::<D>(&mut l);
    common::wasm_host::link_value_imports::<D>(&mut l);
    let mut store = wasmi::Store::new(&engine, seen.clone());
    crate::common::wasm_host::link_unmodelled_imports(&mut l);
    let inst = l
        .instantiate(&mut store, &m)
        .unwrap_or_else(|e| panic!("must instantiate: {e}"))
        .start(&mut store)
        .unwrap_or_else(|e| panic!("main must not trap: {e}"));
    inst.get_typed_func::<(), ()>(&store, "main")
        .unwrap()
        .call(&mut store, ())
        .expect("main runs");
    let class = *seen
        .borrow()
        .first()
        .expect("the program must Show exactly one collection");
    class
}

#[test]
fn a_seq_of_scalars_is_stamped_scalar_header() {
    assert_eq!(
        class_byte_of_shown_collection("## Main\nLet xs be [1, 2, 3].\nShow xs.\n"),
        1 /* ScalarHeader */
    );
}

#[test]
fn a_seq_of_text_is_stamped_handle_header() {
    assert_eq!(
        class_byte_of_shown_collection("## Main\nLet xs be [\"a\", \"b\"].\nShow xs.\n"),
        2 /* HandleHeader */
    );
}

#[test]
fn a_set_of_int_is_stamped_scalar_header() {
    assert_eq!(
        class_byte_of_shown_collection(
            "## Main\nLet mutable s be a new Set of Int.\nAdd 3 to s.\nShow s.\n"
        ),
        1 /* ScalarHeader */
    );
}

#[test]
fn a_set_of_text_is_stamped_handle_header() {
    assert_eq!(
        class_byte_of_shown_collection(
            "## Main\nLet mutable s be a new Set of Text.\nAdd \"a\" to s.\nShow s.\n"
        ),
        2 /* HandleHeader */
    );
}

#[test]
fn a_sliced_seq_carries_the_same_class_as_its_source() {
    assert_eq!(
        class_byte_of_shown_collection(
            "## Main\nLet xs be [1, 2, 3, 4].\nShow items 1 through 3 of xs.\n"
        ),
        1 /* ScalarHeader */
    );
}

/// Every Text a program BUILDS at run time — concatenated, appended, formatted, stringified, indexed,
/// iterated, made from a code point, read from the environment — is stamped [`TextHeader`], the counted
/// class a safepoint can free (S12). A literal from the module's text pool is the one Text that is not:
/// it lives in the static region, class 0, immortal by construction. A Text-building site left `Raw` would keep every Text it
/// makes forever; this reads the stamp each one actually carries, off the handle `print_text` receives.
#[test]
fn every_text_a_program_builds_is_stamped_text_header() {
    let src = "## Main
Let n be 7.
Let t be \"ab\".
Show t + \"cd\".
Show \"n={n}\".
Show format(n).
Show \"{2.5}\".
Show item 1 of t.
Show chr(65).
Let mutable acc be \"x\".
Repeat for i from 1 to 3:
    Set acc to acc + \"y\".
Show acc.
Repeat for ch in t:
    Show ch.
Inspect get(\"PATH\"):
    When Some (path):
        Show path.
    When None:
        Show t + \"!\".
Show \"literal\".
";
    let module = compile_to_wasm(src).unwrap_or_else(|e| panic!("must compile: {e:?}\n{src}"));
    let engine = wasmi::Engine::default();
    let m = wasmi::Module::new(&engine, &module[..]).unwrap_or_else(|e| panic!("valid wasm: {e}"));
    type D = std::rc::Rc<std::cell::RefCell<Vec<u8>>>;
    let seen: D = Default::default();
    let mut l = wasmi::Linker::<D>::new(&engine);
    l.func_wrap(
        "env",
        "print_text",
        move |c: wasmi::Caller<'_, D>, h: i32| {
            c.data().borrow_mut().push(live_class(&c, h));
        },
    )
    .expect("print_text");
    common::wasm_host::link_failure_hooks::<D>(&mut l);
    common::wasm_host::link_value_imports::<D>(&mut l);
    let mut store = wasmi::Store::new(&engine, seen.clone());
    crate::common::wasm_host::link_unmodelled_imports(&mut l);
    let inst = l
        .instantiate(&mut store, &m)
        .unwrap_or_else(|e| panic!("must instantiate: {e}"))
        .start(&mut store)
        .unwrap_or_else(|e| panic!("main must not trap: {e}"));
    inst.get_typed_func::<(), ()>(&store, "main")
        .unwrap()
        .call(&mut store, ())
        .expect("main runs");
    let classes = seen.borrow().clone();
    assert!(
        classes.len() >= 10,
        "the program shows every kind of built Text, and the literal: {classes:?}"
    );
    let (built, literal) = classes.split_at(classes.len() - 1);
    for (index, &class) in built.iter().enumerate() {
        assert_eq!(
            class, 7, /* TextHeader */
            "built Text {index} carries class {class}"
        );
    }
    assert_eq!(literal[0], 0, "a pooled literal is immortal, class 0");
}
