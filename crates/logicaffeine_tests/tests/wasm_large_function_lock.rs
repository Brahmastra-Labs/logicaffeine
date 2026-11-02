//! ★ LOCK — a function with MORE THAN 64 basic blocks runs on the WASM tier exactly like the
//! interpreter. ★
//!
//! The dispatch-loop lowering (`vm/wasm/cfg.rs`) sets the "next block" local with an
//! `i32.const <block index>`. `i32.const` takes a SIGNED LEB128 immediate; the emitter wrote it
//! with the UNSIGNED encoder. The two encodings agree below 64, so every small function was
//! correct — but a block index in 64..=127 is one unsigned byte with bit 6 set, which a wasm
//! engine decodes as a NEGATIVE number: the `br_table` falls to its default arm, control leaves
//! the loop, and the function's epilogue `unreachable` traps. That is exactly the cumulative
//! "every section works in isolation, the whole function traps" shape the JSON number parser hit
//! (`json_on_wasm_lock::json_number_parse_runs_like_interpreter`).
//!
//! Three locks, so the class of bug can never return:
//!  1. a synthetic straight-line program grown past 64 blocks runs like the interpreter (the
//!     failure message names the first block count that diverges);
//!  2. the real `jsonTryParse("42")` runs like the interpreter (the field repro, cross-referenced);
//!  3. an ENCODING-DISCIPLINE lint: no source file under `src/vm/wasm/` may follow an
//!     `i32.const`/`i64.const` opcode push with the unsigned `leb_u32` — the typed `i32_const`
//!     helper (or `leb_i32`/`leb_i64`) is the only legal spelling.

use std::fs;
use std::path::PathBuf;

#[path = "compile_common/mod.rs"]
mod common;

/// (3) Encoding discipline — source-level, always-on (needs no feature).
#[test]
fn const_immediates_never_use_the_unsigned_encoder() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../logicaffeine_compile/src")
        .join("vm")
        .join("wasm");
    let mut offenders = Vec::new();
    for entry in fs::read_dir(&dir).expect("src/vm/wasm exists") {
        let path = entry.expect("entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let src = fs::read_to_string(&path).expect("read");
        let lines: Vec<&str> = src.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            let is_const_opcode = line.contains("push(0x41)") || line.contains("push(0x42)");
            if !is_const_opcode {
                continue;
            }
            // The immediate is written on the same line or the next non-empty one.
            let next = lines.get(i + 1).copied().unwrap_or("");
            if line.contains("leb_u32") || next.contains("leb_u32") {
                offenders.push(format!(
                    "{}:{}: {}",
                    path.file_name().unwrap().to_string_lossy(),
                    i + 1,
                    line.trim()
                ));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "`i32.const`/`i64.const` immediates are SIGNED LEB128 — an unsigned `leb_u32` misencodes 64..=127 (and 8192..=16383, …). \
         Use `encode::i32_const` / `leb_i64` at:\n{}",
        offenders.join("\n")
    );
}

#[cfg(all(feature = "wasm-jit", not(target_arch = "wasm32")))]
mod runs_under_wasmi {
    use logicaffeine_compile::compile::{compile_to_wasm, tw_outcome, vm_outcome};

    use std::cell::RefCell;
    use std::rc::Rc;

    type Out = Rc<RefCell<Vec<String>>>;

    fn read_text(c: &wasmi::Caller<'_, Out>, handle: i32) -> String {
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

    /// Instantiate the emitted module, run `main`, and return the joined `Show` output or the
    /// trap text.
    fn run(module: &[u8]) -> Result<String, String> {
        let engine = wasmi::Engine::default();
        let m = wasmi::Module::new(&engine, module).expect("emitted bytes are valid wasm");
        let out: Out = Rc::new(RefCell::new(Vec::new()));
        let mut store = wasmi::Store::new(&engine, out.clone());
        let mut linker = wasmi::Linker::<Out>::new(&engine);
        // Every import whose answer the ABI fixes — one implementation, shared, so no test can get
        // `fmt_i64_into`'s return value wrong again (see `wasm_host_stub_lock`).
        crate::common::wasm_host::link_value_imports::<Out>(&mut linker);
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
                "print_text",
                |c: wasmi::Caller<'_, Out>, handle: i32| {
                    let s = read_text(&c, handle);
                    c.data().borrow_mut().push(s);
                },
            )
            .unwrap();
        linker
            .func_wrap("env", "print_i64", |c: wasmi::Caller<'_, Out>, v: i64| {
                c.data().borrow_mut().push(v.to_string());
            })
            .unwrap();
        linker
            .func_wrap("env", "print_bool", |c: wasmi::Caller<'_, Out>, v: i32| {
                c.data().borrow_mut().push(if v != 0 {
                    "true".into()
                } else {
                    "false".into()
                });
            })
            .unwrap();
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

    /// tw == VM == wasm on `Show` output; returns the wasm trap text instead of panicking so a ramp
    /// can report the first size that diverges.
    fn wasm_vs_interpreter(src: &str) -> Result<(), String> {
        let tw = tw_outcome(src);
        let vm = vm_outcome(src);
        assert_eq!(
            tw.error, None,
            "the interpreter must run this program cleanly:\n{src}"
        );
        assert_eq!(
            tw.error, vm.error,
            "tree-walker != VM error (base equivalence broken):\n{src}"
        );
        assert_eq!(
            tw.output.trim(),
            vm.output.trim(),
            "tree-walker != VM output (base equivalence broken):\n{src}"
        );
        let module = compile_to_wasm(src).map_err(|e| format!("refused to lower: {e:?}"))?;
        let got = run(&module).map_err(|e| format!("trapped: {e}"))?;
        if got.trim() == tw.output.trim() {
            Ok(())
        } else {
            Err(format!(
                "wasm printed {:?}, interpreter printed {:?}",
                got.trim(),
                tw.output.trim()
            ))
        }
    }

    /// A function whose body is `n` consecutive `If x is equal to K:` blocks — each If is at least
    /// two basic blocks (the test + the guarded body), so the function has more than `2n` dispatch
    /// blocks and its LAST blocks have indices past 64 once `n >= 32`. The final `Show` sits in the
    /// highest-numbered block: reaching it proves every fallthrough/jump index was decoded as the
    /// positive number the emitter meant.
    fn wide_function(n: usize) -> String {
        let mut s = String::from("## To wide (x: Int) -> Int:\n    Let mutable acc be 0.\n");
        for k in 1..=n {
            s.push_str(&format!(
                "    If x is equal to {k}:\n        Set acc to acc + {k}.\n"
            ));
        }
        s.push_str("    Return acc + x.\n\n## Main\nShow wide(7).\nShow wide(");
        s.push_str(&n.to_string());
        s.push_str(").\n");
        s
    }

    /// (1) The size ramp: 8 blocks is the pre-bug regime, 96+ crosses the 64-index boundary.
    #[test]
    fn a_function_with_more_than_64_basic_blocks_runs_like_the_interpreter() {
        let mut failures = Vec::new();
        for n in [4usize, 16, 31, 32, 33, 48, 64, 96, 200] {
            if let Err(e) = wasm_vs_interpreter(&wide_function(n)) {
                failures.push(format!("n={n} (~{} blocks): {e}", 2 * n + 2));
            }
        }
        assert!(
            failures.is_empty(),
            "the WASM tier diverges from the interpreter once a function grows past 64 basic blocks \
             (a mis-encoded `i32.const` block index):\n{}",
            failures.join("\n")
        );
    }

    /// (2) The field repro: parsing a JSON number lowers AND runs — `jsonParseNumber` is the
    /// ~400-op stdlib function whose block count crosses the boundary.
    #[test]
    fn json_number_parse_runs_like_the_interpreter() {
        let src = "## Main\n\
                   Let r be jsonTryParse(\"42\").\n\
                   Inspect r:\n\
                   \x20   When JOk (v):\n\
                   \x20       Inspect v:\n\
                   \x20           When JNum (nn):\n\
                   \x20               Show nn.\n\
                   \x20           Otherwise:\n\
                   \x20               Show \"not-number\".\n\
                   \x20   When JErr (e):\n\
                   \x20       Show e.\n";
        if let Err(e) = wasm_vs_interpreter(src) {
            panic!("JSON number parse must run on the WASM tier like the interpreter: {e}");
        }
    }
}
