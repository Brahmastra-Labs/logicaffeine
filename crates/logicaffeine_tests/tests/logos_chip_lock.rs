//! logos_chip_lock — "send LOGOS code over the UART with a python script": the one-command FABRIC demo.
//!
//! `scripts/logos-chip.py run <target> program.lg` is the demo: the host compiles the LOGOS SOURCE on the PC into
//! the whole-program fabric engine's flat image, streams those bytes over the board's byte channel, reads the
//! chip's answer back, and compares it to the VM golden it derived from the same source. Two boards, one engine:
//!
//!   wpe      Board B  the whole-program fabric engine, flip-flop profile      .lg -> .wire via `emit_wpe_soc`
//!   wpe200   Board A  the same engine on the 200T, block-RAM profile          .lg -> .wire via `emit_wpe_soc`
//!
//! This lock pins the Python host to the Rust compiler at the one seam that matters: the bytes the host puts on
//! the wire MUST be the bytes `WImage::wire_bytes` produces for the lifter the benchmark lane would choose, and
//! the golden it prints MUST be the VM's answer. If the host drifted (a stale binary, a mis-joined argument list,
//! a different lifter), it would still "run" and print a plausible number — which is exactly how the DEPTH=4-vs-8
//! silent recursion corruption survived. The offline tests below need no hardware and always run; the physical
//! run is board-gated (`LOGOS_FPGA_BOARD=1`).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

mod common;

use logicaffeine_compile::codegen_sva::fabric_rtl::program_cfg_fabric;
use logicaffeine_compile::codegen_sva::wprog_engine_rtl::{
    eval_wprog_config, program_calls_fabric, program_main_fabric, serialize_program,
    wprog_config_fits_hw_engine, wprog_config_image_unchecked, WProgConfig,
};
use logicaffeine_compile::compile::vm_outcome;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("logos_chip_lock_{tag}_{}", std::process::id()));
    std::fs::create_dir_all(&d).expect("scratch dir");
    d
}

/// Build the compiler examples the host shells out to ONCE per test process, then hand the host `--no-build` so
/// ten invocations do not queue ten cargo lock acquisitions on a shared box (each is seconds when the tree is
/// fresh and minutes when another session holds the build lock).
fn compiler_examples_built() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let r = Command::new("cargo")
            .args(["build", "-q", "-p", "logicaffeine-compile"])
            .args([
                "--example",
                "emit_wpe_soc",
                "--example",
                "lgvm",
                "--example",
                "chip_host",
            ])
            .current_dir(root())
            .output()
            .expect("launch cargo build");
        assert!(
            r.status.success(),
            "building the compiler examples failed:\n{}",
            String::from_utf8_lossy(&r.stderr)
        );
    });
}

/// Run `scripts/logos-chip.py <args>` from the repo root and return its output. The subcommands that run a
/// compiler example (compile, run, kit, and selftest, which reads the protocol table from `chip_host`) get `--no-build`
/// after the one-time build above; the others take no such flag.
fn chip(args: &[&str]) -> Output {
    let mut argv: Vec<&str> = args.to_vec();
    if matches!(
        args.first(),
        Some(&"compile") | Some(&"run") | Some(&"selftest") | Some(&"kit")
    ) {
        compiler_examples_built();
        argv.push("--no-build");
    }
    Command::new("python3")
        .arg("scripts/logos-chip.py")
        .args(&argv)
        .current_dir(root())
        .output()
        .expect("launch scripts/logos-chip.py")
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

/// The `key=value` line the host prints for machine consumers (`golden=…`, `stream=…`, `target=…`).
fn field(out: &str, key: &str) -> Option<String> {
    out.lines()
        .find_map(|l| l.strip_prefix(&format!("{key}=")).map(str::to_string))
}

/// The VM golden for a function-form program: `f(args)` shown from a synthesized `## Main`.
fn vm_f(src: &str, func: &str, args: &[i64]) -> i64 {
    let al = args
        .iter()
        .map(|a| {
            if *a < 0 {
                format!("(0 - {})", -a)
            } else {
                a.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    let prog = format!("{src}## Main\n    Show {func}({al}).\n");
    let out = vm_outcome(&prog);
    assert!(
        out.error.is_none(),
        "VM error calling {func}: {:?}",
        out.error
    );
    out.output
        .split_whitespace()
        .next()
        .expect("a Show")
        .parse::<i64>()
        .expect("an Int")
}

/// The lifter choice the whole-program engine lane makes: the single-function CFG lifter when it agrees with
/// the VM on these arguments, else the multi-function (call-stack) lifter. The host must make the SAME choice.
fn engine_config(src: &str, func: &str, args: &[i64]) -> WProgConfig {
    let vm = vm_f(src, func, args);
    let cfg = std::panic::catch_unwind(|| serialize_program(&program_cfg_fabric(src, func))).ok();
    if let Some(c) = cfg {
        if eval_wprog_config(&c, args) == vm {
            return c;
        }
    }
    let calls = program_calls_fabric(src, func);
    assert_eq!(
        eval_wprog_config(&calls, args),
        vm,
        "neither lifter matches the VM for {func}({args:?})"
    );
    calls
}

#[test]
fn the_host_compiles_logos_source_for_the_fabric_engine_on_both_profiles() {
    let out_dir = scratch("wpe");
    // (program, function, args, expected) — loops, collections, arithmetic, and genuine recursion (the frame stack).
    let corpus: [(&str, &str, &[i64], i64); 5] = [
        ("benchmarks/silicon/programs/wpe_loop.lg", "f", &[5], 15),
        (
            "benchmarks/silicon/programs/wpe_collections.lg",
            "f",
            &[7],
            106,
        ),
        ("benchmarks/silicon/programs/wpe_incr.lg", "f", &[41], 42),
        ("benchmarks/silicon/programs/wpe_mul3.lg", "f", &[14], 42),
        ("benchmarks/silicon/programs/wpe_fib_rec.lg", "f", &[8], 21),
    ];
    for (lg, func, args, expected) in corpus {
        let src = std::fs::read_to_string(root().join(lg)).expect(lg);
        assert_eq!(
            vm_f(&src, func, args),
            expected,
            "{lg}: corpus expectation is not the VM's answer"
        );
        let want = wprog_config_image_unchecked(&engine_config(&src, func, args)).wire_bytes(args);
        let argcsv = args
            .iter()
            .map(|a| a.to_string())
            .collect::<Vec<_>>()
            .join(",");
        for target in ["wpe", "wpe200"] {
            let o = chip(&[
                "compile",
                target,
                lg,
                "--fn",
                func,
                "--args",
                &argcsv,
                "--out",
                out_dir.to_str().unwrap(),
            ]);
            assert!(
                o.status.success(),
                "compile {target} {lg} failed:\n{}\n{}",
                stdout(&o),
                stderr(&o)
            );
            let so = stdout(&o);
            assert_eq!(
                field(&so, "target").as_deref(),
                Some(target),
                "{lg}: target= line:\n{so}"
            );
            assert_eq!(
                field(&so, "golden"),
                Some(expected.to_string()),
                "{lg} on {target}: golden:\n{so}"
            );
            let stream =
                field(&so, "stream").unwrap_or_else(|| panic!("{lg}: no stream= line:\n{so}"));
            let got = std::fs::read(&stream).expect("the .wire the host named");
            assert_eq!(
                got, want,
                "{lg} on {target}: the wire bytes differ from WImage::wire_bytes"
            );
            assert_eq!(
                got.last(),
                Some(&0x5A),
                "{lg}: a .wire ends with the GO marker"
            );
        }
    }
}

/// THE demo form: a whole `## Main` program. The host must lower it whole, report the VM's OUTPUT (one line per
/// `Show`) as the golden, and put on the wire exactly the bytes `WImage::wire_bytes` builds for the Main lifter.
#[test]
fn the_host_compiles_a_main_program_and_reports_the_vms_output_lines() {
    let out_dir = scratch("main");
    let corpus = [
        "benchmarks/silicon/programs/hello_fabric.lg",
        "benchmarks/silicon/fpga/wire_fib.lg",
    ];
    for lg in corpus {
        let src = std::fs::read_to_string(root().join(lg)).expect(lg);
        let vm = vm_outcome(&src);
        assert!(vm.error.is_none(), "{lg}: VM error {:?}", vm.error);
        let lines: Vec<String> = vm.output.trim_end().lines().map(str::to_string).collect();
        assert!(!lines.is_empty(), "{lg}: a demo program shows something");
        let cfg = program_main_fabric(&src);
        let want = wprog_config_image_unchecked(&cfg).wire_bytes(&[]);
        for target in ["wpe", "wpe200"] {
            let o = chip(&["compile", target, lg, "--out", out_dir.to_str().unwrap()]);
            // The 100T flip-flop engine is small (8 blocks): a program it cannot hold must be REFUSED for `wpe`
            // with the cap named, never streamed; the 200T engine must take every corpus program.
            if target == "wpe" && wprog_config_fits_hw_engine(&cfg).is_err() {
                assert!(
                    !o.status.success(),
                    "{lg} exceeds the 100T caps but the host accepted it for wpe:\n{}",
                    stdout(&o)
                );
                assert!(
                    format!("{}{}", stdout(&o), stderr(&o)).contains("does not fit"),
                    "{lg}: the refusal says why"
                );
                continue;
            }
            assert!(
                o.status.success(),
                "compile {target} {lg} failed:\n{}\n{}",
                stdout(&o),
                stderr(&o)
            );
            let so = stdout(&o);
            assert_eq!(
                field(&so, "lifter").as_deref(),
                Some("main"),
                "{lg}: lowered whole, from Main:\n{so}"
            );
            let golden: Vec<String> = serde_json::from_str(
                &field(&so, "golden_json").unwrap_or_else(|| panic!("{lg}: no golden_json:\n{so}")),
            )
            .expect("golden_json is a JSON list of lines");
            assert_eq!(
                golden, lines,
                "{lg} on {target}: the host's golden lines are not the VM's output"
            );
            let stream =
                field(&so, "stream").unwrap_or_else(|| panic!("{lg}: no stream= line:\n{so}"));
            assert_eq!(
                std::fs::read(&stream).expect("wire"),
                want,
                "{lg} on {target}: wire bytes differ from WImage::wire_bytes"
            );
        }
    }
}

/// A program the flip-flop (100T) engine cannot hold but the block-RAM (200T) engine can. The host must REFUSE
/// it for `wpe` and accept it for `wpe200` — a `.wire` carries no record of the profile it was built for, and an
/// over-cap image loads fine and returns a plausible wrong number.
#[test]
fn the_host_refuses_a_program_that_does_not_fit_the_target_engine() {
    let out_dir = scratch("refuse");
    let mut src = String::from("## To f (x: Int) -> Int:\n");
    for k in 1..=24 {
        src.push_str(&format!("    Let a{k} be x + {k}.\n"));
    }
    src.push_str("    Return ");
    src.push_str(
        &(1..=24)
            .map(|k| format!("a{k}"))
            .collect::<Vec<_>>()
            .join(" + "),
    );
    src.push_str(".\n");
    let lg = out_dir.join("wide.lg");
    std::fs::write(&lg, &src).expect("write wide.lg");
    let expected = vm_f(&src, "f", &[1]);

    let o = chip(&[
        "compile",
        "wpe",
        lg.to_str().unwrap(),
        "--args",
        "1",
        "--out",
        out_dir.to_str().unwrap(),
    ]);
    assert!(
        !o.status.success(),
        "the flip-flop engine profile accepted a 24-register program:\n{}",
        stdout(&o)
    );
    let why = format!("{}{}", stdout(&o), stderr(&o));
    assert!(
        why.contains("does not fit"),
        "the refusal must say WHY, got:\n{why}"
    );
    assert!(
        field(&stdout(&o), "stream").is_none(),
        "a refused program must not leave a stream to send"
    );

    let o = chip(&[
        "compile",
        "wpe200",
        lg.to_str().unwrap(),
        "--args",
        "1",
        "--out",
        out_dir.to_str().unwrap(),
    ]);
    assert!(
        o.status.success(),
        "the block-RAM engine profile refused a program that fits it:\n{}\n{}",
        stdout(&o),
        stderr(&o)
    );
    assert_eq!(field(&stdout(&o), "golden"), Some(expected.to_string()));
}

/// Frame depth is data-dependent: no static profile check can see it, and the engine on the board has a fixed
/// DEPTH. The host must MEASURE it on the model with the exact arguments and refuse a program that would recurse
/// past the flashed image's caps — the `bench_fib 21 → 8` silent truncation, closed at the host.
#[test]
fn the_host_measures_recursion_depth_and_heap_against_the_flashed_caps() {
    let out_dir = scratch("caps");
    let lg = "benchmarks/silicon/programs/wpe_fib_rec.lg";
    // fib(20) needs 19 frames: refused under a DEPTH=8 image, accepted under DEPTH=32.
    let o = chip(&[
        "compile",
        "wpe200",
        lg,
        "--args",
        "20",
        "--caps",
        "DEPTH=8",
        "--out",
        out_dir.to_str().unwrap(),
    ]);
    assert!(
        !o.status.success(),
        "a 19-frame recursion was accepted for a DEPTH=8 image:\n{}",
        stdout(&o)
    );
    let why = format!("{}{}", stdout(&o), stderr(&o));
    assert!(
        why.contains("DEPTH needs 19") && why.contains("DEPTH=8"),
        "the refusal names the measured depth and the cap:\n{why}"
    );
    let o = chip(&[
        "compile",
        "wpe200",
        lg,
        "--args",
        "20",
        "--caps",
        "DEPTH=32,NREG=256",
        "--out",
        out_dir.to_str().unwrap(),
    ]);
    assert!(
        o.status.success(),
        "fib(20) fits a DEPTH=32 image:\n{}\n{}",
        stdout(&o),
        stderr(&o)
    );
    assert_eq!(field(&stdout(&o), "golden"), Some("6765".to_string()));
    // An image built without the float lane (recipe `FPU=0`) must refuse a float program rather than stream it.
    let o = chip(&[
        "compile",
        "wpe200",
        "benchmarks/silicon/programs/float_show.lg",
        "--caps",
        "FPU=0",
        "--out",
        out_dir.to_str().unwrap(),
    ]);
    assert!(
        !o.status.success(),
        "a float program was accepted for an FPU=0 image"
    );
    assert!(
        format!("{}{}", stdout(&o), stderr(&o)).contains("FPU=0"),
        "the refusal names the missing float lane"
    );
    let o = chip(&[
        "compile",
        "wpe200",
        "benchmarks/silicon/programs/float_show.lg",
        "--caps",
        "FPU=1",
        "--out",
        out_dir.to_str().unwrap(),
    ]);
    assert!(
        o.status.success(),
        "a float program fits an FPU=1 image:\n{}\n{}",
        stdout(&o),
        stderr(&o)
    );
    // The heap footprint is measured the same way: a 3-element list needs more than 2 words.
    let o = chip(&[
        "compile",
        "wpe200",
        "benchmarks/silicon/programs/wpe_collections.lg",
        "--args",
        "7",
        "--caps",
        "HEAP=2",
        "--out",
        out_dir.to_str().unwrap(),
    ]);
    assert!(
        !o.status.success(),
        "a heap-using program was accepted for a 2-word heap"
    );
    assert!(
        format!("{}{}", stdout(&o), stderr(&o)).contains("HEAP needs"),
        "the refusal names the heap footprint"
    );
}

/// The host's reply decoders, exercised on synthetic frames with no hardware: the engine's
/// `0xA5 result[63:0] cycles[31:0] 0x5A` frame (negative result, a leading power-on byte to skip), the frame-stack
/// overflow terminator `0xE1` (must be reported, never read as a result), and the soft-core's tagged multi-Show
/// frame via the decoder nexys-host.py pins against the RTL, and that decoder's own selftest, whose golden vectors
/// are the RTL's bytes.
#[test]
fn the_host_decoders_pass_their_selftest() {
    let o = chip(&["selftest"]);
    assert!(
        o.status.success(),
        "logos-chip.py selftest failed:\n{}\n{}",
        stdout(&o),
        stderr(&o)
    );
    assert!(stdout(&o).contains("selftest: OK"), "{}", stdout(&o));
    let core = Command::new("python3")
        .args(["scripts/nexys-host.py", "--selftest"])
        .current_dir(root())
        .output()
        .expect("launch scripts/nexys-host.py");
    assert!(
        core.status.success() && stdout(&core).contains("selftest OK"),
        "the soft-core codec no longer reads the RTL's frame, byte for byte:\n{}\n{}",
        stdout(&core),
        stderr(&core)
    );
}

/// The physical proof, reproducible: LOGOS SOURCE streamed to both boards by the python host, answers ≡ VM.
///
///   LOGOS_FPGA_BOARD=1 cargo test -p logicaffeine-tests --test logos_chip_lock -- --nocapture
#[test]
fn logos_source_runs_on_both_boards_over_the_wire_matching_the_vm() {
    if std::env::var("LOGOS_FPGA_BOARD").as_deref() != Ok("1") {
        eprintln!(
            "LOGOS_FPGA_BOARD != 1 — skipping the physical two-board proof. Reproduce with: \
             LOGOS_FPGA_BOARD=1 cargo test -p logicaffeine-tests --test logos_chip_lock"
        );
        return;
    }
    // Each row: target, program, extra args. `--flash` re-programs the board first so every run starts from the
    // same image (the engines loop for the next program, but a fresh load is the reproducible starting state).
    let runs: [(&str, &str, &[&str]); 6] = [
        ("wpe", "benchmarks/silicon/programs/hello_fabric.lg", &[]),
        (
            "wpe",
            "benchmarks/silicon/programs/wpe_loop.lg",
            &["--args", "5"],
        ),
        (
            "wpe",
            "benchmarks/silicon/programs/wpe_fib_rec.lg",
            &["--args", "8"],
        ),
        ("wpe200", "benchmarks/silicon/programs/hello_fabric.lg", &[]),
        ("wpe200", "benchmarks/silicon/fpga/wire_fib.lg", &[]),
        (
            "wpe200",
            "benchmarks/silicon/programs/wpe_fib_rec.lg",
            &["--args", "8"],
        ),
    ];
    let mut failures = Vec::new();
    for (target, lg, extra) in runs {
        let mut args = vec!["run", target, lg, "--flash"];
        args.extend_from_slice(extra);
        let o = chip(&args);
        let so = stdout(&o);
        eprintln!("{so}");
        if !o.status.success() || !so.contains("SILICON == VM") {
            failures.push(format!("{target} {lg}:\n{so}\n{}", stderr(&o)));
        }
    }
    assert!(
        failures.is_empty(),
        "silicon disagreed with the VM:\n{}",
        failures.join("\n")
    );
}

const BOARD_A: &str = "210276C08FE0";
const BOARD_B: &str = "210292BFA310";

/// Ask `scripts/lib/ftdi_ports.py` which device in `listing` carries (FTDI serial, USB interface). A listing row
/// is what `serial.tools.list_ports` reports for one device: (device, serial number, location).
fn resolve(listing: &[(&str, &str, &str)], serial: &str, interface: u8) -> Option<String> {
    let rows: Vec<[&str; 3]> = listing.iter().map(|(d, s, l)| [*d, *s, *l]).collect();
    let o = Command::new("python3")
        .args(["scripts/lib/ftdi_ports.py", "resolve", "--listing"])
        .arg(serde_json::to_string(&rows).expect("listing json"))
        .args([serial, &interface.to_string()])
        .current_dir(root())
        .output()
        .expect("launch scripts/lib/ftdi_ports.py");
    let out = stdout(&o).trim().to_string();
    match o.status.code() {
        Some(0) => Some(out),
        Some(1) => {
            assert!(
                out.is_empty(),
                "an absent channel prints no device, got {out:?}"
            );
            None
        }
        _ => panic!(
            "ftdi_ports.py resolve failed:\n{}\n{}",
            stdout(&o),
            stderr(&o)
        ),
    }
}

/// A board's FT2232 is two serial devices, one per USB interface, and the device NAMES are not stable: Linux
/// numbers `/dev/ttyUSBn` in enumeration order (this box's two boards have held 0/1 and 2/3 both ways round),
/// macOS names a node after the serial plus a per-driver suffix. So every host finds a channel by
/// (FTDI serial, interface), read from what the OS reports — on each OS the demo runs from.
#[test]
fn the_port_resolver_names_each_board_channel_on_linux_and_macos() {
    // Linux, exactly as pyserial lists this box: the location is `<bus>-<port path>:<config>.<interface>`.
    let linux = [
        ("/dev/ttyS0", "", ""),
        ("/dev/ttyUSB3", BOARD_A, "1-5.2:1.1"),
        ("/dev/ttyUSB2", BOARD_A, "1-5.2:1.0"),
        ("/dev/ttyUSB1", BOARD_B, "1-5.1:1.1"),
        ("/dev/ttyUSB0", BOARD_B, "1-5.1:1.0"),
    ];
    // macOS, Apple's driver: a callout (`cu.`) and a dial-in (`tty.`) node per channel, named after the serial
    // plus the interface digit; the location is an IORegistry path that names no interface.
    let mac_apple = [
        ("/dev/cu.Bluetooth-Incoming-Port", "", ""),
        ("/dev/tty.usbserial-210292BFA3101", BOARD_B, "0-1.1"),
        ("/dev/tty.usbserial-210292BFA3100", BOARD_B, "0-1.1"),
        ("/dev/cu.usbserial-210292BFA3100", BOARD_B, "0-1.1"),
        ("/dev/cu.usbserial-210292BFA3101", BOARD_B, "0-1.1"),
        ("/dev/tty.usbserial-210276C08FE01", BOARD_A, "0-1.2"),
        ("/dev/cu.usbserial-210276C08FE00", BOARD_A, "0-1.2"),
        ("/dev/cu.usbserial-210276C08FE01", BOARD_A, "0-1.2"),
    ];
    // macOS, FTDI's own VCP driver: the channel letter in place of the digit, and no serial reported.
    let mac_ftdi = [
        ("/dev/cu.usbserial-210276C08FE0B", "", ""),
        ("/dev/cu.usbserial-210276C08FE0A", "", ""),
        ("/dev/cu.usbserial-210292BFA310B", "", ""),
        ("/dev/cu.usbserial-210292BFA310A", "", ""),
    ];
    let cases: [(&str, &[(&str, &str, &str)], [&str; 4]); 3] = [
        (
            "linux",
            &linux,
            [
                "/dev/ttyUSB2",
                "/dev/ttyUSB3",
                "/dev/ttyUSB0",
                "/dev/ttyUSB1",
            ],
        ),
        (
            "macos apple",
            &mac_apple,
            [
                "/dev/cu.usbserial-210276C08FE00",
                "/dev/cu.usbserial-210276C08FE01",
                "/dev/cu.usbserial-210292BFA3100",
                "/dev/cu.usbserial-210292BFA3101",
            ],
        ),
        (
            "macos ftdi",
            &mac_ftdi,
            [
                "/dev/cu.usbserial-210276C08FE0A",
                "/dev/cu.usbserial-210276C08FE0B",
                "/dev/cu.usbserial-210292BFA310A",
                "/dev/cu.usbserial-210292BFA310B",
            ],
        ),
    ];
    for (os, listing, [a0, a1, b0, b1]) in cases {
        for (serial, interface, want) in [
            (BOARD_A, 0, a0),
            (BOARD_A, 1, a1),
            (BOARD_B, 0, b0),
            (BOARD_B, 1, b1),
        ] {
            assert_eq!(
                resolve(listing, serial, interface).as_deref(),
                Some(want),
                "{os}: board {serial} interface {interface}"
            );
        }
        assert_eq!(
            resolve(listing, "210000000000", 1),
            None,
            "{os}: an unplugged board resolves to no device"
        );
    }
    // One board plugged in: the other's channels are absent, never borrowed from the board that is there.
    assert_eq!(resolve(&linux[3..], BOARD_A, 1), None);
    assert_eq!(
        resolve(&linux[3..], BOARD_B, 1).as_deref(),
        Some("/dev/ttyUSB1")
    );
}

/// The loader command a host runs, per OS: Linux through `sudo -n` (the JTAG node is root-owned on this box),
/// macOS as the user (a sudo there stops to ask for a password), `LOGOS_CHIP_SUDO=1`/`0` forcing either way.
#[test]
fn the_loader_runs_under_sudo_only_where_the_os_needs_it() {
    let argv = |platform: &str, sudo: Option<&str>| -> Vec<String> {
        let mut c = Command::new("python3");
        c.args([
            "scripts/lib/ftdi_ports.py",
            "loader",
            "--platform",
            platform,
            "--cable",
            "digilent",
            "--serial",
            BOARD_B,
        ])
        .args(["--", "--freq", "1000000", "x.bit"])
        .current_dir(root())
        .env_remove("LOGOS_CHIP_SUDO");
        if let Some(v) = sudo {
            c.env("LOGOS_CHIP_SUDO", v);
        }
        let o = c.output().expect("launch scripts/lib/ftdi_ports.py");
        assert!(
            o.status.success(),
            "loader {platform}:\n{}\n{}",
            stdout(&o),
            stderr(&o)
        );
        serde_json::from_str(stdout(&o).trim()).expect("the loader argv is a JSON list")
    };
    let tail = [
        "-c",
        "digilent",
        "--ftdi-serial",
        BOARD_B,
        "--freq",
        "1000000",
        "x.bit",
    ];
    let check = |got: Vec<String>, prefix: &[&str], what: &str| {
        assert_eq!(&got[..prefix.len()], prefix, "{what}: {got:?}");
        assert!(
            got[prefix.len()].ends_with("openFPGALoader"),
            "{what}: the loader follows the prefix: {got:?}"
        );
        assert_eq!(&got[prefix.len() + 1..], &tail, "{what}: {got:?}");
    };
    check(argv("linux", None), &["sudo", "-n"], "linux");
    check(argv("darwin", None), &[], "macos");
    check(
        argv("darwin", Some("1")),
        &["sudo"],
        "macos, LOGOS_CHIP_SUDO=1",
    );
    check(argv("linux", Some("0")), &[], "linux, LOGOS_CHIP_SUDO=0");
}

/// On macOS only root may detach Apple's FTDI driver from a channel, so when libftdi reports the claim refused the
/// loader runs once more under sudo; a failure of any other kind is the loader's own and is not retried. Driven by
/// a stand-in loader that refuses the claim unless sudo ran it, and a stand-in sudo that records each run.
#[test]
fn a_claim_refused_on_macos_is_retried_under_sudo_and_nothing_else_is() {
    let bin = scratch("fakeloader");
    let log = bin.join("sudo.log");
    let loader = bin.join("openFPGALoader");
    std::fs::write(
        &loader,
        "#!/bin/sh\n\
         if [ \"$FAKE_FAILURE\" = other ]; then echo 'error: x.bit: no such file'; exit 1; fi\n\
         if [ -n \"$UNDER_SUDO\" ]; then echo \"loaded $*\"; exit 0; fi\n\
         echo 'unable to open ftdi device: -5 (unable to claim usb device. Make sure the default FTDI driver is not in use)'\n\
         exit 1\n",
    )
    .expect("write the stand-in loader");
    std::fs::write(
        bin.join("sudo"),
        format!(
            "#!/bin/sh\necho \"sudo $*\" >> '{}'\nUNDER_SUDO=1 exec \"$@\"\n",
            log.display()
        ),
    )
    .expect("write the stand-in sudo");
    for f in [loader.clone(), bin.join("sudo")] {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    let run = |failure: &str| -> (i64, String, String) {
        let _ = std::fs::remove_file(&log);
        let o = Command::new("/usr/bin/python3")
            .args([
                "-c",
                "import json, sys; sys.path.insert(0, 'scripts/lib'); import ftdi_ports as f; \
                 print(json.dumps(f.run_loader('digilent', sys.argv[1], ['x.bit'], platform='darwin')))",
                BOARD_B,
            ])
            .current_dir(root())
            .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
            .env("FAKE_FAILURE", failure)
            .env_remove("LOGOS_CHIP_SUDO")
            .output()
            .expect("launch python3");
        assert!(o.status.success(), "run_loader raised:\n{}", stderr(&o));
        let (code, out): (i64, String) =
            serde_json::from_str(stdout(&o).trim()).expect("(status, output)");
        (code, out, std::fs::read_to_string(&log).unwrap_or_default())
    };
    let (code, out, sudo_runs) = run("claim");
    assert_eq!(code, 0, "the retry under sudo succeeds: {out}");
    assert!(
        out.contains(&format!("loaded -c digilent --ftdi-serial {BOARD_B} x.bit")),
        "{out}"
    );
    assert_eq!(
        sudo_runs.lines().count(),
        1,
        "exactly one retry, under sudo: {sudo_runs:?}"
    );
    assert!(
        sudo_runs.contains(&loader.display().to_string()),
        "sudo runs the loader by its full path: {sudo_runs:?}"
    );
    let (code, out, sudo_runs) = run("other");
    assert_eq!(
        code, 1,
        "a failure that is not a refused claim stands: {out}"
    );
    assert!(
        sudo_runs.is_empty(),
        "no retry for a failure of the loader's own: {sudo_runs:?}"
    );
}

/// Writing a board's configuration flash goes through the loader's SPI-over-JTAG bridge, which it picks by the FPGA's
/// device and package; each board names its own. A write the loader did not perform is a failure even when the
/// loader exits 0, as it does when it cannot choose a bridge. Driven by a stand-in loader that records its
/// arguments and then answers as the real one did on 2026-10-03.
#[test]
fn persisting_an_image_names_the_boards_part_and_reports_a_write_that_did_not_happen() {
    let bin = scratch("persist");
    let args_log = bin.join("args.log");
    let loader = bin.join("openFPGALoader");
    std::fs::write(
        &loader,
        format!(
            "#!/bin/sh\necho \"$*\" > '{}'\n\
             if [ \"$FAKE_ANSWER\" = refuse ]; then echo \"Can't program SPI flash: missing device-package information\"; exit 0; fi\n\
             echo 'Verifying write (May take time)'; echo 'Done'\n",
            args_log.display()
        ),
    )
    .expect("write the stand-in loader");
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&loader, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    let image = bin.join("x.bit");
    std::fs::write(&image, b"bitstream").expect("write a stand-in image");
    let persist = |board: &str, answer: &str| -> (bool, String) {
        let o = Command::new("/usr/bin/python3")
            .args([
                "scripts/lib/ftdi_ports.py",
                "flash",
                board,
                image.to_str().unwrap(),
                "--persist",
            ])
            .current_dir(root())
            .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
            .env("LOGOS_CHIP_SUDO", "0")
            .env("FAKE_ANSWER", answer)
            .output()
            .expect("launch ftdi_ports.py");
        (
            o.status.success(),
            std::fs::read_to_string(&args_log).unwrap_or_default(),
        )
    };
    for (board, serial, cable, part) in [
        ("A", BOARD_A, "digilent_b", "xc7a200tsbg484"),
        ("B", BOARD_B, "digilent", "xc7a100tcsg324"),
    ] {
        let (ok, args) = persist(board, "write");
        assert!(ok, "board {board}: a performed write succeeds");
        let want = format!("-c {cable} --ftdi-serial {serial} --freq 1000000 --write-flash --verify --fpga-part {part} {}", image.display());
        assert_eq!(args.trim(), want, "board {board}: the loader's arguments");
    }
    let (ok, _) = persist("B", "refuse");
    assert!(
        !ok,
        "a write the loader did not perform is reported as a failure, though the loader exits 0"
    );
}

/// The bytes the chip protocol's one definition prints, through the `chip_host` example.
fn chip_host_table() -> Vec<u8> {
    compiler_examples_built();
    let o = Command::new("cargo")
        .args([
            "run",
            "-q",
            "-p",
            "logicaffeine-compile",
            "--example",
            "chip_host",
            "--",
            "--table",
        ])
        .current_dir(root())
        .output()
        .expect("launch chip_host");
    assert!(
        o.status.success(),
        "chip_host --table failed:\n{}",
        stderr(&o)
    );
    o.stdout
}

/// Run a kit's own `scripts/logos-chip.py` the way a laptop with no Rust toolchain would: from the kit's root,
/// with an environment holding no cargo and an empty target directory.
fn kit_host(kit: &Path, args: &[&str]) -> Output {
    let empty_target = kit.with_extension("no-target");
    std::fs::create_dir_all(&empty_target).expect("empty target dir");
    Command::new("/usr/bin/python3")
        .arg("scripts/logos-chip.py")
        .args(args)
        .current_dir(kit)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", std::env::var("HOME").unwrap_or_default())
        .env("CARGO_TARGET_DIR", &empty_target)
        .output()
        .expect("launch the kit's logos-chip.py")
}

/// The `[target][…]` entry `kit.json` records for one exported program.
fn kit_entry(kit: &Path, target: &str, source: &str) -> serde_json::Value {
    let doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(kit.join("kit.json")).expect("kit.json"))
            .expect("kit.json parses");
    doc["targets"][target]["programs"]
        .as_array()
        .unwrap_or_else(|| panic!("kit.json has no programs for {target}: {doc}"))
        .iter()
        .find(|p| p["source"] == source)
        .unwrap_or_else(|| panic!("kit.json has no entry for {source} on {target}: {doc}"))
        .clone()
}

/// A board kit is what a laptop with only Python, pyserial and openFPGALoader needs to drive the boards: a sparse
/// copy of the repo holding the host scripts, the target's image and build recipe, every exported program
/// precompiled with its VM golden, and the protocol table exactly as `chip_host --table` printed it. Its host
/// runs with no cargo anywhere — one that still reached for the compiler would work on this box and fail on
/// the laptop — and it refuses anything the kit does not carry instead of guessing.
#[test]
fn a_board_kit_runs_with_no_compiler() {
    if !common::require_tool("fpga-images") {
        return;
    }
    let base = scratch("kit");
    let kit = base.join("kit");
    let _ = std::fs::remove_dir_all(&kit);
    let hello = "benchmarks/silicon/programs/hello_fabric.lg";
    let fib = "benchmarks/silicon/programs/wpe_fib_rec.lg";
    let o = chip(&[
        "kit",
        kit.to_str().unwrap(),
        "wpe",
        hello,
        &format!("{fib}@8"),
    ]);
    assert!(
        o.status.success(),
        "kit export failed:\n{}\n{}",
        stdout(&o),
        stderr(&o)
    );

    assert_eq!(
        std::fs::read(kit.join("kit/protocol.json")).expect("kit/protocol.json"),
        chip_host_table(),
        "the kit's protocol table is not chip_host's, byte for byte"
    );
    for f in [
        "scripts/logos-chip.py",
        "scripts/nexys-host.py",
        "scripts/lib/ftdi_ports.py",
        "scripts/run-tformer-200t.py",
        "scripts/matmul-host.py",
        "benchmarks/silicon/wpe_soc.bit",
        "benchmarks/silicon/wpe_soc.bit.build.txt",
    ] {
        assert_eq!(
            std::fs::read(kit.join(f)).unwrap_or_else(|_| panic!("the kit lacks {f}")),
            std::fs::read(root().join(f)).expect(f),
            "{f} in the kit is not the repo's"
        );
    }

    let src = std::fs::read_to_string(root().join(hello)).expect(hello);
    let vm = vm_outcome(&src);
    let lines: Vec<String> = vm.output.trim_end().lines().map(str::to_string).collect();
    let e = kit_entry(&kit, "wpe", hello);
    let golden: Vec<String> =
        serde_json::from_value(e["golden"].clone()).expect("a Main golden is a list of lines");
    assert_eq!(
        golden, lines,
        "the kit's golden for {hello} is not the VM's output"
    );
    let stream = e["stream"].as_str().expect("stream path");
    assert_eq!(
        std::fs::read(kit.join(stream)).expect("hello stream"),
        wprog_config_image_unchecked(&program_main_fabric(&src)).wire_bytes(&[]),
        "the kit's {hello} stream is not WImage::wire_bytes"
    );
    let fsrc = std::fs::read_to_string(root().join(fib)).expect(fib);
    let e = kit_entry(&kit, "wpe", fib);
    assert_eq!(e["args"], "8");
    assert_eq!(e["golden"], "21", "fib(8) is 21 on the VM");
    assert_eq!(
        std::fs::read(kit.join(e["stream"].as_str().unwrap())).expect("fib stream"),
        wprog_config_image_unchecked(&engine_config(&fsrc, "f", &[8])).wire_bytes(&[8]),
        "the kit's fib(8) stream is not the bytes the lane's lifter produces"
    );

    let cargo_visible = Command::new("/bin/sh")
        .args(["-c", "command -v cargo"])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("sh");
    assert!(
        !cargo_visible.status.success(),
        "precondition: cargo must not be on the kit host's PATH"
    );

    let o = kit_host(&kit, &["selftest"]);
    assert!(
        o.status.success() && stdout(&o).contains("selftest: OK"),
        "kit selftest:\n{}\n{}",
        stdout(&o),
        stderr(&o)
    );

    let o = kit_host(&kit, &["run", "wpe", hello, "--dry-run"]);
    assert!(
        o.status.success(),
        "kit run {hello}:\n{}\n{}",
        stdout(&o),
        stderr(&o)
    );
    let so = stdout(&o);
    assert_eq!(field(&so, "stream").as_deref(), Some(stream), "{so}");
    let dry: Vec<String> =
        serde_json::from_str(&field(&so, "golden_json").expect("golden_json")).expect("lines");
    assert_eq!(dry, lines, "{so}");
    assert!(
        !so.contains("SILICON"),
        "a dry run claims nothing about silicon:\n{so}"
    );

    let o = kit_host(&kit, &["run", "wpe", fib, "--args", "8", "--dry-run"]);
    assert!(
        o.status.success(),
        "kit run fib(8):\n{}\n{}",
        stdout(&o),
        stderr(&o)
    );
    assert_eq!(field(&stdout(&o), "golden").as_deref(), Some("21"));

    let o = kit_host(&kit, &["run", "wpe", fib, "--args", "9", "--dry-run"]);
    assert!(
        !o.status.success(),
        "fib(9) is not in the kit, yet the kit host ran it:\n{}",
        stdout(&o)
    );
    assert!(
        format!("{}{}", stdout(&o), stderr(&o)).contains("not in this kit"),
        "the refusal says why"
    );

    let o = kit_host(&kit, &["compile", "wpe", hello]);
    assert!(
        !o.status.success(),
        "a kit host has no compiler, yet compile succeeded:\n{}",
        stdout(&o)
    );
    assert!(
        format!("{}{}", stdout(&o), stderr(&o)).contains("board kit"),
        "the refusal names the kit"
    );
}

/// The physical proof that a kit drives the boards: exported here, run with no compiler, answers ≡ VM.
///
///   LOGOS_FPGA_BOARD=1 cargo test -p logicaffeine-tests --test logos_chip_lock -- --nocapture
#[test]
fn a_board_kit_drives_both_boards_with_no_compiler() {
    if std::env::var("LOGOS_FPGA_BOARD").as_deref() != Ok("1") {
        eprintln!("LOGOS_FPGA_BOARD != 1 — skipping the physical kit run.");
        return;
    }
    let kit = scratch("kitboards").join("kit");
    let _ = std::fs::remove_dir_all(&kit);
    let hello = "benchmarks/silicon/programs/hello_fabric.lg";
    let fib = "benchmarks/silicon/programs/wpe_fib_rec.lg";
    let lp = "benchmarks/silicon/programs/wpe_loop.lg";
    let o = chip(&[
        "kit",
        kit.to_str().unwrap(),
        "wpe",
        hello,
        &format!("{fib}@8"),
    ]);
    assert!(
        o.status.success(),
        "kit export wpe:\n{}\n{}",
        stdout(&o),
        stderr(&o)
    );
    let o = chip(&[
        "kit",
        kit.to_str().unwrap(),
        "wpe200old",
        &format!("{lp}@100"),
    ]);
    assert!(
        o.status.success(),
        "kit export wpe200old:\n{}\n{}",
        stdout(&o),
        stderr(&o)
    );
    let runs: [&[&str]; 3] = [
        &["run", "wpe", hello, "--flash"],
        &["run", "wpe", fib, "--args", "8"],
        &["run", "wpe200old", lp, "--args", "100", "--flash"],
    ];
    let mut failures = Vec::new();
    for args in runs {
        let o = kit_host(&kit, args);
        let so = stdout(&o);
        eprintln!("{so}");
        if !o.status.success() || !so.contains("SILICON == VM") {
            failures.push(format!("{args:?}:\n{so}\n{}", stderr(&o)));
        }
    }
    assert!(
        failures.is_empty(),
        "the kit's runs disagreed with the VM:\n{}",
        failures.join("\n")
    );
}
