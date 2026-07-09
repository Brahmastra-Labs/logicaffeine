//! `largo emit` — the unified code-emission verb, plus the direct-emit
//! backends shared by `largo build --emit` and `largo run --emit`.
//!
//! `rust` and `c` print (or `-o`-write) the generated source; `wasm` and
//! `wasm-linked` compile straight to a `.wasm` module via the built-in
//! backend (no rustc, cargo, or wasm-bindgen) with a Node.js host shim
//! beside it.

use std::fs;
use std::path::{Path, PathBuf};

use crate::commands::require_project_root;
use crate::project::manifest::Manifest;
use crate::ui;

/// The code targets `largo emit` can produce.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum EmitTarget {
    /// The generated Rust program (what `largo build` hands to cargo).
    Rust,
    /// The C translation (the benchmark backend).
    C,
    /// A self-contained `.wasm` module + Node.js host shim (no toolchain).
    Wasm,
    /// A `.wasm` module linked against the real BigInt runtime (rust-lld).
    WasmLinked,
}

/// Handle `largo emit <target> [FILE] [-o PATH]`.
pub(crate) fn cmd_emit(
    target: EmitTarget,
    file: Option<PathBuf>,
    output: Option<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    match target {
        EmitTarget::Rust => {
            let entry = resolve_entry(file)?;
            let code = crate::compile::compile_project(&entry)?.rust_code;
            write_emitted(&code, output.as_deref())
        }
        EmitTarget::C => {
            let entry = resolve_entry(file)?;
            let source = fs::read_to_string(&entry)?;
            let code = logicaffeine_compile::compile::compile_to_c(&source)
                .map_err(|e| format!("C emission failed: {e:?}"))?;
            write_emitted(&code, output.as_deref())
        }
        EmitTarget::Wasm => emit_wasm_target(file, output, false),
        EmitTarget::WasmLinked => emit_wasm_target(file, output, true),
    }
}

/// The source file to emit from: an explicit FILE, or the project entry
/// (with the standard `.md` fallback).
fn resolve_entry(file: Option<PathBuf>) -> Result<PathBuf, Box<dyn std::error::Error>> {
    match file {
        Some(f) => Ok(f),
        None => {
            let root = require_project_root()?;
            let manifest = Manifest::load(&root)?;
            Ok(crate::commands::resolve_entry_path(&root, &manifest)?)
        }
    }
}

/// Print emitted code to stdout, or write it to `-o` (stdout stays quiet).
fn write_emitted(code: &str, output: Option<&Path>) -> Result<(), Box<dyn std::error::Error>> {
    match output {
        Some(path) => {
            fs::write(path, code)?;
            ui::info(format!("Wrote {}", path.display()));
        }
        None => println!("{code}"),
    }
    Ok(())
}

/// `largo emit wasm[-linked]`: FILE mode compiles a standalone `.lg` and
/// writes beside it (or at `-o`); project mode matches `build --emit wasm`
/// (`target/<name>.wasm`), with `-o` overriding the module path. The host
/// shim always lands beside the module, sharing its stem.
fn emit_wasm_target(
    file: Option<PathBuf>,
    output: Option<PathBuf>,
    linked: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let (source, default_wasm_path) = match &file {
        Some(f) => (fs::read_to_string(f)?, f.with_extension("wasm")),
        None => {
            let root = require_project_root()?;
            let manifest = Manifest::load(&root)?;
            let entry = crate::commands::resolve_entry_path(&root, &manifest)?;
            let source = fs::read_to_string(entry)?;
            let out_dir = root.join("target");
            fs::create_dir_all(&out_dir)?;
            (source, out_dir.join(format!("{}.wasm", manifest.package.name)))
        }
    };
    let wasm = compile_wasm_bytes(&source, linked)?;
    let wasm_path = output.unwrap_or(default_wasm_path);
    if let Some(parent) = wasm_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    fs::write(&wasm_path, &wasm)?;
    let stem = wasm_path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or("output path needs a file name")?
        .to_string();
    let mjs = wasm_path.with_extension("mjs");
    fs::write(&mjs, wasm_host_shim(&stem))?;
    let tier = if linked { "wasm-linked, BigInt runtime" } else { "wasm, no rustc" };
    ui::info(format!("Built {} [{tier}, {} bytes]", wasm_path.display(), wasm.len()));
    ui::info(format!("Run it: node {}", mjs.display()));
    Ok(())
}

/// Compile LOGOS source to wasm bytes on the selected backend.
fn compile_wasm_bytes(source: &str, linked: bool) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    if linked {
        logicaffeine_compile::compile::compile_to_wasm_linked(source)
            .map_err(|e| format!("linked wasm build failed: {e:?}").into())
    } else {
        logicaffeine_compile::compile::compile_to_wasm(source)
            .map_err(|e| format!("wasm build failed: {e:?}").into())
    }
}

/// Compile the project entry DIRECTLY to a standalone `.wasm` via the built-in backend (no
/// rustc/cargo/wasm-bindgen), writing `target/<name>.wasm` and its `<name>.mjs` host shim. Returns
/// `(mjs_path, wasm_byte_len)`. Shared by `--emit wasm` on both `build` and `run`.
pub(crate) fn build_wasm_module(project_root: &std::path::Path, linked: bool) -> Result<(std::path::PathBuf, usize), Box<dyn std::error::Error>> {
    let manifest = Manifest::load(project_root)?;
    let entry_path = crate::commands::resolve_entry_path(project_root, &manifest)?;
    let source = fs::read_to_string(&entry_path)?;
    // `--emit wasm` is fully self-contained (no toolchain; integer overflow wraps per the i64
    // spec). `--emit wasm-linked` instead links the real `logicaffeine_base::BigInt` runtime with
    // `rust-lld`, so an overflowing integer expression computes the exact big number (matching the
    // VM) — at the cost of needing the Rust toolchain + a wasm32 `base` build.
    let wasm = if linked {
        logicaffeine_compile::compile::compile_to_wasm_linked(&source)
            .map_err(|e| format!("linked wasm build failed: {e:?}"))?
    } else {
        logicaffeine_compile::compile::compile_to_wasm(&source).map_err(|e| format!("wasm build failed: {e:?}"))?
    };
    let out_dir = project_root.join("target");
    fs::create_dir_all(&out_dir)?;
    let name = &manifest.package.name;
    fs::write(out_dir.join(format!("{name}.wasm")), &wasm)?;
    let mjs = out_dir.join(format!("{name}.mjs"));
    fs::write(&mjs, wasm_host_shim(name))?;
    Ok((mjs, wasm.len()))
}

/// `largo build --emit wasm[-linked]` — compile to a standalone `.wasm` + `.mjs` host shim.
pub(crate) fn emit_wasm_module(project_root: &std::path::Path, linked: bool) -> Result<(), Box<dyn std::error::Error>> {
    let (mjs, bytes) = build_wasm_module(project_root, linked)?;
    let wasm = mjs.with_extension("wasm");
    let tier = if linked { "wasm-linked, BigInt runtime" } else { "wasm, no rustc" };
    ui::info(format!("Built {} [{tier}, {bytes} bytes]", wasm.display()));
    ui::info(format!("Run it: node {}", mjs.display()));
    Ok(())
}

/// The JavaScript host shim written beside an emitted `.wasm` — supplies every `env.*` import the
/// backend can emit (the display sinks, the `{:.6}`-trimmed float formatter matching the tree-walker,
/// the scalar→Text formatters, `parse_int`, `pow`, and an `args` host that materializes `process.argv`
/// as a `Seq of Text` in the module's linear memory). Running it needs only a wasm engine (node/V8).
pub(crate) fn wasm_host_shim(name: &str) -> String {
    // The module filename lands inside generated JavaScript — JSON-encode it
    // so apostrophes/backslashes in a file stem can't corrupt the shim.
    let wasm_ref = serde_json::to_string(&format!("./{name}.wasm"))
        .expect("a string always JSON-encodes");
    format!(
        r#"// Auto-generated by `largo build --emit wasm`. Run: node <this file> [args…]
import {{ readFileSync }} from 'node:fs';
const bytes = readFileSync(new URL({wasm_ref}, import.meta.url));
let inst;
const dv = () => new DataView(inst.exports.memory.buffer);
const u8 = () => new Uint8Array(inst.exports.memory.buffer);
const dec = new TextDecoder(), enc = new TextEncoder();
const readText = (h) => {{ const d = dv(); const len = d.getInt32(h, true), dp = d.getInt32(h + 8, true); return dec.decode(u8().subarray(dp, dp + len)); }};
const writeStr = (buf, s) => {{ const b = enc.encode(s); u8().set(b, buf); return b.length; }};
const fmtF = (v) => {{ const s = v.toFixed(6).replace(/0+$/, '').replace(/\.$/, ''); return s === '-0' ? '0' : s; }};
const seqInts = (h, fmt) => {{ const d = dv(); const len = d.getInt32(h, true), dp = d.getInt32(h + 8, true); const e = []; for (let i = 0; i < len; i++) e.push(fmt(d, dp + i * 8)); return e; }};
const out = [];
const env = {{
  print_i64: (v) => out.push(v.toString()),
  print_bool: (v) => out.push(v ? 'true' : 'false'),
  print_f64: (v) => out.push(fmtF(v)),
  print_text: (h) => out.push(readText(h)),
  print_date: (v) => out.push(v.toString()),
  print_moment: (v) => out.push(v.toString()),
  print_seq_i64: (h) => out.push('[' + seqInts(h, (d, a) => d.getBigInt64(a, true).toString()).join(', ') + ']'),
  print_seq_f64: (h) => out.push('[' + seqInts(h, (d, a) => fmtF(d.getFloat64(a, true))).join(', ') + ']'),
  print_seq_text: (h) => out.push('[' + seqInts(h, (d, a) => readText(d.getInt32(a, true))).join(', ') + ']'),
  print_set_i64: (h) => out.push('{{' + seqInts(h, (d, a) => d.getBigInt64(a, true).toString()).join(', ') + '}}'),
  fmt_i64_into: (buf, v) => writeStr(buf, v.toString()),
  fmt_f64_into: (buf, v) => writeStr(buf, fmtF(v)),
  fmt_bool_into: (buf, v) => writeStr(buf, v ? 'true' : 'false'),
  fmt_f64_prec_into: (buf, v, prec) => writeStr(buf, v.toFixed(prec)),
  parse_int: (h) => BigInt(parseInt(readText(h).trim(), 10) || 0),
  pow_ff: (a, b) => Math.pow(a, b),
  pow_fi: (a, b) => Math.pow(a, Number(b)),
  today: () => 0,
  now: () => 0n,
  args: () => {{
    // argv lives on a page the SHIM grows: the program's bump allocator and
    // iterator stack are confined to the module's original pages (the
    // emitter never calls memory.grow), so a fresh page can never collide
    // with live program data. Cached: repeated args() calls share one page.
    if (argvSeq) return argvSeq;
    const argv = process.argv.slice(1); // [scriptPath, ...userArgs] — Logos `item 1` is the program
    const seq = inst.exports.memory.grow(1) * 65536;
    const d = dv(), b = u8(); const n = argv.length, seqData = seq + 16; let p = seqData + n * 8; const hs = [];
    for (const s of argv) {{ const t = p, by = enc.encode(s), td = t + 16; d.setInt32(t, by.length, true); d.setInt32(t + 4, by.length, true); d.setInt32(t + 8, td, true); b.set(by, td); hs.push(t); p = (td + by.length + 7) & ~7; }}
    d.setInt32(seq, n, true); d.setInt32(seq + 4, n, true); d.setInt32(seq + 8, seqData, true);
    for (let i = 0; i < n; i++) d.setBigInt64(seqData + i * 8, BigInt(hs[i]), true);
    argvSeq = seq;
    return seq;
  }},
}};
let argvSeq = 0;
const {{ instance }} = await WebAssembly.instantiate(bytes, {{ env }});
inst = instance;
// Flush whatever the program produced even when it traps (overflow etc.);
// a trap still fails the run loudly after the flush.
let trapped = null;
try {{
  instance.exports.main();
}} catch (e) {{
  trapped = e;
}}
process.stdout.write(out.join('\n') + (out.length ? '\n' : ''));
if (trapped) {{
  console.error(`wasm trapped: ${{trapped && trapped.message ? trapped.message : trapped}}`);
  process.exit(134);
}}
"#
    )
}

#[cfg(test)]
mod emit_wasm_tests {
    use super::*;
    use std::env;

    /// `largo build --emit wasm` writes a self-contained, VALID `.wasm` module directly from a LOGOS
    /// project — no rustc, cargo, or wasm-bindgen. Verifies the CLI plumbing (manifest → entry →
    /// `compile_to_wasm` → `target/<name>.wasm`) end to end and that the bytes are a real wasm module.
    #[test]
    fn emit_wasm_writes_a_valid_module() {
        let dir = env::temp_dir().join(format!("largo_emit_wasm_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(
            dir.join("Largo.toml"),
            "[package]\nname=\"emitted\"\nversion=\"0.1.0\"\nentry=\"src/main.lg\"\n",
        )
        .unwrap();
        fs::write(dir.join("src/main.lg"), "## Main\n    Show 6 * 7.\n").unwrap();

        emit_wasm_module(&dir, false).expect("emit wasm should succeed");

        let wasm = fs::read(dir.join("target/emitted.wasm")).expect("output .wasm should exist");
        assert_eq!(&wasm[0..4], b"\0asm", "output must begin with the wasm magic");
        assert!(wasm.len() > 8, "a real module is more than a header");
        fs::remove_dir_all(&dir).ok();
    }
}
