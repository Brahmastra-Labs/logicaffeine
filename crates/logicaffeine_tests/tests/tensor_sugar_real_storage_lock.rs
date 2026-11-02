//! A tensor the program computes is STORED in its element kind on every tier.
//!
//! The tensor sugars desugar to a backing `Seq` and ordinary loops. The dynamic tiers (tree-walker, VM) hold
//! whatever a `Seq` is given, so a `Seq of Int` filled with reals still prints the reals there. The typed tiers
//! (C, RISC-V, AOT-Rust) take the declared element type at its word: a `matmul` of Real tensors backed by a
//! `Seq of Int` stores `3.5` as `3`, and the program prints a different answer on C than on the VM with no error
//! anywhere. So every computed tensor's backing `Seq` carries the result's kind — Real when any operand is Real,
//! Int when every operand is Int — and a `transpose` keeps its input's.
//!
//! Every program below prints `round(v · 1000)` (or an Int as is), so the comparison is exact and independent of
//! how a tier formats a float; every tier must print what the VM prints.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test tensor_sugar_real_storage_lock`
//!
//! ★ MUTATION PROBE (run when this landed): back `matmul` with `Seq of Int` again (`new_tensor_seq(true)` in the
//! parser's `desugar_tensor_matmul`) — the tree-walker and VM stay green, C prints `3000 3000` for `3500 4500`,
//! RISC-V prints the reals' bit patterns as integers, and AOT-Rust fails: the dynamic tiers cannot see this.

use logicaffeine_compile::codegen_riscv::RISCV_RUNTIME_C;
use logicaffeine_compile::compile::{
    compile_to_c, compile_to_riscv, run_logos_source, tw_outcome, vm_outcome,
};
use std::sync::atomic::{AtomicUsize, Ordering};

const A_REAL: &str =
    "Let A be a 1 by 2 tensor of Real.\nSet item 1 of A to 1.5.\nSet item 2 of A to 2.5.\n";
const B_REAL: &str = "Let B be a 2 by 2 tensor of Real.\nSet item 1 of B to 1.5.\nSet item 2 of B to 0.5.\nSet item 3 of B to 0.5.\nSet item 4 of B to 1.5.\n";
const B_INT: &str = "Let B be a 2 by 2 tensor of Int.\nSet item 1 of B to 1.\nSet item 2 of B to 2.\nSet item 3 of B to 3.\nSet item 4 of B to 4.\n";
const P_REAL: &str =
    "Let P be a 1 by 2 tensor of Real.\nSet item 1 of P to 3.25.\nSet item 2 of P to 4.75.\n";

fn shown_scaled(t: &str, n: usize) -> String {
    (1..=n)
        .map(|i| format!("Show round(item {i} of {t} * 1000).\n"))
        .collect()
}

/// The programs, by name: each computes one tensor from Real (or mixed) operands and prints it.
fn programs() -> Vec<(&'static str, String)> {
    vec![
        ("real matmul", format!("## Main\n{A_REAL}{B_REAL}Let C be A matmul B.\n{}", shown_scaled("C", 2))),
        ("real plus", format!("## Main\n{A_REAL}{P_REAL}Let C be A plus P.\n{}", shown_scaled("C", 2))),
        ("real hadamard", format!("## Main\n{A_REAL}{P_REAL}Let C be A hadamard P.\n{}", shown_scaled("C", 2))),
        ("real matmul, then plus", format!("## Main\n{A_REAL}{B_REAL}Let C be A matmul B.\nLet D be C plus C.\n{}", shown_scaled("D", 2))),
        ("real matmul int", format!("## Main\n{A_REAL}{B_INT}Let C be A matmul B.\n{}", shown_scaled("C", 2))),
        ("real transpose", format!("## Main\n{B_REAL}Let T be transpose of B.\n{}", shown_scaled("T", 4))),
        ("int transpose", format!("## Main\n{B_INT}Let T be transpose of B.\nShow item 2 of T.\nShow item 2 of T + item 3 of T.\n")),
    ]
}

fn norm(s: &str) -> Vec<String> {
    s.split_whitespace().map(str::to_string).collect()
}

fn have(cmd: &str) -> bool {
    std::process::Command::new(cmd)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn vm(program: &str) -> Vec<String> {
    let out = vm_outcome(program);
    assert!(
        out.error.is_none(),
        "the VM runs it: {:?}\n{program}",
        out.error
    );
    norm(&out.output)
}

fn c_run(program: &str) -> String {
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let c = compile_to_c(program).expect("the sugar lowers to C");
    let dir = std::env::temp_dir().join(format!("logos_tsrs_c_{}_{n}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let (src, bin) = (dir.join("out.c"), dir.join("out"));
    std::fs::write(&src, &c).unwrap();
    let compiled = std::process::Command::new("gcc")
        .args(["-O2", "-o"])
        .arg(&bin)
        .arg(&src)
        .arg("-lm")
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "the emitted C compiles:\n{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let r = std::process::Command::new(&bin).output().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    String::from_utf8_lossy(&r.stdout).to_string()
}

fn riscv_run(program: &str) -> String {
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let asm = compile_to_riscv(program).expect("the sugar lowers to RISC-V");
    let dir = std::env::temp_dir().join(format!("logos_tsrs_rv_{}_{n}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let (a, rt, bin) = (dir.join("main.s"), dir.join("logos_rt.c"), dir.join("main"));
    std::fs::write(&a, &asm).unwrap();
    std::fs::write(&rt, RISCV_RUNTIME_C).unwrap();
    let c = std::process::Command::new("riscv64-linux-gnu-gcc")
        .args(["-O2", "-static", "-o"])
        .arg(&bin)
        .arg(&a)
        .arg(&rt)
        .arg("-lm")
        .output()
        .unwrap();
    assert!(
        c.status.success(),
        "the emitted RISC-V assembles:\n{}",
        String::from_utf8_lossy(&c.stderr)
    );
    let r = std::process::Command::new("qemu-riscv64")
        .arg(&bin)
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    String::from_utf8_lossy(&r.stdout).to_string()
}

/// ── THE ORACLE ITSELF, and the tree-walker beside it.
#[test]
fn the_vm_and_the_tree_walker_compute_in_the_result_kind() {
    let want: Vec<(&str, Vec<&str>)> = vec![
        ("real matmul", vec!["3500", "4500"]),
        ("real plus", vec!["4750", "7250"]),
        ("real hadamard", vec!["4875", "11875"]),
        ("real matmul, then plus", vec!["7000", "9000"]),
        ("real matmul int", vec!["9000", "13000"]),
        ("real transpose", vec!["1500", "500", "500", "1500"]),
        ("int transpose", vec!["3", "5"]),
    ];
    for ((name, program), (wname, w)) in programs().iter().zip(want) {
        assert_eq!(*name, wname);
        assert_eq!(vm(program), w, "{name}: the VM");
        assert_eq!(
            norm(&tw_outcome(program).output),
            w,
            "{name}: the tree-walker"
        );
    }
}

/// ── C STORES A REAL AS A REAL.
#[test]
fn the_c_backend_stores_each_computed_tensor_in_its_kind() {
    if !have("gcc") {
        eprintln!("SKIP C: no gcc.");
        return;
    }
    for (name, program) in programs() {
        assert_eq!(norm(&c_run(&program)), vm(&program), "{name}: C ≡ VM");
    }
}

/// ── AND SO DOES RISC-V.
#[test]
fn the_riscv_backend_stores_each_computed_tensor_in_its_kind() {
    if !(have("riscv64-linux-gnu-gcc") && have("qemu-riscv64")) {
        eprintln!("SKIP RISC-V: no cross toolchain.");
        return;
    }
    for (name, program) in programs() {
        assert_eq!(
            norm(&riscv_run(&program)),
            vm(&program),
            "{name}: RISC-V ≡ VM"
        );
    }
}

/// ── AND AOT-RUST — one real cargo build, so the programs run as one.
#[test]
fn the_aot_rust_backend_stores_each_computed_tensor_in_its_kind() {
    let progs = programs();
    let mut whole = String::from("## Main\n");
    let mut want = Vec::new();
    for (i, (_, program)) in progs.iter().enumerate() {
        let body = program.strip_prefix("## Main\n").expect("a Main program");
        // Each program's tensors are renamed apart so they share one Main.
        let renamed = ["A", "B", "P", "C", "D", "T"]
            .iter()
            .fold(body.to_string(), |s, t| {
                s.replace(&format!(" {t} "), &format!(" {t}{i} "))
                    .replace(&format!(" {t}.\n"), &format!(" {t}{i}.\n"))
                    .replace(&format!(" {t} *"), &format!(" {t}{i} *"))
            });
        whole.push_str(&renamed);
        want.extend(vm(program));
    }
    assert_eq!(
        vm(&whole),
        want,
        "the joined program is the programs, one after another"
    );
    match run_logos_source(&whole) {
        Ok(out) => assert_eq!(norm(&out), want, "AOT-Rust ≡ VM"),
        Err(e) => panic!("AOT-Rust builds and runs the programs: {e}"),
    }
}
