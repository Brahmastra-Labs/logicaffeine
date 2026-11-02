//! G11/#31 — JSON must work on the WASM (browser frontend) tier. A pure-LOGOS
//! web frontend speaks JSON to its backend; today `json.lg`'s recursive `Json`
//! enum does NOT lower through `compile_to_wasm` (the capstone had to fall back
//! to a line-delimited wire format). This lock pins that a frontend can parse a
//! JSON payload and read a field — the same `jsonTryParse`/`Inspect`/accessor
//! surface the interpreter and VM already run.
//!
//!  ⚠️  YOU DO NOT GET TO WEAKEN THIS LOCK.  ⚠️  "Fully in LOGOS" means the
//!  frontend speaks real JSON, not a bespoke wire hack.

use logicaffeine_compile::compile::compile_to_wasm;

#[path = "compile_common/mod.rs"]
mod common;

/// Parsing a JSON value and matching the result must LOWER to WASM.
#[test]
fn json_parse_and_match_lowers_to_wasm() {
    let src = "## Main\n\
               Let r be jsonTryParse(\"[1, 2, 3]\").\n\
               Inspect r:\n\
               \x20   When JOk (v):\n\
               \x20       Show \"ok\".\n\
               \x20   When JErr (e):\n\
               \x20       Show \"err\".\n";
    match compile_to_wasm(src) {
        Ok(module) => {
            assert!(module.starts_with(b"\0asm"), "emitted a valid wasm module");
        }
        Err(e) => panic!("json parse+match must lower to WASM, but the backend refused: {e:?}"),
    }
}

/// Reading a decoded field (the object case — a recursive `JObj`/`JEntry`) must
/// lower to WASM: the todo-app shape of "parse the response, read a field".
#[test]
fn json_object_field_read_lowers_to_wasm() {
    // The JSON object is written `{{ }}` because `{` opens an INTERPOLATION HOLE in a LOGOS
    // string and a doubled brace is the escape, exactly as in `format!`. The runtime text is
    // byte-identical — the object still reads `{"n": 42}` — and `jsonTryParse` still answers
    // `JOk(JObj)`; only the spelling changed. Written bare, this parsed as a hole holding `"n"`
    // with `: 42` silently DISCARDED, so the fixture was handing the parser a different string
    // than it appeared to. See `interpolation_hole_total_lock`.
    let src = "## Main\n\
               Let r be jsonTryParse(\"{{\\\"n\\\": 42}}\").\n\
               Inspect r:\n\
               \x20   When JOk (v):\n\
               \x20       Inspect v:\n\
               \x20           When JObj (entries):\n\
               \x20               Show \"obj\".\n\
               \x20           Otherwise:\n\
               \x20               Show \"other\".\n\
               \x20   When JErr (e):\n\
               \x20       Show \"err\".\n";
    match compile_to_wasm(src) {
        Ok(module) => assert!(module.starts_with(b"\0asm")),
        Err(e) => panic!("json object field read must lower to WASM: {e:?}"),
    }
}

/// Serializing a Json value back to text must lower to WASM (a frontend building
/// a request body).
#[test]
fn json_serialize_lowers_to_wasm() {
    let src = "## Main\n\
               Let v be a new JArr with elems (a new Seq of Json).\n\
               Show jsonSerialize(v).\n";
    match compile_to_wasm(src) {
        Ok(module) => assert!(module.starts_with(b"\0asm")),
        Err(e) => panic!("json serialize must lower to WASM: {e:?}"),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// EXECUTION PROOF. Compiling to valid wasm is necessary but not sufficient — the
// real bar is that the lowered recursive-enum program RUNS and produces the SAME
// value as the interpreter. These tests instantiate the emitted module under the
// `wasmi` interpreter (the same engine `wasm_aot_lock` uses) with a compact host
// linker holding exactly the two sinks a JSON frontend needs — `print_text` (a
// `Show` of a Text) and `fmt_i64_into` (an Int operand of a `"{n}"` interpolation)
// — then asserts, byte-for-byte, that the wasm `Show` output equals the
// tree-walker's. A parse→read and a parse→serialize round-trip prove the boxed
// recursive `Json` value flows correctly through variant match + field read +
// pure-LOGOS recursive serialization on the browser tier.
//
// Gated on `wasm-jit` (that feature pulls in the `wasmi` dev-dependency), like the
// rest of the wasm-execution locks; the three compile locks above stay always-on.
#[cfg(all(feature = "wasm-jit", not(target_arch = "wasm32")))]
mod runs_under_wasmi {
    use logicaffeine_compile::compile::{compile_to_wasm, tw_outcome, vm_outcome};

    use std::cell::RefCell;
    use std::rc::Rc;

    /// Read a `Text` handle out of the module's linear memory (`[len@0][cap@4][data_ptr@8]`).
    fn read_text(c: &wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, handle: i32) -> String {
        let mem = match c.get_export("memory").and_then(|e| e.into_memory()) {
            Some(m) => m,
            None => return String::new(),
        };
        let d = mem.data(c);
        let h = handle as usize;
        let len = i32::from_le_bytes(d[h..h + 4].try_into().unwrap()) as usize;
        let dp = i32::from_le_bytes(d[h + 8..h + 12].try_into().unwrap()) as usize;
        String::from_utf8_lossy(&d[dp..dp + len]).into_owned()
    }

    /// Instantiate the emitted module, run `main`, and return the joined `Show` output (or the trap
    /// text on a failure — the standalone-module error contract).
    pub(super) fn run(module: &[u8]) -> Result<String, String> {
        let engine = wasmi::Engine::default();
        let m = wasmi::Module::new(&engine, module).expect("emitted bytes are valid wasm");
        let out: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let mut store = wasmi::Store::new(&engine, out.clone());
        let mut linker = wasmi::Linker::<Rc<RefCell<Vec<String>>>>::new(&engine);
        // Every import whose answer the ABI fixes — one implementation, shared, so no test can get
        // `fmt_i64_into`'s return value wrong again (see `wasm_host_stub_lock`).
        crate::common::wasm_host::link_failure_hooks::<Rc<RefCell<Vec<String>>>>(&mut linker);
        crate::common::wasm_host::link_value_imports::<Rc<RefCell<Vec<String>>>>(&mut linker);
        // Every program that CAN fail declares `logos_rt_fail`, and a module importing a
        // function its host does not define FAILS TO INSTANTIATE — silently, with no
        // compile error anywhere. This host does not care why; it must simply be here.
        linker
            .func_wrap(
                "env",
                "print_text",
                |c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, handle: i32| {
                    let s = read_text(&c, handle);
                    c.data().borrow_mut().push(s);
                },
            )
            .unwrap();
        linker
            .func_wrap(
                "env",
                "print_i64",
                |c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, v: i64| {
                    c.data().borrow_mut().push(v.to_string());
                },
            )
            .unwrap();
        // A `Show` of a Bool payload (`When JBool (b)`) renders `true`/`false`.
        linker
            .func_wrap(
                "env",
                "print_bool",
                |c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, v: i32| {
                    c.data().borrow_mut().push(if v != 0 {
                        "true".into()
                    } else {
                        "false".into()
                    });
                },
            )
            .unwrap();
        // The host side of `"{n}"` interpolation: stringify an Int into a module buffer (exactly the
        // tree-walker's `to_display_string`), returning the byte length.
        crate::common::wasm_host::link_unmodelled_imports(&mut linker);
        let instance = linker
            .instantiate(&mut store, &m)
            .unwrap()
            .start(&mut store)
            .unwrap();
        match instance
            .get_typed_func::<(), ()>(&store, "main")
            .unwrap()
            .call(&mut store, ())
        {
            Ok(()) => Ok(out.borrow().clone().join("\n")),
            Err(e) => Err(format!("{e:?}")),
        }
    }

    fn norm(s: &str) -> String {
        s.trim().to_string()
    }

    /// Byte-encode a raw JSON document as a LOGOS `text_from_bytes([...])` expression, so the JSON's
    /// literal `{`/`}`/`"` reach `jsonTryParse` verbatim — a LOGOS string literal would treat `{...}`
    /// as interpolation and `\"` as an escape. This is the same technique `json_conformance_lock` uses
    /// to feed exact bytes across the language boundary.
    fn lit(json: &str) -> String {
        let bytes: Vec<String> = json.bytes().map(|b| b.to_string()).collect();
        format!("text_from_bytes([{}])", bytes.join(", "))
    }

    /// Assert the emitted module RUNS and its `Show` output equals the tree-walker's — with the VM as
    /// the base-equivalence witness (tree-walker == VM == wasm, no engine drifts). This is the real
    /// bar: the boxed recursive `Json` value flows through variant match + field read + pure-LOGOS
    /// recursion on the browser tier byte-identically to the interpreter, not merely "emits bytes".
    fn assert_runs_like_interpreter(src: &str) {
        let tw = tw_outcome(src);
        let vm = vm_outcome(src);
        assert_eq!(
            tw.error, None,
            "the interpreter must run this JSON program cleanly:\n{src}"
        );
        assert_eq!(
            tw.error, vm.error,
            "tree-walker != VM error (base equivalence broken):\n{src}"
        );
        assert_eq!(
            norm(&tw.output),
            norm(&vm.output),
            "tree-walker != VM output (base equivalence broken):\n{src}"
        );
        let module =
            compile_to_wasm(src).unwrap_or_else(|e| panic!("must lower to wasm: {e:?}\n{src}"));
        let got = run(&module).unwrap_or_else(|e| panic!("emitted module trapped: {e}\n{src}"));
        assert_eq!(
            norm(&got),
            norm(&tw.output),
            "WASM != interpreter for a JSON program — a miscompile of the recursive Json enum. \
             Fix vm/wasm/, never this lock.\n{src}"
        );
    }

    /// Parse an ARRAY and read its LENGTH + a scalar element through the boxed recursive enum — the wasm
    /// value must equal the interpreter's.
    #[test]
    fn json_array_parse_read_runs_like_interpreter() {
        assert_runs_like_interpreter(&format!(
            "## Main\n\
             Let r be jsonTryParse({arr}).\n\
             Inspect r:\n\
             \x20   When JOk (v):\n\
             \x20       Inspect v:\n\
             \x20           When JArr (elems):\n\
             \x20               Show length of elems.\n\
             \x20               Inspect item 1 of elems:\n\
             \x20                   When JBool (bb):\n\
             \x20                       Show bb.\n\
             \x20                   Otherwise:\n\
             \x20                       Show \"other\".\n\
             \x20           Otherwise:\n\
             \x20               Show \"not-array\".\n\
             \x20   When JErr (e):\n\
             \x20       Show e.\n",
            arr = lit("[true, false, null]"),
        ));
    }

    /// Read a member out of a parsed OBJECT (the todo-app shape: parse a response, read a field) — the
    /// recursive `JObj`/`JEntry` path, run and value-checked cross-region.
    #[test]
    fn json_object_field_read_runs_like_interpreter() {
        assert_runs_like_interpreter(&format!(
            "## Main\n\
             Let r be jsonTryParse({obj}).\n\
             Inspect r:\n\
             \x20   When JOk (v):\n\
             \x20       Inspect jsonGet(v, \"flag\"):\n\
             \x20           When JBool (bb):\n\
             \x20               Show bb.\n\
             \x20           Otherwise:\n\
             \x20               Show \"missing\".\n\
             \x20   When JErr (e):\n\
             \x20       Show e.\n",
            obj = lit("{\"flag\": true, \"name\": \"hi\"}"),
        ));
    }

    /// A string member value read through the boxed enum — proves a `Text` payload crosses the
    /// recursive-enum boundary byte-identically.
    #[test]
    fn json_object_string_value_runs_like_interpreter() {
        assert_runs_like_interpreter(&format!(
            "## Main\n\
             Let r be jsonTryParse({obj}).\n\
             Inspect r:\n\
             \x20   When JOk (v):\n\
             \x20       Inspect jsonGet(v, \"name\"):\n\
             \x20           When JStr (s):\n\
             \x20               Show s.\n\
             \x20           Otherwise:\n\
             \x20               Show \"missing\".\n\
             \x20   When JErr (e):\n\
             \x20       Show e.\n",
            obj = lit("{\"name\": \"world\"}"),
        ));
    }

    /// The round-trip a frontend performs: PARSE then SERIALIZE — the wasm output text must be
    /// byte-identical to the interpreter's canonical serialization. Exercises `jsonSerialize`'s
    /// pure-LOGOS recursion over object / array / string / bool / null arms.
    #[test]
    fn json_parse_then_serialize_round_trips_like_interpreter() {
        assert_runs_like_interpreter(&format!(
            "## Main\n\
             Let r be jsonTryParse({obj}).\n\
             Inspect r:\n\
             \x20   When JOk (v):\n\
             \x20       Show jsonSerialize(v).\n\
             \x20   When JErr (e):\n\
             \x20       Show e.\n",
            obj = lit("{\"a\": [true, false], \"b\": null, \"c\": \"hi\"}"),
        ));
    }

    /// Build a `Json` value in LOGOS and SERIALIZE it (a frontend constructing a request body) — the
    /// wasm output must equal the interpreter's.
    #[test]
    fn json_build_and_serialize_runs_like_interpreter() {
        assert_runs_like_interpreter(
            "## Main\n\
             Let xs be a new Seq of Json.\n\
             Push (a new JBool with b true) to xs.\n\
             Push (a new JNull) to xs.\n\
             Let v be a new JArr with elems xs.\n\
             Show jsonSerialize(v).\n",
        );
    }

    /// A parse ERROR surfaces the SAME positioned message the interpreter produces — the `JErr` arm
    /// carries a `Text` through the boxed enum and Shows it.
    #[test]
    fn json_parse_error_message_runs_like_interpreter() {
        assert_runs_like_interpreter(&format!(
            "## Main\n\
             Let r be jsonTryParse({bad}).\n\
             Inspect r:\n\
             \x20   When JOk (v):\n\
             \x20       Show jsonSerialize(v).\n\
             \x20   When JErr (e):\n\
             \x20       Show e.\n",
            bad = lit("[true,]"),
        ));
    }

    /// Parsing a NUMBER runs like the interpreter. `jsonParseNumber` is the stdlib's largest
    /// function (~400 ops, well over 64 basic blocks); it was the field repro for the dispatch-loop
    /// encoding bug — block indices written as `i32.const` immediates with the UNSIGNED LEB encoder,
    /// which a wasm engine decodes as NEGATIVE for indices 64..=127, so the `br_table` took its default
    /// and the epilogue `unreachable` trapped. Fixed by routing every block index through
    /// `cfg::set_next_block`; the class of bug is locked by `wasm_large_function_lock`.
    #[test]
    fn json_number_parse_runs_like_interpreter() {
        assert_runs_like_interpreter(&format!(
            "## Main\n\
             Let r be jsonTryParse({n}).\n\
             Inspect r:\n\
             \x20   When JOk (v):\n\
             \x20       Inspect v:\n\
             \x20           When JNum (nn):\n\
             \x20               Show nn.\n\
             \x20           Otherwise:\n\
             \x20               Show \"not-number\".\n\
             \x20   When JErr (e):\n\
             \x20       Show e.\n",
            n = lit("42"),
        ));
    }
}
