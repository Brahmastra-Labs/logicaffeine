//! weights_at_run_time_lock — a program reads its numbers from a file at run time, the same on every tier
//! (WEIGHTS-AT-RUN-TIME).
//!
//! `readReals(path)` is the `Seq of Real` a file of little-endian IEEE-754 binary64 values holds, eight bytes a
//! value, in order. A model's weights are data, not code: compiled in as statements a tiny model's cut is megabytes of
//! code (C, Rust and RISC-V take minutes and tens of gigabytes over it, the VM refuses a literal past its registers),
//! read at run time they are a load. These locks hold that every tier reads the same bits — the tree-walker, the VM,
//! the VM with its native tier, C, Rust, RISC-V under qemu and WebAssembly under the node shim — that a declared tensor
//! filled from a file computes what the same tensor filled statement by statement computes, and that a missing file or
//! a file that is not whole values stops the program by name.

#![cfg(not(target_arch = "wasm32"))]

mod common;
#[path = "support/riscv_runner.rs"]
mod riscv_runner;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::{build_c_release, require_tool, run_all_tiers, run_logos};
use logicaffeine_compile::compile::{compile_to_c, compile_to_riscv, compile_to_wasm, tw_outcome, vm_outcome};

/// Values whose every bit matters: thirds, a value past f32's range, the smallest subnormal, a negative zero's
/// neighbour, integers.
const VALUES: [f64; 8] = [0.5, -0.25, 1.0 / 3.0, 6.02214076e23, 5e-324, -7.0, 0.1 + 0.2, 1e-300];

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("weights_at_run_time_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("scratch dir");
    d
}

/// `values` written as a file of little-endian binary64 values.
fn reals_file(dir: &Path, name: &str, values: &[f64]) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, values.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>()).expect("write the values");
    path
}

/// A program that reads the file at `path` and shows its length and every value.
fn shows_every_value(path: &Path) -> String {
    format!(
        "## Main\nLet w be readReals(\"{}\").\nShow length of w.\nRepeat for x in w:\n    Show x.\n",
        path.display()
    )
}

/// What every tier must print: the VM's own showing of the length and each value.
fn expected() -> String {
    let src = format!(
        "## Main\nShow {}.\n{}",
        VALUES.len(),
        VALUES.iter().map(|v| format!("Show {}.\n", literal(*v))).collect::<String>()
    );
    let vm = vm_outcome(&src);
    assert_eq!(vm.error, None, "the literals run:\n{src}");
    vm.output
}

/// A Real as a literal that reads back to the same bits: shortest round-trip digits, a point always written.
fn literal(x: f64) -> String {
    let s = format!("{x}");
    if s.contains('.') { s } else { format!("{s}.0") }
}

#[test]
fn a_file_of_reals_reads_back_bit_for_bit_in_process() {
    let dir = scratch("in_process");
    let path = reals_file(&dir, "w.f64", &VALUES);
    let r = run_all_tiers(&shows_every_value(&path));
    assert_eq!(r.error, None, "the program runs on the tree-walker, the VM and the VM with its native tier");
    assert_eq!(r.output.trim(), expected().trim());
}

#[test]
fn a_file_of_reals_reads_back_bit_for_bit_through_c() {
    let dir = scratch("c");
    let path = reals_file(&dir, "w.f64", &VALUES);
    let c = compile_to_c(&shows_every_value(&path)).unwrap_or_else(|e| panic!("LOGOS→C: {e:?}"));
    let (src, bin) = (dir.join("main.c"), dir.join("main"));
    std::fs::write(&src, c).unwrap();
    let built = build_c_release(&src, &bin).expect("run the C compiler");
    assert!(built.status.success(), "the C program builds:\n{}", String::from_utf8_lossy(&built.stderr));
    let out = Command::new(&bin).output().expect("run the C program");
    assert!(out.status.success(), "the C program runs:\n{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), expected().trim());
}

#[test]
fn a_file_of_reals_reads_back_bit_for_bit_through_rust() {
    let dir = scratch("rust");
    let path = reals_file(&dir, "w.f64", &VALUES);
    let r = run_logos(&shows_every_value(&path));
    assert!(r.success, "the Rust program builds and runs:\n{}\n{}", r.stderr, r.rust_code);
    assert_eq!(r.stdout.trim(), expected().trim());
}

#[test]
fn a_file_of_reals_reads_back_bit_for_bit_on_riscv() {
    if !require_tool("qemu-riscv64") || !require_tool("riscv64-linux-gnu-gcc") {
        return;
    }
    let dir = scratch("riscv");
    let path = reals_file(&dir, "w.f64", &VALUES);
    let asm = compile_to_riscv(&shows_every_value(&path)).unwrap_or_else(|e| panic!("LOGOS→RISC-V: {e:?}"));
    let linked = riscv_runner::link(&asm, "readReals");
    let run = riscv_runner::run(&linked, &[], &[]);
    assert!(run.success, "the RISC-V program runs: {}", run.describe());
    assert_eq!(run.stdout.trim(), expected().trim());
}

#[test]
fn a_file_of_reals_reads_back_bit_for_bit_in_the_node_shim() {
    if !require_tool("node") {
        return;
    }
    let dir = scratch("wasm");
    let path = reals_file(&dir, "w.f64", &VALUES);
    let module = compile_to_wasm(&shows_every_value(&path)).unwrap_or_else(|e| panic!("LOGOS→WebAssembly: {e:?}"));
    std::fs::write(dir.join("probe.wasm"), module).unwrap();
    std::fs::write(dir.join("probe.mjs"), logicaffeine_compile::web_host::wasm_host_shim("probe")).unwrap();
    let out = Command::new("node").arg(dir.join("probe.mjs")).current_dir(&dir).output().expect("run node");
    assert!(out.status.success(), "the module runs under node:\n{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), expected().trim());
}

/// A declared tensor set from a file is the tensor filled item by item: the same product, bit for bit.
#[test]
fn a_tensor_filled_from_a_file_computes_what_its_statements_compute() {
    let dir = scratch("tensor");
    let w: Vec<f64> = (0..6).map(|i| (i as f64 - 2.5) / 7.0).collect();
    let path = reals_file(&dir, "w.f64", &w);
    let head = "## Main\nLet x be a 1 by 2 tensor of Real.\nSet item 1 of x to 0.75.\nSet item 2 of x to -1.5.\n\
                Let W be a 2 by 3 tensor of Real.\n";
    let tail = "Let y be x matmul W.\nShow item 1 of y.\nShow item 2 of y.\nShow item 3 of y.\n";
    let from_file = format!("{head}Set W to readReals(\"{}\").\n{tail}", path.display());
    let by_items = format!(
        "{head}{}{tail}",
        w.iter().enumerate().map(|(i, v)| format!("Set item {} of W to {}.\n", i + 1, literal(*v))).collect::<String>()
    );
    let (a, b) = (vm_outcome(&from_file), vm_outcome(&by_items));
    assert_eq!(a.error, None, "{from_file}");
    assert_eq!(a.output, b.output, "the tensor read from the file computes what the tensor set item by item computes");
}

/// A missing file, and a file that is not a whole number of eight-byte values, stop the program by name on the
/// in-process tiers.
#[test]
fn a_missing_file_or_a_partial_value_stops_by_name() {
    let dir = scratch("refused");
    let missing = dir.join("absent.f64");
    let partial = dir.join("partial.f64");
    std::fs::write(&partial, [0u8; 12]).unwrap();
    for (path, why) in [(&missing, "absent.f64"), (&partial, "12 bytes")] {
        let src = format!("## Main\nLet w be readReals(\"{}\").\nShow length of w.\n", path.display());
        for (tier, r) in [("tw", tw_outcome(&src)), ("vm", vm_outcome(&src))] {
            let e = r.error.unwrap_or_else(|| panic!("[{tier}] {path:?} is refused, not read: {}", r.output));
            assert!(e.contains("readReals") && e.contains(why), "[{tier}] the refusal names the call and the file: {e}");
        }
    }
}
