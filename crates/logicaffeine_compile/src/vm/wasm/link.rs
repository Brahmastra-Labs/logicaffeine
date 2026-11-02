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
        FuncSymbol {
            func_index: probe_func_index,
            defined_name: None,
        },
        FuncSymbol {
            func_index: main_func_index,
            defined_name: Some("main".to_string()),
        },
    ];
    let relocs = vec![CodeReloc {
        offset: reloc_offset,
        sym_index: 0,
    }];

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
    custom_section(
        "reloc.CODE",
        &encode_reloc_code(code_section_index, &relocs),
        &mut out,
    );
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
        FuncSymbol {
            func_index: write_func_index,
            defined_name: None,
        },
        FuncSymbol {
            func_index: main_func_index,
            defined_name: Some("main".to_string()),
        },
    ];
    let relocs = vec![CodeReloc {
        offset: reloc_offset,
        sym_index: 0,
    }];

    let mut out = Vec::new();
    out.extend_from_slice(b"\0asm");
    out.extend_from_slice(&[0x01, 0x00, 0x00, 0x00]);
    section(1, &types, &mut out);
    section(2, &imports, &mut out);
    section(3, &funcs, &mut out);
    let code_section_index: u32 = 3;
    section(10, &code, &mut out);
    custom_section("linking", &encode_linking(&symbols), &mut out);
    custom_section(
        "reloc.CODE",
        &encode_reloc_code(code_section_index, &relocs),
        &mut out,
    );
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
        FuncSymbol {
            func_index: mul_func_index,
            defined_name: None,
        },
        FuncSymbol {
            func_index: main_func_index,
            defined_name: Some("main".to_string()),
        },
    ];
    let relocs = vec![CodeReloc {
        offset: reloc_offset,
        sym_index: 0,
    }];

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
    for (name, ty) in [
        ("logos_rt_bigint_from_i64", 0u64),
        ("logos_rt_bigint_mul", 1),
        ("logos_rt_bigint_to_text", 2),
    ] {
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
    let relocs: Vec<CodeReloc> = calls
        .iter()
        .map(|&(off, sym)| CodeReloc {
            offset: prefix + off,
            sym_index: sym,
        })
        .collect();

    let symbols = vec![
        FuncSymbol {
            func_index: from_i64,
            defined_name: None,
        },
        FuncSymbol {
            func_index: mul,
            defined_name: None,
        },
        FuncSymbol {
            func_index: to_text,
            defined_name: None,
        },
        FuncSymbol {
            func_index: main_idx,
            defined_name: Some("main".to_string()),
        },
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
    let symbols = vec![FuncSymbol {
        func_index: 0,
        defined_name: Some(name.to_string()),
    }];
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
                        out.push(FuncImportSig {
                            name: field,
                            params,
                            results,
                        });
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
        .ok_or(WasmLowerError::Unsupported(
            "could not determine the host triple",
        ))
}

/// The toolchain sysroot (`rustc --print sysroot`).
fn sysroot() -> R<PathBuf> {
    let out = Command::new("rustc")
        .args(["--print", "sysroot"])
        .output()
        .map_err(|_| WasmLowerError::Unsupported("rustc not found for the wasm linker"))?;
    Ok(PathBuf::from(
        String::from_utf8_lossy(&out.stdout).trim().to_string(),
    ))
}

/// The path to the toolchain's `rust-lld` (= `wasm-ld` under `-flavor wasm`).
pub(crate) fn rust_lld_path() -> R<PathBuf> {
    let host = host_triple()?;
    let p = sysroot()?
        .join("lib/rustlib")
        .join(host)
        .join("bin/rust-lld");
    if p.exists() {
        Ok(p)
    } else {
        Err(WasmLowerError::Unsupported(
            "rust-lld not present in the toolchain",
        ))
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
        Err(WasmLowerError::Unsupported(
            "a wasm32 sysroot rlib is missing",
        ))
    };
    Ok(vec![
        find("liballoc-")?,
        find("libcore-")?,
        find("libcompiler_builtins-")?,
    ])
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
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .any(|l| l == "wasm32-unknown-unknown")
        })
        .unwrap_or(false)
}

/// Compile a runtime `.rs` source to a `wasm32-unknown-unknown` RELOCATABLE object (`--emit=obj`),
/// returning the object bytes. `dir` is a scratch directory for the intermediate files.
pub(crate) fn build_runtime_object(src: &str, dir: &std::path::Path) -> R<Vec<u8>> {
    std::fs::create_dir_all(dir)
        .map_err(|_| WasmLowerError::Unsupported("cannot create link scratch dir"))?;
    let rs = dir.join("logos_rt.rs");
    let obj = dir.join("logos_rt.o");
    std::fs::write(&rs, src)
        .map_err(|_| WasmLowerError::Unsupported("cannot write runtime source"))?;
    let status = Command::new("rustc")
        .args([
            "--target",
            "wasm32-unknown-unknown",
            "--crate-type=lib",
            "--emit=obj",
            "-Copt-level=2",
        ])
        .arg(&rs)
        .arg("-o")
        .arg(&obj)
        .status()
        .map_err(|_| {
            WasmLowerError::Unsupported("failed to invoke rustc for the runtime object")
        })?;
    if !status.success() {
        return Err(WasmLowerError::Unsupported(
            "rustc failed to build the runtime object",
        ));
    }
    std::fs::read(&obj).map_err(|_| WasmLowerError::Unsupported("cannot read the runtime object"))
}

/// Link relocatable objects into one wasm module with `rust-lld -flavor wasm` against the minimal
/// `wasm32` runtime (a `no_std` + `#[global_allocator]` object): `liballoc`/`libcore`/
/// `libcompiler_builtins` + both synthesized allocator-shim defs. The common case.
pub(crate) fn link_objects(objects: &[&[u8]], dir: &std::path::Path) -> R<Vec<u8>> {
    link_objects_with_rlibs(objects, &wasm_runtime_rlibs()?, true, dir)
}

/// Each link owns a fresh directory under the caller's scratch root.
/// Atomic directory creation isolates threads and processes; dropping the owner
/// releases its inputs and output on both successful and failed links.
struct LinkScratch(std::path::PathBuf);

impl LinkScratch {
    fn new(root: &std::path::Path) -> R<Self> {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        std::fs::create_dir_all(root)
            .map_err(|_| WasmLowerError::Unsupported("cannot create link scratch root"))?;
        loop {
            let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let path = root.join(format!("link-{}-{id}", std::process::id()));
            match std::fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(_) => {
                    return Err(WasmLowerError::Unsupported(
                        "cannot create owned link scratch dir",
                    ))
                }
            }
        }
    }
}

impl Drop for LinkScratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
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
    let scratch = LinkScratch::new(dir)?;
    let dir = &scratch.0;
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
                shim_objs.push(emit_shim_object(
                    &imp.name,
                    &imp.params,
                    &imp.results,
                    false,
                ));
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
    for (i, obj) in objects
        .iter()
        .copied()
        .chain(shim_objs.iter().map(|v| v.as_slice()))
        .enumerate()
    {
        let p = dir.join(format!("obj{i}.o"));
        std::fs::write(&p, obj)
            .map_err(|_| WasmLowerError::Unsupported("cannot write link input"))?;
        obj_paths.push(p);
    }
    let out = dir.join("linked.wasm");
    let mut cmd = Command::new(&lld);
    // rustc's `wasm32-unknown-unknown` binary flags — a 1 MiB shadow stack placed first, the heap/data
    // markers exported, dead-section GC — plus `--export=main` and `--allow-undefined` (host `env.*`
    // functions stay imports). `--no-demangle` keeps symbol names raw. `--strip-debug` drops the DWARF
    // the runtime was built with, which describes the whole runtime rather than the functions the GC
    // keeps; the `name` section stays, so a trap still names its function.
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
        "--strip-debug",
        "--allow-undefined",
    ]);
    for p in &obj_paths {
        cmd.arg(p);
    }
    for rlib in rlibs {
        cmd.arg(rlib);
    }
    cmd.arg("-o").arg(&out);
    let output = cmd
        .output()
        .map_err(|_| WasmLowerError::Unsupported("failed to invoke rust-lld"))?;
    if !output.success() {
        return Err(WasmLowerError::Unsupported(
            "rust-lld failed to link the module",
        ));
    }
    std::fs::read(&out).map_err(|_| WasmLowerError::Unsupported("cannot read the linked module"))
}

/// Every precompiled `.rlib` in the `wasm32-unknown-unknown` sysroot lib dir — `libstd`, `libdlmalloc`,
/// and their deps included. `rust-lld --gc-sections` pulls in only referenced members, so passing the
/// whole set is robust across toolchain versions; needed when the runtime uses `std` (e.g. real BigInt).
fn wasm_all_sysroot_rlibs() -> R<Vec<PathBuf>> {
    let dir = sysroot()?.join("lib/rustlib/wasm32-unknown-unknown/lib");
    let entries = std::fs::read_dir(&dir)
        .map_err(|_| WasmLowerError::Unsupported("wasm32 sysroot lib dir missing"))?;
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
/// with `--extern` `logicaffeine_base` and `logicaffeine_compile`, built for wasm32; it reaches the exact
/// extension's values only through `logicaffeine_data`'s C ABI, linked from that crate's archive.
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
/// `|z|` — the `f64` modulus of an exact `Complex`, byte-identical to the interpreter's `abs(Complex)`
/// (`RuntimeValue::Float(c.abs_f64())`). The one complex op whose result is a Float, not a handle.
#[no_mangle] pub extern "C" fn logos_rt_complex_abs(h: i32) -> f64 {
    let c = unsafe { &*(h as *const Complex) };
    c.abs_f64()
}
/// `complex(re, im)` with FLOAT parts — the float-complex tier (`cExp`/`cLog`/`cSqrt`/… return a
/// `ComplexF` in the interpreter). Each part becomes its EXACT dyadic `Rational` (round-trips), so
/// the handle reuses the whole exact-`Complex` machinery (`to_text`, `re`/`im`, arithmetic) and stays
/// byte-identical to the interpreter — mirroring the C tier's `cplx_reals`.
#[no_mangle] pub extern "C" fn logos_rt_complex_from_f64(re: f64, im: f64) -> i32 {
    use logicaffeine_base::numeric::rational_from_f64_exact as rat_f64;
    let (re, im) = (
        rat_f64(re).unwrap_or_else(Rational::zero),
        rat_f64(im).unwrap_or_else(Rational::zero),
    );
    Box::into_raw(Box::new(Complex::new(re, im))) as i32
}
/// Real / imaginary part of a Complex handle as an `f64` — `re(z)` / `im(z)`. The dyadic part
/// round-trips, so `re(complex(x, y)) == x` bit-for-bit.
#[no_mangle] pub extern "C" fn logos_rt_complex_re_f64(h: i32) -> f64 {
    let c = unsafe { &*(h as *const Complex) };
    c.re().to_f64()
}
#[no_mangle] pub extern "C" fn logos_rt_complex_im_f64(h: i32) -> f64 {
    let c = unsafe { &*(h as *const Complex) };
    c.im().to_f64()
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
// `÷` in the ring = multiply by the modular inverse of the divisor (errors if not coprime), and
// `pow(m, e)` is modular exponentiation — the exact mirrors of the VM's `Modular::div`/`Modular::pow`.
#[no_mangle] pub extern "C" fn logos_rt_modular_div(a: i32, b: i32) -> i32 {
    let a = unsafe { &*(a as *const Modular) };
    let b = unsafe { &*(b as *const Modular) };
    Box::into_raw(Box::new(a.div(b).expect("modular divisor has no inverse"))) as i32
}
#[no_mangle] pub extern "C" fn logos_rt_modular_pow(a: i32, e: i64) -> i32 {
    let a = unsafe { &*(a as *const Modular) };
    Box::into_raw(Box::new(a.pow(e as u64))) as i32
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
    logos_rt_rational_vm(a, b, logicaffeine_compile::semantics::arith::exact_divide)
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
// The order of two rationals, `-1`, `0` or `1`: exact, as the VM compares them.
#[no_mangle] pub extern "C" fn logos_rt_rational_cmp(a: i32, b: i32) -> i32 {
    let a = unsafe { &*(a as *const Rational) };
    let b = unsafe { &*(b as *const Rational) };
    a.cmp(b) as i32
}
// ---- A rational under the VM's rules for exact numbers: a whole rational answers `/`, `//`, `%`, `**` and the bitwise
// operators as the integer it equals, a fraction by the rational rules. Each runs the VM's own operation
// (`semantics::arith`) over the two values and refuses in its words through the host's `logos_rt_fail`.
use logicaffeine_compile::interpreter::RuntimeValue as LogosVmValue;
fn logos_rt_vm_refuse(why: String) -> ! {
    unsafe { logos_rt_fail(logos_rt_value_text_of(why.into_bytes())) };
    std::process::abort()
}
fn logos_rt_vm_rational(h: i32) -> LogosVmValue {
    LogosVmValue::from_rational(unsafe { &*(h as *const Rational) }.clone())
}
fn logos_rt_rational_handle(v: LogosVmValue) -> i32 {
    let r = match v {
        LogosVmValue::Int(n) => Rational::from_i64(n),
        LogosVmValue::BigInt(b) => Rational::from_bigint((*b).clone()),
        LogosVmValue::Rational(r) => (*r).clone(),
        other => logos_rt_vm_refuse(format!("an exact operation answered a {}", other.type_name())),
    };
    Box::into_raw(Box::new(r)) as i32
}
fn logos_rt_rational_vm(
    a: i32,
    b: i32,
    op: fn(LogosVmValue, LogosVmValue) -> Result<LogosVmValue, String>,
) -> i32 {
    match op(logos_rt_vm_rational(a), logos_rt_vm_rational(b)) {
        Ok(v) => logos_rt_rational_handle(v),
        Err(why) => logos_rt_vm_refuse(why),
    }
}
#[no_mangle] pub extern "C" fn logos_rt_rational_quotient(a: i32, b: i32) -> i32 {
    logos_rt_rational_vm(a, b, logicaffeine_compile::semantics::arith::divide)
}
#[no_mangle] pub extern "C" fn logos_rt_rational_floor_quotient(a: i32, b: i32) -> i32 {
    logos_rt_rational_vm(a, b, logicaffeine_compile::semantics::arith::floor_divide)
}
#[no_mangle] pub extern "C" fn logos_rt_rational_remainder(a: i32, b: i32) -> i32 {
    logos_rt_rational_vm(a, b, logicaffeine_compile::semantics::arith::modulo)
}
#[no_mangle] pub extern "C" fn logos_rt_rational_power(a: i32, b: i32) -> i32 {
    logos_rt_rational_vm(a, b, logicaffeine_compile::semantics::arith::power)
}
// `a & b`, `a | b`, `a ^ b`, `a << b`, `a >> b` (`op` 0..=4) of two rationals: the integer two whole ones make.
#[no_mangle] pub extern "C" fn logos_rt_rational_bitwise(a: i32, b: i32, op: i32) -> i64 {
    use logicaffeine_compile::ast::stmt::BinaryOpKind;
    let kind = match op {
        0 => BinaryOpKind::BitAnd,
        1 => BinaryOpKind::BitOr,
        2 => BinaryOpKind::BitXor,
        3 => BinaryOpKind::Shl,
        _ => BinaryOpKind::Shr,
    };
    match logicaffeine_compile::semantics::arith::binary_op(kind, logos_rt_vm_rational(a), logos_rt_vm_rational(b)) {
        Ok(LogosVmValue::Int(n)) => n,
        Ok(other) => logos_rt_vm_refuse(format!("a bitwise operation answered a {}", other.type_name())),
        Err(why) => logos_rt_vm_refuse(why),
    }
}
// The order of a rational against a real by the real's exact value: `-1`, `0` or `1`, and `2` for a NaN.
#[no_mangle] pub extern "C" fn logos_rt_rational_cmp_f64(a: i32, f: f64) -> i32 {
    let a = unsafe { &*(a as *const Rational) };
    match logicaffeine_base::numeric::cmp_rational_f64_exact(a, f) {
        Some(o) => o as i32,
        None => 2,
    }
}
// The double the VM computes with when a rational meets a real.
#[no_mangle] pub extern "C" fn logos_rt_rational_to_f64(a: i32) -> f64 {
    unsafe { &*(a as *const Rational) }.to_f64()
}
// A rational where the VM takes a machine integer (`place`, an `IntegerPlace` by position): the integer a whole one
// within i64 equals, else the VM's refusal there.
#[no_mangle] pub extern "C" fn logos_rt_rational_as_i64(a: i32, place: i32) -> i64 {
    let r = unsafe { &*(a as *const Rational) };
    if let Some(n) = r.to_i64() {
        return n;
    }
    let (past_word, fraction) = logos_rt_place_refusals(place);
    logos_rt_vm_refuse(if r.is_integer() { past_word } else { fraction })
}
// What the VM says at the place `code` names (`IntegerPlace::code`) for an integer past i64 and for a fraction.
fn logos_rt_place_refusals(code: i32) -> (String, String) {
    use logicaffeine_compile::semantics::collections::{integer_place_refusals, IntegerPlace};
    integer_place_refusals(IntegerPlace::of_code(code).expect("a place code the module emits"))
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
// Civil-component constructors — the WASM mirrors of `makeDate`/`makeMoment`. Same `days_from_civil`
// core as `calendar.timegm` and every other tier, so a constructed value is bit-identical everywhere.
#[no_mangle] pub extern "C" fn logos_rt_make_date(year: i64, month: i64, day: i64) -> i32 {
    logicaffeine_base::temporal::days_from_civil(year, month as u32, day as u32) as i32
}
#[no_mangle] pub extern "C" fn logos_rt_make_moment(
    year: i64,
    month: i64,
    day: i64,
    hour: i64,
    minute: i64,
    second: i64,
) -> i64 {
    let days = logicaffeine_base::temporal::days_from_civil(year, month as u32, day as u32);
    days * logicaffeine_base::temporal::NANOS_PER_DAY
        + (hour * 3600 + minute * 60 + second) * logicaffeine_base::temporal::NANOS_PER_SECOND
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
// ---- The exact extension: a value it reaches is a handle into `logicaffeine_data`'s value model, made and
// answered by that crate's C ABI (`logicaffeine_data::abi`, built with `c-abi` and linked from its archive), which
// the module calls by name as a C program does. These adapt that ABI to the emitter's: a value laid out as a
// `Text`, a scalar sink's machine answer. A refusal stops the program with the runtime's own sentence through the
// host's `logos_rt_fail`, then traps, as every failure of a module does.
type LogosRtValue = core::ffi::c_void;
extern "C" {
    fn logos_rt_fail(text: i32);
    fn logos_rt_value_error(len: *mut usize) -> *const u8;
    fn logos_rt_value_show(a: *const LogosRtValue, len: *mut usize) -> *mut u8;
    fn logos_rt_value_text_free(ptr: *mut u8, len: usize);
    fn logos_rt_value_text(ptr: *const u8, len: usize) -> *mut LogosRtValue;
    fn logos_rt_value_length(a: *const LogosRtValue, out: *mut i64) -> i32;
    fn logos_rt_value_truth(a: *const LogosRtValue, out: *mut i32) -> i32;
    fn logos_rt_value_contains(a: *const LogosRtValue, v: *const LogosRtValue, out: *mut i32) -> i32;
    fn logos_rt_value_int_of(a: *const LogosRtValue, out: *mut i64) -> i32;
    fn logos_rt_value_int_at(
        a: *const LogosRtValue,
        past_word: *const u8,
        past_word_len: usize,
        fraction: *const u8,
        fraction_len: usize,
        out: *mut i64,
    ) -> i32;
}
unsafe fn logos_rt_value_text_of(bytes: Vec<u8>) -> i32 {
    let len = bytes.len();
    let data = bytes.leak();
    let header = std::alloc::alloc(std::alloc::Layout::from_size_align_unchecked(16, 4)) as *mut i32;
    *header.add(0) = len as i32;
    *header.add(1) = len as i32;
    *header.add(2) = data.as_ptr() as i32;
    *header.add(3) = 0;
    header as i32
}
fn logos_rt_value_stop() -> ! {
    unsafe {
        let mut len = 0usize;
        let why = logos_rt_value_error(&mut len);
        let bytes = if why.is_null() { Vec::new() } else { std::slice::from_raw_parts(why, len).to_vec() };
        logos_rt_fail(logos_rt_value_text_of(bytes));
    }
    std::process::abort()
}
#[no_mangle] pub extern "C" fn logos_rt_value_checked(h: i32) -> i32 {
    if h == 0 {
        logos_rt_value_stop()
    }
    h
}
#[no_mangle] pub extern "C" fn logos_rt_value_to_text(h: i32) -> i32 {
    unsafe {
        let mut len = 0usize;
        let shown = logos_rt_value_show(h as *const LogosRtValue, &mut len);
        if shown.is_null() {
            logos_rt_value_stop()
        }
        let bytes = std::slice::from_raw_parts(shown, len).to_vec();
        logos_rt_value_text_free(shown, len);
        logos_rt_value_text_of(bytes)
    }
}
#[no_mangle] pub extern "C" fn logos_rt_value_of_text(text: i32) -> i32 {
    let h = text as *const i32;
    let (len, data) = unsafe { (*h as usize, *h.add(2) as *const u8) };
    logos_rt_value_checked(unsafe { logos_rt_value_text(data, len) } as i32)
}
#[no_mangle] pub extern "C" fn logos_rt_value_length_i64(h: i32) -> i64 {
    let mut n = 0i64;
    if unsafe { logos_rt_value_length(h as *const LogosRtValue, &mut n) } == 0 {
        logos_rt_value_stop()
    }
    n
}
#[no_mangle] pub extern "C" fn logos_rt_value_truth_i32(h: i32) -> i32 {
    let mut truth = 0i32;
    if unsafe { logos_rt_value_truth(h as *const LogosRtValue, &mut truth) } == 0 {
        logos_rt_value_stop()
    }
    truth
}
#[no_mangle] pub extern "C" fn logos_rt_value_contains_i64(h: i32, v: i32) -> i64 {
    let mut found = 0i32;
    if unsafe { logos_rt_value_contains(h as *const LogosRtValue, v as *const LogosRtValue, &mut found) } == 0 {
        logos_rt_value_stop()
    }
    i64::from(found)
}
#[no_mangle] pub extern "C" fn logos_rt_value_to_i64(h: i32) -> i64 {
    let mut n = 0i64;
    if unsafe { logos_rt_value_int_of(h as *const LogosRtValue, &mut n) } == 0 {
        logos_rt_value_stop()
    }
    n
}
// A real number a closure of the module defines (`definedReal`): `kind` and `name` the module's Texts, `body` the
// closure's body, which the runtime reads back through the module's own `logos_program_defines(body, n)` whenever a
// refinement of the number asks.
extern "C" {
    fn logos_rt_value_defined_real(
        kind: *const u8,
        kind_len: usize,
        base: i64,
        name: *const u8,
        name_len: usize,
        call: unsafe extern "C" fn(*mut core::ffi::c_void, *mut core::ffi::c_void, i64) -> *mut LogosRtValue,
        code: *mut core::ffi::c_void,
        env: *mut core::ffi::c_void,
    ) -> *mut LogosRtValue;
    fn logos_program_defines(body: i32, n: i64) -> i32;
}
unsafe extern "C" fn logos_rt_value_read_definer(
    _code: *mut core::ffi::c_void,
    env: *mut core::ffi::c_void,
    n: i64,
) -> *mut LogosRtValue {
    logos_program_defines(env as i32, n) as *mut LogosRtValue
}
#[no_mangle] pub extern "C" fn logos_rt_value_define(kind: i32, base: i64, name: i32, body: i32) -> i32 {
    let text = |t: i32| unsafe { (*(t as *const i32) as usize, *(t as *const i32).add(2) as *const u8) };
    let ((kind_len, kind), (name_len, name)) = (text(kind), text(name));
    logos_rt_value_checked(unsafe {
        logos_rt_value_defined_real(
            kind,
            kind_len,
            base,
            name,
            name_len,
            logos_rt_value_read_definer,
            core::ptr::null_mut(),
            body as *mut core::ffi::c_void,
        )
    } as i32)
}
// A value of the runtime where the VM takes a machine integer (`place`, an `IntegerPlace::code`): the `Int` it is,
// an integer past i64 or a fraction refused in the VM's words there.
#[no_mangle] pub extern "C" fn logos_rt_value_integer_at(h: i32, place: i32) -> i64 {
    let (past_word, fraction) = logos_rt_place_refusals(place);
    let mut n = 0i64;
    let held = unsafe {
        logos_rt_value_int_at(
            h as *const LogosRtValue,
            past_word.as_ptr(),
            past_word.len(),
            fraction.as_ptr(),
            fraction.len(),
            &mut n,
        )
    };
    if held == 0 {
        logos_rt_value_stop()
    }
    n
}
"#;

/// Build the linked runtime object: `cargo build` `logicaffeine_compile` for `wasm32-unknown-unknown`
/// with `logicaffeine_data`'s C ABI (`c-abi`), locate the produced rlibs, then `rustc --emit=obj` the
/// [`BIGINT_RUNTIME_SRC`] against `base`, `data` and `compile`. Returns `(runtime object bytes, rlibs)` —
/// the build's own archives, which the std link needs beyond the sysroot set.
fn build_bigint_runtime(dir: &std::path::Path) -> R<(Vec<u8>, Vec<PathBuf>)> {
    std::fs::create_dir_all(dir)
        .map_err(|_| WasmLowerError::Unsupported("cannot create link scratch dir"))?;
    // Build `logicaffeine_compile` for wasm32 in a SHARED target dir — built ONCE across all linked tests
    // (cargo's own build lock serializes concurrent callers), NOT per-test. It transitively includes
    // `logicaffeine_base` (the numeric-tower/uuid/temporal/lanes/money runtime types) AND the `marshal`
    // codec the `wireBytes` runtime fns call. `rust-lld --gc-sections --export=main` then strips the entire
    // compiler from any program that does not reference a wire builtin, so a non-wire module stays small.
    let shared = checkout_scratch("logos_wire_compile_shared");
    let built = Command::new("cargo")
        .args([
            "build",
            "-p",
            "logicaffeine-compile",
            "--features",
            "logicaffeine-data/c-abi",
            "--target",
            "wasm32-unknown-unknown",
            "--message-format=json-render-diagnostics",
        ])
        .arg("--target-dir")
        .arg(&shared)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .stderr(std::process::Stdio::inherit())
        .output()
        .map_err(|_| WasmLowerError::Unsupported("cargo not found"))?;
    let mut messages = Vec::new();
    for line in built
        .stdout
        .split(|b| *b == b'\n')
        .filter(|line| !line.is_empty())
    {
        let message: serde_json::Value = serde_json::from_slice(line).map_err(|_| {
            WasmLowerError::Unsupported("cargo returned an invalid artifact message")
        })?;
        if let Some(rendered) = message
            .get("message")
            .and_then(|m| m.get("rendered"))
            .and_then(|r| r.as_str())
        {
            eprint!("{rendered}");
        }
        messages.push(message);
    }
    if !built.status.success()
        || !messages
            .iter()
            .any(|message| message["reason"] == "build-finished" && message["success"] == true)
    {
        return Err(WasmLowerError::Unsupported(
            "cargo failed to build logicaffeine-compile for wasm32",
        ));
    }
    let wasm_deps = shared.join("wasm32-unknown-unknown/debug/deps");
    let host_deps = shared.join("debug/deps");
    let mut libraries = std::collections::BTreeSet::new();
    let mut base_roots = std::collections::BTreeSet::new();
    let mut compile_roots = std::collections::BTreeSet::new();
    for message in &messages {
        if message["reason"] != "compiler-artifact" {
            continue;
        }
        let Some(paths) = message.get("filenames").and_then(|p| p.as_array()) else {
            continue;
        };
        for path in paths.iter().filter_map(|p| p.as_str()).map(PathBuf::from) {
            // Cargo reports host build dependencies too. Only this target's
            // rlibs belong to the linked wasm dependency closure.
            if !path.starts_with(wasm_deps.parent().expect("target profile directory"))
                || path.extension().is_none_or(|e| e != "rlib")
            {
                continue;
            }
            match message["target"]["name"].as_str() {
                Some("logicaffeine_base") => {
                    base_roots.insert(path.clone());
                }
                Some("logicaffeine_compile") => {
                    compile_roots.insert(path.clone());
                }
                _ => {}
            }
            libraries.insert(path);
        }
    }
    if base_roots.len() != 1 || compile_roots.len() != 1 {
        return Err(WasmLowerError::Unsupported(
            "cargo did not report unique wasm runtime roots",
        ));
    }
    let base = base_roots.into_iter().next().expect("one base root");
    let compile = compile_roots.into_iter().next().expect("one compile root");
    let rlibs: Vec<PathBuf> = libraries.into_iter().collect();
    let rs = dir.join("bigint_rt.rs");
    let obj = dir.join("bigint_rt.o");
    std::fs::write(&rs, BIGINT_RUNTIME_SRC)
        .map_err(|_| WasmLowerError::Unsupported("cannot write runtime source"))?;
    let ok = Command::new("rustc")
        .args([
            "--target",
            "wasm32-unknown-unknown",
            "--edition",
            "2021",
            "--crate-type=lib",
            "--emit=obj",
            "-Copt-level=2",
        ])
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
        .map_err(|_| {
            WasmLowerError::Unsupported("failed to invoke rustc for the bigint+wire runtime")
        })?
        .success();
    if !ok {
        return Err(WasmLowerError::Unsupported(
            "rustc failed to build the bigint+wire runtime object",
        ));
    }
    let bytes = std::fs::read(&obj)
        .map_err(|_| WasmLowerError::Unsupported("cannot read the bigint runtime object"))?;
    // Offer exactly the successful build's target artifacts, in deterministic
    // order. A directory scan can include stale feature or source variants.
    Ok((bytes, rlibs))
}

/// Each checkout owns its build cache; unrelated source trees never reuse one
/// another's Cargo artifacts even when their package names are identical.
fn checkout_scratch(name: &str) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    env!("CARGO_MANIFEST_DIR").hash(&mut hash);
    std::env::temp_dir().join(format!("{name}-{:016x}", hash.finish()))
}

/// A call owns its temporary files. The persistent Cargo build cache is separate.
struct RuntimeScratch(PathBuf);

impl RuntimeScratch {
    fn new() -> R<Self> {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        loop {
            let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("logos-runtime-{}-{id}", std::process::id()));
            match std::fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(_) => {
                    return Err(WasmLowerError::Unsupported(
                        "cannot create runtime scratch directory",
                    ))
                }
            }
        }
    }
}

impl Drop for RuntimeScratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The lock is a sibling of the Cargo target, so replacing that target does not
/// replace the synchronization object. Closing the file releases the OS lock.
fn lock_runtime_build() -> R<std::fs::File> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let path = checkout_scratch("logos_wire_compile_shared").with_extension("lock");
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map_err(|_| WasmLowerError::Unsupported("cannot open runtime build lock"))?;
        fs2::FileExt::lock_exclusive(&file)
            .map_err(|_| WasmLowerError::Unsupported("cannot lock runtime build cache"))?;
        Ok(file)
    }
    #[cfg(target_arch = "wasm32")]
    {
        Err(WasmLowerError::Unsupported(
            "native toolchain linking is unavailable in the browser",
        ))
    }
}

struct RuntimeArchive {
    name: std::ffi::OsString,
    bytes: Vec<u8>,
}

/// One complete runtime snapshot per process. No cached path borrows the mutable
/// Cargo target, and a linked program only clones an Arc, not the archive bytes.
struct RuntimeSnapshot {
    object: Vec<u8>,
    archives: Vec<RuntimeArchive>,
}

impl RuntimeSnapshot {
    fn build() -> R<Self> {
        let _build_lock = lock_runtime_build()?;
        let scratch = RuntimeScratch::new()?;
        let (object, mut paths) = build_bigint_runtime(&scratch.0)?;
        paths.extend(wasm_all_sysroot_rlibs()?);
        let mut archives = Vec::with_capacity(paths.len());
        for path in paths {
            let name = path
                .file_name()
                .ok_or(WasmLowerError::Unsupported(
                    "runtime archive has no filename",
                ))?
                .to_owned();
            let bytes = std::fs::read(&path).map_err(|_| {
                WasmLowerError::Unsupported("cannot snapshot the completed runtime archive")
            })?;
            archives.push(RuntimeArchive { name, bytes });
        }
        Ok(Self { object, archives })
    }

    fn materialize(&self, dir: &std::path::Path) -> R<Vec<PathBuf>> {
        let mut paths = Vec::with_capacity(self.archives.len());
        for (index, archive) in self.archives.iter().enumerate() {
            // Preserve archive basenames, including any equal names from
            // different artifact roots, without allowing them to overwrite.
            let parent = dir.join(format!("archive-{index}"));
            std::fs::create_dir(&parent).map_err(|_| {
                WasmLowerError::Unsupported("cannot create runtime archive directory")
            })?;
            let path = parent.join(&archive.name);
            std::fs::write(&path, &archive.bytes)
                .map_err(|_| WasmLowerError::Unsupported("cannot materialize runtime archive"))?;
            paths.push(path);
        }
        Ok(paths)
    }
}

static BIGINT_RUNTIME: std::sync::Mutex<Option<std::sync::Arc<RuntimeSnapshot>>> =
    std::sync::Mutex::new(None);

/// Link against an immutable runtime snapshot. A failed build leaves the cache
/// empty for retry; successful or failed links release their own disk scratch.
pub(crate) fn link_relocatable_bigint(relocatable: &[u8]) -> R<Vec<u8>> {
    let runtime = {
        let mut guard = BIGINT_RUNTIME
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if guard.is_none() {
            *guard = Some(std::sync::Arc::new(RuntimeSnapshot::build()?));
        }
        std::sync::Arc::clone(guard.as_ref().expect("runtime cache populated above"))
    };
    let scratch = RuntimeScratch::new()?;
    let archives = runtime.materialize(&scratch.0)?;
    link_objects_with_rlibs(
        &[relocatable, &runtime.object],
        &archives,
        false,
        &scratch.0,
    )
}

#[cfg(test)]
#[path = "../../../../logicaffeine_tests/unit/logicaffeine_compile/vm/wasm/link/tests.rs"]
mod tests;

#[cfg(all(test, unix))]
#[path = "../../../../logicaffeine_tests/unit/logicaffeine_compile/vm/wasm/link/cache_artifact_tests.rs"]
mod cache_artifact_tests;

#[cfg(all(test, unix))]
#[path = "../../../../logicaffeine_tests/unit/logicaffeine_compile/vm/wasm/link/scratch_isolation.rs"]
mod scratch_isolation;

#[cfg(all(test, unix))]
#[path = "../../../../logicaffeine_tests/unit/logicaffeine_compile/vm/wasm/link/cache_snapshot_tests.rs"]
mod cache_snapshot_tests;

#[cfg(all(test, unix))]
#[path = "../../../../logicaffeine_tests/unit/logicaffeine_compile/vm/wasm/link/cache_process_tests.rs"]
mod cache_process_tests;
