//! The C device model is the Rust device model, call for call.
//!
//! The model a host binary links is C generated from the regions and their lane routines: `accel::c_model`.
//! Built with gcc and fed every curated call, it must give the value the Rust lane evaluation gives, or hand
//! the call back with the same divergence. Its driver, `logos_dev_call`, must count what it did into the
//! accounting file `LOGOS_ACCEL_STATS` names, hand every call back under `LOGOS_ACCEL_OFFLOAD=off`, and never
//! write to standard output or standard error.

#[path = "common/scratch.rs"]
mod scratch;
#[path = "accel_common/mod.rs"]
mod accel_common;

use std::io::Write;
use std::process::{Command, Stdio};

use accel_common::{cases, region_of};
use logicaffeine_compile::accel::c_model::model_c;
use logicaffeine_compile::accel::lane::{ExactLanes, LaneShape};
use logicaffeine_compile::accel::region::{Halt, RegionCfg};

const FUEL: u64 = 10_000_000;

/// A `main` that reads `region arg…` lines and prints `ok VALUE` or `declined CODE` for each, calling each
/// region's function directly so the divergence code is visible.
fn harness(regions: &[(u32, RegionCfg)]) -> String {
    let mut cases = String::new();
    for (id, _) in regions {
        cases.push_str(&format!("            case {id}: ok = logos_region_{id}(args, &out, &fault); break;\n"));
    }
    format!(
        r#"
int main(void) {{
    char line[8192];
    while (fgets(line, sizeof line, stdin)) {{
        char *p = line;
        int region = (int)strtol(p, &p, 10);
        int64_t args[32];
        int n = 0;
        while (*p == ' ') p++;
        while (*p && *p != '\n' && n < 32) {{ args[n++] = strtoll(p, &p, 10); while (*p == ' ') p++; }}
        int64_t out = 0;
        uint32_t fault = 0;
        int ok = 0;
        switch (region) {{
{cases}            default: return 2;
        }}
        if (ok) printf("ok %lld\n", (long long)out); else printf("declined %u\n", fault);
    }}
    return 0;
}}
"#
    )
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = scratch::dir("accel_c_model").join(name);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

fn build(c: &str, name: &str) -> std::path::PathBuf {
    let dir = scratch(name);
    std::fs::write(dir.join("model.c"), c).expect("write the model");
    let out = Command::new("gcc")
        .args(["-O2", "-ffp-contract=off", "-o", "model", "model.c", "-lm"])
        .current_dir(&dir)
        .output()
        .expect("gcc starts");
    assert!(out.status.success(), "the generated model builds with gcc:\n{}", String::from_utf8_lossy(&out.stderr));
    dir.join("model")
}

/// How long one model run may take. Every curated call finishes in milliseconds, so a model still running
/// past this loops where the Rust model does not.
const DEADLINE: std::time::Duration = std::time::Duration::from_secs(60);

fn run(bin: &std::path::Path, input: &str, envs: &[(&str, &str)]) -> (String, String) {
    let mut cmd = Command::new(bin);
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().expect("the model starts");
    child.stdin.take().expect("stdin").write_all(input.as_bytes()).expect("feed the calls");
    let (mut stdout, mut stderr) = (child.stdout.take().expect("stdout"), child.stderr.take().expect("stderr"));
    let out_reader = std::thread::spawn(move || {
        let mut b = Vec::new();
        std::io::Read::read_to_end(&mut stdout, &mut b).map(|_| b)
    });
    let err_reader = std::thread::spawn(move || {
        let mut b = Vec::new();
        std::io::Read::read_to_end(&mut stderr, &mut b).map(|_| b)
    });
    let started = std::time::Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("the model's status") {
            break status;
        }
        if started.elapsed() > DEADLINE {
            let _ = child.kill();
            let _ = child.wait();
            panic!("the C model ran past its {DEADLINE:?} deadline, looping where the Rust model returns");
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    let stdout = out_reader.join().expect("the stdout reader").expect("read stdout");
    let stderr = err_reader.join().expect("the stderr reader").expect("read stderr");
    assert!(status.success(), "the model ran to the end: {status:?}");
    (String::from_utf8_lossy(&stdout).into_owned(), String::from_utf8_lossy(&stderr).into_owned())
}

#[test]
fn the_c_model_is_the_rust_model_at_every_device_shape() {
    let regions: Vec<(u32, RegionCfg)> = cases().iter().enumerate().map(|(i, c)| (i as u32, region_of(c))).collect();
    let mut bad = Vec::new();
    for shape in [LaneShape::new(32, 16), LaneShape::new(32, 8), LaneShape::new(8, 4)] {
        let mut c = model_c(&regions, shape, "probe");
        c.push_str(&harness(&regions));
        let bin = build(&c, &format!("w{}m{}", shape.word, shape.mul));
        let mut input = String::new();
        let mut want = Vec::new();
        for (case, (id, region)) in cases().iter().zip(&regions) {
            for args in &case.calls {
                let words: Vec<i64> = args.iter().map(|a| a.word()).collect();
                input.push_str(&format!("{id} {}\n", words.iter().map(|w| w.to_string()).collect::<Vec<_>>().join(" ")));
                let mut lanes = ExactLanes { shape };
                let expected = match region.eval_on_lanes(&mut lanes, &words, FUEL) {
                    Ok((v, _)) => format!("ok {v}"),
                    Err(Halt::Declined { divergence, .. }) => format!("declined {}", divergence.code()),
                    Err(Halt::OutOfFuel) => panic!("{}: out of fuel", case.func),
                };
                want.push((format!("{}({args:?})", case.func), expected));
            }
        }
        let (stdout, _) = run(&bin, &input, &[]);
        let got: Vec<&str> = stdout.lines().collect();
        assert_eq!(got.len(), want.len(), "{shape:?}: one answer per call");
        for ((call, w), g) in want.iter().zip(got) {
            if g != w {
                bad.push(format!("{shape:?} {call}: C {g}, Rust {w}"));
            }
        }
    }
    assert!(bad.is_empty(), "{} C model answers differ from the Rust model:\n{}", bad.len(), bad.join("\n"));
}

/// A `main` that calls `logos_dev_call` for each `region arg…` line and prints nothing at all.
const SILENT_MAIN: &str = r#"
int logos_dev_call(int region, const int64_t *args, int64_t *out);
int main(void) {
    char line[8192];
    while (fgets(line, sizeof line, stdin)) {
        char *p = line;
        int region = (int)strtol(p, &p, 10);
        int64_t args[32];
        int n = 0;
        while (*p == ' ') p++;
        while (*p && *p != '\n' && n < 32) { args[n++] = strtoll(p, &p, 10); while (*p == ' ') p++; }
        int64_t out = 0;
        (void)logos_dev_call(region, args, &out);
    }
    return 0;
}
"#;

#[test]
fn the_driver_accounts_every_call_and_never_speaks() {
    let all = cases();
    let pick = |name: &str| all.iter().position(|c| c.func == name).expect("a curated function");
    let (div, add) = (pick("divmix"), pick("add3"));
    let regions = vec![(0u32, region_of(&all[add])), (1u32, region_of(&all[div]))];
    let mut c = model_c(&regions, LaneShape::new(32, 16), "probe");
    c.push_str(SILENT_MAIN);
    let bin = build(&c, "driver");
    let input = "0 1 2 3\n0 4 5 6\n1 17 5\n1 5 0\n1 5 0\n7 1 2\n";
    let stats = scratch("stats").join("stats.jsonl");
    let _ = std::fs::remove_file(&stats);
    let (stdout, stderr) = run(&bin, input, &[("LOGOS_ACCEL_STATS", stats.to_str().expect("a path"))]);
    assert!(stdout.is_empty() && stderr.is_empty(), "the model never writes the program's streams: {stdout:?} {stderr:?}");
    let line = std::fs::read_to_string(&stats).expect("the accounting line is written at exit");
    let v: serde_json::Value = serde_json::from_str(line.trim()).expect("one JSON line");
    assert_eq!(v["device"], "probe");
    assert_eq!(v["regions"][0]["name"], "add3");
    assert_eq!(v["regions"][0]["done"], 2, "two add3 calls completed");
    assert_eq!(v["regions"][1]["done"], 1, "one divmix call completed");
    assert_eq!(v["regions"][1]["declined"]["divide-by-zero"], 2, "two divmix calls handed back a zero divisor");
    let _ = std::fs::remove_file(&stats);
    let (stdout, stderr) = run(&bin, input, &[("LOGOS_ACCEL_STATS", stats.to_str().expect("a path")), ("LOGOS_ACCEL_OFFLOAD", "off")]);
    assert!(stdout.is_empty() && stderr.is_empty());
    let line = std::fs::read_to_string(&stats).expect("accounting under offload=off");
    let v: serde_json::Value = serde_json::from_str(line.trim()).expect("one JSON line");
    assert_eq!(v["regions"][0]["done"], 0, "nothing runs on the device with offload off");
    assert_eq!(v["regions"][0]["offload_off"], 2);
    assert_eq!(v["regions"][1]["offload_off"], 3);
}

/// A lane routine writes every register before it reads it, so the model's register files hold nothing
/// from one call into the next: a call depends on its inputs alone.
#[test]
fn every_lane_routine_writes_a_register_before_reading_it() {
    use logicaffeine_compile::accel::cells::{all_ops, routine};
    use logicaffeine_compile::accel::lane::{LOp, LStmt};
    fn reads(op: &LOp) -> Vec<u32> {
        match *op {
            LOp::Const(_) => vec![],
            LOp::Mov(a) | LOp::Not { a } | LOp::Shl { a, .. } | LOp::Shr { a, .. } => vec![a],
            LOp::Add { a, b, cin: c } | LOp::Sub { a, b, bin: c } | LOp::Sel { c: a, t: b, f: c } => vec![a, b, c],
            LOp::Mul { a, b } | LOp::And { a, b } | LOp::Or { a, b } | LOp::Xor { a, b } | LOp::Ltu { a, b } | LOp::Eq { a, b } => vec![a, b],
        }
    }
    fn walk(body: &[LStmt], written: &mut std::collections::BTreeSet<u32>, bad: &mut Vec<u32>) {
        for s in body {
            match s {
                LStmt::Inst(i) => {
                    bad.extend(reads(&i.op).into_iter().filter(|r| !written.contains(r)));
                    written.insert(i.dst);
                    written.extend(i.dst2);
                }
                LStmt::Repeat { body, .. } => walk(body, written, bad),
            }
        }
    }
    let mut bad = Vec::new();
    for shape in [LaneShape::new(32, 16), LaneShape::new(32, 8), LaneShape::new(8, 4), LaneShape::new(2, 1)] {
        for op in all_ops() {
            let r = routine(shape, op);
            let mut written: std::collections::BTreeSet<u32> = r.inputs.iter().copied().collect();
            let mut unread = Vec::new();
            walk(&r.body, &mut written, &mut unread);
            if !unread.is_empty() {
                bad.push(format!("{shape:?} {}: reads {unread:?} before writing them", op.name()));
            }
        }
    }
    assert!(bad.is_empty(), "{} routines read a register before writing it:\n{}", bad.len(), bad.join("\n"));
}
