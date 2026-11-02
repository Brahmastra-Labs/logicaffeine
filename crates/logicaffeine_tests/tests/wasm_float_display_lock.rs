//! ★ LOCK — a `Real` shows as the same Text on every WebAssembly host as on every other tier. ★
//!
//! `logicaffeine_data::fmt::fmt_f64` is the one float display: the shortest digits that read back to the same
//! bits, never scientific notation, an integral value bare, `-0`, `NaN`, `inf` and `-inf` spelled as Rust spells
//! them. The tree-walker, the VM and the compiled binary call it; a WebAssembly module hands its float to the
//! host (`print_f64`, `fmt_f64_into`, `print_seq_f64`), and each host (the browser runtime, `runtime.js`, and the
//! node shim beside an emitted `.wasm`, `web_host::wasm_host_shim`) formats it in JavaScript. Both hosts carry one
//! definition of that formatter, and it is held here to `fmt_f64` over random bit patterns and the edges, and
//! end to end through a compiled module against the VM.

mod common;
use common::require_tool;

use std::process::Command;

use logicaffeine_compile::compile::{compile_to_wasm, vm_outcome};
use logicaffeine_compile::web_host::{
    browser_host_js, wasm_host_shim, BROWSER_RUNTIME_JS, FMT_F64_JS,
};
use logicaffeine_data::fmt::fmt_f64;

/// The formatter's definition, as a host carries it: the statement that begins `const fmtF = `.
fn formatter_of(host: &str, js: &str) -> String {
    let start = js
        .find("const fmtF = ")
        .unwrap_or_else(|| panic!("{host} defines no `const fmtF`"));
    let end = js[start..]
        .find("};\n")
        .unwrap_or_else(|| panic!("{host}: `fmtF` has no end"))
        + start
        + 2;
    js[start..end].to_string()
}

fn hosts() -> [(&'static str, String); 2] {
    [
        (
            "the browser runtime",
            formatter_of("runtime.js", BROWSER_RUNTIME_JS),
        ),
        (
            "the node shim",
            formatter_of("the node shim", &wasm_host_shim("probe")),
        ),
    ]
}

fn scratch(tag: &str) -> std::path::PathBuf {
    let dir =
        std::env::temp_dir().join(format!("logos-float-display-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn node(dir: &std::path::Path, file: &str) -> (String, String, bool) {
    let out = Command::new("node")
        .arg(dir.join(file))
        .output()
        .expect("node preflighted present but failed to spawn");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.success(),
    )
}

/// The doubles every host is held to: the edges by name (exact ties among them), then seeded random bit patterns
/// (every exponent, subnormals, both signs) and seeded values of everyday magnitude.
fn doubles() -> Vec<f64> {
    let mut out = vec![
        0.0,
        -0.0,
        1.0,
        -14.0,
        1.0 / 3.0,
        0.1 + 0.2,
        0.0000001,
        1e-6,
        1e21,
        1e22,
        5e-324,
        f64::MIN_POSITIVE,
        f64::MAX,
        -f64::MAX,
        9007199254740992.0,
        9007199254740994.0,
        123456789012345680000000000000.0,
        1.5e-300,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NAN,
        // Exactly halfway between two shortest candidates: the larger one, as Rust's `{}` spells it.
        f64::from_bits(0x4317_9085_685d_83c9),
        f64::from_bits(0xc30d_eb9e_e957_fc2a),
        f64::from_bits(0x42ea_8090_bb0f_6d84),
    ];
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for _ in 0..100_000 {
        out.push(f64::from_bits(next()));
    }
    for _ in 0..20_000 {
        out.push((next() >> 11) as f64 / (1u64 << 53) as f64 * 2e6 - 1e6);
    }
    out
}

#[test]
fn both_hosts_carry_one_float_formatter() {
    for (host, formatter) in hosts() {
        assert_eq!(
            formatter, FMT_F64_JS,
            "{host} must format a float with `web_host::FMT_F64_JS`, the one definition"
        );
    }
}

#[test]
fn every_host_formats_every_double_as_fmt_f64_does() {
    if !require_tool("node") {
        return;
    }
    let values = doubles();
    let bits: Vec<String> = values
        .iter()
        .map(|v| format!("{:016x}", v.to_bits()))
        .collect();
    for (host, formatter) in hosts() {
        let dir = scratch(&host.replace(' ', "-"));
        std::fs::write(dir.join("bits.txt"), bits.join("\n")).expect("write bits");
        std::fs::write(
            dir.join("probe.mjs"),
            format!(
                "import {{ readFileSync }} from 'node:fs';\n{formatter}\n\
                 const view = new DataView(new ArrayBuffer(8));\n\
                 const out = readFileSync(new URL('./bits.txt', import.meta.url), 'utf8').split('\\n').map((h) => {{ view.setBigUint64(0, BigInt('0x' + h)); return fmtF(view.getFloat64(0)); }});\n\
                 process.stdout.write(out.join('\\n'));\n"
            ),
        )
        .expect("write probe");
        let (stdout, stderr, ok) = node(&dir, "probe.mjs");
        assert!(ok, "{host}: the formatter runs:\n{stderr}");
        let got: Vec<&str> = stdout.split('\n').collect();
        assert_eq!(got.len(), values.len(), "{host}: one line per double");
        let wrong: Vec<String> = values
            .iter()
            .zip(&got)
            .filter(|(v, g)| fmt_f64(**v) != **g)
            .take(8)
            .map(|(v, g)| {
                format!(
                    "{:016x}: fmt_f64 {:?}, {host} {g:?}",
                    v.to_bits(),
                    fmt_f64(*v)
                )
            })
            .collect();
        assert!(
            wrong.is_empty(),
            "{host} formats doubles otherwise than fmt_f64 (first {}):\n{}",
            wrong.len(),
            wrong.join("\n")
        );
    }
}

/// Reals whose display is hard: a third, the sum that shows its rounding, a tiny value, one past 2^53, a
/// negative zero, a harmonic sum and its eight-lane reordering, and a seeded spread of magnitudes.
const REALS: &str = "\
## Main
Show 1.0 / 3.0.
Show 0.1 + 0.2.
Show 0.0000001.
Show 9007199254740993.0 * 1000000.0.
Show 0.0 * (0.0 - 1.0).
Let mutable h be 0.0.
Repeat for i from 1 to 30:
    Set h to h + 1.0 / i.
Show h.
Let mutable seed be 5.
Let mutable scale be 0.000001.
Repeat for i from 1 to 24:
    Set seed to (seed * 1103515245 + 12345) % 2147483648.
    Show seed / 2147483648.0 * scale.
    Set scale to scale * 10.0.
";

fn in_node_shim(src: &str) -> (String, String, bool) {
    let wasm = compile_to_wasm(src).unwrap_or_else(|e| panic!("must compile: {e:?}\n{src}"));
    let dir = scratch("shim-module");
    std::fs::write(dir.join("probe.wasm"), &wasm).expect("write wasm");
    std::fs::write(dir.join("probe.mjs"), wasm_host_shim("probe")).expect("write shim");
    node(&dir, "probe.mjs")
}

const MIN_GLOBALS: &str = r#"
globalThis.document = { body: {}, addEventListener() {} };
globalThis.window = { addEventListener() {}, history: { pushState() {}, replaceState() {} }, location: { pathname: '/', search: '' } };
"#;

fn in_browser_host(src: &str) -> (String, String, bool) {
    let wasm = compile_to_wasm(src).unwrap_or_else(|e| panic!("must compile: {e:?}\n{src}"));
    let csv = wasm
        .iter()
        .map(|b| b.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let from = "const bytes = await (await fetch(WASM_URL)).arrayBuffer();";
    let host = browser_host_js("__float_probe__");
    assert!(
        host.contains(from),
        "the wasm loader shape in browser_host_js drifted"
    );
    let dir = scratch("browser-module");
    std::fs::write(
        dir.join("probe.mjs"),
        format!(
            "{MIN_GLOBALS}\n{}\n",
            host.replace(from, &format!("const bytes = new Uint8Array([{csv}]);"))
        ),
    )
    .expect("write probe");
    node(&dir, "probe.mjs")
}

#[test]
fn a_compiled_module_shows_reals_as_the_vm_does_on_every_host() {
    if !require_tool("node") {
        return;
    }
    let vm = vm_outcome(REALS);
    assert_eq!(vm.error, None, "the VM runs the reals");
    for (host, (stdout, stderr, ok)) in [
        ("the node shim", in_node_shim(REALS)),
        ("the browser runtime", in_browser_host(REALS)),
    ] {
        assert!(
            ok,
            "{host} runs the module:\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
        assert_eq!(stdout, vm.output, "{host} shows the reals as the VM does");
    }
}
