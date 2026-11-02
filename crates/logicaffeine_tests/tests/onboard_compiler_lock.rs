//! The compiler the board runs is the compiler the laptop runs. The board's own CPU executes
//! `riscv64gc-unknown-linux-musl` programs, so the `onboard_compile` example — `codegen_sva::onboard`, the unchanged
//! parser, bytecode compiler and fabric lifters — is built for that target and run under `qemu-riscv64`. For every
//! program it is given, its report (the lifter, the VM's golden, the register bytecode and the fabric wire image, or
//! the sentence it refuses with) must be byte-identical to the report this x86 process computes, and every image it
//! emits must equal the one built here from the primitives themselves: `Compiler::compile` → `binisa`, and
//! `engine_build` → `wire_bytes`.
//!
//! Tool-gated: the rustup target, `riscv64-linux-gnu-gcc` and `qemu-riscv64`.

mod common;
#[path = "support/fabric_program_gen.rs"]
mod fabric_program_gen;

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use logicaffeine_compile::codegen_sva::engine_build::{lower, lower_main, Profile};
use logicaffeine_compile::codegen_sva::onboard::{self, Run};
use logicaffeine_compile::codegen_sva::wprog_engine_rtl::wprog_config_image_unchecked;
use logicaffeine_compile::vm::{binisa, Compiler};

/// The address space one compile on the hart gets, in megabytes; qemu-user takes about 256 MB of it for itself.
const RUN_CAP_MB: u64 = 2048;

/// How long one compile on the hart may take, in seconds, before it is stopped and fails.
const RUN_SECONDS: u64 = 300;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn programs() -> PathBuf {
    root().join("benchmarks/silicon/programs")
}

fn tools() -> bool {
    common::require_tool("riscv64-musl")
        && common::require_tool("riscv64-linux-gnu-gcc")
        && common::require_tool("qemu-riscv64")
}

/// The `onboard_compile` example built for the hart, once per test binary.
fn hart_compiler() -> &'static Path {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(|| {
        let out = std::process::Command::new("bash")
            .arg(root().join("scripts/hw/build-onboard-firmware.sh"))
            .arg("onboard_compile")
            .output()
            .expect("run scripts/hw/build-onboard-firmware.sh");
        assert!(
            out.status.success(),
            "the onboard_compile example did not build for riscv64gc-unknown-linux-musl:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let path = String::from_utf8_lossy(&out.stdout)
            .lines()
            .last()
            .map(PathBuf::from)
            .expect("the build script names the program on its last line");
        assert!(path.is_file(), "the build script named {} but nothing is there", path.display());
        path
    })
}

/// Compile `program` on the hart (under `qemu-riscv64`) and return its report.
fn on_hart(program: &Path, run: &Run, caps: &str) -> String {
    let mut args: Vec<String> = vec!["--program".into(), program.display().to_string(), "--caps".into(), caps.into()];
    match run {
        Run::Main => args.push("--main".into()),
        Run::Call { func, args: a } => {
            args.extend(["--fn".into(), func.clone()]);
            args.extend([
                "--args".into(),
                a.iter().map(i64::to_string).collect::<Vec<_>>().join(","),
            ]);
        }
    }
    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg("ulimit -v \"$1\" && t=\"$2\" && shift 2 && exec timeout \"$t\" qemu-riscv64 \"$@\"")
        .arg("sh")
        .arg((RUN_CAP_MB * 1024).to_string())
        .arg(RUN_SECONDS.to_string())
        .arg(hart_compiler())
        .args(&args)
        .output()
        .expect("run qemu-riscv64");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        matches!(out.status.code(), Some(0) | Some(1)) && !stdout.is_empty(),
        "{} on the hart did not report (exit {:?}):\n{}",
        program.display(),
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    stdout
}

/// The report fields of a successful compile, read back from the text: (lifter, golden line, bytecode, wire).
fn fields(report: &str) -> Option<(String, String, Vec<u8>, Vec<u8>)> {
    let get = |key: &str| {
        report
            .lines()
            .find_map(|l| l.strip_prefix(&format!("{key}=")))
            .map(str::to_string)
    };
    let unhex = |s: String| -> Vec<u8> {
        assert!(s.len() % 2 == 0, "odd hex length in {s:?}");
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex"))
            .collect()
    };
    let lifter = get("lifter")?;
    let golden = get("golden").or_else(|| get("golden_json"))?;
    Some((lifter, golden, unhex(get("bytecode")?), unhex(get("wire")?)))
}

/// The report's facts for `run`, built here from the primitives and not through `onboard`: the lifter's name, the
/// golden line, the bytecode and the wire image.
fn from_primitives(src: &str, run: &Run) -> (String, String, Vec<u8>, Vec<u8>) {
    let bytecode = logicaffeine_compile::ui_bridge::with_parsed_program(src, |parsed, interner| {
        let (stmts, _, _) = parsed.expect("the program parses");
        let program = Compiler::compile(stmts, interner).expect("the program compiles");
        binisa::encode_program(&program, interner)
    });
    let (cfg, lifter, golden, params) = match run {
        Run::Main => {
            let (cfg, lines) = lower_main(src).expect("the program lowers");
            let golden = serde_json::to_string(&lines).expect("lines serialize");
            (cfg, "main".to_string(), golden, Vec::new())
        }
        Run::Call { func, args } => {
            let (cfg, lifter, answer) = lower(src, func, args).expect("the function lowers");
            (cfg, lifter.name().to_string(), answer.to_string(), args.clone())
        }
    };
    (lifter, golden, bytecode, wprog_config_image_unchecked(&cfg).wire_bytes(&params))
}

/// Compile `program` both ways and hold the hart to the host: the reports equal, and a successful one's images
/// equal the primitives'. Returns whether it compiled.
fn agree(program: &Path, run: &Run, caps: &str) -> bool {
    let src = std::fs::read_to_string(program).unwrap_or_else(|e| panic!("{}: {e}", program.display()));
    let host = onboard::report(&onboard::compile(&src, run, Profile::Hw, caps));
    let hart = on_hart(program, run, caps);
    assert_eq!(
        hart,
        host,
        "{} {run:?}: the hart's compile differs from the host's",
        program.display()
    );
    match fields(&hart) {
        Some((lifter, golden, bytecode, wire)) => {
            let (want_lifter, want_golden, want_bytecode, want_wire) = from_primitives(&src, run);
            assert_eq!(lifter, want_lifter, "{} {run:?}: the lifter is not engine_build's", program.display());
            assert_eq!(golden, want_golden, "{} {run:?}: the golden is not the VM's", program.display());
            assert_eq!(bytecode, want_bytecode, "{} {run:?}: bytecode is not Compiler::compile's", program.display());
            assert_eq!(wire, want_wire, "{} {run:?}: wire is not engine_build's", program.display());
            true
        }
        None => {
            assert!(hart.starts_with("error="), "{}: neither a compile nor a refusal:\n{hart}", program.display());
            false
        }
    }
}

/// `benchmarks/silicon/programs/terminal_library.txt`: (title, program file, function, arguments) per entry.
fn library() -> Vec<(String, PathBuf, Run)> {
    let text = std::fs::read_to_string(programs().join("terminal_library.txt")).expect("terminal_library.txt");
    text.lines()
        .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
        .map(|l| {
            let f: Vec<&str> = l.split('|').map(str::trim).collect();
            assert_eq!(f.len(), 4, "library line {l:?}");
            let run = if f[2].is_empty() {
                Run::Main
            } else {
                Run::Call {
                    func: f[2].to_string(),
                    args: f[3]
                        .split(',')
                        .filter(|a| !a.trim().is_empty())
                        .map(|a| a.trim().parse().expect("argument"))
                        .collect(),
                }
            };
            (f[0].to_string(), programs().join(f[1]), run)
        })
        .collect()
}

#[test]
fn every_library_program_compiles_on_the_hart_to_the_hosts_bytes() {
    if !tools() {
        return;
    }
    let entries = library();
    assert!(entries.len() >= 10, "the library lists {} programs", entries.len());
    for (title, program, run) in &entries {
        assert!(
            agree(program, run, &onboard::hw_caps()),
            "library entry {title:?} must compile for Board B, on the hart as on the host"
        );
    }
}

#[test]
fn every_silicon_demo_program_compiles_on_the_hart_as_on_the_host() {
    if !tools() {
        return;
    }
    let mut paths: Vec<PathBuf> = std::fs::read_dir(programs())
        .expect("programs dir")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "lg"))
        .collect();
    paths.sort();
    let mut compiled = 0;
    for p in &paths {
        let src = std::fs::read_to_string(p).unwrap();
        if src.contains("## Main") && agree(p, &Run::Main, &onboard::hw_caps()) {
            compiled += 1;
        }
    }
    assert!(compiled >= 8, "only {compiled} Main programs compiled for Board B; the corpus has drifted off the engine");
}

#[test]
fn generated_programs_compile_on_the_hart_to_the_hosts_bytes() {
    if !tools() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("onboard_compiler_lock_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut rng = fabric_program_gen::Rng::new(0x0b0a_7d00);
    let mut compiled = 0;
    for n in 0..24 {
        let src = if n % 4 == 3 {
            fabric_program_gen::gen_recursive_2arg(&mut rng)
        } else {
            fabric_program_gen::gen(&mut rng).0
        };
        let args = if src.contains("(a: Int, b: Int)") { vec![4, 9] } else { vec![5] };
        let path = dir.join(format!("gen{n}.lg"));
        std::fs::write(&path, &src).unwrap();
        let run = Run::Call {
            func: "f".into(),
            args,
        };
        if agree(&path, &run, &onboard::hw_caps()) {
            compiled += 1;
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(compiled >= 12, "only {compiled} of 24 generated programs compiled; the generator has drifted");
}

#[test]
fn the_hart_refuses_in_the_hosts_words() {
    if !tools() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("onboard_refusals_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let hw = onboard::hw_caps();
    let rows: [(&str, &str, Run, &str, &str); 4] = [
        (
            "not LOGOS",
            "## Main\n    Show (1 +.\n",
            Run::Main,
            &hw,
            "I expected an expression here",
        ),
        (
            "no such function",
            "## To f (n: Int) -> Int:\n    Return n.\n",
            Run::Call { func: "g".into(), args: vec![1] },
            &hw,
            "Unknown function: g",
        ),
        (
            "fails on the VM",
            "## Main\n    Let z be 0.\n    Show 7 / z.\n",
            Run::Main,
            &hw,
            "Division by zero",
        ),
        (
            "deeper than the flashed image",
            "## To f (n: Int) -> Int:\n    If n is less than 2:\n        Return n.\n    Return f(n - 1) + f(n - 2).\n",
            Run::Call { func: "f".into(), args: vec![6] },
            "DEPTH=3",
            "DEPTH needs 5 but the flashed image has DEPTH=3",
        ),
    ];
    for (what, src, run, caps, reason) in rows {
        let path = dir.join("refused.lg");
        std::fs::write(&path, src).unwrap();
        assert!(!agree(&path, &run, caps), "{what}: the program must be refused");
        let said = on_hart(&path, &run, caps);
        assert!(said.contains(reason), "{what}: refused for the wrong reason:\n{said}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// What the engine sends for a run, as its RTL serializes it: each `Show` the engine model makes as a show record
/// (a Text's bytes taken from the heap), then the result frame.
fn engine_bytes(run: &logicaffeine_compile::codegen_sva::wprog_engine_rtl::WRun, end: u8) -> Vec<u8> {
    use logicaffeine_compile::codegen_sva::chip_protocol::*;
    let mut out = vec![0x00];
    for s in &run.shows {
        let v = match s.tag {
            TAG_FLOAT => ChipValue::Float(s.value as u64),
            TAG_WORD32 => ChipValue::Word32(s.value as u32),
            TAG_BOOL => ChipValue::Bool(s.value != 0),
            TAG_TEXT => {
                let ptr = s.value as usize;
                let len = run.heap[ptr] as usize;
                ChipValue::Text(String::from_utf8((1..=len).map(|i| run.heap[ptr + i] as u8).collect()).unwrap())
            }
            _ => ChipValue::Int(s.value),
        };
        out.extend(Record::Show(v).encode());
    }
    out.extend(ResultFrame { result: run.result, cycles: run.steps as u32, end }.encode());
    out
}

/// The board judges FABRIC == VM from the engine's bytes as the laptop does. For every library program and a batch
/// of generated ones, the engine model's run — serialized as the engine sends it, preceded by its power-on byte — is
/// read in every prefix: never an error, never over before its last byte, its shows only ever growing; whole, it
/// agrees with the VM's golden. A changed show, an overflowed run and a host call do not agree.
#[test]
fn the_engines_reply_is_judged_as_the_laptop_judges_it() {
    use logicaffeine_compile::codegen_sva::chip_protocol::{
        ChipValue, HostCall, Record, FRAME_END, FRAME_END_CALL_STACK,
    };
    use logicaffeine_compile::codegen_sva::onboard::{agrees, engine_reply, Golden};
    use logicaffeine_compile::codegen_sva::wprog_engine_rtl::eval_wprog_config_run;
    let mut cases: Vec<(String, String, Run)> = library()
        .into_iter()
        .map(|(title, path, run)| (title, std::fs::read_to_string(path).unwrap(), run))
        .collect();
    let mut rng = fabric_program_gen::Rng::new(0x0e9a_1e00);
    for n in 0..16 {
        let src = fabric_program_gen::gen(&mut rng).0;
        let args = if src.contains("(a: Int, b: Int)") { vec![4, 9] } else { vec![5] };
        cases.push((format!("generated {n}"), src, Run::Call { func: "f".into(), args }));
    }
    let mut judged = 0;
    for (title, src, run) in &cases {
        let (cfg, params, golden) = match run {
            Run::Main => match lower_main(src) {
                Ok((cfg, lines)) => (cfg, Vec::new(), Golden::Lines(lines)),
                Err(_) => continue,
            },
            Run::Call { func, args } => match lower(src, func, args) {
                Ok((cfg, _, answer)) => (cfg, args.clone(), Golden::Answer(answer)),
                Err(_) => continue,
            },
        };
        let model = eval_wprog_config_run(&cfg, &params, &[]);
        let bytes = engine_bytes(&model, FRAME_END);
        let mut seen = 0;
        for n in 0..bytes.len() {
            let r = engine_reply(&bytes[..n]).unwrap_or_else(|e| panic!("{title}: {n} bytes in: {e}"));
            assert!(r.ended.is_none(), "{title}: over after {n} of {} bytes", bytes.len());
            assert!(r.shows.len() >= seen, "{title}: shows went backwards at {n} bytes");
            seen = r.shows.len();
        }
        let whole = engine_reply(&bytes).unwrap();
        assert!(agrees(&golden, &whole), "{title}: FABRIC != VM on the model's own run: {whole:?} vs {golden:?}");

        // An overflowed run is no answer.
        let overflowed = engine_reply(&engine_bytes(&model, FRAME_END_CALL_STACK)).unwrap();
        assert!(!agrees(&golden, &overflowed), "{title}: an overflowed run agreed");
        // A changed value disagrees: an Int, Word32 or Bool show's (a Text's value is where its bytes are, and every
        // NaN prints alike).
        let number = model.shows.iter().position(|s| {
            use logicaffeine_compile::codegen_sva::chip_protocol::{TAG_FLOAT, TAG_TEXT};
            s.tag != TAG_TEXT && s.tag != TAG_FLOAT
        });
        match (&golden, number) {
            (Golden::Lines(lines), Some(i)) => {
                let mut changed = model.clone();
                changed.shows[i].value ^= 1;
                let r = engine_reply(&engine_bytes(&changed, FRAME_END)).unwrap();
                assert_ne!(r.shows[i], lines[i], "{title}: show {i} renders the same changed");
                assert!(!agrees(&golden, &r), "{title}: a changed show agreed");
            }
            (Golden::Answer(_), _) => {
                let mut changed = model.clone();
                changed.result ^= 1;
                assert!(!agrees(&golden, &engine_reply(&engine_bytes(&changed, FRAME_END)).unwrap()), "{title}");
            }
            _ => {}
        }
        judged += 1;
    }
    assert!(judged >= 16, "only {judged} programs were judged");

    // A program that calls the host is refused by name.
    let mut call = vec![0x00];
    call.extend(Record::Call { call: HostCall::Sync, args: vec![ChipValue::Int(1)] }.encode());
    let refused = engine_reply(&call).unwrap_err();
    assert!(refused.contains("calls the host") && refused.contains(HostCall::Sync.name()), "{refused}");
}
