//! The P2 linker — emit LLVM-compatible RELOCATABLE wasm objects and link them against the prebuilt
//! Rust runtime with the toolchain's `rust-lld`.
//!
//! # Why a linker
//!
//! The self-contained AOT backend ([`super::module`]) reimplements the heap value model directly in
//! emitted wasm. That works for the scalar + collection fragment, but some features can NOT be
//! honestly reimplemented in emitted wasm without diverging from the VM — arbitrary-precision `BigInt`
//! canonicalization, the real `FxHashMap` iteration order, the scheduler/relay transports. For those,
//! the plan is to REUSE the existing Rust runtime (`logicaffeine_base`/`data`/`system`), compiled once
//! to `wasm32-unknown-unknown`, and STATICALLY LINK it into the program at emit time so there is one
//! linear memory + one allocator and no second implementation to keep in sync.
//!
//! # The object format (reverse-engineered from `rustc --emit=obj`)
//!
//! A relocatable wasm object is a normal module whose function-index references are PLACEHOLDERS that
//! the linker renumbers when it merges objects. It carries two extra custom sections:
//!   * `linking` — a symbol table: each function is a symbol, either DEFINED (our `main`, with a name)
//!     or UNDEFINED (a `logos_rt_*`/host import the linker resolves by name).
//!   * `reloc.CODE` — for every function-index appearing in the code section, a relocation
//!     (`R_WASM_FUNCTION_INDEX_LEB`, a byte offset, the symbol it refers to) so the linker can rewrite
//!     the 5-byte padded LEB in place.
//!
//! This module builds exactly that, then invokes `rust-lld -flavor wasm`.

use std::path::PathBuf;
use std::process::Command;

use super::WasmLowerError;

type R<T> = Result<T, WasmLowerError>;

/// wasm value types.
const I64: u8 = 0x7e;

/// Append the unsigned LEB128 of `v`.
fn leb_u(mut v: u64, out: &mut Vec<u8>) {
    loop {
        let byte = (v & 0x7f) as u8;
        v >>= 7;
        if v != 0 {
            out.push(byte | 0x80);
        } else {
            out.push(byte);
            break;
        }
    }
}

/// Append `v` as a FIXED 5-byte unsigned LEB128 — the relocatable slot a `reloc.CODE` entry overwrites
/// (the linker rewrites the final function index in place, so the field must be full width regardless
/// of the placeholder's magnitude).
fn leb_u32_padded(v: u32, out: &mut Vec<u8>) {
    let mut v = v;
    for i in 0..5 {
        let mut byte = (v & 0x7f) as u8;
        v >>= 7;
        if i < 4 {
            byte |= 0x80;
        }
        out.push(byte);
    }
}

/// Append the SIGNED LEB128 of `v` — the encoding an `i64.const` operand uses.
fn leb_i64(mut v: i64, out: &mut Vec<u8>) {
    loop {
        let byte = (v & 0x7f) as u8;
        v >>= 7; // arithmetic shift (sign-extending)
        let done = (v == 0 && byte & 0x40 == 0) || (v == -1 && byte & 0x40 != 0);
        out.push(if done { byte } else { byte | 0x80 });
        if done {
            break;
        }
    }
}

/// Append a section: id, byte length, body.
fn section(id: u8, body: &[u8], out: &mut Vec<u8>) {
    out.push(id);
    leb_u(body.len() as u64, out);
    out.extend_from_slice(body);
}

/// Append a custom section: id 0, then (name, body) as the payload.
fn custom_section(name: &str, body: &[u8], out: &mut Vec<u8>) {
    let mut payload = Vec::new();
    leb_u(name.len() as u64, &mut payload);
    payload.extend_from_slice(name.as_bytes());
    payload.extend_from_slice(body);
    section(0, &payload, out);
}

/// A function symbol in the object's linking table.
struct FuncSymbol {
    /// The function index this symbol refers to (import index for undefined, module index for defined).
    func_index: u32,
    /// `Some(name)` for a DEFINED function (carries its name); `None` for an UNDEFINED import (the name
    /// comes from the import entry, which the linker matches by).
    defined_name: Option<String>,
}

/// A relocation against the code section: rewrite the padded function-index LEB at `offset` (relative
/// to the code section's content) to point at symbol `sym_index`.
struct CodeReloc {
    offset: u32,
    sym_index: u32,
}

/// Emit a relocatable object exporting `main() -> i64` whose body is `logos_rt_probe(41)` — the
/// smallest program that calls the runtime by an undefined symbol. It proves our object encoder is
/// consumable by `rust-lld` end-to-end (encode → link → run), the foundation every later runtime call
/// builds on. `logos_rt_probe` is the runtime's link smoke-test function (`x + 1`).
pub(crate) fn emit_probe_object() -> Vec<u8> {
    // ---- Type section: type 0 = (i64)->i64 (probe), type 1 = ()->i64 (main). ----
    let mut types = Vec::new();
    leb_u(2, &mut types);
    types.extend_from_slice(&[0x60, 0x01, I64, 0x01, I64]); // (i64) -> i64
    types.extend_from_slice(&[0x60, 0x00, 0x01, I64]); // () -> i64

    // ---- Import section: import env.logos_rt_probe : type 0  → imported func index 0. ----
    let mut imports = Vec::new();
    leb_u(1, &mut imports);
    let name_bytes = |m: &str, f: &str, out: &mut Vec<u8>| {
        leb_u(m.len() as u64, out);
        out.extend_from_slice(m.as_bytes());
        leb_u(f.len() as u64, out);
        out.extend_from_slice(f.as_bytes());
    };
    name_bytes("env", "logos_rt_probe", &mut imports);
    imports.push(0x00); // kind: function
    leb_u(0, &mut imports); // type index 0

    // ---- Function section: 1 defined function of type 1 → module func index 1 (after the import). ----
    let probe_func_index: u32 = 0;
    let main_func_index: u32 = 1;
    let mut funcs = Vec::new();
    leb_u(1, &mut funcs);
    leb_u(1, &mut funcs); // type index 1

    // ---- Code section: main body = `i64.const 41; call <padded probe idx>; end`. Record the reloc. ----
    let mut body = Vec::new();
    leb_u(0, &mut body); // 0 local groups
    body.push(0x42); // i64.const
    leb_u(41, &mut body); // 41
    body.push(0x10); // call
    let call_leb_off_in_body = body.len() as u32; // offset of the (padded) function index within the body
    leb_u32_padded(probe_func_index, &mut body);
    body.push(0x0b); // end

    let mut code = Vec::new();
    leb_u(1, &mut code); // 1 entry
    let size_field_start = code.len();
    leb_u(body.len() as u64, &mut code); // body size
    let body_start_in_content = code.len() as u32; // where the body begins within the code section content
    code.extend_from_slice(&body);
    let reloc_offset = body_start_in_content + call_leb_off_in_body;
    let _ = size_field_start;

    // Symbol table: symbol 0 = undefined logos_rt_probe (the reloc target); symbol 1 = defined main.
    let symbols = vec![
        FuncSymbol { func_index: probe_func_index, defined_name: None },
        FuncSymbol { func_index: main_func_index, defined_name: Some("main".to_string()) },
    ];
    let relocs = vec![CodeReloc { offset: reloc_offset, sym_index: 0 }];

    // ---- Assemble the module. Section order fixes the code section's index for `reloc.CODE`. ----
    let mut out = Vec::new();
    out.extend_from_slice(b"\0asm");
    out.extend_from_slice(&[0x01, 0x00, 0x00, 0x00]); // version 1
    section(1, &types, &mut out);
    section(2, &imports, &mut out);
    section(3, &funcs, &mut out);
    let code_section_index: u32 = 3; // type(0), import(1), func(2), code(3)
    section(10, &code, &mut out);
    custom_section("linking", &encode_linking(&symbols), &mut out);
    custom_section("reloc.CODE", &encode_reloc_code(code_section_index, &relocs), &mut out);
    out
}

/// Emit a relocatable object proving SHARED MEMORY with the runtime: `main() -> i64` calls the
/// undefined `imported_fn` (which returns a pointer into the shared linear memory holding an `i64`),
/// then `i64.load`s from that address and returns it. This forces our object to import
/// `env.__linear_memory` — the same memory `rust-lld` gives the runtime object — so a pointer the
/// runtime produces is valid on our side. Used to prove both a runtime STATIC (S2) and a runtime
/// dynamic ALLOCATION (S3a) land in the one shared memory our emitted code can read.
pub(crate) fn emit_load_object(imported_fn: &str) -> Vec<u8> {
    // Types: 0 = ()->i32 (write_answer, returns a pointer), 1 = ()->i64 (main).
    let mut types = Vec::new();
    leb_u(2, &mut types);
    types.extend_from_slice(&[0x60, 0x00, 0x01, 0x7f]); // () -> i32
    types.extend_from_slice(&[0x60, 0x00, 0x01, I64]); // () -> i64

    // Imports: 0 = env.__linear_memory (memory, min 0) — NO func index; 1 = env.logos_rt_write_answer
    // (func, type 0) → imported func index 0. The memory import needs no symbol-table entry.
    let mut imports = Vec::new();
    leb_u(2, &mut imports);
    let import_name = |m: &str, f: &str, out: &mut Vec<u8>| {
        leb_u(m.len() as u64, out);
        out.extend_from_slice(m.as_bytes());
        leb_u(f.len() as u64, out);
        out.extend_from_slice(f.as_bytes());
    };
    import_name("env", "__linear_memory", &mut imports);
    imports.push(0x02); // kind: memory
    imports.push(0x00); // limits flags: min only
    leb_u(0, &mut imports); // min 0 pages
    import_name("env", imported_fn, &mut imports);
    imports.push(0x00); // kind: function
    leb_u(0, &mut imports); // type index 0

    let write_func_index: u32 = 0;
    let main_func_index: u32 = 1;
    let mut funcs = Vec::new();
    leb_u(1, &mut funcs);
    leb_u(1, &mut funcs); // main : type 1

    // main body: `call write_answer; i64.load align=3 off=0; end`.
    let mut body = Vec::new();
    leb_u(0, &mut body); // 0 locals
    body.push(0x10); // call
    let call_leb_off_in_body = body.len() as u32;
    leb_u32_padded(write_func_index, &mut body);
    body.push(0x29); // i64.load
    leb_u(3, &mut body); // alignment 2^3 = 8
    leb_u(0, &mut body); // offset 0
    body.push(0x0b); // end

    let mut code = Vec::new();
    leb_u(1, &mut code);
    leb_u(body.len() as u64, &mut code);
    let body_start_in_content = code.len() as u32;
    code.extend_from_slice(&body);
    let reloc_offset = body_start_in_content + call_leb_off_in_body;

    let symbols = vec![
        FuncSymbol { func_index: write_func_index, defined_name: None },
        FuncSymbol { func_index: main_func_index, defined_name: Some("main".to_string()) },
    ];
    let relocs = vec![CodeReloc { offset: reloc_offset, sym_index: 0 }];

    let mut out = Vec::new();
    out.extend_from_slice(b"\0asm");
    out.extend_from_slice(&[0x01, 0x00, 0x00, 0x00]);
    section(1, &types, &mut out);
    section(2, &imports, &mut out);
    section(3, &funcs, &mut out);
    let code_section_index: u32 = 3;
    section(10, &code, &mut out);
    custom_section("linking", &encode_linking(&symbols), &mut out);
    custom_section("reloc.CODE", &encode_reloc_code(code_section_index, &relocs), &mut out);
    out
}

/// Emit a relocatable object proving the TEXT-HANDLE ABI (S3b): `main() -> i32` calls
/// `logos_rt_i64_mul_to_text(a, b)` — the runtime multiplies into an `i128`, formats it to a decimal
/// string, and builds a `Text` in the shared memory in the emitter's own layout (`[len][cap][data_ptr]`
/// 16-byte header + separate bytes) — and returns that Text handle. This is exactly the shape `Show`
/// reads via `print_text`, so it proves a runtime call can hand back a heap value our side (and the
/// self-contained emitter's existing Show path) consumes.
pub(crate) fn emit_mul_text_object(a: i64, b: i64) -> Vec<u8> {
    // Types: 0 = (i64,i64)->i32 (mul_to_text), 1 = ()->i32 (main).
    let mut types = Vec::new();
    leb_u(2, &mut types);
    types.extend_from_slice(&[0x60, 0x02, I64, I64, 0x01, 0x7f]); // (i64, i64) -> i32
    types.extend_from_slice(&[0x60, 0x00, 0x01, 0x7f]); // () -> i32

    // Imports: 0 = env.__linear_memory (memory), 1 = env.logos_rt_i64_mul_to_text (func, type 0).
    let mut imports = Vec::new();
    leb_u(2, &mut imports);
    let import_name = |m: &str, f: &str, out: &mut Vec<u8>| {
        leb_u(m.len() as u64, out);
        out.extend_from_slice(m.as_bytes());
        leb_u(f.len() as u64, out);
        out.extend_from_slice(f.as_bytes());
    };
    import_name("env", "__linear_memory", &mut imports);
    imports.push(0x02);
    imports.push(0x00);
    leb_u(0, &mut imports);
    import_name("env", "logos_rt_i64_mul_to_text", &mut imports);
    imports.push(0x00);
    leb_u(0, &mut imports); // type 0

    let mul_func_index: u32 = 0;
    let main_func_index: u32 = 1;
    let mut funcs = Vec::new();
    leb_u(1, &mut funcs);
    leb_u(1, &mut funcs); // main : type 1

    // main body: `i64.const a; i64.const b; call mul_to_text; end`.
    let mut body = Vec::new();
    leb_u(0, &mut body);
    body.push(0x42);
    leb_i64(a, &mut body);
    body.push(0x42);
    leb_i64(b, &mut body);
    body.push(0x10);
    let call_leb_off_in_body = body.len() as u32;
    leb_u32_padded(mul_func_index, &mut body);
    body.push(0x0b);

    let mut code = Vec::new();
    leb_u(1, &mut code);
    leb_u(body.len() as u64, &mut code);
    let body_start_in_content = code.len() as u32;
    code.extend_from_slice(&body);
    let reloc_offset = body_start_in_content + call_leb_off_in_body;

    let symbols = vec![
        FuncSymbol { func_index: mul_func_index, defined_name: None },
        FuncSymbol { func_index: main_func_index, defined_name: Some("main".to_string()) },
    ];
    let relocs = vec![CodeReloc { offset: reloc_offset, sym_index: 0 }];

    let mut out = Vec::new();
    out.extend_from_slice(b"\0asm");
    out.extend_from_slice(&[0x01, 0x00, 0x00, 0x00]);
    section(1, &types, &mut out);
    section(2, &imports, &mut out);
    section(3, &funcs, &mut out);
    section(10, &code, &mut out);
    custom_section("linking", &encode_linking(&symbols), &mut out);
    custom_section("reloc.CODE", &encode_reloc_code(3, &relocs), &mut out);
    out
}

/// Emit a relocatable object proving REAL arbitrary-precision BigInt (S3-flagship): `main() -> i32`
/// calls the `logos_rt_bigint_*` ABI to compute `(10¹²)^8 = 10⁹⁶` — a 97-digit value far beyond
/// `i128` — as a chain `from_i64(10¹²)` then `mul(h,h)` three times (squaring), and returns the
/// `to_text` handle. This exercises the exact ABI shape the emitter will call: a scalar → handle
/// (`from_i64`), handle × handle → handle (`mul`, passing intermediate handles through a local), and
/// handle → `Text` handle (`to_text`). The runtime behind it is the real `logicaffeine_base::BigInt`.
pub(crate) fn emit_bigint_object() -> Vec<u8> {
    // Types: 0=(i64)->i32 from_i64, 1=(i32,i32)->i32 mul, 2=(i32)->i32 to_text, 3=()->i32 main.
    let mut types = Vec::new();
    leb_u(4, &mut types);
    types.extend_from_slice(&[0x60, 0x01, I64, 0x01, 0x7f]);
    types.extend_from_slice(&[0x60, 0x02, 0x7f, 0x7f, 0x01, 0x7f]);
    types.extend_from_slice(&[0x60, 0x01, 0x7f, 0x01, 0x7f]);
    types.extend_from_slice(&[0x60, 0x00, 0x01, 0x7f]);

    // Imports: memory + the three runtime functions → func indices 0,1,2.
    let mut imports = Vec::new();
    leb_u(4, &mut imports);
    let import_name = |m: &str, f: &str, out: &mut Vec<u8>| {
        leb_u(m.len() as u64, out);
        out.extend_from_slice(m.as_bytes());
        leb_u(f.len() as u64, out);
        out.extend_from_slice(f.as_bytes());
    };
    import_name("env", "__linear_memory", &mut imports);
    imports.push(0x02);
    imports.push(0x00);
    leb_u(0, &mut imports);
    for (name, ty) in [("logos_rt_bigint_from_i64", 0u64), ("logos_rt_bigint_mul", 1), ("logos_rt_bigint_to_text", 2)] {
        import_name("env", name, &mut imports);
        imports.push(0x00);
        leb_u(ty, &mut imports);
    }
    let (from_i64, mul, to_text, main_idx): (u32, u32, u32, u32) = (0, 1, 2, 3);

    let mut funcs = Vec::new();
    leb_u(1, &mut funcs);
    leb_u(3, &mut funcs); // main : type 3

    // A call records the byte offset of its (padded) function-index LEB within the body, and the
    // linking-table symbol it refers to.
    fn emit_call(body: &mut Vec<u8>, func: u32, sym: u32, offs: &mut Vec<(u32, u32)>) {
        body.push(0x10); // call
        offs.push((body.len() as u32, sym));
        leb_u32_padded(func, body);
    }

    let mut body = Vec::new();
    leb_u(1, &mut body); // 1 local group
    leb_u(1, &mut body); // of 1 local
    body.push(0x7f); // i32 (the running handle)
    let mut calls: Vec<(u32, u32)> = Vec::new();
    body.push(0x42); // i64.const 10^12
    leb_i64(1_000_000_000_000, &mut body);
    emit_call(&mut body, from_i64, 0, &mut calls); // symbol 0 = from_i64
    body.extend_from_slice(&[0x21, 0x00]); // local.set 0
    for _ in 0..3 {
        body.extend_from_slice(&[0x20, 0x00, 0x20, 0x00]); // local.get 0 twice
        emit_call(&mut body, mul, 1, &mut calls); // symbol 1 = mul
        body.extend_from_slice(&[0x21, 0x00]); // local.set 0
    }
    body.extend_from_slice(&[0x20, 0x00]); // local.get 0
    emit_call(&mut body, to_text, 2, &mut calls); // symbol 2 = to_text
    body.push(0x0b); // end

    let mut code = Vec::new();
    leb_u(1, &mut code);
    leb_u(body.len() as u64, &mut code);
    let prefix = code.len() as u32;
    code.extend_from_slice(&body);
    let relocs: Vec<CodeReloc> =
        calls.iter().map(|&(off, sym)| CodeReloc { offset: prefix + off, sym_index: sym }).collect();

    let symbols = vec![
        FuncSymbol { func_index: from_i64, defined_name: None },
        FuncSymbol { func_index: mul, defined_name: None },
        FuncSymbol { func_index: to_text, defined_name: None },
        FuncSymbol { func_index: main_idx, defined_name: Some("main".to_string()) },
    ];

    let mut out = Vec::new();
    out.extend_from_slice(b"\0asm");
    out.extend_from_slice(&[0x01, 0x00, 0x00, 0x00]);
    section(1, &types, &mut out);
    section(2, &imports, &mut out);
    section(3, &funcs, &mut out);
    section(10, &code, &mut out);
    custom_section("linking", &encode_linking(&symbols), &mut out);
    custom_section("reloc.CODE", &encode_reloc_code(3, &relocs), &mut out);
    out
}

/// Encode the `linking` custom section body: metadata version 2 + a `WASM_SYMBOL_TABLE` (id 8) of the
/// object's function symbols.
fn encode_linking(symbols: &[FuncSymbol]) -> Vec<u8> {
    const WASM_SYM_UNDEFINED: u32 = 0x10;
    let mut symtab = Vec::new();
    leb_u(symbols.len() as u64, &mut symtab);
    for s in symbols {
        symtab.push(0x00); // SYMTAB_FUNCTION
        match &s.defined_name {
            Some(name) => {
                leb_u(0, &mut symtab); // flags: defined
                leb_u(u64::from(s.func_index), &mut symtab);
                leb_u(name.len() as u64, &mut symtab);
                symtab.extend_from_slice(name.as_bytes());
            }
            None => {
                leb_u(u64::from(WASM_SYM_UNDEFINED), &mut symtab); // flags: undefined
                leb_u(u64::from(s.func_index), &mut symtab); // import index; name comes from the import
            }
        }
    }
    let mut body = Vec::new();
    leb_u(2, &mut body); // metadata version
    body.push(0x08); // WASM_SYMBOL_TABLE
    leb_u(symtab.len() as u64, &mut body);
    body.extend_from_slice(&symtab);
    body
}

/// Encode a `reloc.CODE` custom section body: the target section index, then one
/// `R_WASM_FUNCTION_INDEX_LEB` (type 0) entry per code function-index reference.
fn encode_reloc_code(section_index: u32, relocs: &[CodeReloc]) -> Vec<u8> {
    const R_WASM_FUNCTION_INDEX_LEB: u8 = 0x00;
    let mut body = Vec::new();
    leb_u(u64::from(section_index), &mut body);
    leb_u(relocs.len() as u64, &mut body);
    for r in relocs {
        body.push(R_WASM_FUNCTION_INDEX_LEB);
        leb_u(u64::from(r.offset), &mut body);
        leb_u(u64::from(r.sym_index), &mut body);
    }
    body
}

/// Emit a relocatable object DEFINING `name` with signature `(params) -> (results)`. This supplies the
/// two ALLOCATOR-SHIM symbols a `#[global_allocator]` references but a `--emit=obj` lib build doesn't
/// materialize (a final cdylib/bin's allocator shim would): the `no_alloc_shim` marker (`trap=false`,
/// a do-nothing body — it is CALLED and must return) and `__rust_alloc_error_handler` (`trap=true`, an
/// `unreachable` body — aborting on allocation failure is the correct wasm behavior). Everything else
/// the runtime references (`handle_error`, `__multi3`, `core::fmt`) comes from the real linked rlibs.
fn emit_shim_object(name: &str, params: &[u8], results: &[u8], trap: bool) -> Vec<u8> {
    let mut types = Vec::new();
    leb_u(1, &mut types);
    types.push(0x60);
    leb_u(params.len() as u64, &mut types);
    types.extend_from_slice(params);
    leb_u(results.len() as u64, &mut types);
    types.extend_from_slice(results);
    let mut funcs = Vec::new();
    leb_u(1, &mut funcs);
    leb_u(0, &mut funcs);
    // Body: 0 locals, then (`unreachable`)? `end`. `unreachable` is stack-polymorphic, so it validates
    // for any result signature; the empty body is only used for the no-result marker.
    let mut body = vec![0x00u8];
    if trap {
        body.push(0x00); // unreachable
    }
    body.push(0x0b); // end
    let mut code = Vec::new();
    leb_u(1, &mut code);
    leb_u(body.len() as u64, &mut code);
    code.extend_from_slice(&body);
    let symbols = vec![FuncSymbol { func_index: 0, defined_name: Some(name.to_string()) }];
    let mut out = Vec::new();
    out.extend_from_slice(b"\0asm");
    out.extend_from_slice(&[0x01, 0x00, 0x00, 0x00]);
    section(1, &types, &mut out);
    section(3, &funcs, &mut out);
    section(10, &code, &mut out);
    custom_section("linking", &encode_linking(&symbols), &mut out);
    out
}

/// Read an unsigned LEB128 at `*i`, advancing `*i`.
fn read_uleb(bytes: &[u8], i: &mut usize) -> u64 {
    let mut r = 0u64;
    let mut s = 0;
    loop {
        let b = bytes[*i];
        *i += 1;
        r |= u64::from(b & 0x7f) << s;
        if b & 0x80 == 0 {
            break;
        }
        s += 7;
    }
    r
}

/// A function import's field name + its signature (valtype bytes), so a shim can be synthesized with
/// the matching type.
struct FuncImportSig {
    name: String,
    params: Vec<u8>,
    results: Vec<u8>,
}

/// The function type signatures declared in an object's type section (valtypes are one byte each).
fn parse_types(object: &[u8]) -> Vec<(Vec<u8>, Vec<u8>)> {
    let mut out = Vec::new();
    if object.len() < 8 {
        return out;
    }
    let mut i = 8;
    while i < object.len() {
        let sid = object[i];
        i += 1;
        let size = read_uleb(object, &mut i) as usize;
        let end = i + size;
        if sid == 1 {
            let mut j = i;
            let count = read_uleb(object, &mut j);
            for _ in 0..count {
                j += 1; // 0x60 functype tag
                let np = read_uleb(object, &mut j) as usize;
                let params = object[j..j + np].to_vec();
                j += np;
                let nr = read_uleb(object, &mut j) as usize;
                let results = object[j..j + nr].to_vec();
                j += nr;
                out.push((params, results));
            }
        }
        i = end;
    }
    out
}

/// The `env` FUNCTION imports of a relocatable object with resolved signatures — scanned for the
/// allocator-shim symbols a `--emit=obj` lib references but doesn't define.
fn function_imports(object: &[u8]) -> Vec<FuncImportSig> {
    let types = parse_types(object);
    let mut out = Vec::new();
    if object.len() < 8 {
        return out;
    }
    let mut i = 8;
    while i < object.len() {
        let sid = object[i];
        i += 1;
        let size = read_uleb(object, &mut i) as usize;
        let end = i + size;
        if sid == 2 {
            let mut j = i;
            let count = read_uleb(object, &mut j);
            for _ in 0..count {
                let ml = read_uleb(object, &mut j) as usize;
                j += ml;
                let fl = read_uleb(object, &mut j) as usize;
                let field = String::from_utf8_lossy(&object[j..j + fl]).to_string();
                j += fl;
                let kind = object[j];
                j += 1;
                match kind {
                    0 => {
                        let ti = read_uleb(object, &mut j) as usize;
                        let (params, results) = types.get(ti).cloned().unwrap_or_default();
                        out.push(FuncImportSig { name: field, params, results });
                    }
                    1 => {
                        j += 1;
                        let flags = object[j];
                        j += 1;
                        read_uleb(object, &mut j);
                        if flags & 1 != 0 {
                            read_uleb(object, &mut j);
                        }
                    }
                    2 => {
                        let flags = object[j];
                        j += 1;
                        read_uleb(object, &mut j);
                        if flags & 1 != 0 {
                            read_uleb(object, &mut j);
                        }
                    }
                    3 => j += 2,
                    _ => {}
                }
            }
        }
        i = end;
    }
    out
}

/// The host triple (`rustc -vV`'s `host:` line) — the directory `rust-lld` lives under.
fn host_triple() -> R<String> {
    let out = Command::new("rustc")
        .arg("-vV")
        .output()
        .map_err(|_| WasmLowerError::Unsupported("rustc not found for the wasm linker"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines()
        .find_map(|l| l.strip_prefix("host: "))
        .map(|s| s.trim().to_string())
        .ok_or(WasmLowerError::Unsupported("could not determine the host triple"))
}

/// The toolchain sysroot (`rustc --print sysroot`).
fn sysroot() -> R<PathBuf> {
    let out = Command::new("rustc")
        .args(["--print", "sysroot"])
        .output()
        .map_err(|_| WasmLowerError::Unsupported("rustc not found for the wasm linker"))?;
    Ok(PathBuf::from(String::from_utf8_lossy(&out.stdout).trim().to_string()))
}

/// The path to the toolchain's `rust-lld` (= `wasm-ld` under `-flavor wasm`).
pub(crate) fn rust_lld_path() -> R<PathBuf> {
    let host = host_triple()?;
    let p = sysroot()?.join("lib/rustlib").join(host).join("bin/rust-lld");
    if p.exists() {
        Ok(p)
    } else {
        Err(WasmLowerError::Unsupported("rust-lld not present in the toolchain"))
    }
}

/// The precompiled `wasm32-unknown-unknown` Rust libraries `rust-lld` needs to resolve everything a
/// runtime object references that isn't its own code: `liballoc` (the alloc-shim marker, `Vec`/`String`
/// + their `handle_error`), `libcore` (`core::fmt` number formatting, slicing, …), `libcompiler_builtins`
/// (the wide-integer / mem intrinsics — `__multi3`, `__udivti3`, `memcpy`). Returned in the SAME order
/// rustc passes them; `lld --gc-sections` pulls in only the members actually referenced. This is
/// exactly how rustc links a wasm binary, so the runtime may use the real `std`/`core`/`alloc`.
fn wasm_runtime_rlibs() -> R<Vec<PathBuf>> {
    let dir = sysroot()?.join("lib/rustlib/wasm32-unknown-unknown/lib");
    let find = |prefix: &str| -> R<PathBuf> {
        let entries = std::fs::read_dir(&dir)
            .map_err(|_| WasmLowerError::Unsupported("wasm32 sysroot lib dir missing"))?;
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with(prefix) && name.ends_with(".rlib") {
                return Ok(entry.path());
            }
        }
        Err(WasmLowerError::Unsupported("a wasm32 sysroot rlib is missing"))
    };
    Ok(vec![find("liballoc-")?, find("libcore-")?, find("libcompiler_builtins-")?])
}

/// Whether the wasm link toolchain (rustc + the `wasm32-unknown-unknown` target + `rust-lld`) is
/// available, so tests can skip cleanly on a host without it rather than fail.
pub(crate) fn toolchain_available() -> bool {
    if rust_lld_path().is_err() {
        return false;
    }
    Command::new("rustc")
        .args(["--print", "target-list"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().any(|l| l == "wasm32-unknown-unknown"))
        .unwrap_or(false)
}

/// Compile a runtime `.rs` source to a `wasm32-unknown-unknown` RELOCATABLE object (`--emit=obj`),
/// returning the object bytes. `dir` is a scratch directory for the intermediate files.
pub(crate) fn build_runtime_object(src: &str, dir: &std::path::Path) -> R<Vec<u8>> {
    std::fs::create_dir_all(dir).map_err(|_| WasmLowerError::Unsupported("cannot create link scratch dir"))?;
    let rs = dir.join("logos_rt.rs");
    let obj = dir.join("logos_rt.o");
    std::fs::write(&rs, src).map_err(|_| WasmLowerError::Unsupported("cannot write runtime source"))?;
    let status = Command::new("rustc")
        .args(["--target", "wasm32-unknown-unknown", "--crate-type=lib", "--emit=obj", "-Copt-level=2"])
        .arg(&rs)
        .arg("-o")
        .arg(&obj)
        .status()
        .map_err(|_| WasmLowerError::Unsupported("failed to invoke rustc for the runtime object"))?;
    if !status.success() {
        return Err(WasmLowerError::Unsupported("rustc failed to build the runtime object"));
    }
    std::fs::read(&obj).map_err(|_| WasmLowerError::Unsupported("cannot read the runtime object"))
}

/// Link relocatable objects into one wasm module with `rust-lld -flavor wasm` against the minimal
/// `wasm32` runtime (a `no_std` + `#[global_allocator]` object): `liballoc`/`libcore`/
/// `libcompiler_builtins` + both synthesized allocator-shim defs. The common case.
pub(crate) fn link_objects(objects: &[&[u8]], dir: &std::path::Path) -> R<Vec<u8>> {
    link_objects_with_rlibs(objects, &wasm_runtime_rlibs()?, true, dir)
}

/// Link `objects` against an explicit `rlibs` set. `emit_handler_shim` supplies the diverging
/// `__rust_alloc_error_handler` (undefined only for a `no_std` + `#[global_allocator]` runtime; a
/// `std` runtime links `libstd`, which DEFINES it — passing `true` there would duplicate it). The
/// no-op `__rust_no_alloc_shim_is_unstable_v2` marker is always synthesized (both runtime kinds
/// reference it). Flags mirror rustc's wasm link line; genuine host `env.*` functions stay imports.
pub(crate) fn link_objects_with_rlibs(
    objects: &[&[u8]],
    rlibs: &[PathBuf],
    emit_handler_shim: bool,
    dir: &std::path::Path,
) -> R<Vec<u8>> {
    std::fs::create_dir_all(dir).map_err(|_| WasmLowerError::Unsupported("cannot create link scratch dir"))?;
    let lld = rust_lld_path()?;

    // The allocator-shim symbols a `--emit=obj` lib leaves undefined (a final cdylib/bin's allocator
    // shim would define them). They live in the compiler's synthetic `___rustc` shim crate: the no-op
    // `__rust_no_alloc_shim_is_unstable_v2` marker (our runtime object imports it) and — for a `no_std`
    // custom-allocator runtime only — `__rust_alloc_error_handler` (`liballoc`'s fallible path imports
    // it, so it's NOT in our objects' imports; derive its name from the marker's crate prefix).
    let mut shim_objs: Vec<Vec<u8>> = Vec::new();
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for obj in objects {
        for imp in function_imports(obj) {
            if imp.name.contains("no_alloc_shim") && seen.insert(imp.name.clone()) {
                shim_objs.push(emit_shim_object(&imp.name, &imp.params, &imp.results, false));
                if emit_handler_shim {
                    if let Some(idx) = imp.name.find("___rustc") {
                        let prefix = &imp.name[..idx + "___rustc".len()];
                        let handler = format!("{prefix}26___rust_alloc_error_handler");
                        if seen.insert(handler.clone()) {
                            shim_objs.push(emit_shim_object(&handler, &[0x7f, 0x7f], &[], true));
                        }
                    }
                }
            }
        }
    }

    let mut obj_paths = Vec::new();
    for (i, obj) in objects.iter().copied().chain(shim_objs.iter().map(|v| v.as_slice())).enumerate() {
        let p = dir.join(format!("obj{i}.o"));
        std::fs::write(&p, obj).map_err(|_| WasmLowerError::Unsupported("cannot write link input"))?;
        obj_paths.push(p);
    }
    let out = dir.join("linked.wasm");
    let mut cmd = Command::new(&lld);
    // rustc's `wasm32-unknown-unknown` binary flags — a 1 MiB shadow stack placed first, the heap/data
    // markers exported, dead-section GC — plus `--export=main` and `--allow-undefined` (host `env.*`
    // functions stay imports). `--no-demangle` keeps symbol names raw.
    cmd.args([
        "-flavor",
        "wasm",
        "--no-entry",
        "--export=main",
        "--export=__heap_base",
        "--export=__data_end",
        "-z",
        "stack-size=1048576",
        "--stack-first",
        "--no-demangle",
        "--gc-sections",
        "--allow-undefined",
    ]);
    for p in &obj_paths {
        cmd.arg(p);
    }
    for rlib in rlibs {
        cmd.arg(rlib);
    }
    cmd.arg("-o").arg(&out);
    let output = cmd.output().map_err(|_| WasmLowerError::Unsupported("failed to invoke rust-lld"))?;
    if !output.success() {
        return Err(WasmLowerError::Unsupported("rust-lld failed to link the module"));
    }
    std::fs::read(&out).map_err(|_| WasmLowerError::Unsupported("cannot read the linked module"))
}

/// Every precompiled `.rlib` in the `wasm32-unknown-unknown` sysroot lib dir — `libstd`, `libdlmalloc`,
/// and their deps included. `rust-lld --gc-sections` pulls in only referenced members, so passing the
/// whole set is robust across toolchain versions; needed when the runtime uses `std` (e.g. real BigInt).
fn wasm_all_sysroot_rlibs() -> R<Vec<PathBuf>> {
    let dir = sysroot()?.join("lib/rustlib/wasm32-unknown-unknown/lib");
    let entries =
        std::fs::read_dir(&dir).map_err(|_| WasmLowerError::Unsupported("wasm32 sysroot lib dir missing"))?;
    let mut rlibs: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().map_or(false, |x| x == "rlib"))
        .collect();
    rlibs.sort();
    Ok(rlibs)
}

trait Success {
    fn success(&self) -> bool;
}
impl Success for std::process::Output {
    fn success(&self) -> bool {
        self.status.success()
    }
}

/// The runtime source for the link smoke test: the `logos_rt_probe` function the probe object calls.
/// `#![no_std]` (no allocator needed yet) with the single panic handler the whole link provides.
#[cfg(test)]
const PROBE_RUNTIME_SRC: &str = r#"#![no_std]
#[panic_handler] fn ph(_: &core::panic::PanicInfo) -> ! { loop {} }
#[no_mangle] pub extern "C" fn logos_rt_probe(x: i64) -> i64 { x.wrapping_add(1) }
"#;

/// The runtime source for the shared-memory proof: writes an `i64` into a static (which `rust-lld`
/// places in the shared linear memory) and returns its address for the emitted code to `load`.
#[cfg(test)]
const MEM_RUNTIME_SRC: &str = r#"#![no_std]
#[panic_handler] fn ph(_: &core::panic::PanicInfo) -> ! { loop {} }
static mut CELL: i64 = 0;
#[no_mangle] pub extern "C" fn logos_rt_write_answer() -> i32 {
    unsafe { let p = core::ptr::addr_of_mut!(CELL); *p = 42; p as i32 }
}
"#;

/// The runtime source for the DYNAMIC-allocation proof: `no_std` + the `alloc` crate + a single-
/// threaded bump `#[global_allocator]` (a wasm module is single-threaded, so `static mut` bump state
/// is race-free; a real `free` — dlmalloc — arrives with the collection runtime). It `Box`-allocates
/// an `i64` and leaks the pointer, proving the allocator links and runs on `wasm32-unknown-unknown`
/// `--emit=obj` and that allocations land in the shared memory our emitted code reads.
#[cfg(test)]
const ALLOC_RUNTIME_SRC: &str = r#"#![no_std]
extern crate alloc;
use core::alloc::{GlobalAlloc, Layout};
#[panic_handler] fn ph(_: &core::panic::PanicInfo) -> ! { loop {} }
struct Bump;
static mut ARENA: [u8; 1 << 20] = [0; 1 << 20];
static mut OFF: usize = 0;
unsafe impl GlobalAlloc for Bump {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let base = core::ptr::addr_of_mut!(ARENA) as *mut u8;
        let off = core::ptr::addr_of!(OFF).read();
        let aligned = (off + l.align() - 1) & !(l.align() - 1);
        core::ptr::addr_of_mut!(OFF).write(aligned + l.size());
        base.add(aligned)
    }
    unsafe fn dealloc(&self, _: *mut u8, _: Layout) {}
}
#[global_allocator] static A: Bump = Bump;
#[no_mangle] pub extern "C" fn logos_rt_alloc_answer() -> i32 {
    alloc::boxed::Box::into_raw(alloc::boxed::Box::new(42i64)) as i32
}
"#;

/// The runtime source for the Text-handle proof (S3b): multiplies two `i64`s into an `i128` (a value
/// that overflows `i64` — the exact "emitted wasm can't, the runtime can"), formats it to a decimal
/// string, and lays it out as a `Text` in the emitter's ABI (`[len@0][cap@4][data_ptr@8][refcount@12]`
/// header + separate bytes), returning the handle. Same bump allocator as `ALLOC_RUNTIME_SRC`.
#[cfg(test)]
const MUL_TEXT_RUNTIME_SRC: &str = r#"#![no_std]
extern crate alloc;
use core::alloc::{GlobalAlloc, Layout};
use alloc::string::ToString;
#[panic_handler] fn ph(_: &core::panic::PanicInfo) -> ! { loop {} }
struct Bump;
static mut ARENA: [u8; 1 << 20] = [0; 1 << 20];
static mut OFF: usize = 0;
unsafe impl GlobalAlloc for Bump {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let base = core::ptr::addr_of_mut!(ARENA) as *mut u8;
        let off = core::ptr::addr_of!(OFF).read();
        let a = (off + l.align() - 1) & !(l.align() - 1);
        core::ptr::addr_of_mut!(OFF).write(a + l.size());
        base.add(a)
    }
    unsafe fn dealloc(&self, _: *mut u8, _: Layout) {}
}
#[global_allocator] static A: Bump = Bump;
#[no_mangle] pub extern "C" fn logos_rt_i64_mul_to_text(a: i64, b: i64) -> i32 {
    let product: i128 = (a as i128) * (b as i128);
    let s = product.to_string();
    let bytes = s.as_bytes();
    let len = bytes.len();
    unsafe {
        let data = alloc::alloc::alloc(Layout::from_size_align_unchecked(len.max(1), 1));
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), data, len);
        let header = alloc::alloc::alloc(Layout::from_size_align_unchecked(16, 4)) as *mut i32;
        *header.add(0) = len as i32;
        *header.add(1) = len as i32;
        *header.add(2) = data as i32;
        *header.add(3) = 0;
        header as i32
    }
}
"#;

/// The REAL BigInt runtime (S3-flagship): a `std` crate using `logicaffeine_base::BigInt` (the SAME
/// arbitrary-precision type the VM uses — no reimplementation). `std::alloc::System` as the global
/// allocator wires `__rust_alloc` to the linked `dlmalloc` (real `free`). Handles are leaked
/// `Box<BigInt>` pointers; `to_text` lays the decimal out as a `Text` in the emitter's ABI. Compiled
/// with `--extern logicaffeine_base` against `base` built for wasm32.
const BIGINT_RUNTIME_SRC: &str = r#"#[global_allocator] static GLOBAL: std::alloc::System = std::alloc::System;
use logicaffeine_base::BigInt;
#[no_mangle] pub extern "C" fn logos_rt_bigint_from_i64(x: i64) -> i32 {
    Box::into_raw(Box::new(BigInt::from_i64(x))) as i32
}
#[no_mangle] pub extern "C" fn logos_rt_alloc(size: i32) -> i32 {
    // A raw 8-aligned block from the runtime's global allocator (dlmalloc), so the emitter's bump
    // allocator can carve a SLAB the runtime owns — the two allocators never overlap in the shared memory.
    unsafe { std::alloc::alloc(std::alloc::Layout::from_size_align_unchecked(size.max(8) as usize, 8)) as i32 }
}
#[no_mangle] pub extern "C" fn logos_rt_bigint_mul(a: i32, b: i32) -> i32 {
    let a = unsafe { &*(a as *const BigInt) };
    let b = unsafe { &*(b as *const BigInt) };
    Box::into_raw(Box::new(a.mul(b))) as i32
}
#[no_mangle] pub extern "C" fn logos_rt_bigint_add(a: i32, b: i32) -> i32 {
    let a = unsafe { &*(a as *const BigInt) };
    let b = unsafe { &*(b as *const BigInt) };
    Box::into_raw(Box::new(a.add(b))) as i32
}
#[no_mangle] pub extern "C" fn logos_rt_bigint_sub(a: i32, b: i32) -> i32 {
    let a = unsafe { &*(a as *const BigInt) };
    let b = unsafe { &*(b as *const BigInt) };
    Box::into_raw(Box::new(a.sub(b))) as i32
}
#[no_mangle] pub extern "C" fn logos_rt_bigint_div(a: i32, b: i32) -> i32 {
    let a = unsafe { &*(a as *const BigInt) };
    let b = unsafe { &*(b as *const BigInt) };
    Box::into_raw(Box::new(a.div_rem(b).expect("division by zero").0)) as i32
}
#[no_mangle] pub extern "C" fn logos_rt_bigint_mod(a: i32, b: i32) -> i32 {
    let a = unsafe { &*(a as *const BigInt) };
    let b = unsafe { &*(b as *const BigInt) };
    Box::into_raw(Box::new(a.div_rem(b).expect("division by zero").1)) as i32
}
#[no_mangle] pub extern "C" fn logos_rt_bigint_pow(base: i32, exp: i64) -> i32 {
    let base = unsafe { &*(base as *const BigInt) };
    Box::into_raw(Box::new(base.pow(exp as u32))) as i32
}
#[no_mangle] pub extern "C" fn logos_rt_bigint_to_text(h: i32) -> i32 {
    let n = unsafe { &*(h as *const BigInt) };
    let bytes = n.to_string().into_bytes();
    let len = bytes.len();
    let data = bytes.leak();
    unsafe {
        let header = std::alloc::alloc(std::alloc::Layout::from_size_align_unchecked(16, 4)) as *mut i32;
        *header.add(0) = len as i32;
        *header.add(1) = len as i32;
        *header.add(2) = data.as_ptr() as i32;
        *header.add(3) = 0;
        header as i32
    }
}
// ---- Complex (numeric tower): an EXACT `Complex { re: Rational, im: Rational }` behind an i32 handle
// (a leaked Box pointer), mirroring the BigInt ABI. `complex(re, im)` takes two integer components.
use logicaffeine_base::{Complex, Rational};
#[no_mangle] pub extern "C" fn logos_rt_complex_from_i64(re: i64, im: i64) -> i32 {
    Box::into_raw(Box::new(Complex::new(Rational::from_i64(re), Rational::from_i64(im)))) as i32
}
#[no_mangle] pub extern "C" fn logos_rt_complex_add(a: i32, b: i32) -> i32 {
    let a = unsafe { &*(a as *const Complex) };
    let b = unsafe { &*(b as *const Complex) };
    Box::into_raw(Box::new(a.add(b))) as i32
}
#[no_mangle] pub extern "C" fn logos_rt_complex_sub(a: i32, b: i32) -> i32 {
    let a = unsafe { &*(a as *const Complex) };
    let b = unsafe { &*(b as *const Complex) };
    Box::into_raw(Box::new(a.sub(b))) as i32
}
#[no_mangle] pub extern "C" fn logos_rt_complex_mul(a: i32, b: i32) -> i32 {
    let a = unsafe { &*(a as *const Complex) };
    let b = unsafe { &*(b as *const Complex) };
    Box::into_raw(Box::new(a.mul(b))) as i32
}
#[no_mangle] pub extern "C" fn logos_rt_complex_to_text(h: i32) -> i32 {
    let c = unsafe { &*(h as *const Complex) };
    let bytes = c.to_string().into_bytes();
    let len = bytes.len();
    let data = bytes.leak();
    unsafe {
        let header = std::alloc::alloc(std::alloc::Layout::from_size_align_unchecked(16, 4)) as *mut i32;
        *header.add(0) = len as i32;
        *header.add(1) = len as i32;
        *header.add(2) = data.as_ptr() as i32;
        *header.add(3) = 0;
        header as i32
    }
}
// ---- Modular (ℤ/nℤ): an exact `Modular { value, modulus }` behind an i32 handle. `modular(v, n)`
// reduces on construction; `+ - *` wrap in the ring (moduli must match — the VM guarantees it).
use logicaffeine_base::Modular;
#[no_mangle] pub extern "C" fn logos_rt_modular_from_i64(v: i64, n: i64) -> i32 {
    Box::into_raw(Box::new(Modular::from_i64(v, n).expect("invalid modulus"))) as i32
}
#[no_mangle] pub extern "C" fn logos_rt_modular_add(a: i32, b: i32) -> i32 {
    let a = unsafe { &*(a as *const Modular) };
    let b = unsafe { &*(b as *const Modular) };
    Box::into_raw(Box::new(a.add(b).expect("modulus mismatch"))) as i32
}
#[no_mangle] pub extern "C" fn logos_rt_modular_sub(a: i32, b: i32) -> i32 {
    let a = unsafe { &*(a as *const Modular) };
    let b = unsafe { &*(b as *const Modular) };
    Box::into_raw(Box::new(a.sub(b).expect("modulus mismatch"))) as i32
}
#[no_mangle] pub extern "C" fn logos_rt_modular_mul(a: i32, b: i32) -> i32 {
    let a = unsafe { &*(a as *const Modular) };
    let b = unsafe { &*(b as *const Modular) };
    Box::into_raw(Box::new(a.mul(b).expect("modulus mismatch"))) as i32
}
#[no_mangle] pub extern "C" fn logos_rt_modular_to_text(h: i32) -> i32 {
    let m = unsafe { &*(h as *const Modular) };
    let bytes = m.to_string().into_bytes();
    let len = bytes.len();
    let data = bytes.leak();
    unsafe {
        let header = std::alloc::alloc(std::alloc::Layout::from_size_align_unchecked(16, 4)) as *mut i32;
        *header.add(0) = len as i32;
        *header.add(1) = len as i32;
        *header.add(2) = data.as_ptr() as i32;
        *header.add(3) = 0;
        header as i32
    }
}
// ---- Decimal (exact base-10): `decimal("19.99")` parses a Text handle (read from the SHARED linear
// memory: len@0, data_ptr@8) into an exact `Decimal`; an Int operand promotes via `from_i64`; `+ - *`
// keep exact scale; `to_text` renders it. (Division/comparison carry a scale/mode — deferred.)
use logicaffeine_base::Decimal;
#[no_mangle] pub extern "C" fn logos_rt_decimal_from_text(h: i32) -> i32 {
    let (len, dp) = unsafe { (*(h as *const i32) as usize, *((h as *const i32).add(2))) };
    let s = unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(dp as *const u8, len)) };
    Box::into_raw(Box::new(Decimal::parse(s.trim()).expect("invalid decimal literal"))) as i32
}
#[no_mangle] pub extern "C" fn logos_rt_decimal_from_i64(x: i64) -> i32 {
    Box::into_raw(Box::new(Decimal::from_i64(x))) as i32
}
#[no_mangle] pub extern "C" fn logos_rt_decimal_add(a: i32, b: i32) -> i32 {
    let a = unsafe { &*(a as *const Decimal) };
    let b = unsafe { &*(b as *const Decimal) };
    Box::into_raw(Box::new(a.add(b))) as i32
}
#[no_mangle] pub extern "C" fn logos_rt_decimal_sub(a: i32, b: i32) -> i32 {
    let a = unsafe { &*(a as *const Decimal) };
    let b = unsafe { &*(b as *const Decimal) };
    Box::into_raw(Box::new(a.sub(b))) as i32
}
#[no_mangle] pub extern "C" fn logos_rt_decimal_mul(a: i32, b: i32) -> i32 {
    let a = unsafe { &*(a as *const Decimal) };
    let b = unsafe { &*(b as *const Decimal) };
    Box::into_raw(Box::new(a.mul(b))) as i32
}
#[no_mangle] pub extern "C" fn logos_rt_decimal_to_text(h: i32) -> i32 {
    let d = unsafe { &*(h as *const Decimal) };
    let bytes = d.to_string().into_bytes();
    let len = bytes.len();
    let data = bytes.leak();
    unsafe {
        let header = std::alloc::alloc(std::alloc::Layout::from_size_align_unchecked(16, 4)) as *mut i32;
        *header.add(0) = len as i32;
        *header.add(1) = len as i32;
        *header.add(2) = data.as_ptr() as i32;
        *header.add(3) = 0;
        header as i32
    }
}
// ---- Money (exact amount + currency): `money(amount, "USD")` where amount is a Decimal handle or an
// Int; the currency is a Text CODE read from shared memory (`currency::by_code`). `+ -` require matching
// currencies; `to_text` renders it. (Division / cross-currency deferred.)
use logicaffeine_base::Money;
unsafe fn logos_rt_read_text(h: i32) -> String {
    let len = *(h as *const i32) as usize;
    let dp = *((h as *const i32).add(2));
    String::from_utf8_lossy(std::slice::from_raw_parts(dp as *const u8, len)).into_owned()
}
#[no_mangle] pub extern "C" fn logos_rt_money_from_decimal(dec: i32, cur: i32) -> i32 {
    let amount = unsafe { &*(dec as *const Decimal) }.clone();
    let code = unsafe { logos_rt_read_text(cur) };
    let c = logicaffeine_base::money::currency::by_code(code.trim()).expect("unknown currency");
    Box::into_raw(Box::new(Money::of(amount, c))) as i32
}
#[no_mangle] pub extern "C" fn logos_rt_money_from_i64(v: i64, cur: i32) -> i32 {
    let code = unsafe { logos_rt_read_text(cur) };
    let c = logicaffeine_base::money::currency::by_code(code.trim()).expect("unknown currency");
    Box::into_raw(Box::new(Money::of(Decimal::from_i64(v), c))) as i32
}
#[no_mangle] pub extern "C" fn logos_rt_money_add(a: i32, b: i32) -> i32 {
    let a = unsafe { &*(a as *const Money) };
    let b = unsafe { &*(b as *const Money) };
    Box::into_raw(Box::new(a.add(b).expect("currency mismatch"))) as i32
}
#[no_mangle] pub extern "C" fn logos_rt_money_sub(a: i32, b: i32) -> i32 {
    let a = unsafe { &*(a as *const Money) };
    let b = unsafe { &*(b as *const Money) };
    Box::into_raw(Box::new(a.sub(b).expect("currency mismatch"))) as i32
}
#[no_mangle] pub extern "C" fn logos_rt_money_to_text(h: i32) -> i32 {
    let m = unsafe { &*(h as *const Money) };
    let bytes = m.to_string().into_bytes();
    let len = bytes.len();
    let data = bytes.leak();
    unsafe {
        let header = std::alloc::alloc(std::alloc::Layout::from_size_align_unchecked(16, 4)) as *mut i32;
        *header.add(0) = len as i32;
        *header.add(1) = len as i32;
        *header.add(2) = data.as_ptr() as i32;
        *header.add(3) = 0;
        header as i32
    }
}
// ---- Quantity (exact magnitude + display unit): `quantity(value, "unit")` and `X in "unit"`
// (`convert`). The magnitude rides the exact rational tower normalized to the SI base; the display
// unit is carried alongside (presentation only). `+ -` keep the left display unit (dimension-checked
// at compile time); `× ÷` combine dimensions and render in the SI/dimension form; `to_text` mirrors
// the interpreter's `QuantityValue::display` byte-for-byte (the arithmetic itself stays in base).
use logicaffeine_base::{Quantity, Unit};
struct Qv { q: Quantity, unit: Unit }
unsafe fn qv_ref(h: i32) -> &'static Qv { &*(h as *const Qv) }
fn qv_box(q: Quantity, unit: Unit) -> i32 { Box::into_raw(Box::new(Qv { q, unit })) as i32 }
#[no_mangle] pub extern "C" fn logos_rt_quantity_of_i64(v: i64, name: i32) -> i32 {
    let nm = unsafe { logos_rt_read_text(name) };
    let unit = logicaffeine_base::quantity::units::by_name(nm.trim()).expect("unknown unit");
    let q = Quantity::of(Rational::from_i64(v), &unit);
    qv_box(q, unit)
}
#[no_mangle] pub extern "C" fn logos_rt_quantity_convert(h: i32, name: i32) -> i32 {
    let a = unsafe { qv_ref(h) };
    let nm = unsafe { logos_rt_read_text(name) };
    let unit = logicaffeine_base::quantity::units::by_name(nm.trim()).expect("unknown unit");
    qv_box(a.q.clone(), unit)
}
#[no_mangle] pub extern "C" fn logos_rt_quantity_add(a: i32, b: i32) -> i32 {
    let a = unsafe { qv_ref(a) };
    let b = unsafe { qv_ref(b) };
    qv_box(a.q.add(&b.q).expect("dimension mismatch"), a.unit.clone())
}
#[no_mangle] pub extern "C" fn logos_rt_quantity_sub(a: i32, b: i32) -> i32 {
    let a = unsafe { qv_ref(a) };
    let b = unsafe { qv_ref(b) };
    qv_box(a.q.sub(&b.q).expect("dimension mismatch"), a.unit.clone())
}
#[no_mangle] pub extern "C" fn logos_rt_quantity_mul(a: i32, b: i32) -> i32 {
    let a = unsafe { qv_ref(a) };
    let b = unsafe { qv_ref(b) };
    let q = a.q.mul(&b.q);
    let u = Unit::linear("", q.dimension(), Rational::one());
    qv_box(q, u)
}
#[no_mangle] pub extern "C" fn logos_rt_quantity_div(a: i32, b: i32) -> i32 {
    let a = unsafe { qv_ref(a) };
    let b = unsafe { qv_ref(b) };
    let q = a.q.div(&b.q).expect("division by a zero quantity");
    let u = Unit::linear("", q.dimension(), Rational::one());
    qv_box(q, u)
}
#[no_mangle] pub extern "C" fn logos_rt_quantity_to_text(h: i32) -> i32 {
    let a = unsafe { qv_ref(h) };
    let magnitude = a.q.in_unit(&a.unit).expect("a display unit shares its quantity's dimension");
    let s = if a.unit.symbol.is_empty() {
        format!("{} {}", magnitude, a.q.dimension())
    } else {
        format!("{} {}", magnitude, a.unit.symbol)
    };
    let bytes = s.into_bytes();
    let len = bytes.len();
    let data = bytes.leak();
    unsafe {
        let header = std::alloc::alloc(std::alloc::Layout::from_size_align_unchecked(16, 4)) as *mut i32;
        *header.add(0) = len as i32;
        *header.add(1) = len as i32;
        *header.add(2) = data.as_ptr() as i32;
        *header.add(3) = 0;
        header as i32
    }
}
// ---- Rational (exact BigInt-backed fraction): `a / b` in a Rational context and `+ - * /` on
// rationals. num/den are BigInt so num/den can exceed i64 without drift (unlike the self-contained
// i64/i64 form); `to_text` renders `num/den` (or `num` when the fraction is a whole number), exactly
// the VM's `Rational::to_string`. Int operands promote via `from_i64`, BigInt operands via `from_bigint`.
#[no_mangle] pub extern "C" fn logos_rt_rational_from_i64(x: i64) -> i32 {
    Box::into_raw(Box::new(Rational::from_i64(x))) as i32
}
#[no_mangle] pub extern "C" fn logos_rt_rational_from_bigint(b: i32) -> i32 {
    let b = unsafe { &*(b as *const BigInt) };
    Box::into_raw(Box::new(Rational::from_bigint(b.clone()))) as i32
}
#[no_mangle] pub extern "C" fn logos_rt_rational_add(a: i32, b: i32) -> i32 {
    let a = unsafe { &*(a as *const Rational) };
    let b = unsafe { &*(b as *const Rational) };
    Box::into_raw(Box::new(a.add(b))) as i32
}
#[no_mangle] pub extern "C" fn logos_rt_rational_sub(a: i32, b: i32) -> i32 {
    let a = unsafe { &*(a as *const Rational) };
    let b = unsafe { &*(b as *const Rational) };
    Box::into_raw(Box::new(a.sub(b))) as i32
}
#[no_mangle] pub extern "C" fn logos_rt_rational_mul(a: i32, b: i32) -> i32 {
    let a = unsafe { &*(a as *const Rational) };
    let b = unsafe { &*(b as *const Rational) };
    Box::into_raw(Box::new(a.mul(b))) as i32
}
#[no_mangle] pub extern "C" fn logos_rt_rational_div(a: i32, b: i32) -> i32 {
    let a = unsafe { &*(a as *const Rational) };
    let b = unsafe { &*(b as *const Rational) };
    Box::into_raw(Box::new(a.div(b).expect("rational division by zero"))) as i32
}
#[no_mangle] pub extern "C" fn logos_rt_rational_to_text(h: i32) -> i32 {
    let r = unsafe { &*(h as *const Rational) };
    let bytes = r.to_string().into_bytes();
    let len = bytes.len();
    let data = bytes.leak();
    unsafe {
        let header = std::alloc::alloc(std::alloc::Layout::from_size_align_unchecked(16, 4)) as *mut i32;
        *header.add(0) = len as i32;
        *header.add(1) = len as i32;
        *header.add(2) = data.as_ptr() as i32;
        *header.add(3) = 0;
        header as i32
    }
}
// Rounding a Rational: `floor`/`ceil`/`round` are EXACT (BigInt num/den, never `as f64`) and return a
// BigInt handle; `abs` stays a Rational handle (`|-7/2| = 7/2`). These match the VM's exact rational
// floor/ceil/round/abs — the WASM `f64` floor/ceil path (a lossy cast) is only for Float/Int operands.
#[no_mangle] pub extern "C" fn logos_rt_rational_floor(h: i32) -> i32 {
    let r = unsafe { &*(h as *const Rational) };
    Box::into_raw(Box::new(r.floor())) as i32
}
#[no_mangle] pub extern "C" fn logos_rt_rational_ceil(h: i32) -> i32 {
    let r = unsafe { &*(h as *const Rational) };
    Box::into_raw(Box::new(r.ceil())) as i32
}
#[no_mangle] pub extern "C" fn logos_rt_rational_round(h: i32) -> i32 {
    let r = unsafe { &*(h as *const Rational) };
    Box::into_raw(Box::new(r.round())) as i32
}
#[no_mangle] pub extern "C" fn logos_rt_rational_abs(h: i32) -> i32 {
    let r = unsafe { &*(h as *const Rational) };
    Box::into_raw(Box::new(r.abs())) as i32
}
// ---- Uuid (RFC 9562, a 16-byte value): `uuid("…")` parses, `uuid_nil`/`uuid_max`/`uuid_dns`/… are
// constants, `uuid_version` reads the version nibble, equality compares the 16 bytes, `to_text` renders
// the canonical lowercase form — all delegating to `base::Uuid` (parse + Display), never reimplemented.
use logicaffeine_base::Uuid;
#[no_mangle] pub extern "C" fn logos_rt_uuid_parse(text: i32) -> i32 {
    let s = unsafe { logos_rt_read_text(text) };
    Box::into_raw(Box::new(Uuid::parse(s.trim()).expect("invalid uuid"))) as i32
}
#[no_mangle] pub extern "C" fn logos_rt_uuid_nil() -> i32 { Box::into_raw(Box::new(Uuid::NIL)) as i32 }
#[no_mangle] pub extern "C" fn logos_rt_uuid_max() -> i32 { Box::into_raw(Box::new(Uuid::MAX)) as i32 }
#[no_mangle] pub extern "C" fn logos_rt_uuid_dns() -> i32 { Box::into_raw(Box::new(Uuid::NAMESPACE_DNS)) as i32 }
#[no_mangle] pub extern "C" fn logos_rt_uuid_url() -> i32 { Box::into_raw(Box::new(Uuid::NAMESPACE_URL)) as i32 }
#[no_mangle] pub extern "C" fn logos_rt_uuid_oid() -> i32 { Box::into_raw(Box::new(Uuid::NAMESPACE_OID)) as i32 }
#[no_mangle] pub extern "C" fn logos_rt_uuid_x500() -> i32 { Box::into_raw(Box::new(Uuid::NAMESPACE_X500)) as i32 }
#[no_mangle] pub extern "C" fn logos_rt_uuid_version(h: i32) -> i64 {
    let u = unsafe { &*(h as *const Uuid) };
    u.version() as i64
}
#[no_mangle] pub extern "C" fn logos_rt_uuid_eq(a: i32, b: i32) -> i32 {
    let a = unsafe { &*(a as *const Uuid) };
    let b = unsafe { &*(b as *const Uuid) };
    (a == b) as i32
}
#[no_mangle] pub extern "C" fn logos_rt_uuid_from_ptr(ptr: i32) -> i32 {
    let mut b = [0u8; 16];
    unsafe { std::ptr::copy_nonoverlapping(ptr as *const u8, b.as_mut_ptr(), 16); }
    Box::into_raw(Box::new(Uuid::from_bytes(b))) as i32
}
#[no_mangle] pub extern "C" fn logos_rt_uuid_to_text(h: i32) -> i32 {
    let u = unsafe { &*(h as *const Uuid) };
    let bytes = u.to_string().into_bytes();
    let len = bytes.len();
    let data = bytes.leak();
    unsafe {
        let header = std::alloc::alloc(std::alloc::Layout::from_size_align_unchecked(16, 4)) as *mut i32;
        *header.add(0) = len as i32;
        *header.add(1) = len as i32;
        *header.add(2) = data.as_ptr() as i32;
        *header.add(3) = 0;
        header as i32
    }
}
// ---- Extended temporal that needs base's calendar logic (LINKER MODE): `format_timestamp(m)` renders
// a Moment as its RFC-3339 UTC string (a `Text` handle), `months_between`/`years_between` count complete
// calendar months/years. Each delegates to the SAME `base::temporal` the VM uses, so bit-identical.
#[no_mangle] pub extern "C" fn logos_rt_format_timestamp(nanos: i64) -> i32 {
    let bytes = logicaffeine_base::temporal::format_rfc3339(nanos).into_bytes();
    let len = bytes.len();
    let data = bytes.leak();
    unsafe {
        let header = std::alloc::alloc(std::alloc::Layout::from_size_align_unchecked(16, 4)) as *mut i32;
        *header.add(0) = len as i32;
        *header.add(1) = len as i32;
        *header.add(2) = data.as_ptr() as i32;
        *header.add(3) = 0;
        header as i32
    }
}
#[no_mangle] pub extern "C" fn logos_rt_months_between(a: i64, b: i64) -> i64 {
    logicaffeine_base::temporal::months_between(a, b)
}
#[no_mangle] pub extern "C" fn logos_rt_years_between(a: i64, b: i64) -> i64 {
    logicaffeine_base::temporal::years_between(a, b)
}
// Zoned temporal: the zone name is an EMITTER-side `Text` handle (`[len@0][cap@4][data_ptr@8]`) in the
// SAME linear memory (valid after linking), so the runtime reads its bytes in place. `in_zone(m, zone)`
// → the local wall-clock `Text` (`…±HH:MM`), `local_instant(m, zone)` → the local-as-UTC `Moment` nanos.
// An unknown zone traps (the VM errors — no output-comparable value). Delegates to `base::temporal`.
unsafe fn logos_rt_zone_str<'a>(zone: i32) -> &'a str {
    let h = zone as *const i32;
    let len = *h as usize;
    let dp = *h.add(2) as usize;
    std::str::from_utf8(std::slice::from_raw_parts(dp as *const u8, len)).expect("zone name is UTF-8")
}
#[no_mangle] pub extern "C" fn logos_rt_in_zone(nanos: i64, zone: i32) -> i32 {
    let s = unsafe { logos_rt_zone_str(zone) };
    let bytes = logicaffeine_base::temporal::format_zoned(nanos, s).expect("unknown time zone").into_bytes();
    let len = bytes.len();
    let data = bytes.leak();
    unsafe {
        let header = std::alloc::alloc(std::alloc::Layout::from_size_align_unchecked(16, 4)) as *mut i32;
        *header.add(0) = len as i32;
        *header.add(1) = len as i32;
        *header.add(2) = data.as_ptr() as i32;
        *header.add(3) = 0;
        header as i32
    }
}
#[no_mangle] pub extern "C" fn logos_rt_local_instant(nanos: i64, zone: i32) -> i64 {
    let s = unsafe { logos_rt_zone_str(zone) };
    logicaffeine_base::temporal::local_instant_nanos(nanos, s).expect("unknown time zone")
}
// ---- SIMD lane vectors (general `base::LanesVal`, LINKER MODE): the SSE byte/word-lane vocabulary a
// Logos codec (hex nibbles, Poly1305 limbs, …) compiles to. A lane vector is a leaked `Box<LanesVal>`
// i32 handle; the constructors read a `Seq` (i64-per-slot) from the SHARED memory, the extractor builds
// one, the ops delegate to the pure-Rust `base::word` spec (bit-identical to the VM — SSSE3 on x86, a
// scalar fallback on wasm32). A width-mismatched op traps (the VM errors). `LanesVal` is `Copy`.
unsafe fn logos_rt_seq_vals(handle: i32) -> Vec<i64> {
    let h = handle as *const i32;
    let len = *h as usize;
    let dp = *h.add(2) as usize;
    (0..len).map(|i| *((dp + i * 8) as *const i64)).collect()
}
unsafe fn logos_rt_lanes_build_seq(vals: &[i64]) -> i32 {
    let n = vals.len();
    let data = std::alloc::alloc(std::alloc::Layout::from_size_align_unchecked((n * 8).max(8), 8)) as *mut i64;
    for (i, v) in vals.iter().enumerate() { *data.add(i) = *v; }
    let header = std::alloc::alloc(std::alloc::Layout::from_size_align_unchecked(16, 4)) as *mut i32;
    *header.add(0) = n as i32;
    *header.add(1) = n as i32;
    *header.add(2) = data as i32;
    *header.add(3) = 0;
    header as i32
}
#[no_mangle] pub extern "C" fn logos_rt_lanes16_from_bytes(seq: i32) -> i32 {
    let bytes: Vec<u8> = unsafe { logos_rt_seq_vals(seq) }.iter().map(|&v| v as u8).collect();
    Box::into_raw(Box::new(logicaffeine_base::LanesVal::L16W8(logicaffeine_base::Lanes16Word8::from_bytes(&bytes)))) as i32
}
#[no_mangle] pub extern "C" fn logos_rt_lanes8_from_words(seq: i32) -> i32 {
    let words: Vec<logicaffeine_base::Word32> = unsafe { logos_rt_seq_vals(seq) }.iter().map(|&v| logicaffeine_base::Word32(v as u32)).collect();
    Box::into_raw(Box::new(logicaffeine_base::LanesVal::L8W32(logicaffeine_base::Lanes8Word32::from_words(&words)))) as i32
}
#[no_mangle] pub extern "C" fn logos_rt_lanes4w64_from_words(seq: i32) -> i32 {
    let words: Vec<logicaffeine_base::Word64> = unsafe { logos_rt_seq_vals(seq) }.iter().map(|&v| logicaffeine_base::Word64(v as u64)).collect();
    Box::into_raw(Box::new(logicaffeine_base::LanesVal::L4W64(logicaffeine_base::Lanes4Word64::from_words(&words)))) as i32
}
#[no_mangle] pub extern "C" fn logos_rt_lanes_splat16(x: i64) -> i32 {
    Box::into_raw(Box::new(logicaffeine_base::LanesVal::L16W8(logicaffeine_base::Lanes16Word8::splat(x as u8)))) as i32
}
#[no_mangle] pub extern "C" fn logos_rt_lanes_splat8(x: i64) -> i32 {
    Box::into_raw(Box::new(logicaffeine_base::LanesVal::L8W32(logicaffeine_base::Lanes8Word32::splat(x as u32)))) as i32
}
#[no_mangle] pub extern "C" fn logos_rt_lanes_to_seq(handle: i32) -> i32 {
    let v = unsafe { *(handle as *const logicaffeine_base::LanesVal) };
    let vals: Vec<i64> = (0..v.lanes()).map(|i| v.lane(i) as i64).collect();
    unsafe { logos_rt_lanes_build_seq(&vals) }
}
#[no_mangle] pub extern "C" fn logos_rt_lanes_shuffle(a: i32, b: i32) -> i32 {
    let (a, b) = unsafe { (*(a as *const logicaffeine_base::LanesVal), *(b as *const logicaffeine_base::LanesVal)) };
    Box::into_raw(Box::new(a.shuffle(b).expect("lane width mismatch"))) as i32
}
#[no_mangle] pub extern "C" fn logos_rt_lanes_interleave_lo(a: i32, b: i32) -> i32 {
    let (a, b) = unsafe { (*(a as *const logicaffeine_base::LanesVal), *(b as *const logicaffeine_base::LanesVal)) };
    Box::into_raw(Box::new(a.interleave_lo(b).expect("lane width mismatch"))) as i32
}
#[no_mangle] pub extern "C" fn logos_rt_lanes_interleave_hi(a: i32, b: i32) -> i32 {
    let (a, b) = unsafe { (*(a as *const logicaffeine_base::LanesVal), *(b as *const logicaffeine_base::LanesVal)) };
    Box::into_raw(Box::new(a.interleave_hi(b).expect("lane width mismatch"))) as i32
}
#[no_mangle] pub extern "C" fn logos_rt_lanes_byte_add(a: i32, b: i32) -> i32 {
    let (a, b) = unsafe { (*(a as *const logicaffeine_base::LanesVal), *(b as *const logicaffeine_base::LanesVal)) };
    Box::into_raw(Box::new(a.byte_add(b).expect("lane width mismatch"))) as i32
}
#[no_mangle] pub extern "C" fn logos_rt_lanes_maddubs(a: i32, b: i32) -> i32 {
    let (a, b) = unsafe { (*(a as *const logicaffeine_base::LanesVal), *(b as *const logicaffeine_base::LanesVal)) };
    Box::into_raw(Box::new(a.maddubs_bytes(b).expect("lane width mismatch"))) as i32
}
#[no_mangle] pub extern "C" fn logos_rt_lanes_packus(a: i32, b: i32) -> i32 {
    let (a, b) = unsafe { (*(a as *const logicaffeine_base::LanesVal), *(b as *const logicaffeine_base::LanesVal)) };
    Box::into_raw(Box::new(a.packus_bytes(b).expect("lane width mismatch"))) as i32
}
#[no_mangle] pub extern "C" fn logos_rt_lanes_shr_bytes(a: i32, n: i64) -> i32 {
    let a = unsafe { *(a as *const logicaffeine_base::LanesVal) };
    Box::into_raw(Box::new(a.shr_bytes(n as u32).expect("lane width mismatch"))) as i32
}
// ---- Money FX (LINKER MODE): `base::money`'s ambient exchange-rate table is a `thread_local`
// (`AMBIENT_RATES`), wasm-safe and persistent across calls in ONE module. `set_rate`/`set_rates` install
// rates (the runtime reads the currency-code `Text` handle + the rate as a `Rational`; the emitter coerces
// Int/Decimal→Rational), `to_currency` converts a `Money` (reads the code Text, looks up the currency,
// `ambient_convert`s). An unknown currency / missing rate traps (the VM errors). `set_rate*` return the
// `Nothing` handle (0). Reads a `Map`'s 16-byte `[key:i64][value:i64]` entries from the shared memory.
#[no_mangle] pub extern "C" fn logos_rt_decimal_to_rational(h: i32) -> i32 {
    Box::into_raw(Box::new(unsafe { &*(h as *const Decimal) }.to_rational())) as i32
}
#[no_mangle] pub extern "C" fn logos_rt_set_rate(code: i32, rate: i32) -> i32 {
    let code = unsafe { logos_rt_zone_str(code) };
    let rate = unsafe { &*(rate as *const Rational) };
    logicaffeine_base::money::set_ambient_rate(code, rate.clone());
    0
}
#[no_mangle] pub extern "C" fn logos_rt_to_currency(money: i32, code: i32) -> i32 {
    let m = unsafe { &*(money as *const Money) };
    let code = unsafe { logos_rt_zone_str(code) };
    let to = logicaffeine_base::money::currency::by_code(code).expect("unknown currency");
    let out = logicaffeine_base::money::ambient_convert(m, to).expect("no exchange rate in scope");
    Box::into_raw(Box::new(out)) as i32
}
unsafe fn logos_rt_map_entries(map: i32) -> Vec<(i64, i64)> {
    let h = map as *const i32;
    let num = *h as usize;
    let dp = *h.add(2) as usize;
    (0..num).map(|i| (*((dp + i * 16) as *const i64), *((dp + i * 16 + 8) as *const i64))).collect()
}
#[no_mangle] pub extern "C" fn logos_rt_set_rates_int(map: i32) -> i32 {
    for (k, v) in unsafe { logos_rt_map_entries(map) } {
        logicaffeine_base::money::set_ambient_rate(unsafe { logos_rt_zone_str(k as i32) }, Rational::from_i64(v));
    }
    0
}
#[no_mangle] pub extern "C" fn logos_rt_set_rates_rational(map: i32) -> i32 {
    for (k, v) in unsafe { logos_rt_map_entries(map) } {
        let rate = unsafe { &*((v as i32) as *const Rational) };
        logicaffeine_base::money::set_ambient_rate(unsafe { logos_rt_zone_str(k as i32) }, rate.clone());
    }
    0
}
#[no_mangle] pub extern "C" fn logos_rt_set_rates_decimal(map: i32) -> i32 {
    for (k, v) in unsafe { logos_rt_map_entries(map) } {
        let dec = unsafe { &*((v as i32) as *const Decimal) };
        logicaffeine_base::money::set_ambient_rate(unsafe { logos_rt_zone_str(k as i32) }, dec.to_rational());
    }
    0
}
// ---- Wire codec (`wireBytes`, LINKER MODE): marshal a value to its wire bytes via the REAL codec
// (`logicaffeine_compile::concurrency::marshal::encode_value_raw` over a reconstructed `RuntimeValue`),
// NOT a reimplementation — bit-identical to the VM's `bytes_to_seq(encode_value_raw(v))`. The emitter
// dispatches on the arg's Kind to the right reconstruction. `gc-sections` strips this whole path (and
// the compiler it pulls in) from any program that does not call `wireBytes`.
fn logos_rt_wire_seq(v: &logicaffeine_compile::interpreter::RuntimeValue) -> i32 {
    let bytes = logicaffeine_compile::concurrency::marshal::encode_value_raw(v).expect("wire encode");
    let vals: Vec<i64> = bytes.iter().map(|&b| b as i64).collect();
    unsafe { logos_rt_lanes_build_seq(&vals) }
}
#[no_mangle] pub extern "C" fn logos_rt_wire_bytes_int(n: i64) -> i32 {
    logos_rt_wire_seq(&logicaffeine_compile::interpreter::RuntimeValue::Int(n))
}
#[no_mangle] pub extern "C" fn logos_rt_wire_bytes_bool(b: i64) -> i32 {
    logos_rt_wire_seq(&logicaffeine_compile::interpreter::RuntimeValue::Bool(b != 0))
}
#[no_mangle] pub extern "C" fn logos_rt_wire_bytes_float(f: f64) -> i32 {
    logos_rt_wire_seq(&logicaffeine_compile::interpreter::RuntimeValue::Float(f))
}
#[no_mangle] pub extern "C" fn logos_rt_wire_bytes_text(handle: i32) -> i32 {
    let s = unsafe { logos_rt_zone_str(handle) }.to_string();
    logos_rt_wire_seq(&logicaffeine_compile::interpreter::RuntimeValue::Text(std::rc::Rc::new(s)))
}
// ---- Wire INPUT + received-code eval (LINKER MODE, over the REAL codec): `readWireProgram` decodes a
// host-supplied frame into a leaked `Box<RuntimeValue>` — Kind::Dynamic, the ONE boxed value in the AOT,
// because a wire program's type is only known at runtime. `dynamic_to_text` renders it (`to_display_string`)
// and `run_accepted` sandbox-evals a wire-received SHIPPED function through the acceptance contract.
#[no_mangle] pub extern "C" fn logos_rt_read_wire_program(buf: i32, len: i32) -> i32 {
    let bytes = unsafe { std::slice::from_raw_parts(buf as *const u8, len as usize) };
    let v = logicaffeine_compile::concurrency::marshal::decode_value_raw(bytes).expect("malformed wire program");
    Box::into_raw(Box::new(v)) as i32
}
#[no_mangle] pub extern "C" fn logos_rt_dynamic_to_text(h: i32) -> i32 {
    let v = unsafe { &*(h as *const logicaffeine_compile::interpreter::RuntimeValue) };
    let bytes = v.to_display_string().into_bytes();
    let len = bytes.len();
    let data = bytes.leak();
    unsafe {
        let header = std::alloc::alloc(std::alloc::Layout::from_size_align_unchecked(16, 4)) as *mut i32;
        *header.add(0) = len as i32;
        *header.add(1) = len as i32;
        *header.add(2) = data.as_ptr() as i32;
        *header.add(3) = 0;
        header as i32
    }
}
#[no_mangle] pub extern "C" fn logos_rt_run_accepted(fn_h: i32, arg: i64, lo: i64, hi: i64) -> i64 {
    let f = unsafe { &*(fn_h as *const logicaffeine_compile::interpreter::RuntimeValue) };
    logicaffeine_compile::semantics::acceptance::AcceptanceContract::new(lo, hi)
        .apply(f, arg)
        .expect("run_accepted: the acceptance contract refused this function/argument")
}
// ---- Calendar span arithmetic: `Moment + <span>` / `Date + <span>` (civil, months clamp end-of-month
// and respect leap years, the time-of-day rides along). `moment_add_span` delegates to the SAME
// `base::temporal` the VM uses; `date_add_span` reuses it via a midnight days↔nanos conversion.
#[no_mangle] pub extern "C" fn logos_rt_moment_add_span(nanos: i64, months: i32, days: i32) -> i64 {
    let dt = logicaffeine_base::temporal::civil_from_unix_nanos(nanos);
    let shifted = logicaffeine_base::temporal::add_span(dt, months as i64, days as i64);
    logicaffeine_base::temporal::unix_nanos_from_civil(shifted)
}
// `date_add_span` is the VM's OWN inline civil-date logic (NOT base::add_span, which the moment path
// uses) — replicated VERBATIM so `Date + <span>` is bit-identical to the interpreter, including the
// end-of-month clamp (Jan 31 + 1 month = Feb 28/29).
fn logos_rt_days_in_month(year: i32, month: i32) -> i32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => if (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0) { 29 } else { 28 },
        _ => 30,
    }
}
#[no_mangle] pub extern "C" fn logos_rt_date_add_span(days_since_epoch: i32, months: i32, days: i32) -> i32 {
    let z = days_since_epoch + 719468;
    let era = if z >= 0 { z / 146097 } else { (z - 146096) / 146097 };
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i32 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let mut year = y + if m <= 2 { 1 } else { 0 };
    let mut month = m as i32;
    let mut day = d as i32;
    let total_months = year.wrapping_mul(12).wrapping_add(month - 1).wrapping_add(months);
    year = total_months / 12;
    month = total_months % 12 + 1;
    if month <= 0 {
        month += 12;
        year -= 1;
    }
    let dim = logos_rt_days_in_month(year, month);
    if day > dim {
        day = dim;
    }
    let yp = year - if month <= 2 { 1 } else { 0 };
    let era2 = if yp >= 0 { yp / 400 } else { (yp - 399) / 400 };
    let yoe2 = (yp - era2 * 400) as u32;
    let mp2 = if month > 2 { month as u32 - 3 } else { month as u32 + 9 };
    let doy2 = (153 * mp2 + 2) / 5 + day as u32 - 1;
    let doe2 = yoe2 * 365 + yoe2 / 4 - yoe2 / 100 + doy2;
    let result = era2 * 146097 + doe2 as i32 - 719468;
    result.wrapping_add(days)
}
// ---- SHA-1 SHA-NI lane intrinsics: `sha1rnds4`/`sha1msg1`/`sha1msg2`/`sha1nexte` over a 128-bit
// `Lanes4Word32` (a 16-byte `[u32; 4]` block in shared memory). The emitter builds/reads the lane blocks
// inline; only these four ops (the actual SHA-1 rounds) delegate to the SAME `base` software spec the VM
// uses — so the compiled Logos SHA-1 (and thus `uuid_v3`/`uuid_v5`) is bit-identical to the interpreter.
use logicaffeine_base::Lanes4Word32;
#[no_mangle] pub extern "C" fn logos_rt_sha1rnds4(abcd: i32, msg: i32, func: i64) -> i32 {
    let a = unsafe { *(abcd as *const Lanes4Word32) };
    let m = unsafe { *(msg as *const Lanes4Word32) };
    Box::into_raw(Box::new(a.sha1rnds4(m, func as u32))) as i32
}
#[no_mangle] pub extern "C" fn logos_rt_sha1msg1(a: i32, b: i32) -> i32 {
    let a = unsafe { *(a as *const Lanes4Word32) };
    let b = unsafe { *(b as *const Lanes4Word32) };
    Box::into_raw(Box::new(a.sha1msg1(b))) as i32
}
#[no_mangle] pub extern "C" fn logos_rt_sha1msg2(a: i32, b: i32) -> i32 {
    let a = unsafe { *(a as *const Lanes4Word32) };
    let b = unsafe { *(b as *const Lanes4Word32) };
    Box::into_raw(Box::new(a.sha1msg2(b))) as i32
}
#[no_mangle] pub extern "C" fn logos_rt_sha1nexte(a: i32, b: i32) -> i32 {
    let a = unsafe { *(a as *const Lanes4Word32) };
    let b = unsafe { *(b as *const Lanes4Word32) };
    Box::into_raw(Box::new(a.sha1nexte(b))) as i32
}
// Lane-wise wrapping add of two lane vectors (the `st + abcdSave` fold at the end of a SHA-1 block).
#[no_mangle] pub extern "C" fn logos_rt_lanes4_add(a: i32, b: i32) -> i32 {
    let a = unsafe { *(a as *const Lanes4Word32) };
    let b = unsafe { *(b as *const Lanes4Word32) };
    Box::into_raw(Box::new(a + b)) as i32
}
// Lane-wise `xor` (the `m0 xor m2` message-schedule fold).
#[no_mangle] pub extern "C" fn logos_rt_lanes4_xor(a: i32, b: i32) -> i32 {
    let a = unsafe { *(a as *const Lanes4Word32) };
    let b = unsafe { *(b as *const Lanes4Word32) };
    Box::into_raw(Box::new(a ^ b)) as i32
}
"#;

/// Build the real-BigInt runtime object: `cargo build` `logicaffeine_base` for `wasm32-unknown-unknown`
/// (into `dir`), locate the produced `base` + `bumpalo` rlibs, then `rustc --emit=obj` the
/// [`BIGINT_RUNTIME_SRC`] against them. Returns `(runtime object bytes, [base.rlib, bumpalo.rlib])` —
/// the extra archives the std link needs beyond the sysroot set.
fn build_bigint_runtime(dir: &std::path::Path) -> R<(Vec<u8>, Vec<PathBuf>)> {
    std::fs::create_dir_all(dir).map_err(|_| WasmLowerError::Unsupported("cannot create link scratch dir"))?;
    // Build `logicaffeine_compile` for wasm32 in a SHARED target dir — built ONCE across all linked tests
    // (cargo's own build lock serializes concurrent callers), NOT per-test. It transitively includes
    // `logicaffeine_base` (the numeric-tower/uuid/temporal/lanes/money runtime types) AND the `marshal`
    // codec the `wireBytes` runtime fns call. `rust-lld --gc-sections --export=main` then strips the entire
    // compiler from any program that does not reference a wire builtin, so a non-wire module stays small.
    let shared = std::env::temp_dir().join("logos_wire_compile_shared");
    let ok = Command::new("cargo")
        .args(["build", "-p", "logicaffeine-compile", "--target", "wasm32-unknown-unknown"])
        .arg("--target-dir")
        .arg(&shared)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .status()
        .map_err(|_| WasmLowerError::Unsupported("cargo not found"))?
        .success();
    if !ok {
        return Err(WasmLowerError::Unsupported("cargo failed to build logicaffeine-compile for wasm32"));
    }
    let wasm_deps = shared.join("wasm32-unknown-unknown/debug/deps");
    let host_deps = shared.join("debug/deps"); // host-compiled proc-macros (serde_derive, …) rustc must resolve
    let find = |prefix: &str| -> R<PathBuf> {
        // The NEWEST matching rlib (an incremental rebuild can leave stale ones with different hashes).
        let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
        for e in std::fs::read_dir(&wasm_deps)
            .map_err(|_| WasmLowerError::Unsupported("compile wasm32 deps dir missing"))?
            .flatten()
        {
            let n = e.file_name();
            let n = n.to_string_lossy();
            if n.starts_with(prefix) && n.ends_with(".rlib") {
                let t = e.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
                if best.as_ref().is_none_or(|(bt, _)| t >= *bt) {
                    best = Some((t, e.path()));
                }
            }
        }
        best.map(|(_, p)| p).ok_or(WasmLowerError::Unsupported("a compile-crate wasm32 rlib is missing"))
    };
    let base = find("liblogicaffeine_base-")?;
    let compile = find("liblogicaffeine_compile-")?;
    let rs = dir.join("bigint_rt.rs");
    let obj = dir.join("bigint_rt.o");
    std::fs::write(&rs, BIGINT_RUNTIME_SRC).map_err(|_| WasmLowerError::Unsupported("cannot write runtime source"))?;
    let ok = Command::new("rustc")
        .args(["--target", "wasm32-unknown-unknown", "--edition", "2021", "--crate-type=lib", "--emit=obj", "-Copt-level=2"])
        .arg("--extern")
        .arg(format!("logicaffeine_base={}", base.display()))
        .arg("--extern")
        .arg(format!("logicaffeine_compile={}", compile.display()))
        .arg("-L")
        .arg(&wasm_deps)
        .arg("-L")
        .arg(&host_deps)
        .arg(&rs)
        .arg("-o")
        .arg(&obj)
        .status()
        .map_err(|_| WasmLowerError::Unsupported("failed to invoke rustc for the bigint+wire runtime"))?
        .success();
    if !ok {
        return Err(WasmLowerError::Unsupported("rustc failed to build the bigint+wire runtime object"));
    }
    let bytes = std::fs::read(&obj).map_err(|_| WasmLowerError::Unsupported("cannot read the bigint runtime object"))?;
    // ALL wasm32 rlibs are offered to the linker; `--gc-sections` pulls in only the members `main`
    // transitively references, so a program that never calls a wire builtin never embeds the compiler.
    let mut rlibs = Vec::new();
    for e in std::fs::read_dir(&wasm_deps)
        .map_err(|_| WasmLowerError::Unsupported("compile wasm32 deps dir missing"))?
        .flatten()
    {
        let n = e.file_name();
        if n.to_string_lossy().ends_with(".rlib") {
            rlibs.push(e.path());
        }
    }
    Ok((bytes, rlibs))
}

/// The prebuilt base-`BigInt` runtime object + its extra rlibs, built ONCE per process (the wasm32
/// `base` build + `rustc --emit=obj` of [`BIGINT_RUNTIME_SRC`] + the whole sysroot rlib set) and
/// reused for every linked program, so the amortized per-program cost is just the `rust-lld` link.
///
/// A PROCESS-WIDE mutex (not a `thread_local`): the linked differential tests run on parallel threads,
/// and `build_bigint_runtime` writes into a shared `--target-dir`. If each thread built independently,
/// one thread's `rustc --extern <rlib>` could read the rlib path while another thread's concurrent
/// `cargo build` is mid-rewrite of that exact file (cargo's own lock covers the build but not the
/// downstream `find`→`rustc` window) — surfacing as "extern location does not exist". Holding this lock
/// across the whole build serializes all callers: the first builds, the rest wait and reuse the cache.
static BIGINT_RUNTIME: std::sync::Mutex<Option<(Vec<u8>, Vec<PathBuf>)>> = std::sync::Mutex::new(None);

/// Link a relocatable Logos program object (from [`super::reloc::module_to_relocatable`] applied to an
/// [`super::module::assemble_program_linked`] module) against the REAL `logicaffeine_base::BigInt`
/// runtime, producing one self-standing `.wasm`. The runtime object + rlibs are built once and cached;
/// each call then only runs `rust-lld`. TOOLCHAIN-DEPENDENT (builds `base` for wasm32) — errors if the
/// toolchain or a base wasm32 build is unavailable, so the caller can fall back or surface the reason.
pub(crate) fn link_relocatable_bigint(relocatable: &[u8]) -> R<Vec<u8>> {
    let dir = std::env::temp_dir().join("logos_wasm_linked_bigint");
    let (runtime, rlibs) = {
        // Hold the lock across the build so exactly ONE thread ever runs cargo/rustc against the shared
        // target dir; the rest block here, then clone the cached result. A poisoned lock (a prior build
        // panicked) is recoverable — the cache is either `None` (rebuild) or a finished value (reuse).
        let mut guard = BIGINT_RUNTIME.lock().unwrap_or_else(|p| p.into_inner());
        if guard.is_none() {
            let (rt, mut rlibs) = build_bigint_runtime(&dir)?;
            rlibs.extend(wasm_all_sysroot_rlibs()?);
            *guard = Some((rt, rlibs));
        }
        guard.as_ref().expect("runtime cache populated above").clone()
    };
    // libstd DEFINES the OOM handler, so no handler shim (a shim would duplicate it).
    link_objects_with_rlibs(&[relocatable, &runtime], &rlibs, false, &dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The foundational link proof: our hand-emitted relocatable object (`main` calls the undefined
    /// `logos_rt_probe`) links against the Rust-compiled runtime object via `rust-lld`, and the linked
    /// module runs — `main()` returns `probe(41) = 42`. Skips cleanly if the wasm toolchain is absent.
    #[test]
    fn probe_object_links_against_runtime_and_runs() {
        if !toolchain_available() {
            eprintln!("SKIP probe_object_links_against_runtime_and_runs: wasm link toolchain unavailable");
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
        let main = instance.get_typed_func::<(), i64>(&store, "main").expect("main export");
        assert_eq!(main.call(&mut store, ()).expect("main runs"), 42, "linked runtime call must return probe(41)");
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
        let main = instance.get_typed_func::<(), i64>(&store, "main").expect("main export");
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
            eprintln!("SKIP emitted_code_reads_runtime_heap_allocation: wasm link toolchain unavailable");
            return;
        }
        let dir = std::env::temp_dir().join("logos_wasm_link_alloc");
        let runtime = build_runtime_object(ALLOC_RUNTIME_SRC, &dir).expect("build allocator runtime object");
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
        let main = instance.get_typed_func::<(), i64>(&store, "main").expect("main export");
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
            eprintln!("SKIP runtime_returns_a_text_handle_our_side_reads: wasm link toolchain unavailable");
            return;
        }
        let dir = std::env::temp_dir().join("logos_wasm_link_text");
        let runtime = build_runtime_object(MUL_TEXT_RUNTIME_SRC, &dir).expect("build mul-text runtime object");
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
        let main = instance.get_typed_func::<(), i32>(&store, "main").expect("main export");
        let handle = main.call(&mut store, ()).expect("main runs") as usize;

        // Read the Text `[len][cap][data_ptr]` header + bytes out of the shared linear memory.
        let mem = instance.get_memory(&store, "memory").expect("exported memory");
        let data = mem.data(&store);
        let len = i32::from_le_bytes(data[handle..handle + 4].try_into().unwrap()) as usize;
        let dptr = i32::from_le_bytes(data[handle + 8..handle + 12].try_into().unwrap()) as usize;
        let text = std::str::from_utf8(&data[dptr..dptr + len]).expect("utf8 Text bytes");
        assert_eq!(text, "1000000000000000000000000", "the runtime-built Text handle must hold 10^24");
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
            eprintln!("SKIP runtime_computes_real_base_bigint_beyond_i128: wasm link toolchain unavailable");
            return;
        }
        let dir = std::env::temp_dir().join("logos_wasm_link_bigint");
        let (runtime, mut rlibs) = match build_bigint_runtime(&dir) {
            Ok(v) => v,
            Err(_) => {
                eprintln!("SKIP runtime_computes_real_base_bigint_beyond_i128: base wasm32 build unavailable");
                return;
            }
        };
        // Link against base/bumpalo first, then the whole sysroot rlib set (libstd/dlmalloc/…). std
        // defines the OOM handler, so `emit_handler_shim = false` (a shim would duplicate it).
        rlibs.extend(wasm_all_sysroot_rlibs().expect("sysroot rlibs"));
        let program = emit_bigint_object();
        let linked = link_objects_with_rlibs(&[&program, &runtime], &rlibs, false, &dir).expect("link bigint program");

        let engine = wasmi::Engine::default();
        let module = wasmi::Module::new(&engine, &linked[..]).expect("linked module is valid wasm");
        let mut store = wasmi::Store::new(&engine, ());
        let instance = wasmi::Linker::new(&engine)
            .instantiate(&mut store, &module)
            .expect("instantiate linked module")
            .start(&mut store)
            .expect("start linked module");
        let main = instance.get_typed_func::<(), i32>(&store, "main").expect("main export");
        let handle = main.call(&mut store, ()).expect("main runs") as usize;

        let mem = instance.get_memory(&store, "memory").expect("exported memory");
        let data = mem.data(&store);
        let len = i32::from_le_bytes(data[handle..handle + 4].try_into().unwrap()) as usize;
        let dptr = i32::from_le_bytes(data[handle + 8..handle + 12].try_into().unwrap()) as usize;
        let text = std::str::from_utf8(&data[dptr..dptr + len]).expect("utf8 Text bytes");
        let expected = format!("1{}", "0".repeat(96)); // 10^96
        assert_eq!(text, expected, "real base BigInt must compute 10^96 exactly (far beyond i128)");
    }

    /// Instantiate a linked module and return everything it printed via `print_text` — the only sink a
    /// `Show <BigInt Text>` uses — joined by newlines and read out of the shared memory exactly as the
    /// AOT host does. After linking, `env.print_text` is the module's only remaining undefined import
    /// (the `logos_rt_bigint_*` symbols + `__linear_memory` are resolved by `rust-lld`).
    #[cfg(test)]
    thread_local! {
        /// The wire frame the `read_wire_frame` host serves — a `readWireProgram`/`run_accepted` test
        /// installs a known-value frame here (via `encode_value_raw`) before running the linked module.
        static WIRE_FRAME: std::cell::RefCell<Vec<u8>> = const { std::cell::RefCell::new(Vec::new()) };
    }

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
        l.func_wrap("env", "print_i64", |c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, v: i64| {
            c.data().borrow_mut().push(v.to_string());
        })
        .unwrap();
        // A `Show` of a Bool (e.g. a `uuid == uuid` result) renders lowercase `true`/`false`, matching the
        // VM's `RuntimeValue::Bool` display.
        l.func_wrap("env", "print_bool", |c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, v: i32| {
            c.data().borrow_mut().push(if v != 0 { "true" } else { "false" }.to_string());
        })
        .unwrap();
        l.func_wrap("env", "print_text", |c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, h: i32| {
            let mem = c.get_export("memory").unwrap().into_memory().unwrap();
            let d = mem.data(&c);
            let h = h as usize;
            let len = i32::from_le_bytes(d[h..h + 4].try_into().unwrap()) as usize;
            let dp = i32::from_le_bytes(d[h + 8..h + 12].try_into().unwrap()) as usize;
            c.data().borrow_mut().push(String::from_utf8_lossy(&d[dp..dp + len]).to_string());
        })
        .unwrap();
        // A whole `Seq of Int` (e.g. `uuid_bytes(u)`): `[n, …]` — each 8-byte slot a signed decimal.
        l.func_wrap("env", "print_seq_i64", |c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, h: i32| {
            let mem = c.get_export("memory").unwrap().into_memory().unwrap();
            let d = mem.data(&c);
            let h = h as usize;
            let len = i32::from_le_bytes(d[h..h + 4].try_into().unwrap()) as usize;
            let dp = i32::from_le_bytes(d[h + 8..h + 12].try_into().unwrap()) as usize;
            let items: Vec<String> = (0..len)
                .map(|i| i64::from_le_bytes(d[dp + i * 8..dp + i * 8 + 8].try_into().unwrap()).to_string())
                .collect();
            c.data().borrow_mut().push(format!("[{}]", items.join(", ")));
        })
        .unwrap();
        // A `Moment` (nanos-since-epoch) formats via the VM's own display (for `Moment ± Span` results).
        l.func_wrap("env", "print_moment", |c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, nanos: i64| {
            c.data().borrow_mut().push(crate::interpreter::RuntimeValue::Moment(nanos).to_display_string());
        })
        .unwrap();
        // A whole `Seq of Word32` (e.g. `seqOfLanes4W32(v)`): `[u, …]` — each 8-byte slot's low word as an
        // UNSIGNED decimal, matching the VM's `RuntimeValue::List` of `Word`.
        l.func_wrap("env", "print_seq_word32", |c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, h: i32| {
            let mem = c.get_export("memory").unwrap().into_memory().unwrap();
            let d = mem.data(&c);
            let h = h as usize;
            let len = i32::from_le_bytes(d[h..h + 4].try_into().unwrap()) as usize;
            let dp = i32::from_le_bytes(d[h + 8..h + 12].try_into().unwrap()) as usize;
            let items: Vec<String> = (0..len)
                .map(|i| u32::from_le_bytes(d[dp + i * 8..dp + i * 8 + 4].try_into().unwrap()).to_string())
                .collect();
            c.data().borrow_mut().push(format!("[{}]", items.join(", ")));
        })
        .unwrap();
        // `parse_timestamp(text) -> Moment nanos` via the same RFC-3339 parser the VM uses.
        l.func_wrap("env", "parse_timestamp", |c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, h: i32| -> i64 {
            let mem = c.get_export("memory").unwrap().into_memory().unwrap();
            let d = mem.data(&c);
            let h = h as usize;
            let len = i32::from_le_bytes(d[h..h + 4].try_into().unwrap()) as usize;
            let dp = i32::from_le_bytes(d[h + 8..h + 12].try_into().unwrap()) as usize;
            let s = std::str::from_utf8(&d[dp..dp + len]).unwrap().trim();
            logicaffeine_base::temporal::parse_rfc3339(s).expect("valid RFC 3339 timestamp")
        })
        .unwrap();
        // `read_wire_frame(buf, max) -> len` — the embedder-side stdin frame source: write the wire frame
        // the test installed in `WIRE_FRAME` into the module's memory at `buf`, return its byte length.
        l.func_wrap("env", "read_wire_frame", |mut c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, buf: i32, max: i32| -> i32 {
            let frame = WIRE_FRAME.with(|f| f.borrow().clone());
            let n = frame.len().min(max as usize);
            let mem = c.get_export("memory").unwrap().into_memory().unwrap();
            let d = mem.data_mut(&mut c);
            d[buf as usize..buf as usize + n].copy_from_slice(&frame[..n]);
            n as i32
        })
        .unwrap();
        let instance =
            l.instantiate(&mut store, &module).expect("instantiate linked module").start(&mut store).expect("start");
        instance.get_typed_func::<(), ()>(&store, "main").expect("main export").call(&mut store, ()).expect("main runs");
        let lines = out.borrow().clone();
        lines.join("\n")
    }

    /// A tiny deterministic LCG step (no `Math.random`/`Date` — reproducible so a failure repro's exactly).
    fn lcg(state: &mut u64) -> u32 {
        *state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (*state >> 33) as u32
    }
    /// A random arithmetic LEAF: a small int, an OVERFLOWING power (drives the BigInt path), or a larger int.
    fn fuzz_leaf(state: &mut u64) -> String {
        match lcg(state) % 3 {
            0 => format!("{}", 1 + lcg(state) % 90),
            1 => format!("({} to the power of {})", 2 + lcg(state) % 4, 10 + lcg(state) % 30),
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

        let compile_reloc = |source: &str| -> Result<Vec<u8>, String> {
            crate::ui_bridge::with_parsed_program(source, |parsed, interner| -> Result<Vec<u8>, String> {
                let (stmts, types, policies) = parsed?;
                let oracle = crate::optimize::oracle_analyze_with(stmts, interner);
                let program = crate::vm::Compiler::compile_with_oracle(stmts, interner, Some(types), Some(oracle))
                    .map_err(|e| format!("compile: {e}"))?;
                let module = super::super::module::assemble_program_linked(&program, &policies, interner)
                    .map_err(|e| format!("assemble: {e}"))?;
                super::super::reloc::module_to_relocatable(&module).map_err(|e| format!("reloc: {e}"))
            })
        };

        let mut state: u64 = 0x2545_F491_4F6C_DD1D;
        let mut checked = 0;
        for _ in 0..40 {
            let src = format!("## Main\nShow {}.\n", fuzz_expr(&mut state, 3));
            let vm = crate::compile::vm_outcome(&src);
            if vm.error.is_some() {
                continue; // the VM itself rejects this shape (not expected here) — skip
            }
            let reloc = match compile_reloc(&src) {
                Ok(r) => r,
                Err(_) => continue, // the linked emitter SOUNDLY declines this shape — a refusal, not a divergence
            };
            let linked = link_objects_with_rlibs(&[&reloc, &runtime], &rlibs, false, &dir)
                .unwrap_or_else(|e| panic!("link `{src}`: {e}"));
            let got = run_linked_capturing_text(&linked);
            assert_eq!(got.trim(), vm.output.trim(), "linked wasm DIVERGED from the VM on:\n{src}");
            checked += 1;
        }
        assert!(checked >= 20, "fuzz exercised too few programs ({checked}/40) — the generator or skip logic is off");
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
                eprintln!("SKIP linked_complex_arithmetic_matches_the_vm: base wasm32 build unavailable");
                return;
            }
        };
        rlibs.extend(wasm_all_sysroot_rlibs().expect("sysroot rlibs"));
        let compile_reloc = |source: &str| -> Result<Vec<u8>, String> {
            crate::ui_bridge::with_parsed_program(source, |parsed, interner| -> Result<Vec<u8>, String> {
                let (stmts, types, policies) = parsed?;
                let oracle = crate::optimize::oracle_analyze_with(stmts, interner);
                let program = crate::vm::Compiler::compile_with_oracle(stmts, interner, Some(types), Some(oracle))
                    .map_err(|e| format!("compile: {e}"))?;
                let module = super::super::module::assemble_program_linked(&program, &policies, interner)
                    .map_err(|e| format!("assemble: {e}"))?;
                super::super::reloc::module_to_relocatable(&module).map_err(|e| format!("reloc: {e}"))
            })
        };
        for src in [
            "## Main\n    Let i be complex(0, 1).\n    Show i * i.\n",                                // -1
            "## Main\n    Let a be complex(2, 3).\n    Let b be complex(1, 0 - 1).\n    Show a + b.\n", // 3+2i
            "## Main\n    Let a be complex(1, 1).\n    Let b be complex(1, 0 - 1).\n    Show a * b.\n", // 2
            "## Main\n    Let z be complex(2, 3).\n    Show z.\n",                                     // 2+3i
            "## Main\n    Let a be complex(5, 2).\n    Let b be complex(3, 4).\n    Show a - b.\n",     // 2-2i
        ] {
            let vm = crate::compile::vm_outcome(src);
            assert!(vm.error.is_none(), "the VM itself errored on `{src}`: {:?}", vm.error);
            let reloc = compile_reloc(src).unwrap_or_else(|e| panic!("compile-reloc `{src}`: {e}"));
            let linked = link_objects_with_rlibs(&[&reloc, &runtime], &rlibs, false, &dir)
                .unwrap_or_else(|e| panic!("link `{src}`: {e}"));
            let got = run_linked_capturing_text(&linked);
            assert_eq!(got.trim(), vm.output.trim(), "linked Complex DIVERGED from the VM on:\n{src}");
        }
    }

    /// LINKED `Modular` (numeric-tower ℤ/nℤ): `modular(v, n)` reduces on construction; `+ - *` wrap in
    /// the ring — exact, byte-identical to the VM. `modular(10,7) = 3 (mod 7)`, `(5+4) mod 7 = 2`,
    /// `(5*4) mod 7 = 6`, `(3-5) mod 7 = 5`. Skips if the toolchain is unavailable.
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
                eprintln!("SKIP linked_modular_arithmetic_matches_the_vm: base wasm32 build unavailable");
                return;
            }
        };
        rlibs.extend(wasm_all_sysroot_rlibs().expect("sysroot rlibs"));
        let compile_reloc = |source: &str| -> Result<Vec<u8>, String> {
            crate::ui_bridge::with_parsed_program(source, |parsed, interner| -> Result<Vec<u8>, String> {
                let (stmts, types, policies) = parsed?;
                let oracle = crate::optimize::oracle_analyze_with(stmts, interner);
                let program = crate::vm::Compiler::compile_with_oracle(stmts, interner, Some(types), Some(oracle))
                    .map_err(|e| format!("compile: {e}"))?;
                let module = super::super::module::assemble_program_linked(&program, &policies, interner)
                    .map_err(|e| format!("assemble: {e}"))?;
                super::super::reloc::module_to_relocatable(&module).map_err(|e| format!("reloc: {e}"))
            })
        };
        for src in [
            "## Main\n    Let a be modular(10, 7).\n    Show a.\n",                                 // 3 (mod 7)
            "## Main\n    Let a be modular(5, 7).\n    Let b be modular(4, 7).\n    Show a + b.\n",   // 2 (mod 7)
            "## Main\n    Let a be modular(5, 7).\n    Let b be modular(4, 7).\n    Show a * b.\n",   // 6 (mod 7)
            "## Main\n    Let a be modular(3, 7).\n    Let b be modular(5, 7).\n    Show a - b.\n",   // 5 (mod 7)
        ] {
            let vm = crate::compile::vm_outcome(src);
            assert!(vm.error.is_none(), "the VM itself errored on `{src}`: {:?}", vm.error);
            let reloc = compile_reloc(src).unwrap_or_else(|e| panic!("compile-reloc `{src}`: {e}"));
            let linked = link_objects_with_rlibs(&[&reloc, &runtime], &rlibs, false, &dir)
                .unwrap_or_else(|e| panic!("link `{src}`: {e}"));
            let got = run_linked_capturing_text(&linked);
            assert_eq!(got.trim(), vm.output.trim(), "linked Modular DIVERGED from the VM on:\n{src}");
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
                eprintln!("SKIP linked_decimal_arithmetic_matches_the_vm: base wasm32 build unavailable");
                return;
            }
        };
        rlibs.extend(wasm_all_sysroot_rlibs().expect("sysroot rlibs"));
        let compile_reloc = |source: &str| -> Result<Vec<u8>, String> {
            crate::ui_bridge::with_parsed_program(source, |parsed, interner| -> Result<Vec<u8>, String> {
                let (stmts, types, policies) = parsed?;
                let oracle = crate::optimize::oracle_analyze_with(stmts, interner);
                let program = crate::vm::Compiler::compile_with_oracle(stmts, interner, Some(types), Some(oracle))
                    .map_err(|e| format!("compile: {e}"))?;
                let module = super::super::module::assemble_program_linked(&program, &policies, interner)
                    .map_err(|e| format!("assemble: {e}"))?;
                super::super::reloc::module_to_relocatable(&module).map_err(|e| format!("reloc: {e}"))
            })
        };
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
            let reloc = compile_reloc(src).unwrap_or_else(|e| panic!("compile-reloc `{src}`: {e}"));
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
        let compile_reloc = |source: &str| -> Result<Vec<u8>, String> {
            crate::ui_bridge::with_parsed_program(source, |parsed, interner| -> Result<Vec<u8>, String> {
                let (stmts, types, policies) = parsed?;
                let oracle = crate::optimize::oracle_analyze_with(stmts, interner);
                let program = crate::vm::Compiler::compile_with_oracle(stmts, interner, Some(types), Some(oracle))
                    .map_err(|e| format!("compile: {e}"))?;
                let module = super::super::module::assemble_program_linked(&program, &policies, interner)
                    .map_err(|e| format!("assemble: {e}"))?;
                super::super::reloc::module_to_relocatable(&module).map_err(|e| format!("reloc: {e}"))
            })
        };
        for src in [
            "## Main\n    Let a be money(decimal(\"0.10\"), \"USD\").\n    Let b be money(decimal(\"0.20\"), \"USD\").\n    Show a + b.\n", // $0.30
            "## Main\n    Let a be money(decimal(\"30.00\"), \"USD\").\n    Let b be money(decimal(\"10.00\"), \"USD\").\n    Show a - b.\n", // $20.00
            "## Main\n    Let m be money(10, \"USD\").\n    Show m.\n",   // $10.00 (Int amount)
            "## Main\n    Let m be money(100, \"JPY\").\n    Show m.\n",  // ¥100 (scale 0)
            "## Main\n    Let m be money(decimal(\"19.99\"), \"USD\").\n    Show m.\n",
        ] {
            let vm = crate::compile::vm_outcome(src);
            assert!(vm.error.is_none(), "the VM itself errored on `{src}`: {:?}", vm.error);
            let reloc = compile_reloc(src).unwrap_or_else(|e| panic!("compile-reloc `{src}`: {e}"));
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
            eprintln!("SKIP linked_quantity_arithmetic_matches_the_vm: wasm link toolchain unavailable");
            return;
        }
        let dir = std::env::temp_dir().join("logos_wasm_link_quantity");
        let (runtime, mut rlibs) = match build_bigint_runtime(&dir) {
            Ok(v) => v,
            Err(_) => {
                eprintln!("SKIP linked_quantity_arithmetic_matches_the_vm: base wasm32 build unavailable");
                return;
            }
        };
        rlibs.extend(wasm_all_sysroot_rlibs().expect("sysroot rlibs"));
        let compile_reloc = |source: &str| -> Result<Vec<u8>, String> {
            crate::ui_bridge::with_parsed_program(source, |parsed, interner| -> Result<Vec<u8>, String> {
                let (stmts, types, policies) = parsed?;
                let oracle = crate::optimize::oracle_analyze_with(stmts, interner);
                let program = crate::vm::Compiler::compile_with_oracle(stmts, interner, Some(types), Some(oracle))
                    .map_err(|e| format!("compile: {e}"))?;
                let module = super::super::module::assemble_program_linked(&program, &policies, interner)
                    .map_err(|e| format!("assemble: {e}"))?;
                super::super::reloc::module_to_relocatable(&module).map_err(|e| format!("reloc: {e}"))
            })
        };
        for src in [
            "## Main\n    Let d be 5 meters.\n    Show d.\n",                    // 5 m
            "## Main\n    Show quantity(2, \"inch\").\n",                          // 2 in
            "## Main\n    Show 2 meters + 3 meters.\n",                           // 5 m
            "## Main\n    Show 5 meters - 2 meters.\n",                           // 3 m
            "## Main\n    Let d be 5 meters.\n    Show d in feet.\n",             // 6250/381 ft (exact)
            "## Main\n    Show 2 meters * 3 meters.\n",                           // combined dimension (× )
            "## Main\n    Show 100 meters / 4 meters.\n",                         // combined dimension (÷ → dimensionless)
        ] {
            let vm = crate::compile::vm_outcome(src);
            assert!(vm.error.is_none(), "the VM itself errored on `{src}`: {:?}", vm.error);
            let reloc = compile_reloc(src).unwrap_or_else(|e| panic!("compile-reloc `{src}`: {e}"));
            let linked = link_objects_with_rlibs(&[&reloc, &runtime], &rlibs, false, &dir)
                .unwrap_or_else(|e| panic!("link `{src}`: {e}"));
            let got = run_linked_capturing_text(&linked);
            assert_eq!(got.trim(), vm.output.trim(), "linked Quantity DIVERGED from the VM on:\n{src}");
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
            eprintln!("SKIP linked_rational_arithmetic_matches_the_vm: wasm link toolchain unavailable");
            return;
        }
        let dir = std::env::temp_dir().join("logos_wasm_link_rational");
        let (runtime, mut rlibs) = match build_bigint_runtime(&dir) {
            Ok(v) => v,
            Err(_) => {
                eprintln!("SKIP linked_rational_arithmetic_matches_the_vm: base wasm32 build unavailable");
                return;
            }
        };
        rlibs.extend(wasm_all_sysroot_rlibs().expect("sysroot rlibs"));
        let compile_reloc = |source: &str| -> Result<Vec<u8>, String> {
            crate::ui_bridge::with_parsed_program(source, |parsed, interner| -> Result<Vec<u8>, String> {
                let (stmts, types, policies) = parsed?;
                let oracle = crate::optimize::oracle_analyze_with(stmts, interner);
                let program = crate::vm::Compiler::compile_with_oracle(stmts, interner, Some(types), Some(oracle))
                    .map_err(|e| format!("compile: {e}"))?;
                let module = super::super::module::assemble_program_linked(&program, &policies, interner)
                    .map_err(|e| format!("assemble: {e}"))?;
                super::super::reloc::module_to_relocatable(&module).map_err(|e| format!("reloc: {e}"))
            })
        };
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
            let reloc = compile_reloc(src).unwrap_or_else(|e| panic!("compile-reloc `{src}`: {e}"));
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
        let compile_reloc = |source: &str| -> Result<Vec<u8>, String> {
            crate::ui_bridge::with_parsed_program(source, |parsed, interner| -> Result<Vec<u8>, String> {
                let (stmts, types, policies) = parsed?;
                let oracle = crate::optimize::oracle_analyze_with(stmts, interner);
                let program = crate::vm::Compiler::compile_with_oracle(stmts, interner, Some(types), Some(oracle))
                    .map_err(|e| format!("compile: {e}"))?;
                let module = super::super::module::assemble_program_linked(&program, &policies, interner)
                    .map_err(|e| format!("assemble: {e}"))?;
                super::super::reloc::module_to_relocatable(&module).map_err(|e| format!("reloc: {e}"))
            })
        };
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
            let reloc = compile_reloc(src).unwrap_or_else(|e| panic!("compile-reloc `{src}`: {e}"));
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
            eprintln!("SKIP linked_span_calendar_arithmetic_matches_the_vm: wasm link toolchain unavailable");
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
        let compile_reloc = |source: &str| -> Result<Vec<u8>, String> {
            crate::ui_bridge::with_parsed_program(source, |parsed, interner| -> Result<Vec<u8>, String> {
                let (stmts, types, policies) = parsed?;
                let oracle = crate::optimize::oracle_analyze_with(stmts, interner);
                let program = crate::vm::Compiler::compile_with_oracle(stmts, interner, Some(types), Some(oracle))
                    .map_err(|e| format!("compile: {e}"))?;
                let module = super::super::module::assemble_program_linked(&program, &policies, interner)
                    .map_err(|e| format!("assemble: {e}"))?;
                super::super::reloc::module_to_relocatable(&module).map_err(|e| format!("reloc: {e}"))
            })
        };
        for src in [
            "## Main\n    Show parse_timestamp(\"2024-01-31T00:00:00Z\") + 1 month.\n", // Feb 29 (leap-year clamp)
            "## Main\n    Show parse_timestamp(\"2024-01-31T00:00:00Z\") - 1 month.\n", // 2023-12-31
            "## Main\n    Show parse_timestamp(\"2023-01-31T00:00:00Z\") + 1 month.\n", // Feb 28 (non-leap clamp)
            "## Main\n    Show parse_timestamp(\"2024-01-15T12:00:00Z\") + 45 days.\n", // days ride the time-of-day
            "## Main\n    Show parse_timestamp(\"2020-02-29T00:00:00Z\") + 1 year.\n",  // Feb 28, 2021 (12 months)
        ] {
            let vm = crate::compile::vm_outcome(src);
            assert!(vm.error.is_none(), "the VM itself errored on `{src}`: {:?}", vm.error);
            let reloc = compile_reloc(src).unwrap_or_else(|e| panic!("compile-reloc `{src}`: {e}"));
            let linked = link_objects_with_rlibs(&[&reloc, &runtime], &rlibs, false, &dir)
                .unwrap_or_else(|e| panic!("link `{src}`: {e}"));
            let got = run_linked_capturing_text(&linked);
            assert_eq!(got.trim(), vm.output.trim(), "linked Span DIVERGED from the VM on:\n{src}");
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
                eprintln!("SKIP linked_extended_temporal_matches_the_vm: base wasm32 build unavailable");
                return;
            }
        };
        rlibs.extend(wasm_all_sysroot_rlibs().expect("sysroot rlibs"));
        let compile_reloc = |source: &str| -> Result<Vec<u8>, String> {
            crate::ui_bridge::with_parsed_program(source, |parsed, interner| -> Result<Vec<u8>, String> {
                let (stmts, types, policies) = parsed?;
                let oracle = crate::optimize::oracle_analyze_with(stmts, interner);
                let program = crate::vm::Compiler::compile_with_oracle(stmts, interner, Some(types), Some(oracle))
                    .map_err(|e| format!("compile: {e}"))?;
                let module = super::super::module::assemble_program_linked(&program, &policies, interner)
                    .map_err(|e| format!("assemble: {e}"))?;
                super::super::reloc::module_to_relocatable(&module).map_err(|e| format!("reloc: {e}"))
            })
        };
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
            let reloc = compile_reloc(src).unwrap_or_else(|e| panic!("compile-reloc `{src}`: {e}"));
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
                eprintln!("SKIP linked_sha1_lane_intrinsics_match_the_vm: base wasm32 build unavailable");
                return;
            }
        };
        rlibs.extend(wasm_all_sysroot_rlibs().expect("sysroot rlibs"));
        let compile_reloc = |source: &str| -> Result<Vec<u8>, String> {
            crate::ui_bridge::with_parsed_program(source, |parsed, interner| -> Result<Vec<u8>, String> {
                let (stmts, types, policies) = parsed?;
                let oracle = crate::optimize::oracle_analyze_with(stmts, interner);
                let program = crate::vm::Compiler::compile_with_oracle(stmts, interner, Some(types), Some(oracle))
                    .map_err(|e| format!("compile: {e}"))?;
                let module = super::super::module::assemble_program_linked(&program, &policies, interner)
                    .map_err(|e| format!("assemble: {e}"))?;
                super::super::reloc::module_to_relocatable(&module).map_err(|e| format!("reloc: {e}"))
            })
        };
        let prelude = "## Main\n    Let a be lanes4Of(word32(1), word32(2), word32(3), word32(4)).\n    Let b be lanes4Of(word32(5), word32(6), word32(7), word32(8)).\n";
        for tail in [
            "    Show seqOfLanes4W32(a).\n",                        // [1, 2, 3, 4] (pack→unpack round-trip)
            "    Show seqOfLanes4W32(sha1msg1(a, b)).\n",
            "    Show seqOfLanes4W32(sha1msg2(a, b)).\n",
            "    Show seqOfLanes4W32(sha1nexte(a, b)).\n",
            "    Show seqOfLanes4W32(sha1rnds4(a, b, 0)).\n",
            "    Show seqOfLanes4W32(sha1rnds4(a, b, 2)).\n",
            "    Show seqOfLanes4W32(a + b).\n",    // lane-wise add
            "    Show seqOfLanes4W32(a xor b).\n",  // lane-wise xor
        ] {
            let src = format!("{prelude}{tail}");
            let vm = crate::compile::vm_outcome(&src);
            assert!(vm.error.is_none(), "the VM itself errored on `{src}`: {:?}", vm.error);
            let reloc = compile_reloc(&src).unwrap_or_else(|e| panic!("compile-reloc `{src}`: {e}"));
            let linked = link_objects_with_rlibs(&[&reloc, &runtime], &rlibs, false, &dir)
                .unwrap_or_else(|e| panic!("link `{src}`: {e}"));
            let got = run_linked_capturing_text(&linked);
            assert_eq!(got.trim(), vm.output.trim(), "linked SHA-1 lane op DIVERGED from the VM on:\n{src}");
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
        let compile_reloc = |source: &str| -> Result<Vec<u8>, String> {
            crate::ui_bridge::with_parsed_program(source, |parsed, interner| -> Result<Vec<u8>, String> {
                let (stmts, types, policies) = parsed?;
                let oracle = crate::optimize::oracle_analyze_with(stmts, interner);
                let program = crate::vm::Compiler::compile_with_oracle(stmts, interner, Some(types), Some(oracle))
                    .map_err(|e| format!("compile: {e}"))?;
                let module = super::super::module::assemble_program_linked(&program, &policies, interner)
                    .map_err(|e| format!("assemble: {e}"))?;
                super::super::reloc::module_to_relocatable(&module).map_err(|e| format!("reloc: {e}"))
            })
        };
        let prelude = "## Main\n    Let bytes be [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15].\n    Let idx be [15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0].\n    Let a be lanes16Word8(bytes).\n    Let b be lanes16Word8(idx).\n";
        for tail in [
            "    Show seqOfLanes16W8(a).\n",                                    // round-trip [0..15]
            "    Show seqOfLanes16W8(splat16Word8(7)).\n",                      // [7 ×16]
            "    Show seqOfLanes16W8(shuffle16(a, b)).\n",                      // reverse (idx selects lane 15..0)
            "    Show seqOfLanes16W8(byteAdd16(a, splat16Word8(1))).\n",        // [1..16]
            "    Show seqOfLanes16W8(interleaveLo16(a, b)).\n",
            "    Show seqOfLanes16W8(interleaveHi16(a, b)).\n",
            "    Show seqOfLanes16W8(shrBytes16(a, 4)).\n",                      // byte-shift right by 4
            "    Show seqOfLanes16W8(packus16(maddubs16(a, b), maddubs16(a, b))).\n", // width-changing pair
            "    Show seqOfLanes8(lanes8Word32([1, 2, 3, 4, 5, 6, 7, 8])).\n",  // 8×Word32 round-trip
        ] {
            let src = format!("{prelude}{tail}");
            let vm = crate::compile::vm_outcome(&src);
            assert!(vm.error.is_none(), "the VM itself errored on `{src}`: {:?}", vm.error);
            let reloc = compile_reloc(&src).unwrap_or_else(|e| panic!("compile-reloc `{src}`: {e}"));
            let linked = link_objects_with_rlibs(&[&reloc, &runtime], &rlibs, false, &dir)
                .unwrap_or_else(|e| panic!("link `{src}`: {e}"));
            let got = run_linked_capturing_text(&linked);
            assert_eq!(got.trim(), vm.output.trim(), "linked SIMD lane op DIVERGED from the VM on:\n{src}");
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
        let compile_reloc = |source: &str| -> Result<Vec<u8>, String> {
            crate::ui_bridge::with_parsed_program(source, |parsed, interner| -> Result<Vec<u8>, String> {
                let (stmts, types, policies) = parsed?;
                let oracle = crate::optimize::oracle_analyze_with(stmts, interner);
                let program = crate::vm::Compiler::compile_with_oracle(stmts, interner, Some(types), Some(oracle))
                    .map_err(|e| format!("compile: {e}"))?;
                let module = super::super::module::assemble_program_linked(&program, &policies, interner)
                    .map_err(|e| format!("assemble: {e}"))?;
                super::super::reloc::module_to_relocatable(&module).map_err(|e| format!("reloc: {e}"))
            })
        };
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
            assert!(vm.error.is_none(), "the VM itself errored on `{src}`: {:?}", vm.error);
            let reloc = compile_reloc(src).unwrap_or_else(|e| panic!("compile-reloc `{src}`: {e}"));
            let linked = link_objects_with_rlibs(&[&reloc, &runtime], &rlibs, false, &dir)
                .unwrap_or_else(|e| panic!("link `{src}`: {e}"));
            let got = run_linked_capturing_text(&linked);
            assert_eq!(got.trim(), vm.output.trim(), "linked Money FX DIVERGED from the VM on:\n{src}");
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
        let compile_reloc = |source: &str| -> Result<Vec<u8>, String> {
            crate::ui_bridge::with_parsed_program(source, |parsed, interner| -> Result<Vec<u8>, String> {
                let (stmts, types, policies) = parsed?;
                let oracle = crate::optimize::oracle_analyze_with(stmts, interner);
                let program = crate::vm::Compiler::compile_with_oracle(stmts, interner, Some(types), Some(oracle))
                    .map_err(|e| format!("compile: {e}"))?;
                let module = super::super::module::assemble_program_linked(&program, &policies, interner)
                    .map_err(|e| format!("assemble: {e}"))?;
                super::super::reloc::module_to_relocatable(&module).map_err(|e| format!("reloc: {e}"))
            })
        };
        for src in [
            "## Main\n    Show wireBytes(42).\n",       // Int → [T_INT, zigzag(42) uvarint …]
            "## Main\n    Show wireBytes(-7).\n",        // negative (zigzag)
            "## Main\n    Show wireBytes(true).\n",      // Bool
            "## Main\n    Show wireBytes(false).\n",
            "## Main\n    Show wireBytes(\"hello\").\n", // Text
            "## Main\n    Show wireBytes(3.5).\n",       // Float
        ] {
            let vm = crate::compile::vm_outcome(src);
            assert!(vm.error.is_none(), "the VM itself errored on `{src}`: {:?}", vm.error);
            let reloc = compile_reloc(src).unwrap_or_else(|e| panic!("compile-reloc `{src}`: {e}"));
            let linked = link_objects_with_rlibs(&[&reloc, &runtime], &rlibs, false, &dir)
                .unwrap_or_else(|e| panic!("link `{src}`: {e}"));
            let got = run_linked_capturing_text(&linked);
            assert_eq!(got.trim(), vm.output.trim(), "linked wireBytes DIVERGED from the VM on:\n{src}");
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
        let compile_reloc = |source: &str| -> Result<Vec<u8>, String> {
            crate::ui_bridge::with_parsed_program(source, |parsed, interner| -> Result<Vec<u8>, String> {
                let (stmts, types, policies) = parsed?;
                let oracle = crate::optimize::oracle_analyze_with(stmts, interner);
                let program = crate::vm::Compiler::compile_with_oracle(stmts, interner, Some(types), Some(oracle))
                    .map_err(|e| format!("compile: {e}"))?;
                let module = super::super::module::assemble_program_linked(&program, &policies, interner)
                    .map_err(|e| format!("assemble: {e}"))?;
                super::super::reloc::module_to_relocatable(&module).map_err(|e| format!("reloc: {e}"))
            })
        };
        // End-to-end: compile the reloc, LINK it (the whole `marshal`/`RuntimeValue` decode+display+apply
        // graph is pulled into the module + gc-stripped down), run it, and assert it reproduces the value
        // the codec/contract compute NATIVELY. (Getting `readWireProgram` classified as a heap op — so
        // `emit_alloc` uses the runtime allocator, not the `__heap_ptr` global undeclared in a linked
        // module — was the fix; the earlier "invalid global relocation" / lld crash were that one bug.)
        let assert_wire = |src: &str, frame: Vec<u8>, expected: &str| {
            WIRE_FRAME.with(|f| *f.borrow_mut() = frame);
            let reloc = compile_reloc(src).unwrap_or_else(|e| panic!("compile-reloc `{src}`: {e}"));
            let linked = link_objects_with_rlibs(&[&reloc, &runtime], &rlibs, false, &dir)
                .unwrap_or_else(|e| panic!("link `{src}`: {e}"));
            assert_eq!(run_linked_capturing_text(&linked).trim(), expected, "wire-input result for:\n{src}");
        };
        // (a) `readWireProgram` of a PLAIN value: the frame encodes Int 42 → the dynamic value shows "42".
        assert_wire("## Main\n    Show readWireProgram().\n", encode_value_raw(&RuntimeValue::Int(42)).expect("encode Int"), "42");
        // (b) `run_accepted` of a wire-received SHIPPED function `3·x + 1`, arg 5 ∈ [0,1000] → 3·5+1 = 16.
        let shipped = RuntimeValue::Function(Box::new(ClosureValue {
            body_index: usize::MAX,
            captured_env: std::collections::HashMap::default(),
            param_names: vec![logicaffeine_base::Symbol::from_index(0)],
            generated: Some(std::rc::Rc::new(GenExpr::Add(
                Box::new(GenExpr::Mul(Box::new(GenExpr::Index), Box::new(GenExpr::Const(3)))),
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
            eprintln!("SKIP linked_crypto_uuid_in_logos_matches_the_vm: wasm link toolchain unavailable");
            return;
        }
        let dir = std::env::temp_dir().join("logos_wasm_link_cryptouuid");
        let (runtime, mut rlibs) = match build_bigint_runtime(&dir) {
            Ok(v) => v,
            Err(_) => {
                eprintln!("SKIP linked_crypto_uuid_in_logos_matches_the_vm: base wasm32 build unavailable");
                return;
            }
        };
        rlibs.extend(wasm_all_sysroot_rlibs().expect("sysroot rlibs"));
        let compile_reloc = |source: &str| -> Result<Vec<u8>, String> {
            crate::ui_bridge::with_parsed_program(source, |parsed, interner| -> Result<Vec<u8>, String> {
                let (stmts, types, policies) = parsed?;
                let oracle = crate::optimize::oracle_analyze_with(stmts, interner);
                let program = crate::vm::Compiler::compile_with_oracle(stmts, interner, Some(types), Some(oracle))
                    .map_err(|e| format!("compile: {e}"))?;
                let module = super::super::module::assemble_program_linked(&program, &policies, interner)
                    .map_err(|e| format!("assemble: {e}"))?;
                super::super::reloc::module_to_relocatable(&module).map_err(|e| format!("reloc: {e}"))
            })
        };
        for src in [
            "## Main\n    Show sha1(text_bytes(\"abc\")).\n",              // the SHA-1 chain (Logos)
            "## Main\n    Show uuid_v5(uuid_dns(), \"example.com\").\n",   // the well-known DNS v5 UUID
            "## Main\n    Show uuid_v3(uuid_dns(), \"example.com\").\n",   // MD5-based v3
            "## Main\n    Show uuid_v5(uuid_url(), \"https://logicaffeine.com\").\n",
        ] {
            let vm = crate::compile::vm_outcome(src);
            assert!(vm.error.is_none(), "the VM itself errored on `{src}`: {:?}", vm.error);
            let reloc = compile_reloc(src).unwrap_or_else(|e| panic!("compile-reloc `{src}`: {e}"));
            let linked = link_objects_with_rlibs(&[&reloc, &runtime], &rlibs, false, &dir)
                .unwrap_or_else(|e| panic!("link `{src}`: {e}"));
            let got = run_linked_capturing_text(&linked);
            assert_eq!(got.trim(), vm.output.trim(), "linked crypto UUID DIVERGED from the VM on:\n{src}");
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
            eprintln!("SKIP linked_mixed_text_and_bigint_matches_the_vm: wasm link toolchain unavailable");
            return;
        }
        let dir = std::env::temp_dir().join("logos_wasm_link_mixed");
        let (runtime, mut rlibs) = match build_bigint_runtime(&dir) {
            Ok(v) => v,
            Err(_) => {
                eprintln!("SKIP linked_mixed_text_and_bigint_matches_the_vm: base wasm32 build unavailable");
                return;
            }
        };
        rlibs.extend(wasm_all_sysroot_rlibs().expect("sysroot rlibs"));
        let compile_reloc = |source: &str| -> Result<Vec<u8>, String> {
            crate::ui_bridge::with_parsed_program(source, |parsed, interner| -> Result<Vec<u8>, String> {
                let (stmts, types, policies) = parsed?;
                let oracle = crate::optimize::oracle_analyze_with(stmts, interner);
                let program = crate::vm::Compiler::compile_with_oracle(stmts, interner, Some(types), Some(oracle))
                    .map_err(|e| format!("compile: {e}"))?;
                let module = super::super::module::assemble_program_linked(&program, &policies, interner)
                    .map_err(|e| format!("assemble: {e}"))?;
                super::super::reloc::module_to_relocatable(&module).map_err(|e| format!("reloc: {e}"))
            })
        };

        for src in [
            "## Main\nShow \"x = \" + (2 to the power of 200).\n",
            "## Main\nShow (2 to the power of 100) + \" is big\".\n",
            "## Main\nShow \"\" + (99999999999 times 99999999999) + \"!\".\n",
            "## Main\nShow \"first \" + (2 to the power of 64) + \" then \" + (3 to the power of 40).\n",
        ] {
            let vm = crate::compile::vm_outcome(src);
            assert!(vm.error.is_none(), "the VM itself errored on `{src}`: {:?}", vm.error);
            let reloc = compile_reloc(src).unwrap_or_else(|e| panic!("assemble `{src}`: {e}"));
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
        let compile_reloc = |source: &str| -> Result<Vec<u8>, String> {
            crate::ui_bridge::with_parsed_program(source, |parsed, interner| -> Result<Vec<u8>, String> {
                let (stmts, types, policies) = parsed?;
                let oracle = crate::optimize::oracle_analyze_with(stmts, interner);
                let program = crate::vm::Compiler::compile_with_oracle(stmts, interner, Some(types), Some(oracle))
                    .map_err(|e| format!("compile: {e}"))?;
                let module = super::super::module::assemble_program_linked(&program, &policies, interner)
                    .map_err(|e| format!("assemble: {e}"))?;
                super::super::reloc::module_to_relocatable(&module).map_err(|e| format!("reloc: {e}"))
            })
        };

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
            let reloc = compile_reloc(src).unwrap_or_else(|e| panic!("assemble `{src}`: {e}"));
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
        let reloc = crate::ui_bridge::with_parsed_program(src, |parsed, interner| -> Result<Vec<u8>, String> {
            let (stmts, types, policies) = parsed?;
            let oracle = crate::optimize::oracle_analyze_with(stmts, interner);
            let program = crate::vm::Compiler::compile_with_oracle(stmts, interner, Some(types), Some(oracle))
                .map_err(|e| format!("compile: {e}"))?;
            let module = super::super::module::assemble_program_linked(&program, &policies, interner)
                .map_err(|e| format!("assemble: {e}"))?;
            super::super::reloc::module_to_relocatable(&module).map_err(|e| format!("reloc: {e}"))
        })
        .expect("compile the large-heap program");
        let linked = link_objects_with_rlibs(&[&reloc, &runtime], &rlibs, false, &dir).expect("link large-heap program");
        let got = run_linked_capturing_text(&linked);
        assert_eq!(got.trim(), vm.output.trim(), "linked >4MiB-heap output disagrees with the VM");
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
        let compile_reloc = |source: &str| -> Result<Vec<u8>, String> {
            crate::ui_bridge::with_parsed_program(source, |parsed, interner| -> Result<Vec<u8>, String> {
                let (stmts, types, policies) = parsed?;
                let oracle = crate::optimize::oracle_analyze_with(stmts, interner);
                let program = crate::vm::Compiler::compile_with_oracle(stmts, interner, Some(types), Some(oracle))
                    .map_err(|e| format!("compile: {e}"))?;
                let module = super::super::module::assemble_program_linked(&program, &policies, interner)
                    .map_err(|e| format!("assemble: {e}"))?;
                super::super::reloc::module_to_relocatable(&module).map_err(|e| format!("reloc: {e}"))
            })
        };
        for src in [
            "## Main\nLet x be 99999999999 times 99999999999.\nShow x.\n",
            "## Main\nLet p be 2 to the power of 100.\nLet q be 3 to the power of 50.\nShow p times q.\n",
            "## Main\nLet x be 2 to the power of 200.\nLet y be x plus 1.\nShow y.\n",
        ] {
            let vm = crate::compile::vm_outcome(src);
            assert!(vm.error.is_none(), "the VM itself errored on `{src}`: {:?}", vm.error);
            match compile_reloc(src) {
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
        let compile_reloc = |source: &str| -> Result<Vec<u8>, String> {
            crate::ui_bridge::with_parsed_program(source, |parsed, interner| -> Result<Vec<u8>, String> {
                let (stmts, types, policies) = parsed?;
                let oracle = crate::optimize::oracle_analyze_with(stmts, interner);
                let program = crate::vm::Compiler::compile_with_oracle(stmts, interner, Some(types), Some(oracle))
                    .map_err(|e| format!("compile: {e}"))?;
                let module = super::super::module::assemble_program_linked(&program, &policies, interner)
                    .map_err(|e| format!("assemble: {e}"))?;
                super::super::reloc::module_to_relocatable(&module).map_err(|e| format!("reloc: {e}"))
            })
        };

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
            let reloc = compile_reloc(src).unwrap_or_else(|e| panic!("assemble `{src}`: {e}"));
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
        // Compile a Logos `Show <base> to the power of <exp>` to a LINKER-MODE relocatable object
        // (integer `Op::Pow` → `logos_rt_bigint_*`, result a Text handle).
        let compile_reloc = |source: &str| -> Vec<u8> {
            crate::ui_bridge::with_parsed_program(source, |parsed, interner| -> Result<Vec<u8>, String> {
                let (stmts, types, policies) = parsed?;
                let oracle = crate::optimize::oracle_analyze_with(stmts, interner);
                let program = crate::vm::Compiler::compile_with_oracle(stmts, interner, Some(types), Some(oracle))
                    .map_err(|e| format!("compile: {e}"))?;
                let module = super::super::module::assemble_program_linked(&program, &policies, interner)
                    .map_err(|e| format!("assemble_linked: {e}"))?;
                super::super::reloc::module_to_relocatable(&module).map_err(|e| format!("reloc: {e}"))
            })
            .unwrap_or_else(|e| panic!("compile `{source}` to relocatable: {e}"))
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
            let expected = logicaffeine_base::BigInt::from_i64(base).pow(exp).to_string();
            assert_eq!(got.trim(), expected, "linked wasm must print the exact {base}^{exp} the base BigInt computes");
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
            let source = format!("## Main\nShow ({a} to the power of {ae}) times ({b} to the power of {be}).\n");
            let reloc = compile_reloc(&source);
            let linked = link_objects_with_rlibs(&[&reloc, &runtime], &rlibs, false, &dir)
                .unwrap_or_else(|e| panic!("link `{source}`: {e}"));
            let got = run_linked_capturing_text(&linked);
            let av = logicaffeine_base::BigInt::from_i64(a).pow(ae);
            let bv = logicaffeine_base::BigInt::from_i64(b).pow(be);
            let expected = av.mul(&bv).to_string();
            assert_eq!(got.trim(), expected, "linked wasm must print the exact ({a}^{ae})*({b}^{be})");
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
            let expected = logicaffeine_base::BigInt::from_i64(a).pow(ae).mul(&logicaffeine_base::BigInt::from_i64(k)).to_string();
            assert_eq!(got.trim(), expected, "linked wasm must print the exact ({a}^{ae})*{k} (mixed BigInt*Int)");
            assert_eq!(
                got.trim(),
                crate::compile::vm_outcome(&source).output.trim(),
                "linked wasm mixed BigInt*Int must equal the VM for ({a}^{ae})*{k}"
            );
        }

        // ADD / SUB on BigInts (linked, `plus`/`minus`): exact, and a `minus` may go NEGATIVE — the sign
        // is rendered by `to_text`, matching the VM. Both BigInt±BigInt and mixed BigInt±Int (`from_i64`).
        for &(a, ae, add, b, be) in
            &[(2i64, 100u32, true, 3i64, 50u32), (3, 80, false, 2, 80), (2, 50, false, 3, 50)]
        {
            let word = if add { "plus" } else { "minus" };
            let source = format!("## Main\nShow ({a} to the power of {ae}) {word} ({b} to the power of {be}).\n");
            let reloc = compile_reloc(&source);
            let linked = link_objects_with_rlibs(&[&reloc, &runtime], &rlibs, false, &dir)
                .unwrap_or_else(|e| panic!("link `{source}`: {e}"));
            let got = run_linked_capturing_text(&linked);
            let av = logicaffeine_base::BigInt::from_i64(a).pow(ae);
            let bv = logicaffeine_base::BigInt::from_i64(b).pow(be);
            let expected = if add { av.add(&bv) } else { av.sub(&bv) }.to_string();
            assert_eq!(got.trim(), expected, "linked wasm must print the exact ({a}^{ae}) {word} ({b}^{be})");
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
            assert_eq!(got.trim(), expected, "linked wasm mixed ({a}^{ae}) {word} {k}");
            assert_eq!(
                got.trim(),
                crate::compile::vm_outcome(&source).output.trim(),
                "linked wasm mixed add/sub must equal the VM for ({a}^{ae}) {word} {k}"
            );
        }

        // DIV / MOD on BigInts (linked): `divided by` = the exact quotient (`div_rem().0`), `the
        // remainder of a and b` = the remainder (`.1`), matching the VM's own `div_rem`.
        for &(a, ae, isdiv, k) in &[(2i64, 200u32, true, 7i64), (2, 200, false, 7), (10, 50, true, 3)] {
            let source = if isdiv {
                format!("## Main\nShow ({a} to the power of {ae}) divided by {k}.\n")
            } else {
                format!("## Main\nShow the remainder of ({a} to the power of {ae}) and {k}.\n")
            };
            let reloc = compile_reloc(&source);
            let linked = link_objects_with_rlibs(&[&reloc, &runtime], &rlibs, false, &dir)
                .unwrap_or_else(|e| panic!("link `{source}`: {e}"));
            let got = run_linked_capturing_text(&linked);
            let (q, r) = logicaffeine_base::BigInt::from_i64(a).pow(ae).div_rem(&logicaffeine_base::BigInt::from_i64(k)).unwrap();
            let expected = if isdiv { q } else { r }.to_string();
            let what = if isdiv { "quotient" } else { "remainder" };
            assert_eq!(got.trim(), expected, "linked wasm must print the exact {what} of ({a}^{ae}) and {k}");
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
                logicaffeine_base::BigInt::from_i64(99999999999).mul(&logicaffeine_base::BigInt::from_i64(99999999999)),
            ),
            (
                "## Main\nShow 9223372036854775807 plus 1.\n",
                logicaffeine_base::BigInt::from_i64(9223372036854775807).add(&logicaffeine_base::BigInt::from_i64(1)),
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
            assert_eq!(got.trim(), expected_big.to_string(), "linked wasm must promote the overflowing `{source}` to the exact BigInt");
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
                logicaffeine_base::BigInt::from_i64(2).pow(128).mul(&logicaffeine_base::BigInt::from_i64(3)).to_string(),
            ),
            (
                "## Main\nShow 99999999999 times 99999999999.\n",
                logicaffeine_base::BigInt::from_i64(99999999999).mul(&logicaffeine_base::BigInt::from_i64(99999999999)).to_string(),
            ),
        ] {
            let linked = crate::compile::compile_to_wasm_linked(source)
                .unwrap_or_else(|e| panic!("compile_to_wasm_linked(`{source}`) failed: {e:?}"));
            let got = run_linked_capturing_text(&linked);
            assert_eq!(got.trim(), expected, "public linked entry must print the exact BigInt for `{source}`");
            assert_eq!(got.trim(), crate::compile::vm_outcome(source).output.trim(), "public linked entry == VM for `{source}`");
        }
    }
}
