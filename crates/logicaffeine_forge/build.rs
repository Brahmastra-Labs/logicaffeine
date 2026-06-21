//! Build-time stencil extraction.
//!
//! Compiles `stencils/int_stencils.rs` to an object file with `rustc
//! --emit=obj` (no linker involved, so this works for ANY installed target),
//! then extracts every `logos_stencil_*` symbol's machine code and relocation
//! records, classifying each relocation against a per-format whitelist. Two
//! hard gates run here so regressions fail the BUILD, never silently at
//! runtime:
//!
//! - **Leaf purity**: a relocation targeting anything but a recognized hole
//!   symbol (`logos_hole_cont_N` / `LOGOS_HOLE_I64_N`) is an error — stencils
//!   must be self-contained.
//! - **Tail calls**: every continuation-hole site must decode as an
//!   UNCONDITIONAL branch (`b` on aarch64, `jmp` on x86-64), confirming LLVM
//!   emitted the CPS chain as sibling calls.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use object::{Object, ObjectSection, ObjectSymbol, RelocationTarget};

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let gen_path = out_dir.join("stencils.rs");
    let target = env::var("TARGET").unwrap();

    // The JIT is native-only; for wasm just emit an empty table so the
    // include! (compiled out for wasm anyway) always resolves.
    if target.contains("wasm32") {
        fs::write(
            &gen_path,
            "/// Empty on wasm (the JIT is native-only).\npub const ADD_STENCIL: &[u8] = &[];\n/// Empty on wasm (the JIT is native-only).\npub static STENCILS: &[Stencil] = &[];\n",
        )
        .unwrap();
        return;
    }

    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let stencil_src = manifest.join("stencils/int_stencils.rs");
    println!("cargo:rerun-if-changed={}", stencil_src.display());
    println!("cargo:rerun-if-changed=build.rs");

    // REGISTER-THREADING VARIANTS (EXODIA 3.1): generate location variants
    // for the integer 3-address families — each operand either
    // frame-resident (a const hole) or one of the four threaded registers.
    // The combined source (hand-written stencils + generated variants)
    // compiles as ONE codegen unit, exactly like before.
    let combined_src = out_dir.join("stencils_combined.rs");
    {
        let hand = fs::read_to_string(&stencil_src).expect("read stencil source");
        let mut gen = String::with_capacity(1 << 20);
        gen.push_str("\n// ===== GENERATED register-threading variants =====\n");
        let regs = ["r0", "r1", "r2", "r3"];
        let loc_read = |loc: usize, hole: u32| -> String {
            if loc == 0 {
                format!("*base.add(LOGOS_HOLE_I64_{hole} as usize)")
            } else {
                regs[loc - 1].to_string()
            }
        };
        // Binary ALU + comparison families: dst/lhs/rhs each in 5 locations.
        let binops: &[(&str, &str)] = &[
            ("add", "a.wrapping_add(b)"),
            ("sub", "a.wrapping_sub(b)"),
            ("mul", "a.wrapping_mul(b)"),
            ("band", "a & b"),
            ("bor", "a | b"),
            ("bxor", "a ^ b"),
            ("shl", "a.wrapping_shl(b as u32)"),
            ("shr", "a.wrapping_shr(b as u32)"),
            ("lt", "(a < b) as i64"),
            ("le", "(a <= b) as i64"),
            ("eq", "(a == b) as i64"),
            ("ne", "(a != b) as i64"),
        ];
        for (fam, expr) in binops {
            for d in 0..5usize {
                for l in 0..5usize {
                    for r in 0..5usize {
                        let a = loc_read(l, 0);
                        let b = loc_read(r, 1);
                        let store = if d == 0 {
                            format!("*base.add(LOGOS_HOLE_I64_2 as usize) = v;")
                        } else {
                            format!("let {} = v;", regs[d - 1])
                        };
                        gen.push_str(&format!(
                            "#[no_mangle]\npub unsafe extern \"C\" fn logos_stencil_v_{fam}_{d}{l}{r}(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {{\n    let a = {a};\n    let b = {b};\n    let v = {expr};\n    {store}\n    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)\n}}\n",
                        ));
                    }
                }
            }
        }
        // FLOAT 3-address families threaded through the XMM PINS (f0..f3).
        // A float operand at XMM-pin N reads `fN` DIRECTLY (already an f64,
        // no from_bits); a frame-resident operand reads the i64 bits and
        // reinterprets. Writing the dst at XMM-pin N tail-calls the
        // continuation with that fN REPLACED by the result (the updated pin
        // threads on); writing to the frame stores the bits. The 8 XMM arg
        // registers are free under the threaded ABI (base+sp consume 2 of the
        // 6 GP arg regs, leaving r0..r3 for the integer pins), so float pins
        // never compete with integer pins. NO divf: LOGOS treats float
        // division by zero as an ERROR, so DivF keeps its mem-form side-exit;
        // add/sub/mul never error → pure variants.
        let fregs = ["f0", "f1", "f2", "f3", "f4", "f5"];
        // Read a float operand from location `loc` (0 = frame, 1..4 = XMM pin),
        // using const hole `hole` for the frame slot index.
        let floc_read = |loc: usize, hole: u32| -> String {
            if loc == 0 {
                format!("f64::from_bits(*base.add(LOGOS_HOLE_I64_{hole} as usize) as u64)")
            } else {
                fregs[loc - 1].to_string()
            }
        };
        // Forward the four XMM pins to the continuation, with pin `d`
        // (1..4, or 0 = none) replaced by the just-computed value `v`.
        let fwd_floats = |d: usize| -> String {
            let mut parts: Vec<&str> = fregs.to_vec();
            let vname;
            if d != 0 {
                vname = "v";
                parts[d - 1] = vname;
            }
            parts.join(", ")
        };
        let fbinops: &[(&str, &str)] = &[
            ("addf", "a + b"),
            ("subf", "a - b"),
            ("mulf", "a * b"),
        ];
        for (fam, expr) in fbinops {
            for d in 0..7usize {
                for l in 0..7usize {
                    for r in 0..7usize {
                        let a = floc_read(l, 0);
                        let b = floc_read(r, 1);
                        // dst at frame (d==0): compute bits, store, forward the
                        // pins UNCHANGED. dst at XMM pin: keep the value as f64
                        // and thread it on in the cont call (no frame traffic).
                        let (body, floats) = if d == 0 {
                            (
                                format!("    *base.add(LOGOS_HOLE_I64_2 as usize) = ({expr}).to_bits() as i64;\n"),
                                fregs.join(", "),
                            )
                        } else {
                            (format!("    let v = {expr};\n"), fwd_floats(d))
                        };
                        gen.push_str(&format!(
                            "#[no_mangle]\npub unsafe extern \"C\" fn logos_stencil_v_{fam}_{d}{l}{r}(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {{\n    let a = {a};\n    let b = {b};\n{body}    logos_hole_cont_0(base, sp, r0, r1, r2, r3, {floats})\n}}\n",
                        ));
                    }
                }
            }
        }
        // FLOAT sqrt (pure, no side-exit) threaded through the XMM pins. src at
        // floc `s`, dst at floc `d`. Holes: 0 = src slot, 1 = dst slot (frame).
        for d in 0..7usize {
            for s in 0..7usize {
                let a = floc_read(s, 0);
                let (body, floats) = if d == 0 {
                    (
                        format!("    *base.add(LOGOS_HOLE_I64_1 as usize) = r.to_bits() as i64;\n"),
                        fregs.join(", "),
                    )
                } else {
                    (format!("    let v = r;\n"), fwd_floats(d))
                };
                gen.push_str(&format!(
                    "#[no_mangle]\npub unsafe extern \"C\" fn logos_stencil_v_sqrtf_{d}{s}(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {{\n    let a = {a};\n    let r: f64;\n    #[cfg(target_arch = \"x86_64\")]\n    core::arch::asm!(\"sqrtsd {{r}}, {{v}}\", r = out(xmm_reg) r, v = in(xmm_reg) a);\n    #[cfg(target_arch = \"aarch64\")]\n    core::arch::asm!(\"fsqrt {{r:d}}, {{v:d}}\", r = out(vreg) r, v = in(vreg) a);\n{body}    logos_hole_cont_0(base, sp, r0, r1, r2, r3, {floats})\n}}\n",
                ));
            }
        }
        // FLOAT divide threaded through the XMM pins. A divisor `== 0.0` (incl.
        // -0.0, IEEE equality — the kernel's exact check) exits to cont 1 BEFORE
        // any effect, exactly like the frame-form `divf3c`. Holes: 0 = lhs,
        // 1 = rhs, 2 = dst slot (frame).
        for d in 0..7usize {
            for l in 0..7usize {
                for r in 0..7usize {
                    let a = floc_read(l, 0);
                    let b = floc_read(r, 1);
                    let (body, floats) = if d == 0 {
                        (
                            format!("    *base.add(LOGOS_HOLE_I64_2 as usize) = (a / b).to_bits() as i64;\n"),
                            fregs.join(", "),
                        )
                    } else {
                        (format!("    let v = a / b;\n"), fwd_floats(d))
                    };
                    gen.push_str(&format!(
                        "#[no_mangle]\npub unsafe extern \"C\" fn logos_stencil_v_divf_{d}{l}{r}(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {{\n    let b = {b};\n    if b == 0.0 {{\n        return logos_hole_cont_1(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5);\n    }}\n    let a = {a};\n{body}    logos_hole_cont_0(base, sp, r0, r1, r2, r3, {floats})\n}}\n",
                    ));
                }
            }
        }
        // Int→Float threaded through XMM pins: int src from its GP pin / frame,
        // f64 result to an XMM pin / frame. Holes: 0 = src slot, 1 = dst slot.
        for d in 0..7usize {
            for s in 0..5usize {
                let i_read = loc_read(s, 0);
                let (body, floats) = if d == 0 {
                    (
                        "    *base.add(LOGOS_HOLE_I64_1 as usize) = (i as f64).to_bits() as i64;\n".to_string(),
                        fregs.join(", "),
                    )
                } else {
                    ("    let v = i as f64;\n".to_string(), fwd_floats(d))
                };
                gen.push_str(&format!(
                    "#[no_mangle]\npub unsafe extern \"C\" fn logos_stencil_v_i2f_{d}{s}(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {{\n    let i = {i_read};\n{body}    logos_hole_cont_0(base, sp, r0, r1, r2, r3, {floats})\n}}\n",
                ));
            }
        }
        // FUSED float compare-and-branch threaded through XMM pins: operands at
        // floc l/r; TRUE → cont 0, FALSE (incl. NaN-unordered) → cont 1. The
        // epsilon-equality branch uses integer immediates only (leaf-purity).
        let fbrops: &[(&str, &str)] = &[
            ("brltf", "a < b"),
            ("brlef", "a <= b"),
            ("breqf", "((a - b).to_bits() & 0x7FFF_FFFF_FFFF_FFFF) < f64::EPSILON.to_bits()"),
        ];
        for (fam, cond) in fbrops {
            for l in 0..7usize {
                for r in 0..7usize {
                    let a = floc_read(l, 0);
                    let b = floc_read(r, 1);
                    gen.push_str(&format!(
                        "#[no_mangle]\npub unsafe extern \"C\" fn logos_stencil_v_{fam}_{l}{r}(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {{\n    let a = {a};\n    let b = {b};\n    if {cond} {{\n        logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)\n    }} else {{\n        logos_hole_cont_1(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)\n    }}\n}}\n",
                    ));
                }
            }
        }
        // FLOAT moves over {frame, f0..f3} — the spill/reload primitive for
        // XMM pins (spill: pin → frame bits; reload: frame bits → pin). dst at
        // frame uses hole 1 for the slot, src at frame uses hole 0.
        for d in 0..7usize {
            for sloc in 0..7usize {
                let read = floc_read(sloc, 0);
                let (body, floats) = if d == 0 {
                    (
                        format!("    *base.add(LOGOS_HOLE_I64_1 as usize) = ({read}).to_bits() as i64;\n"),
                        fregs.join(", "),
                    )
                } else {
                    (format!("    let v = {read};\n"), fwd_floats(d))
                };
                gen.push_str(&format!(
                    "#[no_mangle]\npub unsafe extern \"C\" fn logos_stencil_v_fmov_{d}{sloc}(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {{\n{body}    logos_hole_cont_0(base, sp, r0, r1, r2, r3, {floats})\n}}\n",
                ));
            }
        }
        // Fused branches: lhs/rhs in 5 locations; TRUE -> cont 0, FALSE -> cont 1.
        let brops: &[(&str, &str)] = &[
            ("brlt", "a < b"),
            ("brle", "a <= b"),
            ("breq", "a == b"),
        ];
        for (fam, cond) in brops {
            for l in 0..5usize {
                for r in 0..5usize {
                    let a = loc_read(l, 0);
                    let b = loc_read(r, 1);
                    gen.push_str(&format!(
                        "#[no_mangle]\npub unsafe extern \"C\" fn logos_stencil_v_{fam}_{l}{r}(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {{\n    let a = {a};\n    let b = {b};\n    if {cond} {{\n        logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)\n    }} else {{\n        logos_hole_cont_1(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)\n    }}\n}}\n",
                    ));
                }
            }
        }
        // Moves: dst/src in 5 locations (mem->mem already exists as MOVSS;
        // generated anyway for table uniformity).
        for d in 0..5usize {
            for sloc in 0..5usize {
                let read = loc_read(sloc, 0);
                let store = if d == 0 {
                    format!("*base.add(LOGOS_HOLE_I64_1 as usize) = v;")
                } else {
                    format!("let {} = v;", regs[d - 1])
                };
                gen.push_str(&format!(
                    "#[no_mangle]\npub unsafe extern \"C\" fn logos_stencil_v_mov_{d}{sloc}(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {{\n    let v = {read};\n    {store}\n    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)\n}}\n",
                ));
            }
        }
        // Constants into a register (mem form exists as CONSTST).
        for d in 1..5usize {
            gen.push_str(&format!(
                "#[no_mangle]\npub unsafe extern \"C\" fn logos_stencil_v_const_{d}(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {{\n    let {} = LOGOS_HOLE_I64_0;\n    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)\n}}\n",
                regs[d - 1]
            ));
        }
        // Return from a register.
        for sloc in 1..5usize {
            gen.push_str(&format!(
                "#[no_mangle]\npub unsafe extern \"C\" fn logos_stencil_v_ret_{sloc}(_base: *mut i64, _sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {{\n    let _ = (r0, r1, r2, r3, f0, f1, f2, f3, f4, f5);\n    {}\n}}\n",
                regs[sloc - 1]
            ));
        }
        // REGISTER-FORM array access: the loop-invariant base POINTER rides a
        // GP pin (r0..r3) instead of being re-read from its frame cell every
        // access. `ptr` comes from the threaded register `loc` (1..4 → r0..r3);
        // the index (hole 0), length (hole 2, checked), and value (hole 3) stay
        // in the frame. Only sound for arrays that DO NOT move during the region
        // (no `ArrPush` → no realloc), which the caller guarantees.
        for loc in 1..5usize {
            let reg = regs[loc - 1];
            // unchecked load
            gen.push_str(&format!(
                "#[no_mangle]\npub unsafe extern \"C\" fn logos_stencil_v_arrld_rptr_{loc}(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {{\n    let i = *base.add(LOGOS_HOLE_I64_0 as usize);\n    let ptr = {reg} as *const i64;\n    *base.add(LOGOS_HOLE_I64_3 as usize) = *ptr.add(i.wrapping_sub(1) as usize);\n    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)\n}}\n",
            ));
            // checked load (bounds side-exit to cont 1, like `arrld`)
            gen.push_str(&format!(
                "#[no_mangle]\npub unsafe extern \"C\" fn logos_stencil_v_arrld_rptr_c_{loc}(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {{\n    let i = *base.add(LOGOS_HOLE_I64_0 as usize);\n    let len = *base.add(LOGOS_HOLE_I64_2 as usize);\n    let im1 = i.wrapping_sub(1);\n    if (im1 as u64) >= (len as u64) {{\n        return logos_hole_cont_1(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5);\n    }}\n    let ptr = {reg} as *const i64;\n    *base.add(LOGOS_HOLE_I64_3 as usize) = *ptr.add(im1 as usize);\n    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)\n}}\n",
            ));
            // unchecked store
            gen.push_str(&format!(
                "#[no_mangle]\npub unsafe extern \"C\" fn logos_stencil_v_arrst_rptr_{loc}(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {{\n    let i = *base.add(LOGOS_HOLE_I64_0 as usize);\n    let ptr = {reg} as *mut i64;\n    *ptr.add(i.wrapping_sub(1) as usize) = *base.add(LOGOS_HOLE_I64_3 as usize);\n    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)\n}}\n",
            ));
            // checked store
            gen.push_str(&format!(
                "#[no_mangle]\npub unsafe extern \"C\" fn logos_stencil_v_arrst_rptr_c_{loc}(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {{\n    let i = *base.add(LOGOS_HOLE_I64_0 as usize);\n    let len = *base.add(LOGOS_HOLE_I64_2 as usize);\n    let im1 = i.wrapping_sub(1);\n    if (im1 as u64) >= (len as u64) {{\n        return logos_hole_cont_1(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5);\n    }}\n    let ptr = {reg} as *mut i64;\n    *ptr.add(im1 as usize) = *base.add(LOGOS_HOLE_I64_3 as usize);\n    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)\n}}\n",
            ));
        }
        // FUSED read-modify-write on a pinned 8-byte int array (mem-form):
        // `buf[idx-1] = buf[idx-1] <op> frame[operand]` in ONE stencil — the
        // peephole collapses ArrLoad + <int ALU> + ArrStore to the SAME array
        // and index when the loaded value and the result are single-use, so the
        // element never round-trips the frame and ONE bounds check covers the
        // load+store. Holes match `arrst`: 0 = idx, 1 = ptr slot, 2 = len slot,
        // 3 = operand. The `_c` twins are unchecked (Oracle-proven index).
        for (op, expr) in [
            ("add", "(*cell).wrapping_add(operand)"),
            ("sub", "(*cell).wrapping_sub(operand)"),
            ("mul", "(*cell).wrapping_mul(operand)"),
            ("and", "*cell & operand"),
            ("or", "*cell | operand"),
            ("xor", "*cell ^ operand"),
        ] {
            // checked (bounds side-exit to cont 1, like `arrst`)
            gen.push_str(&format!(
                "#[no_mangle]\npub unsafe extern \"C\" fn logos_stencil_arrrmw_{op}(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {{\n    let i = *base.add(LOGOS_HOLE_I64_0 as usize);\n    let len = *base.add(LOGOS_HOLE_I64_2 as usize);\n    let im1 = i.wrapping_sub(1);\n    if (im1 as u64) >= (len as u64) {{\n        return logos_hole_cont_1(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5);\n    }}\n    let ptr = *base.add(LOGOS_HOLE_I64_1 as usize) as *mut i64;\n    let operand = *base.add(LOGOS_HOLE_I64_3 as usize);\n    let cell = ptr.add(im1 as usize);\n    *cell = {expr};\n    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)\n}}\n",
            ));
            // unchecked (index proven in range)
            gen.push_str(&format!(
                "#[no_mangle]\npub unsafe extern \"C\" fn logos_stencil_arrrmw_{op}_u(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {{\n    let i = *base.add(LOGOS_HOLE_I64_0 as usize);\n    let ptr = *base.add(LOGOS_HOLE_I64_1 as usize) as *mut i64;\n    let operand = *base.add(LOGOS_HOLE_I64_3 as usize);\n    let cell = ptr.add(i.wrapping_sub(1) as usize);\n    *cell = {expr};\n    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)\n}}\n",
            ));
        }
        // FLOAT read-modify-write twins: the element + operand are reinterpreted
        // as f64 (the nbody `v[i] = v[i] + dx*mag` velocity/position updates).
        // Same hole layout; copy-and-patch never reassociates, so the result is
        // bit-identical to ArrLoad + {AddF,SubF,MulF} + ArrStore.
        for (op, fexpr) in [
            ("addf", "cur + operand"),
            ("subf", "cur - operand"),
            ("mulf", "cur * operand"),
        ] {
            // checked
            gen.push_str(&format!(
                "#[no_mangle]\npub unsafe extern \"C\" fn logos_stencil_arrrmw_{op}(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {{\n    let i = *base.add(LOGOS_HOLE_I64_0 as usize);\n    let len = *base.add(LOGOS_HOLE_I64_2 as usize);\n    let im1 = i.wrapping_sub(1);\n    if (im1 as u64) >= (len as u64) {{\n        return logos_hole_cont_1(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5);\n    }}\n    let ptr = *base.add(LOGOS_HOLE_I64_1 as usize) as *mut i64;\n    let operand = f64::from_bits(*base.add(LOGOS_HOLE_I64_3 as usize) as u64);\n    let cell = ptr.add(im1 as usize);\n    let cur = f64::from_bits(*cell as u64);\n    *cell = ({fexpr}).to_bits() as i64;\n    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)\n}}\n",
            ));
            // unchecked
            gen.push_str(&format!(
                "#[no_mangle]\npub unsafe extern \"C\" fn logos_stencil_arrrmw_{op}_u(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {{\n    let i = *base.add(LOGOS_HOLE_I64_0 as usize);\n    let ptr = *base.add(LOGOS_HOLE_I64_1 as usize) as *mut i64;\n    let operand = f64::from_bits(*base.add(LOGOS_HOLE_I64_3 as usize) as u64);\n    let cell = ptr.add(i.wrapping_sub(1) as usize);\n    let cur = f64::from_bits(*cell as u64);\n    *cell = ({fexpr}).to_bits() as i64;\n    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)\n}}\n",
            ));
        }
        // FUSED float multiply-add (mem-form): `dst = (a*b) + c`, TWO roundings
        // (NOT a hardware single-rounding FMA) so it is bit-identical to MulF +
        // AddF. Holes: 0 = a, 1 = b, 2 = c, 3 = dst. Forwards f0..f5 to the
        // continuation, so the threaded XMM pins survive.
        gen.push_str(
            "#[no_mangle]\npub unsafe extern \"C\" fn logos_stencil_fmaf(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {\n    let a = f64::from_bits(*base.add(LOGOS_HOLE_I64_0 as usize) as u64);\n    let b = f64::from_bits(*base.add(LOGOS_HOLE_I64_1 as usize) as u64);\n    let c = f64::from_bits(*base.add(LOGOS_HOLE_I64_2 as usize) as u64);\n    *base.add(LOGOS_HOLE_I64_3 as usize) = ((a * b) + c).to_bits() as i64;\n    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)\n}\n",
        );
        // FUSED conditional adjacent swap (mem-form): `let a=buf[i1-1]; let
        // b=buf[i2-1]; if a CMP b { buf[i1-1]=b; buf[i2-1]=a }` — the sort
        // inner-loop idiom (ArrLoad×2 + Branch + ArrStore×2 → 1). Atomic
        // (both-or-neither). Holes: 0=idx1, 1=idx2, 2=ptr, 3=len (checked only).
        for (cmp, op) in [("gt", ">"), ("lt", "<"), ("ge", ">="), ("le", "<=")] {
            gen.push_str(&format!(
                "#[no_mangle]\npub unsafe extern \"C\" fn logos_stencil_arrcondswap_{cmp}(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {{\n    let i1 = *base.add(LOGOS_HOLE_I64_0 as usize);\n    let i2 = *base.add(LOGOS_HOLE_I64_1 as usize);\n    let len = *base.add(LOGOS_HOLE_I64_3 as usize);\n    let m1 = i1.wrapping_sub(1);\n    let m2 = i2.wrapping_sub(1);\n    if (m1 as u64) >= (len as u64) || (m2 as u64) >= (len as u64) {{\n        return logos_hole_cont_1(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5);\n    }}\n    let ptr = *base.add(LOGOS_HOLE_I64_2 as usize) as *mut i64;\n    let a = *ptr.add(m1 as usize);\n    let b = *ptr.add(m2 as usize);\n    if a {op} b {{\n        *ptr.add(m1 as usize) = b;\n        *ptr.add(m2 as usize) = a;\n    }}\n    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)\n}}\n",
            ));
            gen.push_str(&format!(
                "#[no_mangle]\npub unsafe extern \"C\" fn logos_stencil_arrcondswap_{cmp}_u(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {{\n    let i1 = *base.add(LOGOS_HOLE_I64_0 as usize);\n    let i2 = *base.add(LOGOS_HOLE_I64_1 as usize);\n    let ptr = *base.add(LOGOS_HOLE_I64_2 as usize) as *mut i64;\n    let m1 = i1.wrapping_sub(1) as usize;\n    let m2 = i2.wrapping_sub(1) as usize;\n    let a = *ptr.add(m1);\n    let b = *ptr.add(m2);\n    if a {op} b {{\n        *ptr.add(m1) = b;\n        *ptr.add(m2) = a;\n    }}\n    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)\n}}\n",
            ));
        }
        // FUSED unconditional adjacent swap (mem-form): `let a=buf[i1-1]; let
        // b=buf[i2-1]; buf[i1-1]=b; buf[i2-1]=a` — the `tmp` exchange idiom
        // (quick/heap/merge sort), 4 ops → 1. Atomic. Holes: 0=idx1, 1=idx2,
        // 2=ptr, 3=len (checked only).
        gen.push_str(
            "#[no_mangle]\npub unsafe extern \"C\" fn logos_stencil_arrswap(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {\n    let i1 = *base.add(LOGOS_HOLE_I64_0 as usize);\n    let i2 = *base.add(LOGOS_HOLE_I64_1 as usize);\n    let len = *base.add(LOGOS_HOLE_I64_3 as usize);\n    let m1 = i1.wrapping_sub(1);\n    let m2 = i2.wrapping_sub(1);\n    if (m1 as u64) >= (len as u64) || (m2 as u64) >= (len as u64) {\n        return logos_hole_cont_1(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5);\n    }\n    let ptr = *base.add(LOGOS_HOLE_I64_2 as usize) as *mut i64;\n    let a = *ptr.add(m1 as usize);\n    let b = *ptr.add(m2 as usize);\n    *ptr.add(m1 as usize) = b;\n    *ptr.add(m2 as usize) = a;\n    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)\n}\n",
        );
        gen.push_str(
            "#[no_mangle]\npub unsafe extern \"C\" fn logos_stencil_arrswap_u(base: *mut i64, sp: *mut i64, r0: i64, r1: i64, r2: i64, r3: i64, f0: f64, f1: f64, f2: f64, f3: f64, f4: f64, f5: f64) -> i64 {\n    let i1 = *base.add(LOGOS_HOLE_I64_0 as usize);\n    let i2 = *base.add(LOGOS_HOLE_I64_1 as usize);\n    let ptr = *base.add(LOGOS_HOLE_I64_2 as usize) as *mut i64;\n    let m1 = i1.wrapping_sub(1) as usize;\n    let m2 = i2.wrapping_sub(1) as usize;\n    let a = *ptr.add(m1);\n    let b = *ptr.add(m2);\n    *ptr.add(m1) = b;\n    *ptr.add(m2) = a;\n    logos_hole_cont_0(base, sp, r0, r1, r2, r3, f0, f1, f2, f3, f4, f5)\n}\n",
        );
        fs::write(&combined_src, format!("{hand}{gen}")).expect("write combined stencils");
    }

    let obj_path = out_dir.join("int_stencils.o");
    let rustc = env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string());
    let status = Command::new(&rustc)
        .args([
            "-C",
            "opt-level=2",
            "-C",
            "panic=abort",
            "-C",
            "codegen-units=1",
            "--emit=obj",
            "--crate-type=lib",
            "--target",
            &target,
            "-o",
        ])
        .arg(&obj_path)
        .arg(&combined_src)
        .status()
        .expect("failed to invoke rustc to compile stencils");
    assert!(status.success(), "rustc stencil compilation failed");

    let data = fs::read(&obj_path).expect("read object file");
    let file = object::File::parse(&*data).expect("parse object file");
    let is_aarch64 = target.starts_with("aarch64");

    // Collect every stencil symbol with its section extent.
    struct SymExtent {
        name: String,
        section: object::SectionIndex,
        addr: u64,
        size: u64,
    }
    let mut stencil_syms: Vec<SymExtent> = Vec::new();
    for sym in file.symbols() {
        let raw = match sym.name() {
            Ok(n) => n,
            Err(_) => continue,
        };
        let name = raw.strip_prefix('_').unwrap_or(raw);
        if !name.starts_with("logos_stencil_") {
            continue;
        }
        let section = match sym.section_index() {
            Some(s) => s,
            None => continue,
        };
        stencil_syms.push(SymExtent { name: name.to_string(), section, addr: sym.address(), size: sym.size() });
    }
    assert!(!stencil_syms.is_empty(), "no logos_stencil_* symbols found");

    // Mach-O symbols frequently report size 0; derive extents from the next
    // symbol in the same section, falling back to the section end.
    for i in 0..stencil_syms.len() {
        if stencil_syms[i].size == 0 {
            let section = file.section_by_index(stencil_syms[i].section).unwrap();
            let mut end = section.address() + section.data().unwrap().len() as u64;
            for s in file.symbols() {
                if s.section_index() == Some(stencil_syms[i].section) {
                    let a = s.address();
                    if a > stencil_syms[i].addr && a < end {
                        end = a;
                    }
                }
            }
            stencil_syms[i].size = end - stencil_syms[i].addr;
        }
    }

    let mut out = String::new();
    out.push_str("// GENERATED by build.rs — extracted stencil table.\n");

    let mut table_entries: Vec<String> = Vec::new();
    for sym in &stencil_syms {
        let section = file.section_by_index(sym.section).expect("section");
        let sect_data = section.data().expect("section data");
        let sect_addr = section.address();
        let start = (sym.addr - sect_addr) as usize;
        let code = &sect_data[start..start + sym.size as usize];

        // Gather + classify this symbol's relocations (function-relative).
        let mut relocs: Vec<(u32, String, String, i64)> = Vec::new();
        let mut pending_macho_addend: i64 = 0;
        for (off, reloc) in section.relocations() {
            if off < sym.addr || off >= sym.addr + sym.size {
                continue;
            }
            let fn_off = (off - sym.addr) as u32;

            // Mach-O ARM64_RELOC_ADDEND pairs with the FOLLOWING reloc.
            if let object::RelocationFlags::MachO { r_type, .. } = reloc.flags() {
                if is_aarch64 && r_type == 10 {
                    pending_macho_addend = reloc.addend();
                    continue;
                }
            }
            let mut addend = if pending_macho_addend != 0 {
                std::mem::take(&mut pending_macho_addend)
            } else {
                reloc.addend()
            };
            // Normalize PC-relative addends so the runtime patcher computes a
            // uniform `disp = target + addend - field_address`:
            // - ELF already bakes the -4 (rip-after-disp) into the addend.
            // - Mach-O x86 pcrel relocs use rip = field + 4 with addend 0.
            // - COFF REL32_N additionally encodes N trailing bytes in the type.
            match reloc.flags() {
                object::RelocationFlags::MachO { r_type, r_pcrel: true, .. } if !is_aarch64 => {
                    let _ = r_type;
                    addend -= 4;
                }
                object::RelocationFlags::Coff { typ } if (4..=9).contains(&typ) => {
                    addend -= 4 + (typ as i64 - 4);
                }
                _ => {}
            }

            let (target_name, via_import) = match reloc.target() {
                RelocationTarget::Symbol(idx) => {
                    let s = file.symbol_by_index(idx).expect("reloc target symbol");
                    let raw = s.name().expect("reloc target name");
                    // COFF expresses GOT-style indirection in the SYMBOL:
                    // `__imp_x` (MSVC import slot) or `.refptr.x` (MinGW
                    // reference-pointer stub) — both are pointer slots for x.
                    if let Some(stripped) = raw.strip_prefix("__imp_") {
                        (stripped.to_string(), true)
                    } else if let Some(stripped) = raw.strip_prefix(".refptr.") {
                        (stripped.to_string(), true)
                    } else {
                        (raw.strip_prefix('_').unwrap_or(raw).to_string(), false)
                    }
                }
                other => panic!("stencil '{}': unsupported reloc target {:?}", sym.name, other),
            };

            let hole = classify_hole(&sym.name, &target_name);
            let kind = classify_kind(
                &sym.name,
                &target_name,
                code,
                fn_off,
                reloc.flags(),
                is_aarch64,
                via_import,
            );

            // Tail-call gate: continuation sites must be unconditional branches.
            if hole.starts_with("HoleId::Cont") {
                assert_tail_call(&sym.name, code, fn_off, &kind, is_aarch64);
            }

            relocs.push((fn_off, kind, hole, addend));
        }
        relocs.sort_by_key(|r| r.0);

        let reloc_src: Vec<String> = relocs
            .iter()
            .map(|(off, kind, hole, addend)| {
                format!("Reloc {{ offset: {off}, kind: {kind}, target: {hole}, addend: {addend} }}")
            })
            .collect();

        let const_name = sym.name.replace("logos_stencil_", "ST_").to_uppercase();
        out.push_str(&format!(
            "/// Extracted `{}` stencil ({} bytes, {} relocs).\n\
             pub static {}: Stencil = Stencil {{ name: \"{}\", code: &{:?}, relocs: &[{}] }};\n",
            sym.name,
            code.len(),
            relocs.len(),
            const_name,
            sym.name,
            code,
            reloc_src.join(", ")
        ));
        table_entries.push(const_name);

        // Back-compat: the relocation-free leaf used by the headline test.
        if sym.name == "logos_stencil_add" {
            assert!(relocs.is_empty(), "logos_stencil_add must stay relocation-free");
            out.push_str(&format!(
                "/// The relocation-free leaf-add stencil (headline smoke test).\n\
                 pub const ADD_STENCIL: &[u8] = &{:?};\n",
                code
            ));
        }
    }

    out.push_str("/// Every extracted stencil, in object-file order.\npub static STENCILS: &[&Stencil] = &[");
    for e in &table_entries {
        out.push_str(&format!("&{e}, "));
    }
    out.push_str("];\n");

    // The register-threading variant tables, indexed by location digits.
    // Binops/cmps: [family][dst][lhs][rhs]; branches: [family][lhs][rhs];
    // moves: [dst][src]; const/ret: [reg].
    let fam_list = ["add", "sub", "mul", "band", "bor", "bxor", "shl", "shr", "lt", "le", "eq", "ne"];
    out.push_str("pub static V_BINOP: [[[[&Stencil; 5]; 5]; 5]; 12] = [\n");
    for fam in fam_list {
        out.push_str("    [");
        for d in 0..5 {
            out.push_str("[");
            for l in 0..5 {
                out.push_str("[");
                for r in 0..5 {
                    out.push_str(&format!("&ST_V_{}_{d}{l}{r}, ", fam.to_uppercase()));
                }
                out.push_str("], ");
            }
            out.push_str("], ");
        }
        out.push_str("],\n");
    }
    out.push_str("];\n");
    let ffam_list = ["addf", "subf", "mulf"];
    out.push_str("pub static V_FBINOP: [[[[&Stencil; 7]; 7]; 7]; 3] = [\n");
    for fam in ffam_list {
        out.push_str("    [");
        for d in 0..7 {
            out.push_str("[");
            for l in 0..7 {
                out.push_str("[");
                for r in 0..7 {
                    out.push_str(&format!("&ST_V_{}_{d}{l}{r}, ", fam.to_uppercase()));
                }
                out.push_str("], ");
            }
            out.push_str("], ");
        }
        out.push_str("],\n");
    }
    out.push_str("];\n");
    // FLOAT mem-form variant tables (XMM-through-mem-form). V_SQRTF[dst][src],
    // V_DIVF[dst][lhs][rhs] over {frame, f0..f3}.
    out.push_str("pub static V_SQRTF: [[&Stencil; 7]; 7] = [");
    for d in 0..7 {
        out.push_str("[");
        for s in 0..7 {
            out.push_str(&format!("&ST_V_SQRTF_{d}{s}, "));
        }
        out.push_str("], ");
    }
    out.push_str("];\n");
    out.push_str("pub static V_DIVF: [[[&Stencil; 7]; 7]; 7] = [\n");
    for d in 0..7 {
        out.push_str("    [");
        for l in 0..7 {
            out.push_str("[");
            for r in 0..7 {
                out.push_str(&format!("&ST_V_DIVF_{d}{l}{r}, "));
            }
            out.push_str("], ");
        }
        out.push_str("],\n");
    }
    out.push_str("];\n");
    // V_I2F[dst_floc][src_loc] (int→float); V_BRANCHF[fam(ltf/lef/eqf)][lhs_floc][rhs_floc].
    out.push_str("pub static V_I2F: [[&Stencil; 5]; 7] = [");
    for d in 0..7 {
        out.push_str("[");
        for s in 0..5 {
            out.push_str(&format!("&ST_V_I2F_{d}{s}, "));
        }
        out.push_str("], ");
    }
    out.push_str("];\n");
    out.push_str("pub static V_BRANCHF: [[[&Stencil; 7]; 7]; 3] = [\n");
    for fam in ["brltf", "brlef", "breqf"] {
        out.push_str("    [");
        for l in 0..7 {
            out.push_str("[");
            for r in 0..7 {
                out.push_str(&format!("&ST_V_{}_{l}{r}, ", fam.to_uppercase()));
            }
            out.push_str("], ");
        }
        out.push_str("],\n");
    }
    out.push_str("];\n");
    let br_list = ["brlt", "brle", "breq"];
    out.push_str("pub static V_BRANCH: [[[&Stencil; 5]; 5]; 3] = [\n");
    for fam in br_list {
        out.push_str("    [");
        for l in 0..5 {
            out.push_str("[");
            for r in 0..5 {
                out.push_str(&format!("&ST_V_{}_{l}{r}, ", fam.to_uppercase()));
            }
            out.push_str("], ");
        }
        out.push_str("],\n");
    }
    out.push_str("];\n");
    out.push_str("pub static V_MOV: [[&Stencil; 5]; 5] = [");
    for d in 0..5 {
        out.push_str("[");
        for sloc in 0..5 {
            out.push_str(&format!("&ST_V_MOV_{d}{sloc}, "));
        }
        out.push_str("], ");
    }
    out.push_str("];\n");
    // Float move table (XMM-pin spill/reload), indexed [dst][src] over
    // {frame, f0..f3}.
    out.push_str("pub static V_FMOV: [[&Stencil; 7]; 7] = [");
    for d in 0..7 {
        out.push_str("[");
        for sloc in 0..7 {
            out.push_str(&format!("&ST_V_FMOV_{d}{sloc}, "));
        }
        out.push_str("], ");
    }
    out.push_str("];\n");
    out.push_str("pub static V_CONST: [&Stencil; 4] = [");
    for d in 1..5 {
        out.push_str(&format!("&ST_V_CONST_{d}, "));
    }
    out.push_str("];\n");
    out.push_str("pub static V_RET: [&Stencil; 4] = [");
    for sloc in 1..5 {
        out.push_str(&format!("&ST_V_RET_{sloc}, "));
    }
    out.push_str("];\n");
    // Register-form array access tables, indexed by the base pointer's GP-pin
    // location (1..4 = r0..r3; index 0 unused). `_C` = bounds-checked.
    for (name, sfx) in [
        ("V_ARRLD_RPTR", "arrld_rptr"),
        ("V_ARRLD_RPTR_C", "arrld_rptr_c"),
        ("V_ARRST_RPTR", "arrst_rptr"),
        ("V_ARRST_RPTR_C", "arrst_rptr_c"),
    ] {
        out.push_str(&format!("pub static {name}: [&Stencil; 5] = [&ST_V_RET_1, "));
        for loc in 1..5 {
            out.push_str(&format!("&ST_V_{}_{loc}, ", sfx.to_uppercase()));
        }
        out.push_str("];\n");
    }

    fs::write(&gen_path, out).unwrap();
}

/// Map a relocation-target symbol name to its hole — the LEAF-PURITY gate.
fn classify_hole(stencil: &str, target: &str) -> String {
    if let Some(n) = target.strip_prefix("logos_hole_cont_") {
        let n: u8 = n.parse().unwrap_or_else(|_| {
            panic!("stencil '{stencil}': malformed continuation hole '{target}'")
        });
        return format!("HoleId::Cont({n})");
    }
    if let Some(n) = target.strip_prefix("LOGOS_HOLE_I64_") {
        let n: u8 = n
            .parse()
            .unwrap_or_else(|_| panic!("stencil '{stencil}': malformed const hole '{target}'"));
        return format!("HoleId::ConstI64({n})");
    }
    panic!(
        "stencil '{stencil}' references non-hole symbol '{target}' — stencils must be \
         self-contained leaves (only logos_hole_cont_N / LOGOS_HOLE_I64_N)"
    );
}

/// Normalize a format-specific relocation to our RelocKind — anything outside
/// the whitelist is a build error naming the stencil and the raw kind.
#[allow(clippy::too_many_arguments)]
fn classify_kind(
    stencil: &str,
    target: &str,
    code: &[u8],
    off: u32,
    flags: object::RelocationFlags,
    is_aarch64: bool,
    via_import: bool,
) -> String {
    match flags {
        object::RelocationFlags::MachO { r_type, .. } => {
            // ARM64_RELOC_*: BRANCH26=2 PAGE21=3 PAGEOFF12=4 GOT_LOAD_PAGE21=5
            // GOT_LOAD_PAGEOFF12=6. X86_64_RELOC_: BRANCH=2 SIGNED=1 GOT_LOAD=3
            // GOT=4 UNSIGNED=0 — distinguished by target arch via the insn.
            match r_type {
                2 => {
                    // ARM64_RELOC_BRANCH26 or X86_64_RELOC_BRANCH.
                    if is_aarch64 {
                        "RelocKind::Branch26".to_string()
                    } else {
                        "RelocKind::Rel32".to_string()
                    }
                }
                3 => {
                    if is_aarch64 {
                        assert!(
                            is_aarch64_adrp(code, off),
                            "stencil '{stencil}': PAGE21 site is not an ADRP"
                        );
                        "RelocKind::Page21".to_string()
                    } else {
                        // X86_64_RELOC_GOT_LOAD
                        "RelocKind::GotRel32".to_string()
                    }
                }
                4 => {
                    if is_aarch64 {
                        let scale = aarch64_pageoff_scale(code, off).unwrap_or_else(|| {
                            panic!("stencil '{stencil}': PAGEOFF12 site is not ADD/LDR")
                        });
                        format!("RelocKind::PageOff12 {{ scale: {scale} }}")
                    } else {
                        // X86_64_RELOC_GOT
                        "RelocKind::GotRel32".to_string()
                    }
                }
                5 => "RelocKind::GotPage21".to_string(),
                6 => "RelocKind::GotPageOff12".to_string(),
                0 => "RelocKind::Abs64".to_string(),
                1 => "RelocKind::Rel32".to_string(),
                other => panic!(
                    "stencil '{stencil}' (target '{target}'): unsupported Mach-O reloc type {other}"
                ),
            }
        }
        object::RelocationFlags::Elf { r_type } => match r_type {
            // x86_64
            1 => "RelocKind::Abs64".to_string(),
            2 | 4 => "RelocKind::Rel32".to_string(),
            9 | 41 | 42 => "RelocKind::GotRel32".to_string(),
            // aarch64
            257 => "RelocKind::Abs64".to_string(),
            282 | 283 => "RelocKind::Branch26".to_string(),
            275 => "RelocKind::Page21".to_string(),
            277 => "RelocKind::PageOff12 { scale: 0 }".to_string(),
            286 => "RelocKind::PageOff12 { scale: 3 }".to_string(),
            311 => "RelocKind::GotPage21".to_string(),
            312 => "RelocKind::GotPageOff12".to_string(),
            other => panic!(
                "stencil '{stencil}' (target '{target}'): unsupported ELF reloc type {other}"
            ),
        },
        object::RelocationFlags::Coff { typ } => match typ {
            1 => "RelocKind::Abs64".to_string(),
            // REL32 .. REL32_5: the trailing distance folds into the addend
            // at patch time via (typ - 4). `__imp_` targets are import-table
            // (GOT-style) indirections.
            4..=9 if via_import => "RelocKind::GotRel32".to_string(),
            4..=9 => "RelocKind::Rel32".to_string(),
            other => panic!(
                "stencil '{stencil}' (target '{target}'): unsupported COFF reloc type {other}"
            ),
        },
        other => panic!("stencil '{stencil}': unsupported reloc flags {other:?}"),
    }
}

fn read_u32(code: &[u8], off: u32) -> u32 {
    let i = off as usize;
    u32::from_le_bytes([code[i], code[i + 1], code[i + 2], code[i + 3]])
}

fn is_aarch64_adrp(code: &[u8], off: u32) -> bool {
    if off as usize + 4 > code.len() {
        return false;
    }
    let insn = read_u32(code, off);
    (insn & 0x9F00_0000) == 0x9000_0000
}

/// For an arm64 lo12 site: the access-size scale (0 for ADD, log2(bytes) for
/// LDR/STR), or None when the instruction is not an arm64 lo12 form.
fn aarch64_pageoff_scale(code: &[u8], off: u32) -> Option<u8> {
    if off as usize + 4 > code.len() {
        return None;
    }
    let insn = read_u32(code, off);
    // ADD (immediate), 64-bit: sf=1 op=0 S=0 100010 …
    if (insn & 0x7F80_0000) == 0x1100_0000 {
        return Some(0);
    }
    // Load/store register (unsigned immediate): size[31:30] 111 V 01 opc …
    if (insn & 0x3B00_0000) == 0x3900_0000 {
        return Some((insn >> 30) as u8);
    }
    None
}

/// The TAIL-CALL gate: a continuation site must be an unconditional branch.
fn assert_tail_call(stencil: &str, code: &[u8], off: u32, kind: &str, is_aarch64: bool) {
    if is_aarch64 {
        assert!(
            kind.contains("Branch26"),
            "stencil '{stencil}': continuation reloc at {off} is {kind}, expected Branch26"
        );
        let insn = read_u32(code, off);
        let top6 = insn >> 26;
        assert!(
            top6 == 0b000101,
            "stencil '{stencil}': continuation at {off} is {} (insn {insn:#010x}), not a tail \
             call — rustc stopped emitting sibling calls; the stencil chain would grow the stack",
            if top6 == 0b100101 { "BL (a call)" } else { "not a branch" }
        );
    } else if kind == "RelocKind::Rel32" {
        // Direct: `jmp rel32` (E9) or a conditional `jcc rel32` (0F 8x) —
        // both transfer control without pushing a return address. A CALL
        // (E8) means rustc stopped emitting sibling calls.
        let i = off as usize;
        let opcode = code[i.checked_sub(1).expect("rel32 at offset 0")];
        let is_jmp = opcode == 0xE9;
        let is_jcc = i >= 2 && code[i - 2] == 0x0F && (opcode & 0xF0) == 0x80;
        assert!(
            is_jmp || is_jcc,
            "stencil '{stencil}': continuation at {off} has opcode {opcode:#04x}, expected JMP \
             (E9) or Jcc (0F 8x) — a CALL (E8) means rustc stopped emitting sibling calls"
        );
    } else if kind == "RelocKind::GotRel32" {
        // Indirect through the GOT, in either of LLVM's two sibling-call
        // shapes — both load the continuation address from the pool slot and
        // transfer control WITHOUT pushing a return address:
        //
        //   (A) `jmp qword ptr [rip+disp]` = FF /4, the disp32 SITES the reloc
        //       (single instruction; the 2-byte opcode+modrm precede it).
        //   (B) `mov reg, [rip+disp]` (8B /r, ModRM mod=00 rm=101) loading the
        //       slot into a scratch register, IMMEDIATELY followed by an
        //       indirect `jmp reg` (FF /4, ModRM mod=11 reg=100). LLVM picks
        //       this when forwarding the wide threaded-register set (the GP +
        //       XMM pins) leaves no encoding for a memory-indirect jmp.
        //
        // A `call` (FF /2 direct, or a CALL after the mov) would mean rustc
        // stopped emitting sibling calls and the stencil chain would grow the
        // stack — that is what this gate rejects.
        let i = off as usize;
        let pre_op = code[i.checked_sub(2).expect("got site at offset < 2")];
        let pre_modrm = code[i - 1];
        let form_a = pre_op == 0xFF && (pre_modrm & 0x38) == 0x20;
        // Form B: the site is the disp32 of a `mov reg, [rip+disp]` (8B /r,
        // ModRM mod=00 rm=101) that loads the continuation address into a
        // scratch register; the transfer is a later indirect `jmp reg`
        // (FF /4, ModRM mod=11) — either immediately (mod3c) or after a frame
        // epilogue that restores the spilled GP/XMM pins (alloclist). LLVM
        // picks this two-instruction shape when forwarding the wide threaded
        // set (GP + XMM pins) leaves no encoding for a single memory-indirect
        // jmp. Walking forward from the load to the first `FF`-form register
        // transfer, that transfer MUST be a jmp (/4); an indirect CALL (/2)
        // would push a return address and grow the stack per chain step.
        let mov_load = pre_op == 0x8B && (pre_modrm & 0xC7) == 0x05;
        let form_b = mov_load && {
            let mut p = i + 4;
            let mut decided: Option<bool> = None;
            while p + 1 < code.len() {
                if (0x40..=0x4F).contains(&code[p]) && code[p + 1] == 0xFF {
                    p += 1;
                }
                if code[p] == 0xFF {
                    let m = code[p + 1];
                    if (m & 0xC0) == 0xC0 {
                        // mod=11: indirect register transfer. /4 = jmp (safe),
                        // /2 = call (a return-pushing call: reject).
                        decided = Some((m & 0x38) == 0x20);
                        break;
                    }
                }
                p += 1;
            }
            decided.unwrap_or(false)
        };
        assert!(
            form_a || form_b,
            "stencil '{stencil}': continuation at {off} is {pre_op:#04x} {pre_modrm:#04x}, \
             expected an indirect JMP (FF /4) or a GOT-load whose register is tail-jumped \
             (mov reg,[rip]; … ; jmp reg) — an indirect CALL (FF /2) means rustc stopped \
             emitting sibling calls"
        );
    } else {
        panic!("stencil '{stencil}': continuation reloc at {off} is {kind}, expected Rel32/GotRel32");
    }
}
