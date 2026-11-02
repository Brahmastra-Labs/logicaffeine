use super::*;

/// The foundational link proof: our hand-emitted relocatable object (`main` calls the undefined
/// `logos_rt_probe`) links against the Rust-compiled runtime object via `rust-lld`, and the linked
/// module runs — `main()` returns `probe(41) = 42`. Skips cleanly if the wasm toolchain is absent.
#[test]
fn probe_object_links_against_runtime_and_runs() {
    if !toolchain_available() {
        eprintln!(
            "SKIP probe_object_links_against_runtime_and_runs: wasm link toolchain unavailable"
        );
        return;
    }
    let dir = std::env::temp_dir().join("logos_wasm_link_probe");
    let runtime = build_runtime_object(PROBE_RUNTIME_SRC, &dir).expect("build runtime object");
    let program = emit_probe_object();
    let linked = link_objects(&[&program, &runtime], &dir).expect("link program + runtime");

    // The linked module is valid wasm, and `main` resolves the cross-object call to the runtime.
    let engine = wasmi::Engine::default();
    let module = wasmi::Module::new(&engine, &linked[..]).expect("linked module is valid wasm");
    let mut store = wasmi::Store::new(&engine, ());
    let instance = wasmi::Linker::new(&engine)
        .instantiate(&mut store, &module)
        .expect("instantiate linked module")
        .start(&mut store)
        .expect("start linked module");
    let main = instance
        .get_typed_func::<(), i64>(&store, "main")
        .expect("main export");
    assert_eq!(
        main.call(&mut store, ()).expect("main runs"),
        42,
        "linked runtime call must return probe(41)"
    );
}

/// The shared-memory proof: our emitted `main` calls the runtime (which writes 42 into a static in
/// the shared linear memory and returns its pointer), then `i64.load`s that pointer. If the load
/// yields 42, our object and the runtime object genuinely share ONE linear memory after linking —
/// the prerequisite for every handle-returning runtime call (Text/BigInt/collections).
#[test]
fn emitted_code_reads_runtime_written_shared_memory() {
    if !toolchain_available() {
        eprintln!("SKIP emitted_code_reads_runtime_written_shared_memory: wasm link toolchain unavailable");
        return;
    }
    let dir = std::env::temp_dir().join("logos_wasm_link_mem");
    let runtime = build_runtime_object(MEM_RUNTIME_SRC, &dir).expect("build memory runtime object");
    let program = emit_load_object("logos_rt_write_answer");
    let linked = link_objects(&[&program, &runtime], &dir).expect("link program + runtime");

    let engine = wasmi::Engine::default();
    let module = wasmi::Module::new(&engine, &linked[..]).expect("linked module is valid wasm");
    let mut store = wasmi::Store::new(&engine, ());
    let instance = wasmi::Linker::new(&engine)
        .instantiate(&mut store, &module)
        .expect("instantiate linked module")
        .start(&mut store)
        .expect("start linked module");
    let main = instance
        .get_typed_func::<(), i64>(&store, "main")
        .expect("main export");
    assert_eq!(
        main.call(&mut store, ()).expect("main runs"),
        42,
        "the pointer the runtime returned must be valid in the shared memory our code loads from"
    );
}

/// The dynamic-allocation proof (S3a): the runtime `Box`-allocates the `i64` (via its global
/// allocator) rather than using a static, and our emitted `main` loads the returned pointer. If it
/// reads 42, the allocator links and runs on `wasm32-unknown-unknown` and allocations land in the
/// shared memory — the last prerequisite before handle-returning runtime calls (Text/BigInt).
#[test]
fn emitted_code_reads_runtime_heap_allocation() {
    if !toolchain_available() {
        eprintln!(
            "SKIP emitted_code_reads_runtime_heap_allocation: wasm link toolchain unavailable"
        );
        return;
    }
    let dir = std::env::temp_dir().join("logos_wasm_link_alloc");
    let runtime =
        build_runtime_object(ALLOC_RUNTIME_SRC, &dir).expect("build allocator runtime object");
    let program = emit_load_object("logos_rt_alloc_answer");
    let linked = link_objects(&[&program, &runtime], &dir).expect("link program + runtime");

    let engine = wasmi::Engine::default();
    let module = wasmi::Module::new(&engine, &linked[..]).expect("linked module is valid wasm");
    let mut store = wasmi::Store::new(&engine, ());
    let instance = wasmi::Linker::new(&engine)
        .instantiate(&mut store, &module)
        .expect("instantiate linked module")
        .start(&mut store)
        .expect("start linked module");
    let main = instance
        .get_typed_func::<(), i64>(&store, "main")
        .expect("main export");
    assert_eq!(
        main.call(&mut store, ()).expect("main runs"),
        42,
        "a runtime heap allocation must be readable through the shared memory"
    );
}

/// The Text-handle ABI proof (S3b): our emitted `main` calls the runtime to multiply 10¹² × 10¹²
/// = 10²⁴ (well past `i64`), and the runtime formats it and builds a `Text` in the emitter's own
/// `[len][cap][data_ptr]` layout in the shared memory. We read that Text back out of the module's
/// memory and assert the exact decimal — proving a runtime call can return a heap value the Show
/// path consumes, the shape every real BigInt/collection runtime call will use.
#[test]
fn runtime_returns_a_text_handle_our_side_reads() {
    if !toolchain_available() {
        eprintln!(
            "SKIP runtime_returns_a_text_handle_our_side_reads: wasm link toolchain unavailable"
        );
        return;
    }
    let dir = std::env::temp_dir().join("logos_wasm_link_text");
    let runtime =
        build_runtime_object(MUL_TEXT_RUNTIME_SRC, &dir).expect("build mul-text runtime object");
    let program = emit_mul_text_object(1_000_000_000_000, 1_000_000_000_000);
    let linked = link_objects(&[&program, &runtime], &dir).expect("link program + runtime");

    let engine = wasmi::Engine::default();
    let module = wasmi::Module::new(&engine, &linked[..]).expect("linked module is valid wasm");
    let mut store = wasmi::Store::new(&engine, ());
    let instance = wasmi::Linker::new(&engine)
        .instantiate(&mut store, &module)
        .expect("instantiate linked module")
        .start(&mut store)
        .expect("start linked module");
    let main = instance
        .get_typed_func::<(), i32>(&store, "main")
        .expect("main export");
    let handle = main.call(&mut store, ()).expect("main runs") as usize;

    // Read the Text `[len][cap][data_ptr]` header + bytes out of the shared linear memory.
    let mem = instance
        .get_memory(&store, "memory")
        .expect("exported memory");
    let data = mem.data(&store);
    let len = i32::from_le_bytes(data[handle..handle + 4].try_into().unwrap()) as usize;
    let dptr = i32::from_le_bytes(data[handle + 8..handle + 12].try_into().unwrap()) as usize;
    let text = std::str::from_utf8(&data[dptr..dptr + len]).expect("utf8 Text bytes");
    assert_eq!(
        text, "1000000000000000000000000",
        "the runtime-built Text handle must hold 10^24"
    );
}

/// S3-flagship: REAL arbitrary-precision BigInt through the linker. The runtime is a `std` crate
/// using the SAME `logicaffeine_base::BigInt` the VM uses (compiled to wasm32, linked against the
/// full sysroot rlib set + base/bumpalo); our emitted object drives the `logos_rt_bigint_*` ABI to
/// compute `(10¹²)^8 = 10⁹⁶` (97 digits, far past `i128`) and read back its decimal `Text`. Proves
/// the flagship "emitted wasm can't, the real runtime can" feature — no reimplementation, no
/// divergence from the VM. Skips if the wasm toolchain or a base wasm32 build is unavailable.
#[test]
fn runtime_computes_real_base_bigint_beyond_i128() {
    if !toolchain_available() {
        eprintln!(
            "SKIP runtime_computes_real_base_bigint_beyond_i128: wasm link toolchain unavailable"
        );
        return;
    }
    let dir = std::env::temp_dir().join("logos_wasm_link_bigint");
    let (runtime, mut rlibs) = match build_bigint_runtime(&dir) {
        Ok(v) => v,
        Err(_) => {
            eprintln!(
                "SKIP runtime_computes_real_base_bigint_beyond_i128: base wasm32 build unavailable"
            );
            return;
        }
    };
    // Link against base/bumpalo first, then the whole sysroot rlib set (libstd/dlmalloc/…). std
    // defines the OOM handler, so `emit_handler_shim = false` (a shim would duplicate it).
    rlibs.extend(wasm_all_sysroot_rlibs().expect("sysroot rlibs"));
    let program = emit_bigint_object();
    let linked = link_objects_with_rlibs(&[&program, &runtime], &rlibs, false, &dir)
        .expect("link bigint program");

    let engine = wasmi::Engine::default();
    let module = wasmi::Module::new(&engine, &linked[..]).expect("linked module is valid wasm");
    let mut store = wasmi::Store::new(&engine, ());
    let instance = wasmi::Linker::new(&engine)
        .instantiate(&mut store, &module)
        .expect("instantiate linked module")
        .start(&mut store)
        .expect("start linked module");
    let main = instance
        .get_typed_func::<(), i32>(&store, "main")
        .expect("main export");
    let handle = main.call(&mut store, ()).expect("main runs") as usize;

    let mem = instance
        .get_memory(&store, "memory")
        .expect("exported memory");
    let data = mem.data(&store);
    let len = i32::from_le_bytes(data[handle..handle + 4].try_into().unwrap()) as usize;
    let dptr = i32::from_le_bytes(data[handle + 8..handle + 12].try_into().unwrap()) as usize;
    let text = std::str::from_utf8(&data[dptr..dptr + len]).expect("utf8 Text bytes");
    let expected = format!("1{}", "0".repeat(96)); // 10^96
    assert_eq!(
        text, expected,
        "real base BigInt must compute 10^96 exactly (far beyond i128)"
    );
}

thread_local! {
    /// The wire frame the `read_wire_frame` host serves — a `readWireProgram`/`run_accepted` test
    /// installs a known-value frame here (via `encode_value_raw`) before running the linked module.
    static WIRE_FRAME: std::cell::RefCell<Vec<u8>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// `source`'s relocatable object, built by the shipped pipeline
/// ([`crate::compile::compile_to_wasm_relocatable`], the object `compile_to_wasm_linked` links), so
/// each linked row runs the module a user's program would become.
fn relocatable(source: &str) -> Result<Vec<u8>, String> {
    crate::compile::compile_to_wasm_relocatable(source).map_err(|e| format!("{e:?}"))
}

/// Instantiate a linked module and return everything it printed via `print_text` — the only sink a
/// `Show <BigInt Text>` uses — joined by newlines and read out of the shared memory exactly as the
/// AOT host does. After linking, `env.print_text` is the module's only remaining undefined import
/// (the `logos_rt_bigint_*` symbols + `__linear_memory` are resolved by `rust-lld`).
fn run_linked_capturing_text(linked: &[u8]) -> String {
    use std::cell::RefCell;
    use std::rc::Rc;
    let engine = wasmi::Engine::default();
    let module = wasmi::Module::new(&engine, linked).expect("linked module is valid wasm");
    let out: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let mut store = wasmi::Store::new(&engine, out.clone());
    let mut l = wasmi::Linker::<Rc<RefCell<Vec<String>>>>::new(&engine);
    // A `Show` of a scalar Int (e.g. a small power the optimizer folded to a constant) uses this
    // sink instead of the BigInt `print_text` path — provide both so the harness is robust to either.
    l.func_wrap(
        "env",
        "print_i64",
        |c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, v: i64| {
            c.data().borrow_mut().push(v.to_string());
        },
    )
    .unwrap();
    // ★ THE INLINE-FAILURE SINK. `logos_rt_fail_at(code, value)` is what an INLINE check calls when it
    // fails — a bounds test or a division guard — immediately before the `unreachable` that ends the
    // program. The emitter imports it whenever it lowers such a check (`module.rs` emits it for
    // `IndexOutOfBounds` and `DivisionByZero`), so ANY linked program containing one imports
    // `env::logos_rt_fail_at`, and the harness must define it or instantiation fails outright with
    // `MissingDefinition` — which is what `linked_crypto_uuid_in_logos_matches_the_vm` hit: a program
    // that never actually fails at run time still could not be INSTANTIATED, because the import is
    // resolved at link time and not on the failing path.
    //
    // The message is rendered host-side on purpose: the value only exists at run time, so there is no
    // constant for the module to hand over. `semantics::failcode::message` is the same renderer every
    // other tier uses, which is what keeps the sentence identical across them.
    l.func_wrap(
        "env",
        "logos_rt_fail_at",
        |c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, code: i32, value: i64| {
            c.data()
                .borrow_mut()
                .push(crate::semantics::failcode::message(code, value));
        },
    )
    .unwrap();
    // The value runtime's failure sink. `logos_rt_fail(text)` is what the linked runtime calls when an
    // operation refuses in the VM's words — a rational's `%` by a fraction, a power past a word — the
    // sentence a text handle in the module's memory. Every program that links the rational runtime
    // imports it, so the harness defines it or such a program cannot be instantiated at all.
    l.func_wrap(
        "env",
        "logos_rt_fail",
        |c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, h: i32| {
            let mem = c.get_export("memory").unwrap().into_memory().unwrap();
            let d = mem.data(&c);
            let h = h as usize;
            let len = i32::from_le_bytes(d[h..h + 4].try_into().unwrap()) as usize;
            let dp = i32::from_le_bytes(d[h + 8..h + 12].try_into().unwrap()) as usize;
            let sentence = String::from_utf8_lossy(&d[dp..dp + len]).to_string();
            c.data().borrow_mut().push(sentence);
        },
    )
    .unwrap();
    // A `Show` of a Bool (e.g. a `uuid == uuid` result) renders lowercase `true`/`false`, matching the
    // VM's `RuntimeValue::Bool` display.
    l.func_wrap(
        "env",
        "print_bool",
        |c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, v: i32| {
            c.data()
                .borrow_mut()
                .push(if v != 0 { "true" } else { "false" }.to_string());
        },
    )
    .unwrap();
    l.func_wrap(
        "env",
        "print_text",
        |c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, h: i32| {
            let mem = c.get_export("memory").unwrap().into_memory().unwrap();
            let d = mem.data(&c);
            let h = h as usize;
            let len = i32::from_le_bytes(d[h..h + 4].try_into().unwrap()) as usize;
            let dp = i32::from_le_bytes(d[h + 8..h + 12].try_into().unwrap()) as usize;
            c.data()
                .borrow_mut()
                .push(String::from_utf8_lossy(&d[dp..dp + len]).to_string());
        },
    )
    .unwrap();
    // A whole `Seq of Int` (e.g. `uuid_bytes(u)`): `[n, …]` — each 8-byte slot a signed decimal.
    l.func_wrap(
        "env",
        "print_seq_i64",
        |c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, h: i32| {
            let mem = c.get_export("memory").unwrap().into_memory().unwrap();
            let d = mem.data(&c);
            let h = h as usize;
            let len = i32::from_le_bytes(d[h..h + 4].try_into().unwrap()) as usize;
            let dp = i32::from_le_bytes(d[h + 8..h + 12].try_into().unwrap()) as usize;
            let items: Vec<String> = (0..len)
                .map(|i| {
                    i64::from_le_bytes(d[dp + i * 8..dp + i * 8 + 8].try_into().unwrap())
                        .to_string()
                })
                .collect();
            c.data()
                .borrow_mut()
                .push(format!("[{}]", items.join(", ")));
        },
    )
    .unwrap();
    // A `Moment` (nanos-since-epoch) formats via the VM's own display (for `Moment ± Span` results).
    l.func_wrap(
        "env",
        "print_moment",
        |c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, nanos: i64| {
            c.data()
                .borrow_mut()
                .push(crate::interpreter::RuntimeValue::Moment(nanos).to_display_string());
        },
    )
    .unwrap();
    // A Float `Show` — e.g. `abs(complex(3,4))` (the modulus) — renders exactly as the VM's `Float`.
    // wasmi host-func floats use the `core::F64` wrapper, not raw `f64`.
    l.func_wrap(
        "env",
        "print_f64",
        |c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, x: wasmi::core::F64| {
            let x: f64 = x.into();
            c.data()
                .borrow_mut()
                .push(crate::interpreter::RuntimeValue::Float(x).to_display_string());
        },
    )
    .unwrap();
    // A whole `Seq of Word32` (e.g. `seqOfLanes4W32(v)`): `[u, …]` — each 8-byte slot's low word as an
    // UNSIGNED decimal, matching the VM's `RuntimeValue::List` of `Word`.
    l.func_wrap(
        "env",
        "print_seq_word32",
        |c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, h: i32| {
            let mem = c.get_export("memory").unwrap().into_memory().unwrap();
            let d = mem.data(&c);
            let h = h as usize;
            let len = i32::from_le_bytes(d[h..h + 4].try_into().unwrap()) as usize;
            let dp = i32::from_le_bytes(d[h + 8..h + 12].try_into().unwrap()) as usize;
            let items: Vec<String> = (0..len)
                .map(|i| {
                    u32::from_le_bytes(d[dp + i * 8..dp + i * 8 + 4].try_into().unwrap())
                        .to_string()
                })
                .collect();
            c.data()
                .borrow_mut()
                .push(format!("[{}]", items.join(", ")));
        },
    )
    .unwrap();
    // `parse_timestamp(text) -> Moment nanos` via the same RFC-3339 parser the VM uses.
    l.func_wrap(
        "env",
        "parse_timestamp",
        |c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, h: i32| -> i64 {
            let mem = c.get_export("memory").unwrap().into_memory().unwrap();
            let d = mem.data(&c);
            let h = h as usize;
            let len = i32::from_le_bytes(d[h..h + 4].try_into().unwrap()) as usize;
            let dp = i32::from_le_bytes(d[h + 8..h + 12].try_into().unwrap()) as usize;
            let s = std::str::from_utf8(&d[dp..dp + len]).unwrap().trim();
            logicaffeine_base::temporal::parse_rfc3339(s).expect("valid RFC 3339 timestamp")
        },
    )
    .unwrap();
    // `read_wire_frame(buf, max) -> len` — the embedder-side stdin frame source: write the wire frame
    // the test installed in `WIRE_FRAME` into the module's memory at `buf`, return its byte length.
    l.func_wrap(
        "env",
        "read_wire_frame",
        |mut c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, buf: i32, max: i32| -> i32 {
            let frame = WIRE_FRAME.with(|f| f.borrow().clone());
            let n = frame.len().min(max as usize);
            let mem = c.get_export("memory").unwrap().into_memory().unwrap();
            let d = mem.data_mut(&mut c);
            d[buf as usize..buf as usize + n].copy_from_slice(&frame[..n]);
            n as i32
        },
    )
    .unwrap();
    let instance = l
        .instantiate(&mut store, &module)
        .expect("instantiate linked module")
        .start(&mut store)
        .expect("start");
    instance
        .get_typed_func::<(), ()>(&store, "main")
        .expect("main export")
        .call(&mut store, ())
        .expect("main runs");
    let lines = out.borrow().clone();
    lines.join("\n")
}

/// A tiny deterministic LCG step (no `Math.random`/`Date` — reproducible so a failure repro's exactly).
fn lcg(state: &mut u64) -> u32 {
    *state = state
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    (*state >> 33) as u32
}
/// A random arithmetic LEAF: a small int, an OVERFLOWING power (drives the BigInt path), or a larger int.
fn fuzz_leaf(state: &mut u64) -> String {
    match lcg(state) % 3 {
        0 => format!("{}", 1 + lcg(state) % 90),
        1 => format!(
            "({} to the power of {})",
            2 + lcg(state) % 4,
            10 + lcg(state) % 30
        ),
        _ => format!("{}", 1 + lcg(state) % 5_000_000),
    }
}
/// A random arithmetic expression tree — every internal node parenthesized (explicit precedence), and
/// every `divided by` divisor a POSITIVE leaf (so no division by zero is ever generated).
fn fuzz_expr(state: &mut u64, depth: u32) -> String {
    if depth == 0 || lcg(state) % 3 == 0 {
        return fuzz_leaf(state);
    }
    let l = fuzz_expr(state, depth - 1);
    match lcg(state) % 4 {
        0 => format!("({l} plus {})", fuzz_expr(state, depth - 1)),
        1 => format!("({l} minus {})", fuzz_expr(state, depth - 1)),
        2 => format!("({l} times {})", fuzz_expr(state, depth - 1)),
        _ => format!("({l} divided by {})", 1 + lcg(state) % 90),
    }
}

/// THE PIT OF SUCCESS: a deterministic fuzz stream of random nested integer arithmetic (`+ - * /` and
/// overflowing powers, all through the BigInt tower) — every program, compiled via the linker and run,
/// must print BYTE-IDENTICALLY to the tree-walking VM. Any future change that makes the linked WASM
/// diverge from the VM on ANY arithmetic shape fails HERE. Sound-refusal is respected: an emitter
/// decline (Err) is skipped (never a divergence), but a DIFFERENT answer is a hard failure. Fixed seed
/// ⇒ a failure reproduces exactly. Skips if the toolchain / base wasm32 build is unavailable.
#[test]
fn fuzz_linked_bigint_arithmetic_is_byte_identical_to_the_vm() {
    if !toolchain_available() {
        eprintln!("SKIP fuzz_linked_bigint_arithmetic_is_byte_identical_to_the_vm: wasm link toolchain unavailable");
        return;
    }
    let dir = std::env::temp_dir().join("logos_wasm_link_fuzz");
    let (runtime, mut rlibs) = match build_bigint_runtime(&dir) {
        Ok(v) => v,
        Err(_) => {
            eprintln!("SKIP fuzz_linked_bigint_arithmetic_...: base wasm32 build unavailable");
            return;
        }
    };
    rlibs.extend(wasm_all_sysroot_rlibs().expect("sysroot rlibs"));

    let mut state: u64 = 0x2545_F491_4F6C_DD1D;
    let mut checked = 0;
    for _ in 0..40 {
        let src = format!("## Main\nShow {}.\n", fuzz_expr(&mut state, 3));
        let vm = crate::compile::vm_outcome(&src);
        if vm.error.is_some() {
            continue; // the VM itself rejects this shape (not expected here) — skip
        }
        let reloc = match relocatable(&src) {
            Ok(r) => r,
            Err(_) => continue, // the linked emitter SOUNDLY declines this shape — a refusal, not a divergence
        };
        let linked = link_objects_with_rlibs(&[&reloc, &runtime], &rlibs, false, &dir)
            .unwrap_or_else(|e| panic!("link `{src}`: {e}"));
        let got = run_linked_capturing_text(&linked);
        assert_eq!(
            got.trim(),
            vm.output.trim(),
            "linked wasm DIVERGED from the VM on:\n{src}"
        );
        checked += 1;
    }
    assert!(
        checked >= 20,
        "fuzz exercised too few programs ({checked}/40) — the generator or skip logic is off"
    );
}

/// LINKED `Complex` (numeric-tower phase 1): `complex(re, im)` builds an EXACT `logicaffeine_base::
/// Complex` (Rational components) in the runtime; `+ - *` drive `logos_rt_complex_*`; `Show` renders
/// it via `to_text`. `i * i = -1`, `(2+3i)+(1-i) = 3+2i`, `(1+i)(1-i) = 2` — the exactness the
/// tree-walker gives must survive the linker byte-identically. Skips if the toolchain is unavailable.
#[test]
fn linked_complex_arithmetic_matches_the_vm() {
    if !toolchain_available() {
        eprintln!("SKIP linked_complex_arithmetic_matches_the_vm: wasm link toolchain unavailable");
        return;
    }
    let dir = std::env::temp_dir().join("logos_wasm_link_complex");
    let (runtime, mut rlibs) = match build_bigint_runtime(&dir) {
        Ok(v) => v,
        Err(_) => {
            eprintln!(
                "SKIP linked_complex_arithmetic_matches_the_vm: base wasm32 build unavailable"
            );
            return;
        }
    };
    rlibs.extend(wasm_all_sysroot_rlibs().expect("sysroot rlibs"));
    for src in [
        "## Main\n    Let i be complex(0, 1).\n    Show i * i.\n", // -1
        "## Main\n    Let a be complex(2, 3).\n    Let b be complex(1, 0 - 1).\n    Show a + b.\n", // 3+2i
        "## Main\n    Let a be complex(1, 1).\n    Let b be complex(1, 0 - 1).\n    Show a * b.\n", // 2
        "## Main\n    Let z be complex(2, 3).\n    Show z.\n", // 2+3i
        "## Main\n    Let a be complex(5, 2).\n    Let b be complex(3, 4).\n    Show a - b.\n", // 2-2i
        "## Main\n    Show abs(complex(3, 4)).\n", // 5 (|3+4i| via logos_rt_complex_abs)
    ] {
        let vm = crate::compile::vm_outcome(src);
        assert!(
            vm.error.is_none(),
            "the VM itself errored on `{src}`: {:?}",
            vm.error
        );
        let reloc = relocatable(src).unwrap_or_else(|e| panic!("compile-reloc `{src}`: {e}"));
        let linked = link_objects_with_rlibs(&[&reloc, &runtime], &rlibs, false, &dir)
            .unwrap_or_else(|e| panic!("link `{src}`: {e}"));
        let got = run_linked_capturing_text(&linked);
        assert_eq!(
            got.trim(),
            vm.output.trim(),
            "linked Complex DIVERGED from the VM on:\n{src}"
        );
    }
}

/// LINKED FLOAT-complex (`ComplexF`): the `c*` transcendentals build `complex(realExpr, realExpr)`
/// → `logos_rt_complex_from_f64` (parts as exact dyadic Rationals, round-trip), and `re`/`im` extract
/// via `logos_rt_complex_re_f64`/`_im_f64`. This closes the LAST tier for the complex float family —
/// `cExp`/`cLog`/`cSqrt`/`cConj`/`cLog10` + the inverse trig — proving linked wasm ≡ VM byte-for-byte.
#[test]
fn linked_float_complex_matches_the_vm() {
    if !toolchain_available() {
        eprintln!("SKIP linked_float_complex_matches_the_vm: wasm link toolchain unavailable");
        return;
    }
    let dir = std::env::temp_dir().join("logos_wasm_link_fcomplex");
    let (runtime, mut rlibs) = match build_bigint_runtime(&dir) {
        Ok(v) => v,
        Err(_) => {
            eprintln!("SKIP linked_float_complex_matches_the_vm: base wasm32 build unavailable");
            return;
        }
    };
    rlibs.extend(wasm_all_sysroot_rlibs().expect("sysroot rlibs"));
    for src in [
        "## Main\n    Show cExp(0.0, 0.0).\n",   // 1
        "## Main\n    Show cSqrt(4.0, 0.0).\n",  // 2
        "## Main\n    Show cConj(3, 4).\n",      // 3-4i (float parts)
        "## Main\n    Show cLog10(1.0, 0.0).\n", // 0
        "## Main\n    Show floor(re(cAsinh(0.5, 0.5)) * 1000.0).\n", // 530
        "## Main\n    Show floor(im(cAtan(2.0, 1.0)) * 1000.0).\n", // 173
        "## Main\n    Show floor(re(cAcos(0.5, 0.5)) * 1000.0).\n", // 1118
    ] {
        let vm = crate::compile::vm_outcome(src);
        assert!(
            vm.error.is_none(),
            "the VM itself errored on `{src}`: {:?}",
            vm.error
        );
        let reloc = relocatable(src).unwrap_or_else(|e| panic!("compile-reloc `{src}`: {e}"));
        let linked = link_objects_with_rlibs(&[&reloc, &runtime], &rlibs, false, &dir)
            .unwrap_or_else(|e| panic!("link `{src}`: {e}"));
        let got = run_linked_capturing_text(&linked);
        assert_eq!(
            got.trim(),
            vm.output.trim(),
            "linked float-Complex DIVERGED from the VM on:\n{src}"
        );
    }
}

/// The float-complex LOWERING assembles to a valid linked wasm module — exercises module.rs's
/// `complex(floatExpr)` → `ComplexFromF64` (Int parts converted via `f64.convert_i64_s`), the new
/// `re`/`im` → `logos_rt_complex_re_f64`/`_im_f64` extractors, and the `ComplexF` → `Kind::Complex`
/// flow. Runs NATIVELY (no wasm32 runtime), so it verifies the lowering even where the linked-
/// EXECUTION harness (`linked_float_complex_matches_the_vm`) skips on an unbuildable runtime. A
/// declined op would surface as `Err(Unsupported(..))` here.
#[test]
fn float_complex_lowers_to_linked_wasm() {
    for src in [
        "## Main\n    Show cExp(0.0, 0.0).\n",
        "## Main\n    Show cSqrt(4.0, 0.0).\n",
        "## Main\n    Show cConj(3, 4).\n",
        "## Main\n    Show cLog10(1.0, 0.0).\n",
        "## Main\n    Show floor(re(cAsinh(0.5, 0.5)) * 1000.0).\n",
        "## Main\n    Show floor(im(cAtan(2.0, 1.0)) * 1000.0).\n",
    ] {
        let reloc = relocatable(src);
        assert!(
            reloc.is_ok(),
            "float-complex program FAILED to assemble to linked wasm:\n{src}\nerror: {:?}",
            reloc.err()
        );
    }
}

/// LINKED `Modular` (numeric-tower ℤ/nℤ): `modular(v, n)` reduces on construction; `+ - *` wrap in
/// the ring, `/` multiplies by the modular inverse, and `pow(m, e)` is modular exponentiation — all
/// exact, byte-identical to the VM. `modular(10,7) = 3 (mod 7)`, `(5+4) mod 7 = 2`, `(5*4) mod 7 = 6`,
/// `(3-5) mod 7 = 5`, `(3/5) mod 7 = 2`, `2^5 mod 7 = 4`. Skips if the toolchain is unavailable.
#[test]
fn linked_modular_arithmetic_matches_the_vm() {
    if !toolchain_available() {
        eprintln!("SKIP linked_modular_arithmetic_matches_the_vm: wasm link toolchain unavailable");
        return;
    }
    let dir = std::env::temp_dir().join("logos_wasm_link_modular");
    let (runtime, mut rlibs) = match build_bigint_runtime(&dir) {
        Ok(v) => v,
        Err(_) => {
            eprintln!(
                "SKIP linked_modular_arithmetic_matches_the_vm: base wasm32 build unavailable"
            );
            return;
        }
    };
    rlibs.extend(wasm_all_sysroot_rlibs().expect("sysroot rlibs"));
    for src in [
        "## Main\n    Let a be modular(10, 7).\n    Show a.\n", // 3 (mod 7)
        "## Main\n    Let a be modular(5, 7).\n    Let b be modular(4, 7).\n    Show a + b.\n", // 2 (mod 7)
        "## Main\n    Let a be modular(5, 7).\n    Let b be modular(4, 7).\n    Show a * b.\n", // 6 (mod 7)
        "## Main\n    Let a be modular(3, 7).\n    Let b be modular(5, 7).\n    Show a - b.\n", // 5 (mod 7)
        "## Main\n    Let a be modular(3, 7).\n    Let b be modular(5, 7).\n    Show a / b.\n", // 2 (mod 7)
        "## Main\n    Let a be modular(1, 7).\n    Let b be modular(3, 7).\n    Show a / b.\n", // 5 (mod 7)
        "## Main\n    Show pow(modular(2, 7), 5).\n", // 4 (mod 7)
        "## Main\n    Show pow(modular(3, 10), 4).\n", // 1 (mod 10)
    ] {
        let vm = crate::compile::vm_outcome(src);
        assert!(
            vm.error.is_none(),
            "the VM itself errored on `{src}`: {:?}",
            vm.error
        );
        let reloc = relocatable(src).unwrap_or_else(|e| panic!("compile-reloc `{src}`: {e}"));
        let linked = link_objects_with_rlibs(&[&reloc, &runtime], &rlibs, false, &dir)
            .unwrap_or_else(|e| panic!("link `{src}`: {e}"));
        let got = run_linked_capturing_text(&linked);
        assert_eq!(
            got.trim(),
            vm.output.trim(),
            "linked Modular DIVERGED from the VM on:\n{src}"
        );
    }
}

/// LINKED `Decimal` (exact base-10): `decimal("0.1") + decimal("0.2")` = `0.3` EXACTLY (not the
/// float `0.30000000000000004`); `*`/`-` keep exact scale; an Int operand promotes. Construction
/// parses a Text handle read from the SHARED memory. Byte-identical to the VM. Skips w/o toolchain.
#[test]
fn linked_decimal_arithmetic_matches_the_vm() {
    if !toolchain_available() {
        eprintln!("SKIP linked_decimal_arithmetic_matches_the_vm: wasm link toolchain unavailable");
        return;
    }
    let dir = std::env::temp_dir().join("logos_wasm_link_decimal");
    let (runtime, mut rlibs) = match build_bigint_runtime(&dir) {
        Ok(v) => v,
        Err(_) => {
            eprintln!(
                "SKIP linked_decimal_arithmetic_matches_the_vm: base wasm32 build unavailable"
            );
            return;
        }
    };
    rlibs.extend(wasm_all_sysroot_rlibs().expect("sysroot rlibs"));
    for src in [
        "## Main\n    Let a be decimal(\"0.1\").\n    Let b be decimal(\"0.2\").\n    Show a + b.\n",   // 0.3 EXACT
        "## Main\n    Let a be decimal(\"19.99\").\n    Let b be decimal(\"0.01\").\n    Show a + b.\n", // 20.00
        "## Main\n    Let a be decimal(\"5.5\").\n    Let b be decimal(\"2.2\").\n    Show a * b.\n",     // exact scale
        "## Main\n    Let a be decimal(\"5.75\").\n    Let b be decimal(\"1.25\").\n    Show a - b.\n",   // 4.50
        "## Main\n    Let x be decimal(\"19.99\").\n    Show x.\n",                                      // 19.99
        "## Main\n    Let p be decimal(\"19.99\").\n    Show p * 3.\n",                                  // Int promotion
    ] {
        let vm = crate::compile::vm_outcome(src);
        assert!(vm.error.is_none(), "the VM itself errored on `{src}`: {:?}", vm.error);
        let reloc = relocatable(src).unwrap_or_else(|e| panic!("compile-reloc `{src}`: {e}"));
        let linked = link_objects_with_rlibs(&[&reloc, &runtime], &rlibs, false, &dir)
            .unwrap_or_else(|e| panic!("link `{src}`: {e}"));
        let got = run_linked_capturing_text(&linked);
        assert_eq!(got.trim(), vm.output.trim(), "linked Decimal DIVERGED from the VM on:\n{src}");
    }
}

/// LINKED `Money` (exact amount + currency): `money(amount, "USD")` (amount an Int or a Decimal);
/// `+ -` require matching currencies; renders with the currency's scale/symbol. Byte-identical to
/// the VM. `$0.10 + $0.20 = $0.30`, `money(100,"JPY")` (scale 0). Skips without the toolchain.
#[test]
fn linked_money_arithmetic_matches_the_vm() {
    if !toolchain_available() {
        eprintln!("SKIP linked_money_arithmetic_matches_the_vm: wasm link toolchain unavailable");
        return;
    }
    let dir = std::env::temp_dir().join("logos_wasm_link_money");
    let (runtime, mut rlibs) = match build_bigint_runtime(&dir) {
        Ok(v) => v,
        Err(_) => {
            eprintln!("SKIP linked_money_arithmetic_matches_the_vm: base wasm32 build unavailable");
            return;
        }
    };
    rlibs.extend(wasm_all_sysroot_rlibs().expect("sysroot rlibs"));
    for src in [
        "## Main\n    Let a be money(decimal(\"0.10\"), \"USD\").\n    Let b be money(decimal(\"0.20\"), \"USD\").\n    Show a + b.\n", // $0.30
        "## Main\n    Let a be money(decimal(\"30.00\"), \"USD\").\n    Let b be money(decimal(\"10.00\"), \"USD\").\n    Show a - b.\n", // $20.00
        "## Main\n    Let m be money(10, \"USD\").\n    Show m.\n",   // $10.00 (Int amount)
        "## Main\n    Let m be money(100, \"JPY\").\n    Show m.\n",  // ¥100 (scale 0)
        "## Main\n    Let m be money(decimal(\"19.99\"), \"USD\").\n    Show m.\n",
    ] {
        let vm = crate::compile::vm_outcome(src);
        assert!(vm.error.is_none(), "the VM itself errored on `{src}`: {:?}", vm.error);
        let reloc = relocatable(src).unwrap_or_else(|e| panic!("compile-reloc `{src}`: {e}"));
        let linked = link_objects_with_rlibs(&[&reloc, &runtime], &rlibs, false, &dir)
            .unwrap_or_else(|e| panic!("link `{src}`: {e}"));
        let got = run_linked_capturing_text(&linked);
        assert_eq!(got.trim(), vm.output.trim(), "linked Money DIVERGED from the VM on:\n{src}");
    }
}

/// LINKED `Quantity` (exact magnitude + display unit): the `5 meters` literal and the `quantity(v,
/// "unit")` builtin construct one; `X in <unit>` (`convert`) re-expresses it; `+ -` keep the left
/// unit (dimension-checked); `× ÷` combine dimensions and render in the SI/dimension form. The
/// magnitude rides the exact rational tower, so `5 m in feet` is EXACTLY `6250/381 ft`. Every case
/// must print BYTE-IDENTICALLY to the VM. Skips without the toolchain / base wasm32 build.
#[test]
fn linked_quantity_arithmetic_matches_the_vm() {
    if !toolchain_available() {
        eprintln!(
            "SKIP linked_quantity_arithmetic_matches_the_vm: wasm link toolchain unavailable"
        );
        return;
    }
    let dir = std::env::temp_dir().join("logos_wasm_link_quantity");
    let (runtime, mut rlibs) = match build_bigint_runtime(&dir) {
        Ok(v) => v,
        Err(_) => {
            eprintln!(
                "SKIP linked_quantity_arithmetic_matches_the_vm: base wasm32 build unavailable"
            );
            return;
        }
    };
    rlibs.extend(wasm_all_sysroot_rlibs().expect("sysroot rlibs"));
    for src in [
        "## Main\n    Let d be 5 meters.\n    Show d.\n", // 5 m
        "## Main\n    Show quantity(2, \"inch\").\n",     // 2 in
        "## Main\n    Show 2 meters + 3 meters.\n",       // 5 m
        "## Main\n    Show 5 meters - 2 meters.\n",       // 3 m
        "## Main\n    Let d be 5 meters.\n    Show d in feet.\n", // 6250/381 ft (exact)
        "## Main\n    Show 2 meters * 3 meters.\n",       // combined dimension (× )
        "## Main\n    Show 100 meters / 4 meters.\n",     // combined dimension (÷ → dimensionless)
    ] {
        let vm = crate::compile::vm_outcome(src);
        assert!(
            vm.error.is_none(),
            "the VM itself errored on `{src}`: {:?}",
            vm.error
        );
        let reloc = relocatable(src).unwrap_or_else(|e| panic!("compile-reloc `{src}`: {e}"));
        let linked = link_objects_with_rlibs(&[&reloc, &runtime], &rlibs, false, &dir)
            .unwrap_or_else(|e| panic!("link `{src}`: {e}"));
        let got = run_linked_capturing_text(&linked);
        assert_eq!(
            got.trim(),
            vm.output.trim(),
            "linked Quantity DIVERGED from the VM on:\n{src}"
        );
    }
}

/// LINKED `Rational` (exact BigInt-backed fraction): `Let x: Rational be 7 / 2` and `+ - * /` on
/// rationals, Int operands widening (`r + 3`), reduce-to-whole (`1/2 * 4 = 2` shows `2`). num/den
/// ride the BigInt tower so a sum whose denominator EXCEEDS i64 (`1/10^12 + 1/(10^12-1)`, den 10^24)
/// stays exact — the self-contained i64/i64 form would overflow, the linked runtime matches the VM.
/// Every case must print BYTE-IDENTICALLY to the VM. Skips without the toolchain / base wasm32 build.
#[test]
fn linked_rational_arithmetic_matches_the_vm() {
    if !toolchain_available() {
        eprintln!(
            "SKIP linked_rational_arithmetic_matches_the_vm: wasm link toolchain unavailable"
        );
        return;
    }
    let dir = std::env::temp_dir().join("logos_wasm_link_rational");
    let (runtime, mut rlibs) = match build_bigint_runtime(&dir) {
        Ok(v) => v,
        Err(_) => {
            eprintln!(
                "SKIP linked_rational_arithmetic_matches_the_vm: base wasm32 build unavailable"
            );
            return;
        }
    };
    rlibs.extend(wasm_all_sysroot_rlibs().expect("sysroot rlibs"));
    for src in [
        "## Main\n    Let x: Rational be 7 / 2.\n    Show x.\n",                                  // 7/2
        "## Main\n    Let x: Rational be 6 / 2.\n    Show x.\n",                                  // 3 (reduces to whole)
        "## Main\n    Let a: Rational be 1 / 3.\n    Let b: Rational be 1 / 6.\n    Show a + b.\n", // 1/2
        "## Main\n    Let r: Rational be 1 / 2.\n    Show r + 3.\n",                              // 7/2 (Int widens)
        "## Main\n    Let r: Rational be 1 / 2.\n    Show r * 4.\n",                              // 2 (reduces to whole)
        "## Main\n    Let r: Rational be 3 / 4.\n    Show r - 1 / 4.\n",                          // 1/2
        "## Main\n    Let a: Rational be 1 / 2.\n    Let b: Rational be 1 / 3.\n    Show a / b.\n", // 3/2
        // Denominator 10^24 exceeds i64 (max ~9.2·10^18): the linked BigInt Rational stays exact.
        "## Main\n    Let a: Rational be 1 / 1000000000000.\n    Let b: Rational be 1 / 999999999999.\n    Show a + b.\n",
        // Exact rounding of a Rational: floor/ceil/round yield the BigInt the fraction rounds to,
        // abs stays a Rational — never the lossy `as f64` path.
        "## Main\n    Let r: Rational be 7 / 2.\n    Show floor(r).\n",  // 3
        "## Main\n    Let r: Rational be 7 / 2.\n    Show ceil(r).\n",   // 4
        "## Main\n    Let r: Rational be 7 / 2.\n    Show round(r).\n",  // 4
        "## Main\n    Let r: Rational be -7 / 2.\n    Show floor(r).\n", // -4
        "## Main\n    Let r: Rational be -7 / 2.\n    Show abs(r).\n",   // 7/2
    ] {
        let vm = crate::compile::vm_outcome(src);
        assert!(vm.error.is_none(), "the VM itself errored on `{src}`: {:?}", vm.error);
        let reloc = relocatable(src).unwrap_or_else(|e| panic!("compile-reloc `{src}`: {e}"));
        let linked = link_objects_with_rlibs(&[&reloc, &runtime], &rlibs, false, &dir)
            .unwrap_or_else(|e| panic!("link `{src}`: {e}"));
        let got = run_linked_capturing_text(&linked);
        assert_eq!(got.trim(), vm.output.trim(), "linked Rational DIVERGED from the VM on:\n{src}");
    }
}

/// LINKED `Uuid` (RFC 9562): `uuid("…")` parses, `uuid_nil`/`uuid_max`/`uuid_dns`/… are the well-known
/// constants, `uuid_version` reads the version nibble, equality compares the 16 bytes, and `Show`
/// renders the canonical lowercase form — all via `base::Uuid` (parse + Display), byte-identical to
/// the VM. Every case must print BYTE-IDENTICALLY to the VM. Skips without the toolchain / base build.
#[test]
fn linked_uuid_matches_the_vm() {
    if !toolchain_available() {
        eprintln!("SKIP linked_uuid_matches_the_vm: wasm link toolchain unavailable");
        return;
    }
    let dir = std::env::temp_dir().join("logos_wasm_link_uuid");
    let (runtime, mut rlibs) = match build_bigint_runtime(&dir) {
        Ok(v) => v,
        Err(_) => {
            eprintln!("SKIP linked_uuid_matches_the_vm: base wasm32 build unavailable");
            return;
        }
    };
    rlibs.extend(wasm_all_sysroot_rlibs().expect("sysroot rlibs"));
    for src in [
        "## Main\n    Let u be uuid(\"550e8400-e29b-41d4-a716-446655440000\").\n    Show u.\n", // round-trips canonical
        "## Main\n    Show uuid_nil().\n",   // 00000000-0000-0000-0000-000000000000
        "## Main\n    Show uuid_max().\n",   // ffffffff-ffff-ffff-ffff-ffffffffffff
        "## Main\n    Show uuid_dns().\n",   // 6ba7b810-9dad-11d1-80b4-00c04fd430c8
        "## Main\n    Show uuid_oid().\n",
        "## Main\n    Show uuid_version(uuid(\"550e8400-e29b-41d4-a716-446655440000\")).\n", // 4
        "## Main\n    Show uuid_nil() is equal to uuid_nil().\n",  // true
        "## Main\n    Show uuid_nil() is equal to uuid_max().\n",  // false
        "## Main\n    Let a be uuid(\"550e8400-e29b-41d4-a716-446655440000\").\n    Let b be uuid(\"550E8400-E29B-41D4-A716-446655440000\").\n    Show a is equal to b.\n", // true (case-insensitive parse)
        "## Main\n    Show uuid_bytes(uuid(\"550e8400-e29b-41d4-a716-446655440000\")).\n", // [85, 14, 132, 0, …] the 16 bytes
        "## Main\n    Show uuid_from_bytes(uuid_bytes(uuid(\"550e8400-e29b-41d4-a716-446655440000\"))).\n", // round-trips to the same canonical
    ] {
        let vm = crate::compile::vm_outcome(src);
        assert!(vm.error.is_none(), "the VM itself errored on `{src}`: {:?}", vm.error);
        let reloc = relocatable(src).unwrap_or_else(|e| panic!("compile-reloc `{src}`: {e}"));
        let linked = link_objects_with_rlibs(&[&reloc, &runtime], &rlibs, false, &dir)
            .unwrap_or_else(|e| panic!("link `{src}`: {e}"));
        let got = run_linked_capturing_text(&linked);
        assert_eq!(got.trim(), vm.output.trim(), "linked Uuid DIVERGED from the VM on:\n{src}");
    }
}

/// LINKED `Span` CALENDAR arithmetic: `Moment ± <span>` is CIVIL (months clamp end-of-month + respect
/// leap years, the time-of-day rides along) via `logos_rt_moment_add_span` (base's own add_span). The
/// killer case is `Jan 31 + 1 month = Feb 29` in a leap year. Byte-identical to the VM. Skips w/o toolchain.
#[test]
fn linked_span_calendar_arithmetic_matches_the_vm() {
    if !toolchain_available() {
        eprintln!(
            "SKIP linked_span_calendar_arithmetic_matches_the_vm: wasm link toolchain unavailable"
        );
        return;
    }
    let dir = std::env::temp_dir().join("logos_wasm_link_span");
    let (runtime, mut rlibs) = match build_bigint_runtime(&dir) {
        Ok(v) => v,
        Err(_) => {
            eprintln!("SKIP linked_span_calendar_arithmetic_matches_the_vm: base wasm32 build unavailable");
            return;
        }
    };
    rlibs.extend(wasm_all_sysroot_rlibs().expect("sysroot rlibs"));
    for src in [
        "## Main\n    Show parse_timestamp(\"2024-01-31T00:00:00Z\") + 1 month.\n", // Feb 29 (leap-year clamp)
        "## Main\n    Show parse_timestamp(\"2024-01-31T00:00:00Z\") - 1 month.\n", // 2023-12-31
        "## Main\n    Show parse_timestamp(\"2023-01-31T00:00:00Z\") + 1 month.\n", // Feb 28 (non-leap clamp)
        "## Main\n    Show parse_timestamp(\"2024-01-15T12:00:00Z\") + 45 days.\n", // days ride the time-of-day
        "## Main\n    Show parse_timestamp(\"2020-02-29T00:00:00Z\") + 1 year.\n", // Feb 28, 2021 (12 months)
    ] {
        let vm = crate::compile::vm_outcome(src);
        assert!(
            vm.error.is_none(),
            "the VM itself errored on `{src}`: {:?}",
            vm.error
        );
        let reloc = relocatable(src).unwrap_or_else(|e| panic!("compile-reloc `{src}`: {e}"));
        let linked = link_objects_with_rlibs(&[&reloc, &runtime], &rlibs, false, &dir)
            .unwrap_or_else(|e| panic!("link `{src}`: {e}"));
        let got = run_linked_capturing_text(&linked);
        assert_eq!(
            got.trim(),
            vm.output.trim(),
            "linked Span DIVERGED from the VM on:\n{src}"
        );
    }
}

/// LINKED extended temporal — the calendar-logic builtins whose exact answer lives in `base::temporal`:
/// `format_timestamp(m)` renders a Moment to its RFC-3339 UTC `Text` (the round-trip that closes the
/// timestamp story), `months_between`/`years_between` count complete calendar months/years. Each calls
/// the SAME `base::temporal` the VM uses (via `logos_rt_format_timestamp`/`_months_between`/
/// `_years_between`), so the linked output is bit-identical. `format_timestamp(add_seconds(a, 90))`
/// composes a SELF-CONTAINED `add_seconds` (inline i64) with a LINKED `format_timestamp` in one program.
#[test]
fn linked_extended_temporal_matches_the_vm() {
    if !toolchain_available() {
        eprintln!("SKIP linked_extended_temporal_matches_the_vm: wasm link toolchain unavailable");
        return;
    }
    let dir = std::env::temp_dir().join("logos_wasm_link_exttemporal");
    let (runtime, mut rlibs) = match build_bigint_runtime(&dir) {
        Ok(v) => v,
        Err(_) => {
            eprintln!(
                "SKIP linked_extended_temporal_matches_the_vm: base wasm32 build unavailable"
            );
            return;
        }
    };
    rlibs.extend(wasm_all_sysroot_rlibs().expect("sysroot rlibs"));
    for src in [
        "## Main\n    Show format_timestamp(parse_timestamp(\"2024-03-10T07:30:00Z\")).\n", // round-trip
        "## Main\n    Let a be parse_timestamp(\"2024-03-10T07:30:00Z\").\n    Show format_timestamp(add_seconds(a, 90)).\n", // self-contained add_seconds ∘ linked format
        "## Main\n    Let a be parse_timestamp(\"1969-12-31T23:30:00Z\").\n    Show format_timestamp(a).\n", // pre-epoch
        "## Main\n    Let a be parse_timestamp(\"2024-03-10T00:00:00Z\").\n    Let b be parse_timestamp(\"2025-05-10T00:00:00Z\").\n    Show months_between(a, b).\n    Show years_between(a, b).\n",
        // Zoned: `in_zone` reads the zone Text handle from the shared memory → local wall-clock Text.
        // July = DST (UTC-4), January = standard (UTC-5) — the DST rule is exercised on both sides.
        "## Main\n    Show in_zone(parse_timestamp(\"2024-07-01T12:00:00Z\"), \"America/New_York\").\n",
        "## Main\n    Show in_zone(parse_timestamp(\"2024-01-01T12:00:00Z\"), \"America/New_York\").\n",
        // `local_instant` → the local-as-UTC Moment (here formatted back to a timestamp for display).
        "## Main\n    Show format_timestamp(local_instant(parse_timestamp(\"2024-07-01T12:00:00Z\"), \"Asia/Tokyo\")).\n",
    ] {
        let vm = crate::compile::vm_outcome(src);
        assert!(vm.error.is_none(), "the VM itself errored on `{src}`: {:?}", vm.error);
        let reloc = relocatable(src).unwrap_or_else(|e| panic!("compile-reloc `{src}`: {e}"));
        let linked = link_objects_with_rlibs(&[&reloc, &runtime], &rlibs, false, &dir)
            .unwrap_or_else(|e| panic!("link `{src}`: {e}"));
        let got = run_linked_capturing_text(&linked);
        assert_eq!(got.trim(), vm.output.trim(), "linked extended temporal DIVERGED from the VM on:\n{src}");
    }
}

/// LINKED SHA-1 SHA-NI LANE intrinsics — the SIMD substrate a SHA-1 (and `uuid_v3`/`uuid_v5`) written
/// in Logos compiles to. `lanes4Of` packs 4 `Word32` into a 128-bit lane block, `seqOfLanes4W32`
/// unpacks it (a round-trip proves the layout), and `sha1rnds4`/`sha1msg1`/`sha1msg2`/`sha1nexte`
/// delegate to the SAME `base::sha_ops` spec the VM uses — so each is bit-identical to the interpreter.
#[test]
fn linked_sha1_lane_intrinsics_match_the_vm() {
    if !toolchain_available() {
        eprintln!("SKIP linked_sha1_lane_intrinsics_match_the_vm: wasm link toolchain unavailable");
        return;
    }
    let dir = std::env::temp_dir().join("logos_wasm_link_sha1");
    let (runtime, mut rlibs) = match build_bigint_runtime(&dir) {
        Ok(v) => v,
        Err(_) => {
            eprintln!(
                "SKIP linked_sha1_lane_intrinsics_match_the_vm: base wasm32 build unavailable"
            );
            return;
        }
    };
    rlibs.extend(wasm_all_sysroot_rlibs().expect("sysroot rlibs"));
    let prelude = "## Main\n    Let a be lanes4Of(word32(1), word32(2), word32(3), word32(4)).\n    Let b be lanes4Of(word32(5), word32(6), word32(7), word32(8)).\n";
    for tail in [
        "    Show seqOfLanes4W32(a).\n", // [1, 2, 3, 4] (pack→unpack round-trip)
        "    Show seqOfLanes4W32(sha1msg1(a, b)).\n",
        "    Show seqOfLanes4W32(sha1msg2(a, b)).\n",
        "    Show seqOfLanes4W32(sha1nexte(a, b)).\n",
        "    Show seqOfLanes4W32(sha1rnds4(a, b, 0)).\n",
        "    Show seqOfLanes4W32(sha1rnds4(a, b, 2)).\n",
        "    Show seqOfLanes4W32(a + b).\n",   // lane-wise add
        "    Show seqOfLanes4W32(a xor b).\n", // lane-wise xor
    ] {
        let src = format!("{prelude}{tail}");
        let vm = crate::compile::vm_outcome(&src);
        assert!(
            vm.error.is_none(),
            "the VM itself errored on `{src}`: {:?}",
            vm.error
        );
        let reloc = relocatable(&src).unwrap_or_else(|e| panic!("compile-reloc `{src}`: {e}"));
        let linked = link_objects_with_rlibs(&[&reloc, &runtime], &rlibs, false, &dir)
            .unwrap_or_else(|e| panic!("link `{src}`: {e}"));
        let got = run_linked_capturing_text(&linked);
        assert_eq!(
            got.trim(),
            vm.output.trim(),
            "linked SHA-1 lane op DIVERGED from the VM on:\n{src}"
        );
    }
}

/// LINKED general SIMD lane vocabulary (`base::LanesVal`) — the SSE byte/word-lane ops a Logos codec
/// (hex nibbles, Poly1305 limbs, …) compiles to. `lanes16Word8` packs a `Seq` into a 16-byte vector,
/// `seqOfLanes16W8` unpacks it; `shuffle16`/`byteAdd16`/`interleave*`/`shrBytes16` are the byte-lane
/// ops; `maddubs16`→`packus16` is the WIDTH-CHANGING pair (`Lanes16Word8`→`Lanes16Word16`→`Lanes16Word8`);
/// `lanes8Word32`/`seqOfLanes8` are the 8×`Word32` width. Each delegates to the SAME pure-Rust
/// `base::word` spec the VM uses (bit-identical). All under one `Kind::LanesV` (a `Box<LanesVal>` handle).
#[test]
fn linked_simd_lanes_match_the_vm() {
    if !toolchain_available() {
        eprintln!("SKIP linked_simd_lanes_match_the_vm: wasm link toolchain unavailable");
        return;
    }
    let dir = std::env::temp_dir().join("logos_wasm_link_simdlanes");
    let (runtime, mut rlibs) = match build_bigint_runtime(&dir) {
        Ok(v) => v,
        Err(_) => {
            eprintln!("SKIP linked_simd_lanes_match_the_vm: base wasm32 build unavailable");
            return;
        }
    };
    rlibs.extend(wasm_all_sysroot_rlibs().expect("sysroot rlibs"));
    let prelude = "## Main\n    Let bytes be [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15].\n    Let idx be [15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0].\n    Let a be lanes16Word8(bytes).\n    Let b be lanes16Word8(idx).\n";
    for tail in [
        "    Show seqOfLanes16W8(a).\n",               // round-trip [0..15]
        "    Show seqOfLanes16W8(splat16Word8(7)).\n", // [7 ×16]
        "    Show seqOfLanes16W8(shuffle16(a, b)).\n", // reverse (idx selects lane 15..0)
        "    Show seqOfLanes16W8(byteAdd16(a, splat16Word8(1))).\n", // [1..16]
        "    Show seqOfLanes16W8(interleaveLo16(a, b)).\n",
        "    Show seqOfLanes16W8(interleaveHi16(a, b)).\n",
        "    Show seqOfLanes16W8(shrBytes16(a, 4)).\n", // byte-shift right by 4
        "    Show seqOfLanes16W8(packus16(maddubs16(a, b), maddubs16(a, b))).\n", // width-changing pair
        "    Show seqOfLanes8(lanes8Word32([1, 2, 3, 4, 5, 6, 7, 8])).\n", // 8×Word32 round-trip
    ] {
        let src = format!("{prelude}{tail}");
        let vm = crate::compile::vm_outcome(&src);
        assert!(
            vm.error.is_none(),
            "the VM itself errored on `{src}`: {:?}",
            vm.error
        );
        let reloc = relocatable(&src).unwrap_or_else(|e| panic!("compile-reloc `{src}`: {e}"));
        let linked = link_objects_with_rlibs(&[&reloc, &runtime], &rlibs, false, &dir)
            .unwrap_or_else(|e| panic!("link `{src}`: {e}"));
        let got = run_linked_capturing_text(&linked);
        assert_eq!(
            got.trim(),
            vm.output.trim(),
            "linked SIMD lane op DIVERGED from the VM on:\n{src}"
        );
    }
}

/// LINKED Money FX — the ambient exchange-rate table (`base::money`'s `thread_local`, persistent
/// within one linked module): `Call set_rate with "<code>" and <rate>` installs a rate (Int or
/// `decimal(…)`, coerced to a `Rational`), `Call set_rates with <map>` installs a whole `Map of Text
/// to Decimal`, and `<money> in <currency>` (`to_currency`) converts exactly via the Rational tower.
/// Both programs run their full set-then-convert sequence in ONE module (so the ambient state matches
/// the VM's) and are byte-identical to the interpreter.
#[test]
fn linked_money_fx_matches_the_vm() {
    if !toolchain_available() {
        eprintln!("SKIP linked_money_fx_matches_the_vm: wasm link toolchain unavailable");
        return;
    }
    let dir = std::env::temp_dir().join("logos_wasm_link_moneyfx");
    let (runtime, mut rlibs) = match build_bigint_runtime(&dir) {
        Ok(v) => v,
        Err(_) => {
            eprintln!("SKIP linked_money_fx_matches_the_vm: base wasm32 build unavailable");
            return;
        }
    };
    rlibs.extend(wasm_all_sysroot_rlibs().expect("sysroot rlibs"));
    for src in [
        // `set_rate` (Int + Decimal rates) then `<money> in <currency>` conversion.
        "## Main\n\
             Call set_rate with \"USD\" and 1.\n\
             Call set_rate with \"EUR\" and decimal(\"1.10\").\n\
             Call set_rate with \"GBP\" and decimal(\"1.25\").\n\
             Show 10.00 EUR in USD.\n\
             Show 11.00 USD in EUR.\n\
             Show 10.00 GBP in EUR.\n\
             Show 42.00 USD in USD.\n",
        // `set_rates` from a whole `Map of Text to Decimal` (the value-kind dispatch → set_rates_decimal).
        "## Main\n\
             Let mut rates be a new Map of Text to Decimal.\n\
             Set item \"USD\" of rates to decimal(\"1\").\n\
             Set item \"EUR\" of rates to decimal(\"1.10\").\n\
             Set item \"GBP\" of rates to decimal(\"1.25\").\n\
             Call set_rates with rates.\n\
             Show 10.00 EUR in USD.\n\
             Show 10.00 GBP in EUR.\n",
    ] {
        let vm = crate::compile::vm_outcome(src);
        assert!(
            vm.error.is_none(),
            "the VM itself errored on `{src}`: {:?}",
            vm.error
        );
        let reloc = relocatable(src).unwrap_or_else(|e| panic!("compile-reloc `{src}`: {e}"));
        let linked = link_objects_with_rlibs(&[&reloc, &runtime], &rlibs, false, &dir)
            .unwrap_or_else(|e| panic!("link `{src}`: {e}"));
        let got = run_linked_capturing_text(&linked);
        assert_eq!(
            got.trim(),
            vm.output.trim(),
            "linked Money FX DIVERGED from the VM on:\n{src}"
        );
    }
}

/// LINKED `wireBytes` — marshal a value to its wire bytes (a `Seq of Int`) via the REAL codec
/// (`logicaffeine_compile::concurrency::marshal::encode_value_raw`, compiled to wasm32 and LINKED into
/// the runtime — NOT a reimplementation), so it is byte-identical to the VM's
/// `bytes_to_seq(encode_value_raw(v))`. The scalar kinds (Int/Bool/Float/Text) each reconstruct their
/// `RuntimeValue` in the runtime and encode. Proves the wire codec runs inside emitted wasm.
#[test]
fn linked_wire_bytes_matches_the_vm() {
    if !toolchain_available() {
        eprintln!("SKIP linked_wire_bytes_matches_the_vm: wasm link toolchain unavailable");
        return;
    }
    let dir = std::env::temp_dir().join("logos_wasm_link_wirebytes");
    let (runtime, mut rlibs) = match build_bigint_runtime(&dir) {
        Ok(v) => v,
        Err(_) => {
            eprintln!("SKIP linked_wire_bytes_matches_the_vm: base wasm32 build unavailable");
            return;
        }
    };
    rlibs.extend(wasm_all_sysroot_rlibs().expect("sysroot rlibs"));
    for src in [
        "## Main\n    Show wireBytes(42).\n", // Int → [T_INT, zigzag(42) uvarint …]
        "## Main\n    Show wireBytes(-7).\n", // negative (zigzag)
        "## Main\n    Show wireBytes(true).\n", // Bool
        "## Main\n    Show wireBytes(false).\n",
        "## Main\n    Show wireBytes(\"hello\").\n", // Text
        "## Main\n    Show wireBytes(3.5).\n",       // Float
    ] {
        let vm = crate::compile::vm_outcome(src);
        assert!(
            vm.error.is_none(),
            "the VM itself errored on `{src}`: {:?}",
            vm.error
        );
        let reloc = relocatable(src).unwrap_or_else(|e| panic!("compile-reloc `{src}`: {e}"));
        let linked = link_objects_with_rlibs(&[&reloc, &runtime], &rlibs, false, &dir)
            .unwrap_or_else(|e| panic!("link `{src}`: {e}"));
        let got = run_linked_capturing_text(&linked);
        assert_eq!(
            got.trim(),
            vm.output.trim(),
            "linked wireBytes DIVERGED from the VM on:\n{src}"
        );
    }
}

/// LINKED `readWireProgram` + `run_accepted` — the wire-INPUT/eval core. A `readWireProgram` value is a
/// leaked `Box<RuntimeValue>` (Kind::Dynamic) DECODED (via the real `decode_value_raw`) from a frame the
/// `read_wire_frame` host supplies; `run_accepted` sandbox-evals a wire-received SHIPPED function through
/// the acceptance contract (`AcceptanceContract::apply`). ORACLE: the VM's `readWireProgram` reads REAL
/// stdin and `exit(0)`s on empty, so it's not differentially comparable — instead each frame encodes a
/// KNOWN value (via `encode_value_raw`) and we assert the AOT reproduces the value the codec+contract
/// compute NATIVELY (a direct correctness oracle, not a VM diff).
#[test]
fn linked_read_wire_program_and_run_accepted_match_the_known_value() {
    use crate::concurrency::marshal::{encode_value_raw, GenExpr};
    use crate::interpreter::{ClosureValue, RuntimeValue};
    if !toolchain_available() {
        eprintln!("SKIP linked_read_wire_program: wasm link toolchain unavailable");
        return;
    }
    let dir = std::env::temp_dir().join("logos_wasm_link_readwire");
    let (runtime, mut rlibs) = match build_bigint_runtime(&dir) {
        Ok(v) => v,
        Err(_) => {
            eprintln!("SKIP linked_read_wire_program: base wasm32 build unavailable");
            return;
        }
    };
    rlibs.extend(wasm_all_sysroot_rlibs().expect("sysroot rlibs"));
    // End-to-end: compile the reloc, LINK it (the whole `marshal`/`RuntimeValue` decode+display+apply
    // graph is pulled into the module + gc-stripped down), run it, and assert it reproduces the value
    // the codec/contract compute NATIVELY. (Getting `readWireProgram` classified as a heap op — so
    // `emit_alloc` uses the runtime allocator, not the `__heap_ptr` global undeclared in a linked
    // module — was the fix; the earlier "invalid global relocation" / lld crash were that one bug.)
    let assert_wire = |src: &str, frame: Vec<u8>, expected: &str| {
        WIRE_FRAME.with(|f| *f.borrow_mut() = frame);
        let reloc = relocatable(src).unwrap_or_else(|e| panic!("compile-reloc `{src}`: {e}"));
        let linked = link_objects_with_rlibs(&[&reloc, &runtime], &rlibs, false, &dir)
            .unwrap_or_else(|e| panic!("link `{src}`: {e}"));
        assert_eq!(
            run_linked_capturing_text(&linked).trim(),
            expected,
            "wire-input result for:\n{src}"
        );
    };
    // (a) `readWireProgram` of a PLAIN value: the frame encodes Int 42 → the dynamic value shows "42".
    assert_wire(
        "## Main\n    Show readWireProgram().\n",
        encode_value_raw(&RuntimeValue::Int(42)).expect("encode Int"),
        "42",
    );
    // (b) `run_accepted` of a wire-received SHIPPED function `3·x + 1`, arg 5 ∈ [0,1000] → 3·5+1 = 16.
    let shipped = RuntimeValue::Function(Box::new(ClosureValue {
        body_index: usize::MAX,
        captured_env: std::collections::HashMap::default(),
        param_names: vec![logicaffeine_base::Symbol::from_index(0)],
        generated: Some(std::rc::Rc::new(GenExpr::Add(
            Box::new(GenExpr::Mul(
                Box::new(GenExpr::Index),
                Box::new(GenExpr::Const(3)),
            )),
            Box::new(GenExpr::Const(1)),
        ))),
    }));
    assert_wire(
        "## Main\n    Let f be readWireProgram().\n    Show run_accepted(f, 5, 0, 1000).\n",
        encode_value_raw(&shipped).expect("encode shipped fn"),
        "16",
    );
}

/// LINKED crypto UUIDs — `uuid_v3` (MD5) / `uuid_v5` (SHA-1) are STDLIB functions WRITTEN IN LOGOS
/// (`assets/std/uuid.lg`): they demand-import, hash the namespace+name via `sha1Compress`/`md5` (the
/// SHA-1 lane intrinsics + Word ops), then `stampBytes` + `uuid_from_bytes`. Function-level DCE drops
/// the unused `uuidParse`/`decodeNibbles` (which need `Lanes16Word8`), so the whole chain compiles
/// Logos→linked-wasm and produces the RFC-9562 name-based UUID BIT-IDENTICALLY to the VM.
#[test]
fn linked_crypto_uuid_in_logos_matches_the_vm() {
    if !toolchain_available() {
        eprintln!(
            "SKIP linked_crypto_uuid_in_logos_matches_the_vm: wasm link toolchain unavailable"
        );
        return;
    }
    let dir = std::env::temp_dir().join("logos_wasm_link_cryptouuid");
    let (runtime, mut rlibs) = match build_bigint_runtime(&dir) {
        Ok(v) => v,
        Err(_) => {
            eprintln!(
                "SKIP linked_crypto_uuid_in_logos_matches_the_vm: base wasm32 build unavailable"
            );
            return;
        }
    };
    rlibs.extend(wasm_all_sysroot_rlibs().expect("sysroot rlibs"));
    for src in [
        "## Main\n    Show sha1(text_bytes(\"abc\")).\n", // the SHA-1 chain (Logos)
        "## Main\n    Show uuid_v5(uuid_dns(), \"example.com\").\n", // the well-known DNS v5 UUID
        "## Main\n    Show uuid_v3(uuid_dns(), \"example.com\").\n", // MD5-based v3
        "## Main\n    Show uuid_v5(uuid_url(), \"https://logicaffeine.com\").\n",
    ] {
        let vm = crate::compile::vm_outcome(src);
        assert!(
            vm.error.is_none(),
            "the VM itself errored on `{src}`: {:?}",
            vm.error
        );
        let reloc = relocatable(src).unwrap_or_else(|e| panic!("compile-reloc `{src}`: {e}"));
        let linked = link_objects_with_rlibs(&[&reloc, &runtime], &rlibs, false, &dir)
            .unwrap_or_else(|e| panic!("link `{src}`: {e}"));
        let got = run_linked_capturing_text(&linked);
        assert_eq!(
            got.trim(),
            vm.output.trim(),
            "linked crypto UUID DIVERGED from the VM on:\n{src}"
        );
    }
}

/// MIXED heap + BigInt (the slab): a program that uses BOTH the emitter's heap (a `Text` literal +
/// string concatenation, which run the emitter's bump allocator) AND the runtime's BigInt — e.g.
/// `Show "x = " + (2^200)`. The bump allocator is seeded from a runtime-owned SLAB at the `main`
/// prologue, so it never collides with the runtime's `dlmalloc`; the concat stringifies the BigInt via
/// `to_text`. Every case must print BYTE-IDENTICALLY to the VM. Skips if the toolchain / base wasm32
/// build is unavailable.
#[test]
fn linked_mixed_text_and_bigint_matches_the_vm() {
    if !toolchain_available() {
        eprintln!(
            "SKIP linked_mixed_text_and_bigint_matches_the_vm: wasm link toolchain unavailable"
        );
        return;
    }
    let dir = std::env::temp_dir().join("logos_wasm_link_mixed");
    let (runtime, mut rlibs) = match build_bigint_runtime(&dir) {
        Ok(v) => v,
        Err(_) => {
            eprintln!(
                "SKIP linked_mixed_text_and_bigint_matches_the_vm: base wasm32 build unavailable"
            );
            return;
        }
    };
    rlibs.extend(wasm_all_sysroot_rlibs().expect("sysroot rlibs"));

    for src in [
        "## Main\nShow \"x = \" + (2 to the power of 200).\n",
        "## Main\nShow (2 to the power of 100) + \" is big\".\n",
        "## Main\nShow \"\" + (99999999999 times 99999999999) + \"!\".\n",
        "## Main\nShow \"first \" + (2 to the power of 64) + \" then \" + (3 to the power of 40).\n",
    ] {
        let vm = crate::compile::vm_outcome(src);
        assert!(vm.error.is_none(), "the VM itself errored on `{src}`: {:?}", vm.error);
        let reloc = relocatable(src).unwrap_or_else(|e| panic!("assemble `{src}`: {e}"));
        let linked = link_objects_with_rlibs(&[&reloc, &runtime], &rlibs, false, &dir)
            .unwrap_or_else(|e| panic!("link `{src}`: {e}"));
        let got = run_linked_capturing_text(&linked);
        assert_eq!(got.trim(), vm.output.trim(), "mixed heap+BigInt output disagrees with the VM for:\n{src}");
    }
}

/// LINKED `Repeat` LOOPS (the iterator-stack slab): a program whose `Repeat` drives the emitter's
/// iterator stack (`__iter_sp`), now seeded from the TOP of a runtime-owned SLAB at the `main`
/// prologue (so it grows down inside a block `dlmalloc` owns — no collision), COMBINED with the BigInt
/// tier. Covers a plain loop, a per-iteration BigInt, and a (non-overflowing) accumulator that the
/// demand guard keeps a sound i64. Each must print BYTE-IDENTICALLY to the VM.
#[test]
fn linked_repeat_loops_match_the_vm() {
    if !toolchain_available() {
        eprintln!("SKIP linked_repeat_loops_match_the_vm: wasm link toolchain unavailable");
        return;
    }
    let dir = std::env::temp_dir().join("logos_wasm_link_repeat");
    let (runtime, mut rlibs) = match build_bigint_runtime(&dir) {
        Ok(v) => v,
        Err(_) => {
            eprintln!("SKIP linked_repeat_loops_match_the_vm: base wasm32 build unavailable");
            return;
        }
    };
    rlibs.extend(wasm_all_sysroot_rlibs().expect("sysroot rlibs"));

    for src in [
        // A plain loop over a range — prints each loop variable (drives `__iter_sp`).
        "## Main\nRepeat for n from 1 to 5:\n    Show n.\n",
        // A per-iteration BigInt: `n^50` overflows i64 for n≥3 → a runtime BigInt each pass.
        "## Main\nRepeat for n from 1 to 4:\n    Show n to the power of 50.\n",
        // A (non-overflowing) accumulator: the demand guard keeps `total` an i64 (it's also written by
        // `Let total be 0`), so `Show total` = 5050.
        "## Main\nLet total be 0.\nRepeat for n from 1 to 100:\n    Set total to total + n.\nShow total.\n",
    ] {
        let vm = crate::compile::vm_outcome(src);
        assert!(vm.error.is_none(), "the VM itself errored on `{src}`: {:?}", vm.error);
        let reloc = relocatable(src).unwrap_or_else(|e| panic!("assemble `{src}`: {e}"));
        let linked = link_objects_with_rlibs(&[&reloc, &runtime], &rlibs, false, &dir)
            .unwrap_or_else(|e| panic!("link `{src}`: {e}"));
        let got = run_linked_capturing_text(&linked);
        assert_eq!(got.trim(), vm.output.trim(), "linked Repeat output disagrees with the VM for:\n{src}");
    }
}

/// LINKED heap LARGER than the old 4 MiB slab: a loop that builds a big string by repeated
/// concatenation leaks every intermediate (bump-no-free), so the TOTAL allocated far exceeds 4 MiB.
/// The per-allocation `logos_rt_alloc` path (dlmalloc growing linear memory on demand) handles it; the
/// old fixed slab would overrun. Must print BYTE-IDENTICALLY to the VM.
#[test]
fn linked_large_heap_beyond_the_slab_matches_the_vm() {
    if !toolchain_available() {
        eprintln!("SKIP linked_large_heap_beyond_the_slab_matches_the_vm: wasm link toolchain unavailable");
        return;
    }
    let dir = std::env::temp_dir().join("logos_wasm_link_bigheap");
    let (runtime, mut rlibs) = match build_bigint_runtime(&dir) {
        Ok(v) => v,
        Err(_) => {
            eprintln!("SKIP linked_large_heap_beyond_the_slab_matches_the_vm: base wasm32 build unavailable");
            return;
        }
    };
    rlibs.extend(wasm_all_sysroot_rlibs().expect("sysroot rlibs"));
    // ~3500 concatenations of a growing string leak ≈ Σ i ≈ 6 MiB total — well past the 4 MiB slab.
    let src = "## Main\nLet s be \"x\".\nRepeat for n from 1 to 3500:\n    Set s to s + \"y\".\nShow s.\n";
    let vm = crate::compile::vm_outcome(src);
    assert!(vm.error.is_none(), "the VM itself errored: {:?}", vm.error);
    let reloc = relocatable(src).expect("compile the large-heap program");
    let linked = link_objects_with_rlibs(&[&reloc, &runtime], &rlibs, false, &dir)
        .expect("link large-heap program");
    let got = run_linked_capturing_text(&linked);
    assert_eq!(
        got.trim(),
        vm.output.trim(),
        "linked >4MiB-heap output disagrees with the VM"
    );
}

/// LINKED `Let`-BINDINGS (the Move-chain question): a `Let x be <big expr>. … Show x.` pattern —
/// does the compiler assign the arithmetic DIRECTLY into `x` (so `x` is a demand candidate and
/// promotes), or through a `Move` (which the demand analysis doesn't cross)? These must all print
/// BYTE-IDENTICALLY to the VM; a divergence here means Move-chains are a real gap to close.
#[test]
fn linked_let_bindings_match_the_vm() {
    if !toolchain_available() {
        eprintln!("SKIP linked_let_bindings_match_the_vm: wasm link toolchain unavailable");
        return;
    }
    let dir = std::env::temp_dir().join("logos_wasm_link_let");
    let (runtime, mut rlibs) = match build_bigint_runtime(&dir) {
        Ok(v) => v,
        Err(_) => {
            eprintln!("SKIP linked_let_bindings_match_the_vm: base wasm32 build unavailable");
            return;
        }
    };
    rlibs.extend(wasm_all_sysroot_rlibs().expect("sysroot rlibs"));
    for src in [
        "## Main\nLet x be 99999999999 times 99999999999.\nShow x.\n",
        "## Main\nLet p be 2 to the power of 100.\nLet q be 3 to the power of 50.\nShow p times q.\n",
        "## Main\nLet x be 2 to the power of 200.\nLet y be x plus 1.\nShow y.\n",
    ] {
        let vm = crate::compile::vm_outcome(src);
        assert!(vm.error.is_none(), "the VM itself errored on `{src}`: {:?}", vm.error);
        match relocatable(src) {
            Ok(reloc) => {
                let linked = link_objects_with_rlibs(&[&reloc, &runtime], &rlibs, false, &dir)
                    .unwrap_or_else(|e| panic!("link `{src}`: {e}"));
                let got = run_linked_capturing_text(&linked);
                assert_eq!(got.trim(), vm.output.trim(), "linked `Let`-binding output disagrees with the VM for:\n{src}");
            }
            // A sound refusal (Err) is acceptable (never a wrong answer) — but log it so we know the
            // Move-chain gap manifested for this shape.
            Err(e) => eprintln!("NOTE: linked emitter declined `{src}` (Move-chain gap): {e}"),
        }
    }
}

/// LINKED CLOSURES (via direct calls): a program with a closure (`MakeClosure` + `CallValue`) whose
/// body returns a BigInt. Linker mode lowers the `CallValue` to a DIRECT `call` (the callee is
/// statically resolved), so no function table / element section is emitted — nothing the reloc
/// transform can't handle. Covers a plain closure and a capturing one; each must print
/// BYTE-IDENTICALLY to the VM.
#[test]
fn linked_closures_match_the_vm() {
    if !toolchain_available() {
        eprintln!("SKIP linked_closures_match_the_vm: wasm link toolchain unavailable");
        return;
    }
    let dir = std::env::temp_dir().join("logos_wasm_link_closure");
    let (runtime, mut rlibs) = match build_bigint_runtime(&dir) {
        Ok(v) => v,
        Err(_) => {
            eprintln!("SKIP linked_closures_match_the_vm: base wasm32 build unavailable");
            return;
        }
    };
    rlibs.extend(wasm_all_sysroot_rlibs().expect("sysroot rlibs"));

    for src in [
        // A closure whose body overflows i64 → returns a BigInt. Called directly.
        "## Main\nLet f be (x: Int) -> x to the power of 100.\nShow f(2).\n",
        // A closure held through an ALIAS (`Let f be g`) — the AOT resolves `f`'s callee to `g`.
        "## Main\nLet g be (x: Int) -> x to the power of 90.\nLet f be g.\nShow f(2).\n",
        // A REASSIGNED closure binding (`Let f be g. Let f be h.`) — resolves to the latest, `h`.
        "## Main\nLet g be (x: Int) -> x to the power of 90.\nLet h be (x: Int) -> x to the power of 60.\nLet f be g.\nLet f be h.\nShow f(2).\n",
        // A CAPTURING closure over `base`, returning a BigInt.
        "## Main\nLet base be 3.\nLet g be (e: Int) -> base to the power of e.\nShow g(50).\n",
    ] {
        let vm = crate::compile::vm_outcome(src);
        assert!(vm.error.is_none(), "the VM itself errored on `{src}`: {:?}", vm.error);
        let reloc = relocatable(src).unwrap_or_else(|e| panic!("assemble `{src}`: {e}"));
        let linked = link_objects_with_rlibs(&[&reloc, &runtime], &rlibs, false, &dir)
            .unwrap_or_else(|e| panic!("link `{src}`: {e}"));
        let got = run_linked_capturing_text(&linked);
        assert_eq!(got.trim(), vm.output.trim(), "linked closure output disagrees with the VM for:\n{src}");
    }
}

/// The dynamic-Int emitter END TO END (S4-next): a REAL Logos program `Show 2 to the power of 200`
/// compiled through `assemble_program_linked` (integer `Op::Pow` → `logos_rt_bigint_*`) →
/// `module_to_relocatable` → `rust-lld` against the base `logicaffeine_base::BigInt` runtime, then
/// run. Its output is the exact 61-digit `2^200`, BYTE-IDENTICAL to the VM (whose Pow overflows i64
/// and promotes to the SAME `BigInt`). This is the first Logos SOURCE program to reach native wasm
/// via the linker with a value the self-contained emitter cannot represent — no reimplementation, no
/// divergence. Skips if the wasm toolchain or the base wasm32 build is unavailable.
#[test]
fn linked_program_computes_overflowing_power_as_bigint_matching_the_vm() {
    if !toolchain_available() {
        eprintln!("SKIP linked_program_computes_overflowing_power_as_bigint_matching_the_vm: wasm link toolchain unavailable");
        return;
    }
    // A Logos `Show <base> to the power of <exp>` compiles to a LINKER-MODE relocatable object
    // (integer `Op::Pow` → `logos_rt_bigint_*`, result a Text handle).
    let compile_reloc = |source: &str| -> Vec<u8> {
        relocatable(source).unwrap_or_else(|e| panic!("compile `{source}` to relocatable: {e}"))
    };

    // Build the base BigInt runtime + sysroot rlibs ONCE, then link/run each program against them.
    let dir = std::env::temp_dir().join("logos_wasm_link_prog_bigint");
    let (runtime, mut rlibs) = match build_bigint_runtime(&dir) {
        Ok(v) => v,
        Err(_) => {
            eprintln!("SKIP linked_program_...: base wasm32 build unavailable");
            return;
        }
    };
    rlibs.extend(wasm_all_sysroot_rlibs().expect("sysroot rlibs"));

    // A spread of powers — deep overflow (2^200 = 61 digits), a decimal power (10^50), a coprime
    // base (7^90), AND a SMALL power that fits i64 (3^5 = 243): linked mode routes even a
    // non-overflowing integer power through the real BigInt, so the Text must still be exact. Each
    // is checked against BOTH the `logicaffeine_base::BigInt` ground truth AND the VM (WASM==VM).
    for &(base, exp) in &[(2i64, 200u32), (10, 50), (7, 90), (3, 5), (2, 63), (2, 64)] {
        let source = format!("## Main\nShow {base} to the power of {exp}.\n");
        let reloc = compile_reloc(&source);
        let linked = link_objects_with_rlibs(&[&reloc, &runtime], &rlibs, false, &dir)
            .unwrap_or_else(|e| panic!("link `{source}`: {e}"));
        let got = run_linked_capturing_text(&linked);
        let expected = logicaffeine_base::BigInt::from_i64(base)
            .pow(exp)
            .to_string();
        assert_eq!(
            got.trim(),
            expected,
            "linked wasm must print the exact {base}^{exp} the base BigInt computes"
        );
        assert_eq!(
            got.trim(),
            crate::compile::vm_outcome(&source).output.trim(),
            "linked wasm output must equal the VM for {base}^{exp} (the WASM==VM lock, through the BigInt linker)"
        );
    }

    // CHAINING — the whole point of a BigInt HANDLE (not an eager decimal): `(a^ae) * (b^be)` keeps
    // both powers as real BigInts and multiplies them (`logos_rt_bigint_mul`), rendering to a Text
    // only at the final `Show`. Each product is checked against the base BigInt AND the VM.
    for &(a, ae, b, be) in &[(2i64, 100u32, 3i64, 50u32), (10, 40, 7, 33)] {
        let source =
            format!("## Main\nShow ({a} to the power of {ae}) times ({b} to the power of {be}).\n");
        let reloc = compile_reloc(&source);
        let linked = link_objects_with_rlibs(&[&reloc, &runtime], &rlibs, false, &dir)
            .unwrap_or_else(|e| panic!("link `{source}`: {e}"));
        let got = run_linked_capturing_text(&linked);
        let av = logicaffeine_base::BigInt::from_i64(a).pow(ae);
        let bv = logicaffeine_base::BigInt::from_i64(b).pow(be);
        let expected = av.mul(&bv).to_string();
        assert_eq!(
            got.trim(),
            expected,
            "linked wasm must print the exact ({a}^{ae})*({b}^{be})"
        );
        assert_eq!(
            got.trim(),
            crate::compile::vm_outcome(&source).output.trim(),
            "linked wasm BigInt product must equal the VM for ({a}^{ae})*({b}^{be})"
        );
    }

    // MIXED `BigInt * Int`: a big power times a BARE integer factor (`3`, a `Kind::Int`), which
    // `lower_bigint_mul` promotes to a BigInt with `from_i64` before the multiply.
    for &(a, ae, k) in &[(2i64, 100u32, 3i64), (10, 45, 999)] {
        let source = format!("## Main\nShow ({a} to the power of {ae}) times {k}.\n");
        let reloc = compile_reloc(&source);
        let linked = link_objects_with_rlibs(&[&reloc, &runtime], &rlibs, false, &dir)
            .unwrap_or_else(|e| panic!("link `{source}`: {e}"));
        let got = run_linked_capturing_text(&linked);
        let expected = logicaffeine_base::BigInt::from_i64(a)
            .pow(ae)
            .mul(&logicaffeine_base::BigInt::from_i64(k))
            .to_string();
        assert_eq!(
            got.trim(),
            expected,
            "linked wasm must print the exact ({a}^{ae})*{k} (mixed BigInt*Int)"
        );
        assert_eq!(
            got.trim(),
            crate::compile::vm_outcome(&source).output.trim(),
            "linked wasm mixed BigInt*Int must equal the VM for ({a}^{ae})*{k}"
        );
    }

    // ADD / SUB on BigInts (linked, `plus`/`minus`): exact, and a `minus` may go NEGATIVE — the sign
    // is rendered by `to_text`, matching the VM. Both BigInt±BigInt and mixed BigInt±Int (`from_i64`).
    for &(a, ae, add, b, be) in &[
        (2i64, 100u32, true, 3i64, 50u32),
        (3, 80, false, 2, 80),
        (2, 50, false, 3, 50),
    ] {
        let word = if add { "plus" } else { "minus" };
        let source = format!(
            "## Main\nShow ({a} to the power of {ae}) {word} ({b} to the power of {be}).\n"
        );
        let reloc = compile_reloc(&source);
        let linked = link_objects_with_rlibs(&[&reloc, &runtime], &rlibs, false, &dir)
            .unwrap_or_else(|e| panic!("link `{source}`: {e}"));
        let got = run_linked_capturing_text(&linked);
        let av = logicaffeine_base::BigInt::from_i64(a).pow(ae);
        let bv = logicaffeine_base::BigInt::from_i64(b).pow(be);
        let expected = if add { av.add(&bv) } else { av.sub(&bv) }.to_string();
        assert_eq!(
            got.trim(),
            expected,
            "linked wasm must print the exact ({a}^{ae}) {word} ({b}^{be})"
        );
        assert_eq!(
            got.trim(),
            crate::compile::vm_outcome(&source).output.trim(),
            "linked wasm add/sub must equal the VM for ({a}^{ae}) {word} ({b}^{be})"
        );
    }
    for &(a, ae, add, k) in &[(2i64, 100u32, true, 7i64), (2, 100, false, 7)] {
        let word = if add { "plus" } else { "minus" };
        let source = format!("## Main\nShow ({a} to the power of {ae}) {word} {k}.\n");
        let reloc = compile_reloc(&source);
        let linked = link_objects_with_rlibs(&[&reloc, &runtime], &rlibs, false, &dir)
            .unwrap_or_else(|e| panic!("link `{source}`: {e}"));
        let got = run_linked_capturing_text(&linked);
        let av = logicaffeine_base::BigInt::from_i64(a).pow(ae);
        let kv = logicaffeine_base::BigInt::from_i64(k);
        let expected = if add { av.add(&kv) } else { av.sub(&kv) }.to_string();
        assert_eq!(
            got.trim(),
            expected,
            "linked wasm mixed ({a}^{ae}) {word} {k}"
        );
        assert_eq!(
            got.trim(),
            crate::compile::vm_outcome(&source).output.trim(),
            "linked wasm mixed add/sub must equal the VM for ({a}^{ae}) {word} {k}"
        );
    }

    // DIV / MOD on BigInts (linked): `divided by` = the exact quotient (`div_rem().0`), `the
    // remainder of a and b` = the remainder (`.1`), matching the VM's own `div_rem`.
    for &(a, ae, isdiv, k) in &[
        (2i64, 200u32, true, 7i64),
        (2, 200, false, 7),
        (10, 50, true, 3),
    ] {
        let source = if isdiv {
            format!("## Main\nShow ({a} to the power of {ae}) divided by {k}.\n")
        } else {
            format!("## Main\nShow the remainder of ({a} to the power of {ae}) and {k}.\n")
        };
        let reloc = compile_reloc(&source);
        let linked = link_objects_with_rlibs(&[&reloc, &runtime], &rlibs, false, &dir)
            .unwrap_or_else(|e| panic!("link `{source}`: {e}"));
        let got = run_linked_capturing_text(&linked);
        let (q, r) = logicaffeine_base::BigInt::from_i64(a)
            .pow(ae)
            .div_rem(&logicaffeine_base::BigInt::from_i64(k))
            .unwrap();
        let expected = if isdiv { q } else { r }.to_string();
        let what = if isdiv { "quotient" } else { "remainder" };
        assert_eq!(
            got.trim(),
            expected,
            "linked wasm must print the exact {what} of ({a}^{ae}) and {k}"
        );
        assert_eq!(
            got.trim(),
            crate::compile::vm_outcome(&source).output.trim(),
            "linked wasm div/mod must equal the VM for the {what} of ({a}^{ae}) and {k}"
        );
    }

    // GENERAL Int-overflow→BigInt (the demand analysis): a PURE-integer expression (no BigInt
    // operand — both factors fit i64) whose PRODUCT/SUM overflows i64 and is only `Show`n promotes to
    // BigInt, so it prints the exact value instead of trapping — matching the VM's promote-on-overflow.
    // Includes a NESTED case (the inner product feeds an addition that feeds the Show).
    let overflow_cases: [(&str, logicaffeine_base::BigInt); 3] = [
        (
            "## Main\nShow 99999999999 times 99999999999.\n",
            logicaffeine_base::BigInt::from_i64(99999999999)
                .mul(&logicaffeine_base::BigInt::from_i64(99999999999)),
        ),
        (
            "## Main\nShow 9223372036854775807 plus 1.\n",
            logicaffeine_base::BigInt::from_i64(9223372036854775807)
                .add(&logicaffeine_base::BigInt::from_i64(1)),
        ),
        (
            "## Main\nShow (99999999999 times 99999999999) plus 1.\n",
            logicaffeine_base::BigInt::from_i64(99999999999)
                .mul(&logicaffeine_base::BigInt::from_i64(99999999999))
                .add(&logicaffeine_base::BigInt::from_i64(1)),
        ),
    ];
    for (source, expected_big) in &overflow_cases {
        let reloc = compile_reloc(source);
        let linked = link_objects_with_rlibs(&[&reloc, &runtime], &rlibs, false, &dir)
            .unwrap_or_else(|e| panic!("link `{source}`: {e}"));
        let got = run_linked_capturing_text(&linked);
        assert_eq!(
            got.trim(),
            expected_big.to_string(),
            "linked wasm must promote the overflowing `{source}` to the exact BigInt"
        );
        assert_eq!(
            got.trim(),
            crate::compile::vm_outcome(source).output.trim(),
            "linked wasm overflow promotion must equal the VM for `{source}`"
        );
    }
}

/// The PUBLIC linker entry end-to-end: [`crate::compile::compile_to_wasm_linked`] takes Logos SOURCE
/// straight to a linked `.wasm` (emit → relocatable transform → cached base-BigInt runtime →
/// `rust-lld`) with NO manual composition — the one call a CLI `--emit wasm` linked build would make.
/// Runs it and checks the exact BigInt == the VM. Skips if the toolchain or a base wasm32 build is
/// absent.
#[test]
fn public_compile_to_wasm_linked_entry_runs_a_bigint_program() {
    if !toolchain_available() {
        eprintln!("SKIP public_compile_to_wasm_linked_entry_runs_a_bigint_program: wasm link toolchain unavailable");
        return;
    }
    // Probe the base wasm32 build the way the sibling tests do (CI without it skips cleanly).
    if build_bigint_runtime(&std::env::temp_dir().join("logos_wasm_linked_bigint")).is_err() {
        eprintln!("SKIP public_compile_to_wasm_linked_entry_...: base wasm32 build unavailable");
        return;
    }
    for (source, expected) in [
        (
            "## Main\nShow (2 to the power of 128) times 3.\n",
            logicaffeine_base::BigInt::from_i64(2)
                .pow(128)
                .mul(&logicaffeine_base::BigInt::from_i64(3))
                .to_string(),
        ),
        (
            "## Main\nShow 99999999999 times 99999999999.\n",
            logicaffeine_base::BigInt::from_i64(99999999999)
                .mul(&logicaffeine_base::BigInt::from_i64(99999999999))
                .to_string(),
        ),
    ] {
        let linked = crate::compile::compile_to_wasm_linked(source)
            .unwrap_or_else(|e| panic!("compile_to_wasm_linked(`{source}`) failed: {e:?}"));
        let got = run_linked_capturing_text(&linked);
        assert_eq!(
            got.trim(),
            expected,
            "public linked entry must print the exact BigInt for `{source}`"
        );
        assert_eq!(
            got.trim(),
            crate::compile::vm_outcome(source).output.trim(),
            "public linked entry == VM for `{source}`"
        );
    }
}

// Word sequences must use the same copy-on-write rules in the linked emitter's inline fallback.
fn linked_word_copies(ty: &str, word: &str, unwrap: &str) {
    assert!(
        toolchain_available(),
        "word-copy verification requires the wasm link toolchain"
    );
    let dir = std::env::temp_dir().join(format!("logos_wasm_link_word_cow_{word}"));
    let (runtime, mut rlibs) = build_bigint_runtime(&dir).expect("build the linked wasm runtime");
    rlibs.extend(wasm_all_sysroot_rlibs().expect("wasm sysroot rlibs"));
    let cases = [
        (format!("## Main\nLet mutable words be [{word}(10), {word}(20)].\nLet saved be words.\nSet item 1 of words to {word}(99).\nShow {unwrap}(item 1 of saved).\nShow {unwrap}(item 1 of words).\n"), "10\n99"),
        (format!("## Main\nLet mutable words be [{word}(10), {word}(20)].\nLet saved be words.\nPush {word}(30) to words.\nShow length of saved.\nShow length of words.\n"), "2\n3"),
        (format!("## To change (words: Seq of {ty}) -> {ty}:\n    Set item 1 of words to {word}(99).\n    Return item 1 of words.\n\n## Main\nLet words be [{word}(10), {word}(20)].\nShow {unwrap}(change(words)).\nShow {unwrap}(item 1 of words).\n"), "99\n10"),
    ];
    for (source, expected) in cases {
        let vm = crate::compile::vm_outcome(&source);
        assert_eq!(vm.error, None, "VM oracle runs:\n{source}");
        assert_eq!(
            vm.output.lines().collect::<Vec<_>>(),
            expected.lines().collect::<Vec<_>>()
        );
        let reloc = relocatable(&source).expect("word-copy program lowers to relocatable wasm");
        let linked = link_objects_with_rlibs(&[&reloc, &runtime], &rlibs, false, &dir)
            .expect("word-copy program links");
        let got = run_linked_capturing_text(&linked);
        assert_eq!(
            got.lines().collect::<Vec<_>>(),
            expected.lines().collect::<Vec<_>>(),
            "linked word copies preserve their values:\n{source}"
        );
    }
}

#[test]
fn linked_word32_copies_preserve_values() {
    linked_word_copies("Word32", "word32", "intOfWord32");
}

#[test]
fn linked_word64_copies_preserve_values() {
    linked_word_copies("Word64", "word64", "intOfWord64");
}

fn linked_owned_value_cases(label: &str, sources: &[&str]) {
    assert!(
        toolchain_available(),
        "owned-value verification requires the wasm link toolchain"
    );
    let dir = std::env::temp_dir().join(format!("logos_wasm_link_owned_{label}"));
    let (runtime, mut rlibs) = build_bigint_runtime(&dir).expect("build the linked wasm runtime");
    rlibs.extend(wasm_all_sysroot_rlibs().expect("wasm sysroot rlibs"));
    for source in sources {
        let expected = crate::compile::vm_outcome(source);
        assert_eq!(expected.error, None, "VM oracle runs:\n{source}");
        let reloc = relocatable(source).expect("owned-value program lowers to relocatable wasm");
        let linked = link_objects_with_rlibs(&[&reloc, &runtime], &rlibs, false, &dir)
            .expect("owned-value program links");
        assert_eq!(
            run_linked_capturing_text(&linked)
                .lines()
                .collect::<Vec<_>>(),
            expected.output.lines().collect::<Vec<_>>(),
            "linked value representation:\n{source}"
        );
    }
}

#[test]
fn linked_rational_fields_promote_integer_storage() {
    linked_owned_value_cases("rational_fields", &[
        "## A Box has:\n    a value, which is Rational.\n\n## To read (box: Box) -> Rational:\n    Return box's value.\n\n## Main\nLet box be a new Box with value 42.\nShow read(box).\n",
        "## A Box has:\n    a value, which is Rational.\n\n## Main\nLet mutable box be a new Box with value 42.\nLet saved be box.\nSet box's value to 43.\nShow saved's value.\nShow box's value.\n",
        "## A Box has:\n    a value, which is Rational.\n\n## Main\nLet ratio: Rational be 7 / 3.\nLet box be a new Box with value ratio.\nShow box's value.\n",
    ]);
}

#[test]
fn linked_counter_replicas_preserve_merge_identity() {
    linked_owned_value_cases("counter_ids", &[
        "## A Counter is Shared and has:\n    a points, which is Tally.\n\n## Main\nLet combined be a new Counter.\nRepeat for i from 1 to 100:\n    Let remote be a new Counter.\n    Increase remote's points by i.\n    Merge remote into combined.\n    Merge remote into combined.\nShow combined's points.\n",
    ]);
}

#[test]
fn linked_rational_copies_preserve_immutable_values() {
    linked_owned_value_cases("rational_copies", &[
        "## Main\nLet original: Rational be 7 / 3.\nLet saved be copy of original.\nShow original.\nShow saved.\n",
        "## A Box has:\n    a value, which is Rational.\n\n## Main\nLet ratio: Rational be 7 / 3.\nLet mutable box be a new Box with value ratio.\nLet saved be copy of box.\nSet box's value to 8.\nShow saved's value.\nShow box's value.\n",
        "## A Box has:\n    a value, which is Rational.\n\n## Main\nLet mutable box be a new Box with value 7.\nLet saved be copy of box.\nSet box's value to 8.\nShow saved's value.\nShow box's value.\n",
    ]);
}
