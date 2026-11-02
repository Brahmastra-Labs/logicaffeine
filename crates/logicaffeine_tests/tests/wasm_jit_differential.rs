//! WS6 (FINISH_INTERPRETER Phase 13) — WASM-JIT ↔ VM differential.
//!
//! The WebAssembly module the backend emits from a function's real VM bytecode (run through
//! the conformant `wasmi` interpreter) must produce the SAME result as the bytecode VM. This
//! is the codegen-correctness oracle: a curated set of real functions PLUS a seeded fuzz of
//! random integer arithmetic, each cross-checked against the VM. It runs in the regular fast
//! suite (the runner enables `--features wasm-jit`), so a codegen regression is *caught*,
//! not hoped away. An eligibility counter makes a vacuous pass impossible — if the backend
//! never fires on real bytecode, the test fails loudly.

#![cfg(all(feature = "wasm-jit", not(target_arch = "wasm32")))]

use logicaffeine_compile::compile::vm_run_source;
use logicaffeine_compile::ui_bridge::with_parsed_program;
use logicaffeine_compile::vm::wasm_jit::{compile_function_to_wasm, compile_region_to_wasm};
use logicaffeine_compile::vm::{Compiler, Constant, Op};

/// Run the emitted module's `f(arg)` through wasmi — `None` if it TRAPS. A trap is the
/// WASM-JIT's overflow deopt (checked add/sub/mul `unreachable` on signed overflow): the host
/// surfaces it as an error and `WasmTier::call` returns `None`, falling back to the VM. So a
/// `None` here is a sound deopt, not a failure. (Instantiation also validates the bytes.)
fn run_wasmi(module: &[u8], arg: i64) -> Option<i64> {
    let engine = wasmi::Engine::default();
    let m = wasmi::Module::new(&engine, module).expect("emitted bytes are valid wasm");
    let mut store = wasmi::Store::new(&engine, ());
    let mut linker = wasmi::Linker::<()>::new(&engine);
    // A module that CAN fail declares `logos_rt_fail`; an undefined import is a silent
    // instantiation failure, not a missing feature. These modules decide nothing about it.
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
    let instance = linker
        .instantiate(&mut store, &m)
        .unwrap()
        .start(&mut store)
        .unwrap();
    let f = instance.get_typed_func::<i64, i64>(&store, "f").unwrap();
    f.call(&mut store, arg).ok()
}

/// Two-argument variant of [`run_wasmi`] (for multi-param regions like `f(a, count)`).
fn run_wasmi2(module: &[u8], a: i64, b: i64) -> Option<i64> {
    let engine = wasmi::Engine::default();
    let m = wasmi::Module::new(&engine, module).expect("emitted bytes are valid wasm");
    let mut store = wasmi::Store::new(&engine, ());
    let mut linker = wasmi::Linker::<()>::new(&engine);
    // A module that CAN fail declares `logos_rt_fail`; an undefined import is a silent
    // instantiation failure, not a missing feature. These modules decide nothing about it.
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
    let instance = linker
        .instantiate(&mut store, &m)
        .unwrap()
        .start(&mut store)
        .unwrap();
    let f = instance
        .get_typed_func::<(i64, i64), i64>(&store, "f")
        .unwrap();
    f.call(&mut store, (a, b)).ok()
}

// ── Corner 6b: a memory-backed, VARIABLE-LENGTH wasm journal ─────────────────────────────────────
// Hand-emit a wasm module (linear memory + loops) that journals a DATA-DEPENDENT number of destroyed
// bits to a memory-stack tape and reverses by popping it — the dynamic-count analog of the VM's
// `HistoryTape`, beyond the fixed saved-locals of the straight-line case.

fn leb_u(mut n: u64, out: &mut Vec<u8>) {
    loop {
        let b = (n & 0x7f) as u8;
        n >>= 7;
        if n == 0 {
            out.push(b);
            break;
        }
        out.push(b | 0x80);
    }
}
fn leb_i(mut n: i64, out: &mut Vec<u8>) {
    loop {
        let b = (n & 0x7f) as u8;
        n >>= 7;
        if (n == 0 && b & 0x40 == 0) || (n == -1 && b & 0x40 != 0) {
            out.push(b);
            break;
        }
        out.push(b | 0x80);
    }
}
fn section(module: &mut Vec<u8>, id: u8, content: &[u8]) {
    module.push(id);
    let mut len = Vec::new();
    leb_u(content.len() as u64, &mut len);
    module.extend_from_slice(&len);
    module.extend_from_slice(content);
}

/// Emit `f() -> i64`: journal `n` dropped low-bits of `init` to a MEMORY-STACK tape while shifting it
/// down, then (if `use_journal`) pop the tape to reconstruct `init` — returns 1 iff the reconstruction
/// equals `init`. Locals: 0=acc(i64) 1=sp(i32) 2=i(i32) 3=orig(i64) 4=tmp(i64).
fn emit_memory_tape_reverse(init: i64, n: u32, use_journal: bool) -> Vec<u8> {
    let mut b = Vec::new();
    let (gl, gs, gi, go, gt) = (0u32, 1u32, 2u32, 3u32, 4u32);
    // setup: acc=init; orig=acc; sp=0; i=0
    b.push(0x42);
    leb_i(init, &mut b);
    b.extend([0x21, gl as u8]);
    b.extend([0x20, gl as u8, 0x21, go as u8]);
    b.extend([0x41, 0x00, 0x21, gs as u8]);
    b.extend([0x41, 0x00, 0x21, gi as u8]);
    // forward loop
    b.extend([0x02, 0x40, 0x03, 0x40]); // block; loop
    b.extend([0x20, gi as u8]);
    b.push(0x41);
    leb_i(n as i64, &mut b);
    b.extend([0x4e, 0x0d, 0x01]); // i32.ge_s; br_if 1
    b.extend([0x20, gl as u8, 0x42, 0x01, 0x83, 0x21, gt as u8]); // tmp = acc & 1
    b.extend([0x20, gs as u8, 0x20, gt as u8, 0x37, 0x03, 0x00]); // mem[sp] = tmp (i64.store a=3 o=0)
    b.extend([0x20, gs as u8, 0x41, 0x08, 0x6a, 0x21, gs as u8]); // sp += 8
    b.extend([0x20, gl as u8, 0x42, 0x01, 0x87, 0x21, gl as u8]); // acc >>= 1 (shr_s)
    b.extend([0x20, gi as u8, 0x41, 0x01, 0x6a, 0x21, gi as u8]); // i += 1
    b.extend([0x0c, 0x00, 0x0b, 0x0b]); // br 0; end loop; end block
                                        // reverse loop
    b.extend([0x02, 0x40, 0x03, 0x40]);
    b.extend([0x20, gs as u8, 0x41, 0x00, 0x4c, 0x0d, 0x01]); // sp<=0 ? br_if 1
    b.extend([0x20, gs as u8, 0x41, 0x08, 0x6b, 0x21, gs as u8]); // sp -= 8
    b.extend([0x20, gs as u8, 0x29, 0x03, 0x00, 0x21, gt as u8]); // tmp = mem[sp] (i64.load)
    b.extend([0x20, gl as u8, 0x42, 0x01, 0x86]); // acc << 1
    if use_journal {
        b.extend([0x20, gt as u8, 0x84]); // | tmp  (re-merge the journaled bit)
    }
    b.extend([0x21, gl as u8]);
    b.extend([0x0c, 0x00, 0x0b, 0x0b]);
    // return acc == orig
    b.extend([0x20, gl as u8, 0x20, go as u8, 0x51, 0xad, 0x0b]); // i64.eq; i64.extend_i32_u; end fn

    // locals: (1×i64)(2×i32)(2×i64)
    let mut body = vec![0x03, 0x01, 0x7e, 0x02, 0x7f, 0x02, 0x7e];
    body.extend_from_slice(&b);
    let mut code_entry = Vec::new();
    leb_u(body.len() as u64, &mut code_entry);
    code_entry.extend_from_slice(&body);

    let mut m = vec![0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00];
    section(&mut m, 1, &[0x01, 0x60, 0x00, 0x01, 0x7e]); // type: () -> i64
    section(&mut m, 3, &[0x01, 0x00]); // func 0 : type 0
    section(&mut m, 5, &[0x01, 0x00, 0x01]); // memory: 1 min page
    section(&mut m, 7, &[0x01, 0x01, 0x66, 0x00, 0x00]); // export "f" func 0
    let mut code = Vec::new();
    leb_u(1, &mut code);
    code.extend_from_slice(&code_entry);
    section(&mut m, 10, &code);
    m
}

#[test]
fn wasm_reverses_a_variable_length_journal_over_a_memory_tape() {
    // ★ CORNER 6b (full) — a VARIABLE-LENGTH wasm journal in LINEAR MEMORY ★. A loop shifts `acc` down
    // `n` times, journaling each destroyed low bit to a memory-STACK tape (`mem[sp]`, `sp += 8`); the
    // reverse loop pops the tape (`sp -= 8`) and re-merges each bit (`acc = (acc<<1) | popped`). The
    // number of tape entries is DATA-DEPENDENT (the loop count) — the dynamic-count analog of the VM's
    // `HistoryTape`, in wasm linear memory. Proven `forward;reverse ≡ id`: `26` shifted down 5 and
    // rebuilt from its 5 memory-tape bits returns to `26`, so `f` returns 1.
    let module = emit_memory_tape_reverse(26, 5, true);
    assert_eq!(
        run_wasmi0(&module),
        Some(1),
        "the memory-tape reverse must reconstruct the original from its variable-length journal"
    );
    // Drop the journal re-merge → the shifted-out bits are gone → reconstruction fails → 0.
    let broken = emit_memory_tape_reverse(26, 5, false);
    assert_eq!(
        run_wasmi0(&broken),
        Some(0),
        "without the memory-tape bits the lossy shift cannot reverse — never a false pass"
    );
}

/// Zero-argument variant of [`run_wasmi`] (for the journaled-reverse roundtrip `f() -> i64`).
fn run_wasmi0(module: &[u8]) -> Option<i64> {
    let engine = wasmi::Engine::default();
    let m = wasmi::Module::new(&engine, module).expect("emitted bytes are valid wasm");
    let mut store = wasmi::Store::new(&engine, ());
    let mut linker = wasmi::Linker::<()>::new(&engine);
    // A module that CAN fail declares `logos_rt_fail`; an undefined import is a silent
    // instantiation failure, not a missing feature. These modules decide nothing about it.
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
    let instance = linker
        .instantiate(&mut store, &m)
        .unwrap()
        .start(&mut store)
        .unwrap();
    let f = instance.get_typed_func::<(), i64>(&store, "f").unwrap();
    f.call(&mut store, ()).ok()
}

#[test]
fn wasm_natively_reverses_a_journaled_lossy_op_via_a_saved_tape() {
    // ★ CORNER 6 (part b) — JOURNALED WASM reverse ★. Beyond the zero-tape abelian core: a genuinely
    // LOSSY op reverses on the WASM tier using a wasm-side journal. In-place `a = a & mask` DESTROYS the
    // bits where `mask = 0`; the WASM forward first SAVES exactly those bits to a tape register (`saved =
    // a & ~mask` — corner-1's MaskReconstruct), and the reverse re-merges them (`a = (a & mask) | saved`)
    // to recover the original `a`. The identity genuinely DEPENDS on the journal — without it, `a` stays
    // masked. Proven `forward;reverse ≡ id` IN WASM: `f` returns 1.
    //
    // Registers: r0 = a (working); r1 = mask; r2 = ~mask; r3 = saved (the wasm-side journal); r4 = the
    // original a (for the identity check); r5 = the check result.
    let a = 0xF0F0i64; // 1111_0000_1111_0000
    let mask = 0x00FFi64; // keep the low byte
    let notmask = !mask; // ~mask = the bits the AND destroys (= -256)
    let ops = vec![
        Op::LoadConst { dst: 0, idx: 0 }, // a
        Op::LoadConst { dst: 1, idx: 1 }, // mask
        Op::LoadConst { dst: 2, idx: 2 }, // ~mask
        Op::Move { dst: 4, src: 0 },      // remember the original a for the check
        // ── forward: JOURNAL the destroyed bits, then the LOSSY in-place AND ──
        Op::BitAnd {
            dst: 3,
            lhs: 0,
            rhs: 2,
        }, // saved := a & ~mask  (the bits AND will clear)
        Op::BitAnd {
            dst: 0,
            lhs: 0,
            rhs: 1,
        }, // a := a & mask       (LOSSY — high bits destroyed)
        // ── reverse: re-merge the journaled bits under the complementary mask ──
        Op::BitOr {
            dst: 0,
            lhs: 0,
            rhs: 3,
        }, // a := (a & mask) | saved  ==  original a
        // ── identity check ──
        Op::Eq {
            dst: 5,
            lhs: 0,
            rhs: 4,
        }, // a == original a ?
        Op::Return { src: 5 },
    ];
    let consts = &[
        Constant::Int(a),
        Constant::Int(mask),
        Constant::Int(notmask),
    ];
    let module =
        compile_region_to_wasm(&ops, consts, 0, 6).expect("the journaled roundtrip region emits");
    assert_eq!(
        run_wasmi0(&module),
        Some(1),
        "the WASM journaled reverse must recover the original value from the saved-tape bits"
    );

    // A BROKEN reverse (drop the journal re-merge) leaves `a` masked ≠ original → the check fails (0).
    let mut broken = ops.clone();
    broken.remove(6); // drop the `BitOr a := (a & mask) | saved` — the journal restore
    let broken_mod = compile_region_to_wasm(&broken, consts, 0, 6).expect("emits");
    assert_eq!(
        run_wasmi0(&broken_mod),
        Some(0),
        "without the saved-tape re-merge the lossy op cannot reverse — never a false pass"
    );
}

/// Four-argument variant of [`run_wasmi`] (for the abelian-reverse roundtrip `f(place, d1, d2, d3)`).
fn run_wasmi4(module: &[u8], a: i64, b: i64, c: i64, d: i64) -> Option<i64> {
    let engine = wasmi::Engine::default();
    let m = wasmi::Module::new(&engine, module).expect("emitted bytes are valid wasm");
    let mut store = wasmi::Store::new(&engine, ());
    let mut linker = wasmi::Linker::<()>::new(&engine);
    // A module that CAN fail declares `logos_rt_fail`; an undefined import is a silent
    // instantiation failure, not a missing feature. These modules decide nothing about it.
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
    let instance = linker
        .instantiate(&mut store, &m)
        .unwrap()
        .start(&mut store)
        .unwrap();
    let f = instance
        .get_typed_func::<(i64, i64, i64, i64), i64>(&store, "f")
        .unwrap();
    f.call(&mut store, (a, b, c, d)).ok()
}

/// One abelian core op on the accumulator `r0` with a delta register — the zero-tape reversible subset.
/// `inverse()` returns its inverse per the SAME rules the VM's reverse executor uses (`RevAdd⁻¹ = Sub`,
/// `RevSub⁻¹ = Add`, `RevXor⁻¹ = Xor` — self-inverse), so the emitted reverse is faithful to the VM.
#[derive(Clone, Copy)]
enum Abelian {
    Add(u16),
    Sub(u16),
    Xor(u16),
}

impl Abelian {
    fn inverse(self) -> Abelian {
        match self {
            Abelian::Add(d) => Abelian::Sub(d),
            Abelian::Sub(d) => Abelian::Add(d),
            Abelian::Xor(d) => Abelian::Xor(d), // XOR is its own inverse
        }
    }
    fn lower(self) -> Op {
        match self {
            Abelian::Add(d) => Op::Add {
                dst: 0,
                lhs: 0,
                rhs: d,
            },
            Abelian::Sub(d) => Op::Sub {
                dst: 0,
                lhs: 0,
                rhs: d,
            },
            Abelian::Xor(d) => Op::BitXor {
                dst: 0,
                lhs: 0,
                rhs: d,
            },
        }
    }
}

/// Emit a WASM region that runs the abelian `forward` program on the accumulator `r0`, then its
/// DERIVED reverse (each op's inverse, in reverse order), then checks `r0` returned to its saved
/// initial value — the WASM-NATIVE reverse of the zero-tape abelian core. Returns 1 iff `forward ;
/// reverse ≡ id` holds on the WASM tier. `params`: place + 3 deltas; scratch: saved (r4), result (r5).
fn emit_abelian_reverse_roundtrip(forward: &[Abelian]) -> Vec<u8> {
    let saved = 4u16;
    let result = 5u16;
    let mut ops = vec![Op::Move { dst: saved, src: 0 }]; // save the initial accumulator
    ops.extend(forward.iter().map(|a| a.lower())); // forward
    ops.extend(forward.iter().rev().map(|a| a.inverse().lower())); // reverse: inverses, reversed order
    ops.push(Op::Eq {
        dst: result,
        lhs: 0,
        rhs: saved,
    }); // identity check
    ops.push(Op::Return { src: result });
    compile_region_to_wasm(&ops, &[], 4, 6).expect("the abelian roundtrip region emits")
}

#[test]
fn wasm_natively_reverses_the_abelian_core() {
    // ★ CORNER 6 (part b) — WASM-native reverse ★. The shared-Op WASM backend runs not just forward but
    // the REVERSE of the zero-tape abelian core: a program of RevAdd/RevSub/RevXor, lowered forward, then
    // its inverse (derived by the VM's own reverse rules) in reverse order, is proven `forward;reverse ==
    // id` IN WASM (the emitted `f` returns 1). Checked over arbitrary inputs — a real second reversible
    // tier beside the VM, not just the forward agreement of part (a).
    let forward = [
        Abelian::Add(1),
        Abelian::Xor(2),
        Abelian::Sub(3),
        Abelian::Add(2),
    ];
    let module = emit_abelian_reverse_roundtrip(&forward);
    // Deterministic LCG over (place, d1, d2, d3) — the reverse must restore the accumulator every time.
    let mut seed = 0x1234_5678_9abc_def0u64;
    let mut rng = || {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((seed >> 40) as i64) & 0xffff // small magnitudes — no i64 overflow trap
    };
    for _ in 0..64 {
        let (place, d1, d2, d3) = (rng(), rng(), rng(), rng());
        assert_eq!(
            run_wasmi4(&module, place, d1, d2, d3),
            Some(1),
            "WASM forward;reverse must restore the accumulator for ({place},{d1},{d2},{d3})"
        );
    }

    // A CORRUPTED reverse (drop the last inverse) must NOT restore the accumulator — the check is real.
    let mut ops = vec![Op::Move { dst: 4, src: 0 }];
    ops.extend(forward.iter().map(|a| a.lower()));
    ops.extend(forward.iter().rev().skip(1).map(|a| a.inverse().lower())); // MISSING one inverse
    ops.push(Op::Eq {
        dst: 5,
        lhs: 0,
        rhs: 4,
    });
    ops.push(Op::Return { src: 5 });
    let broken = compile_region_to_wasm(&ops, &[], 4, 6).expect("emits");
    assert_eq!(
        run_wasmi4(&broken, 7, 1, 2, 3),
        Some(0),
        "an incomplete WASM reverse must fail the identity check (0), never falsely pass"
    );
}

/// Emit `fn_name`'s WASM module from `src` — `None` if the function is not WASM-JIT-eligible.
fn wasm_jit_bytes(src: &str, fn_name: &str) -> Option<Vec<u8>> {
    with_parsed_program(src, |parsed, interner| {
        let (stmts, types, _policies) = parsed.ok()?;
        let program = Compiler::compile_with_types(stmts, interner, Some(types)).ok()?;
        let sym = interner.lookup(fn_name)?;
        let fi = *program.fn_index.get(&sym)? as usize;
        compile_function_to_wasm(&program, fi)
    })
}

/// The VM ground truth for `Show fn_name(arg).` — the printed value as a string (it may be a
/// BigInt once the EXACT-integer arithmetic promotes past i64).
fn vm_output(src: &str) -> String {
    vm_run_source(src).expect("VM runs").trim().to_string()
}

/// The SOUNDNESS contract for `f(arg)`, when the function is WASM-JIT-eligible:
/// - the module returns a value ⟹ it computed the whole kernel in i64 without overflowing,
///   so it MUST equal the VM's output exactly (never a silent wrap);
/// - the module traps ⟹ some intermediate overflowed i64 → a sound deopt to the VM. We do NOT
///   compare here: the VM's *final* result may still fit i64 (it carries BigInt intermediates
///   and downsizes — e.g. `(x+5)-5` recovers `i64::MAX`), so a trap does not imply the final
///   value overflows. Trapping is always sound (the WASM-JIT returns nothing, never a wrong
///   value); the in-range correctness is what the returned case pins.
/// Returns whether the function was eligible, so callers can assert the backend actually fired.
fn check(fn_def: &str, fn_name: &str, arg: i64) -> bool {
    let src = format!("{fn_def}\n## Main\n    Show {fn_name}({arg}).\n");
    let vm = vm_output(&src);
    let Some(bytes) = wasm_jit_bytes(&src, fn_name) else {
        return false;
    };
    if let Some(jit) = run_wasmi(&bytes, arg) {
        assert_eq!(
            jit.to_string(),
            vm,
            "WASM-JIT disagrees with the VM for {fn_name}({arg}): jit={jit} vm={vm}\n{fn_def}"
        );
    }
    true
}

#[test]
fn wasm_jit_matches_vm_on_real_functions() {
    // Curated pure-integer functions whose compiled bytecode exercises straight-line
    // arithmetic, a counted loop, and a branch.
    // The large args (e.g. 4_000_000_000) overflow i64 inside the kernel, exercising the
    // checked-arithmetic deopt: the VM promotes to BigInt and the WASM-JIT must TRAP (never
    // wrap), which `check` verifies.
    let cases: &[(&str, &str, &[i64])] = &[
        (
            "## To poly (x: Int) -> Int:\n    Return x * x + x.",
            "poly",
            &[0, 1, 5, 12, 4_000_000_000],
        ),
        (
            "## To tri (n: Int) -> Int:\n    \
             Let acc be 0.\n    \
             Let i be n.\n    \
             While i is greater than 0:\n        \
             Set acc to acc + i.\n        \
             Set i to i - 1.\n    \
             Return acc.",
            "tri",
            &[0, 1, 10, 100],
        ),
        (
            "## To pick (x: Int) -> Int:\n    \
             If x is greater than 10:\n        \
             Return x + 100.\n    \
             Return x * 2.",
            "pick",
            &[3, 9, 10, 50],
        ),
    ];
    let mut eligible = 0usize;
    for (def, name, args) in cases {
        for &arg in *args {
            if check(def, name, arg) {
                eligible += 1;
            }
        }
    }
    assert!(
        eligible >= 1,
        "no curated function was WASM-JIT-eligible — the backend never fired on real bytecode"
    );
}

/// SplitMix64 — deterministic fuzz PRNG (no `Math.random`).
fn next_rand(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A small, total integer expression over `x` and literals `0..9`, using only `+ - *` so it
/// never errors and (with the bounded depth/literals) stays well inside i64 — so VM and WASM
/// (both wrapping i64) agree exactly.
fn gen_expr(state: &mut u64, depth: u32) -> String {
    if depth == 0 || next_rand(state) % 3 == 0 {
        if next_rand(state) % 2 == 0 {
            "x".to_string()
        } else {
            format!("{}", next_rand(state) % 10)
        }
    } else {
        let op = match next_rand(state) % 3 {
            0 => "+",
            1 => "-",
            _ => "*",
        };
        let l = gen_expr(state, depth - 1);
        let r = gen_expr(state, depth - 1);
        format!("({l} {op} {r})")
    }
}

#[test]
fn wasm_jit_fuzz_matches_vm() {
    let mut eligible = 0usize;
    for seed in 0..150u64 {
        let mut state = seed ^ 0xD1B5_4A32_D192_ED03;
        let body = gen_expr(&mut state, 3);
        let def = format!("## To f (x: Int) -> Int:\n    Return {body}.");
        // Small args stay in range (WASM-JIT returns, must equal the VM); the large ones
        // overflow many of the generated expressions, exercising the checked-overflow deopt
        // (WASM-JIT traps, the VM promotes to BigInt) — both paths verified by `check`.
        for &arg in &[0i64, 1, 2, 5, 9, 3_000_000_000, i64::MAX] {
            if check(&def, "f", arg) {
                eligible += 1;
            }
        }
    }
    assert!(
        eligible >= 1,
        "the fuzz produced no WASM-JIT-eligible functions — the backend never fired"
    );
}

#[test]
fn wasm_tier_fires_after_threshold_and_matches_vm() {
    use logicaffeine_compile::vm::wasm_jit::WasmTier;

    let fn_def = "## To sq (x: Int) -> Int:\n    Return x * x.";
    let arg = 9i64;
    let src = format!("{fn_def}\n## Main\n    Show sq({arg}).\n");
    let vm: i64 = vm_output(&src).parse().expect("sq(9) = 81 fits i64"); // 81

    let outcome = with_parsed_program(&src, |parsed, interner| {
        let (stmts, types, _policies) = parsed.ok()?;
        let program = Compiler::compile_with_types(stmts, interner, Some(types)).ok()?;
        let sym = interner.lookup("sq")?;
        let fi = *program.fn_index.get(&sym)?;
        let mut tier = WasmTier::new(3);
        // First two calls warm the counter (bytecode tier); the third crosses the
        // threshold and tiers up; the rest dispatch to the compiled module.
        let r1 = tier.on_call(&program, fi, &[arg]);
        let r2 = tier.on_call(&program, fi, &[arg]);
        let r3 = tier.on_call(&program, fi, &[arg]);
        let r4 = tier.on_call(&program, fi, &[arg]);
        Some((r1, r2, r3, r4, tier.hits()))
    });
    let (r1, r2, r3, r4, hits) = outcome.expect("sq must parse + compile");
    assert_eq!(r1, None, "call 1 is below threshold — bytecode tier");
    assert_eq!(r2, None, "call 2 is below threshold — bytecode tier");
    assert_eq!(r3, Some(vm), "call 3 tiers up and must match the VM ({vm})");
    assert_eq!(r4, Some(vm), "call 4 dispatches to the cached module");
    assert!(
        hits >= 2,
        "the WASM-JIT tier must have fired (got {hits} hits)"
    );
}

/// A function whose return value is a `Bool` (a comparison result) must NOT tier: the WASM-JIT
/// returns an i64 the caller boxes as `Int`, so a `Bool` would be mis-typed (`Show` prints `1`,
/// not `true`). It must deopt to the VM, which types the result correctly. Only declared-`Int`
/// returns are WASM-JIT-eligible — keeping the tiers in sync on return types.
#[test]
fn wasm_jit_declines_non_int_returning_function() {
    let src = "## To gt5 (x: Int) -> Bool:\n    Return x is greater than 5.\n\
               ## Main\n    Show gt5(8).\n";
    assert!(
        wasm_jit_bytes(src, "gt5").is_none(),
        "a Bool-returning function must be WASM-JIT-ineligible (deopt, not mis-boxed as Int)"
    );
    // The program still runs correctly on the VM — and prints the Bool, not `1`.
    assert_eq!(vm_output(src), "true");

    // An Int-returning function with the SAME comparison inside (used only for control flow,
    // returning an Int) stays eligible — the boundary is the RETURN type, not the ops used.
    let int_src = "## To clamp (x: Int) -> Int:\n    \
                   If x is greater than 5:\n        Return 5.\n    Return x.\n\
                   ## Main\n    Show clamp(8).\n";
    assert!(
        wasm_jit_bytes(int_src, "clamp").is_some(),
        "Int-returning fn with a compare must still tier"
    );
}

/// The EXACT-integer contract, in sync with the VM: checked add/sub/mul return the in-range
/// result and TRAP on signed overflow (so the task deopts to the VM, which promotes to
/// BigInt) — never a silent wrap. A trap surfaces as a `None` from [`run_wasmi`].
#[test]
fn wasm_jit_checked_arith_traps_on_overflow_never_wraps() {
    // f(x) = x * x.
    let sq = compile_region_to_wasm(
        &[
            Op::Mul {
                dst: 1,
                lhs: 0,
                rhs: 0,
            },
            Op::Return { src: 1 },
        ],
        &[],
        1,
        2,
    )
    .expect("mul region emits");
    for x in [0i64, 1, -1, 1000, -1000, 3_037_000_499] {
        // 3_037_000_499² = 9_223_372_030_926_249_001 < i64::MAX — in range.
        assert_eq!(run_wasmi(&sq, x), Some(x * x), "in-range x*x for x={x}");
    }
    for x in [
        4_000_000_000i64,
        5_000_000_000,
        i64::MAX,
        i64::MIN,
        0x1_0000_0000,
    ] {
        assert_eq!(
            run_wasmi(&sq, x),
            None,
            "x*x must TRAP on overflow for x={x}, not silently wrap to {}",
            x.wrapping_mul(x)
        );
    }

    // f(x) = x + 1 and x - 1: the boundary cases that promote in the VM.
    let inc = compile_region_to_wasm(
        &[
            Op::LoadConst { dst: 1, idx: 0 },
            Op::Add {
                dst: 2,
                lhs: 0,
                rhs: 1,
            },
            Op::Return { src: 2 },
        ],
        &[Constant::Int(1)],
        1,
        3,
    )
    .expect("add region emits");
    assert_eq!(run_wasmi(&inc, 41), Some(42), "in-range add");
    assert_eq!(run_wasmi(&inc, i64::MAX), None, "i64::MAX + 1 must trap");

    let dec = compile_region_to_wasm(
        &[
            Op::LoadConst { dst: 1, idx: 0 },
            Op::Sub {
                dst: 2,
                lhs: 0,
                rhs: 1,
            },
            Op::Return { src: 2 },
        ],
        &[Constant::Int(1)],
        1,
        3,
    )
    .expect("sub region emits");
    assert_eq!(run_wasmi(&dec, 43), Some(42), "in-range sub");
    assert_eq!(run_wasmi(&dec, i64::MIN), None, "i64::MIN - 1 must trap");
}

/// Bitwise XOR + arithmetic/logical shifts match the VM's exact semantics: `^`, `wrapping_shl`,
/// and `wrapping_shr` (ARITHMETIC for signed i64), with the shift count masked mod 64 — which
/// WASM's `i64.xor` / `i64.shl` / `i64.shr_s` reproduce bit-for-bit.
#[test]
fn wasm_jit_bitwise_and_shift_match_vm_semantics() {
    // f(x) = ((x ^ 0xF0F0) << 3) >> 1   (arithmetic >>, sign-preserving).
    let ops = vec![
        Op::LoadConst { dst: 1, idx: 0 }, // 0xF0F0
        Op::BitXor {
            dst: 2,
            lhs: 0,
            rhs: 1,
        }, // x ^ 0xF0F0
        Op::LoadConst { dst: 3, idx: 1 }, // 3
        Op::Shl {
            dst: 4,
            lhs: 2,
            rhs: 3,
        }, // << 3
        Op::LoadConst { dst: 5, idx: 2 }, // 1
        Op::Shr {
            dst: 6,
            lhs: 4,
            rhs: 5,
        }, // >> 1
        Op::Return { src: 6 },
    ];
    let consts = vec![Constant::Int(0xF0F0), Constant::Int(3), Constant::Int(1)];
    let bytes = compile_region_to_wasm(&ops, &consts, 1, 7).expect("bitwise region emits");
    for x in [0i64, 1, 255, -1, -1000, i64::MIN, i64::MAX, 0x1234_5678] {
        let expect = ((x ^ 0xF0F0).wrapping_shl(3)).wrapping_shr(1);
        assert_eq!(
            run_wasmi(&bytes, x),
            Some(expect),
            "((x ^ 0xF0F0) << 3) >> 1 for x={x}"
        );
    }

    // Shift-count masking mod 64: a 2-param region `f(a, count) = a << count`.
    let shl = compile_region_to_wasm(
        &[
            Op::Shl {
                dst: 2,
                lhs: 0,
                rhs: 1,
            },
            Op::Return { src: 2 },
        ],
        &[],
        2,
        3,
    )
    .expect("shl region emits");
    for (a, c) in [
        (1i64, 0i64),
        (1, 1),
        (1, 63),
        (1, 64),
        (1, 65),
        (-8, 2),
        (255, 4),
    ] {
        assert_eq!(
            run_wasmi2(&shl, a, c),
            Some(a.wrapping_shl(c as u32)),
            "{a} << {c} (mod-64 count)"
        );
    }
}
