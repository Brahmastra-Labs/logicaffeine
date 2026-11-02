//! Convert a finished self-contained wasm module (from [`super::module`]) into an LLVM-compatible
//! RELOCATABLE object, so [`super::link`] can link the real Rust runtime into it.
//!
//! # Why a post-pass (not emitter changes)
//!
//! The whole-program emitter emits `call <function index>` with the FINAL index. After linking with
//! the runtime, every function index shifts (runtime functions are inserted), so each `call` needs a
//! relocation. Rather than thread a recorder through the emitter's ~20 call sites (risking the crypto
//! / guide / benchmark corpus that path already passes), this pass rewrites the finished module: it
//! decodes each function body, rewrites every `call` target to a fixed 5-byte padded LEB, and appends
//! the `linking` symbol table + `reloc.CODE` section. The decoder is TOTAL on the opcode set the
//! emitter uses and REFUSES on anything else (so the caller falls back to the standalone module) —
//! it can never emit a wrong relocation, only decline.
//!
//! Scope today: the CALL relocations (function-index references). Memory/data relocation and
//! `call_indirect` type relocations (needed once a program's own heap coexists with the runtime's
//! allocator, and for closures) are the next slice; this pass refuses a module that would need them.

use super::WasmLowerError;

type R<T> = Result<T, WasmLowerError>;

/// Append the unsigned LEB128 of `v`.
fn leb_u(mut v: u64, out: &mut Vec<u8>) {
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v != 0 {
            out.push(b | 0x80);
        } else {
            out.push(b);
            break;
        }
    }
}

/// Append `v` as a FIXED 5-byte unsigned LEB128 (the relocatable call-target slot).
fn leb_u32_padded(v: u32, out: &mut Vec<u8>) {
    let mut v = v;
    for i in 0..5 {
        let mut b = (v & 0x7f) as u8;
        v >>= 7;
        if i < 4 {
            b |= 0x80;
        }
        out.push(b);
    }
}

/// Read an unsigned LEB128 at `*i`, advancing it.
fn read_uleb(bytes: &[u8], i: &mut usize) -> R<u64> {
    let mut r = 0u64;
    let mut s = 0;
    loop {
        let b = *bytes
            .get(*i)
            .ok_or(WasmLowerError::Unsupported("truncated LEB"))?;
        *i += 1;
        r |= u64::from(b & 0x7f) << s;
        if b & 0x80 == 0 {
            break;
        }
        s += 7;
        if s >= 64 {
            return Err(WasmLowerError::Unsupported("overlong LEB"));
        }
    }
    Ok(r)
}

/// Skip a signed LEB128 at `*i`.
fn skip_sleb(bytes: &[u8], i: &mut usize) -> R<()> {
    loop {
        let b = *bytes
            .get(*i)
            .ok_or(WasmLowerError::Unsupported("truncated SLEB"))?;
        *i += 1;
        if b & 0x80 == 0 {
            break;
        }
    }
    Ok(())
}

/// Skip a block type (`0x40` empty, a single valtype byte, or a signed-LEB type index).
fn skip_blocktype(bytes: &[u8], i: &mut usize) -> R<()> {
    let b = *bytes
        .get(*i)
        .ok_or(WasmLowerError::Unsupported("truncated blocktype"))?;
    // 0x40 = empty; 0x7f/7e/7d/7c i32/i64/f32/f64; 0x7b v128; 0x70/6f funcref/externref.
    if matches!(b, 0x40 | 0x7f | 0x7e | 0x7d | 0x7c | 0x7b | 0x70 | 0x6f) {
        *i += 1;
        Ok(())
    } else {
        skip_sleb(bytes, i) // an s33 type index
    }
}

/// Rewrite one function body so every `call` target becomes a 5-byte padded LEB, returning the new
/// body and, for each rewritten call, `(offset of the padded LEB within the new body, function
/// index)`. Decodes the whole body (locals + instruction stream) so call opcodes are found exactly —
/// refusing (Err) on any opcode outside the emitter's known set, and on `call_indirect` (needs a type
/// relocation this slice doesn't emit).
fn rewrite_body(body: &[u8]) -> R<(Vec<u8>, Vec<(u32, u32)>, Vec<(u32, u32)>)> {
    let mut out = Vec::with_capacity(body.len() + 8);
    let mut calls = Vec::new();
    let mut globals = Vec::new();
    let mut i = 0usize;

    // Locals: vec of (count, valtype). Copy verbatim.
    let start = i;
    let n_groups = read_uleb(body, &mut i)?;
    for _ in 0..n_groups {
        read_uleb(body, &mut i)?; // count
        i += 1; // valtype byte
        if i > body.len() {
            return Err(WasmLowerError::Unsupported("truncated locals"));
        }
    }
    out.extend_from_slice(&body[start..i]);

    // Instructions until the body ends (the outermost `end`). Track nesting so an inner `end` doesn't
    // terminate the walk.
    let mut depth = 0i32;
    loop {
        let op_start = i;
        let op = *body
            .get(i)
            .ok_or(WasmLowerError::Unsupported("truncated body"))?;
        i += 1;
        match op {
            // Control with a block type.
            0x02 | 0x03 | 0x04 => {
                skip_blocktype(body, &mut i)?;
                depth += 1;
                out.extend_from_slice(&body[op_start..i]);
            }
            0x05 => out.extend_from_slice(&body[op_start..i]), // else
            0x0b => {
                out.extend_from_slice(&body[op_start..i]); // end
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            0x0c | 0x0d => {
                read_uleb(body, &mut i)?; // br / br_if : labelidx
                out.extend_from_slice(&body[op_start..i]);
            }
            0x0e => {
                let n = read_uleb(body, &mut i)?; // br_table
                for _ in 0..=n {
                    read_uleb(body, &mut i)?;
                }
                out.extend_from_slice(&body[op_start..i]);
            }
            0x00 | 0x01 | 0x0f | 0x1a | 0x1b => out.extend_from_slice(&body[op_start..i]), // unreachable/nop/return/drop/select
            // call — REWRITE the function index to a padded LEB and record the relocation.
            0x10 => {
                let func = read_uleb(body, &mut i)? as u32;
                out.push(0x10);
                calls.push((out.len() as u32, func));
                leb_u32_padded(func, &mut out);
            }
            0x11 => {
                return Err(WasmLowerError::Unsupported(
                    "call_indirect needs a type relocation",
                ))
            }
            // local.get/set/tee — a local index, not relocated.
            0x20 | 0x21 | 0x22 => {
                read_uleb(body, &mut i)?;
                out.extend_from_slice(&body[op_start..i]);
            }
            // global.get/set — REWRITE the global index (lld shifts globals by inserting
            // `__stack_pointer`) to a padded LEB and record the relocation.
            0x23 | 0x24 => {
                let g = read_uleb(body, &mut i)? as u32;
                out.push(op);
                globals.push((out.len() as u32, g));
                leb_u32_padded(g, &mut out);
            }
            // Memory load/store: memarg = align LEB + offset LEB.
            0x28..=0x3e => {
                read_uleb(body, &mut i)?;
                read_uleb(body, &mut i)?;
                out.extend_from_slice(&body[op_start..i]);
            }
            0x3f | 0x40 => {
                i += 1; // memory.size / grow : reserved byte
                out.extend_from_slice(&body[op_start..i]);
            }
            0x41 | 0x42 => {
                skip_sleb(body, &mut i)?; // i32.const / i64.const
                out.extend_from_slice(&body[op_start..i]);
            }
            0x43 => {
                i += 4; // f32.const
                out.extend_from_slice(&body[op_start..i]);
            }
            0x44 => {
                i += 8; // f64.const
                out.extend_from_slice(&body[op_start..i]);
            }
            // Numeric / comparison / conversion ops — no immediate.
            0x45..=0xc4 => out.extend_from_slice(&body[op_start..i]),
            // Saturating-truncation prefix (`i64.trunc_sat_f64_s` etc.) — 0xFC + subopcode 0..=7.
            0xfc => {
                let sub = read_uleb(body, &mut i)?;
                if sub > 7 {
                    return Err(WasmLowerError::Unsupported("unsupported 0xFC subopcode"));
                }
                out.extend_from_slice(&body[op_start..i]);
            }
            _ => {
                return Err(WasmLowerError::Unsupported(
                    "unknown opcode in relocatable rewrite",
                ))
            }
        }
        if i > body.len() {
            return Err(WasmLowerError::Unsupported("body overran"));
        }
    }
    Ok((out, calls, globals))
}

/// A parsed top-level section: its id and body byte-range within the module.
struct Sec {
    id: u8,
    start: usize,
    end: usize,
}

/// Split a module into its top-level sections (ignoring the 8-byte header).
fn sections(module: &[u8]) -> R<Vec<Sec>> {
    if module.len() < 8 || &module[0..4] != b"\0asm" {
        return Err(WasmLowerError::Unsupported("not a wasm module"));
    }
    let mut secs = Vec::new();
    let mut i = 8;
    while i < module.len() {
        let id = module[i];
        i += 1;
        let size = read_uleb(module, &mut i)? as usize;
        let start = i;
        let end = start + size;
        if end > module.len() {
            return Err(WasmLowerError::Unsupported("section overruns module"));
        }
        secs.push(Sec { id, start, end });
        i = end;
    }
    Ok(secs)
}

/// The number of imported FUNCTIONS + each import's field name (function imports only), from the
/// import section body.
fn parse_imports(body: &[u8]) -> R<Vec<String>> {
    let mut names = Vec::new();
    let mut i = 0;
    let count = read_uleb(body, &mut i)?;
    for _ in 0..count {
        let ml = read_uleb(body, &mut i)? as usize;
        i += ml;
        let fl = read_uleb(body, &mut i)? as usize;
        let field = String::from_utf8_lossy(&body[i..i + fl]).to_string();
        i += fl;
        let kind = body[i];
        i += 1;
        match kind {
            0x00 => {
                read_uleb(body, &mut i)?; // type index
                names.push(field);
            }
            0x01 => {
                i += 1;
                let f = body[i];
                i += 1;
                read_uleb(body, &mut i)?;
                if f & 1 != 0 {
                    read_uleb(body, &mut i)?;
                }
            }
            0x02 => {
                let f = body[i];
                i += 1;
                read_uleb(body, &mut i)?;
                if f & 1 != 0 {
                    read_uleb(body, &mut i)?;
                }
            }
            0x03 => i += 2,
            _ => return Err(WasmLowerError::Unsupported("unknown import kind")),
        }
    }
    Ok(names)
}

/// The functions the export section names other than `main`, by function index: a function the
/// module defines for an object it links with to call by name.
fn parse_exported_functions(body: &[u8]) -> R<std::collections::HashMap<u32, String>> {
    let mut named = std::collections::HashMap::new();
    let mut i = 0;
    let count = read_uleb(body, &mut i)?;
    for _ in 0..count {
        let len = read_uleb(body, &mut i)? as usize;
        let name = body
            .get(i..i + len)
            .ok_or(WasmLowerError::Unsupported("truncated export name"))?;
        let name = String::from_utf8_lossy(name).to_string();
        i += len;
        let kind = *body
            .get(i)
            .ok_or(WasmLowerError::Unsupported("truncated export"))?;
        i += 1;
        let index = read_uleb(body, &mut i)? as u32;
        if kind == 0x00 && name != "main" {
            named.insert(index, name);
        }
    }
    Ok(named)
}

/// Append a section (id, length, body) to `out`.
fn put_section(out: &mut Vec<u8>, id: u8, body: &[u8]) {
    out.push(id);
    leb_u(body.len() as u64, out);
    out.extend_from_slice(body);
}

/// Append a custom section (`name`, `body`).
fn put_custom(out: &mut Vec<u8>, name: &str, body: &[u8]) {
    let mut payload = Vec::new();
    leb_u(name.len() as u64, &mut payload);
    payload.extend_from_slice(name.as_bytes());
    payload.extend_from_slice(body);
    put_section(out, 0, &payload);
}

/// Convert a self-contained module into a relocatable object. The function-index space is
/// `[imports 0..K][main = K][functions = K+1..]`; the symbol table lists one function symbol per
/// index in order (imports UNDEFINED with their host name; defined functions with `main`, the name
/// the module exports them by, or `fnN`), so a
/// call's relocation references symbol index = function index. Refuses (Err) any module whose code
/// uses an opcode the rewriter doesn't understand, so the caller can fall back to the standalone form.
pub(crate) fn module_to_relocatable(module: &[u8]) -> R<Vec<u8>> {
    let secs = sections(module)?;

    // This slice relocates FUNCTION- and GLOBAL-index references. Modules with a defined memory + data
    // segments (the heap value model) or an element/table (closures) additionally need memory-address/
    // data/type relocations + data symbols — the next slices. Refuse them here (sound: the caller falls
    // back to the standalone module) rather than emit an object lld would mangle.
    for id in [5u8, 9, 11] {
        if secs.iter().any(|s| s.id == id) {
            return Err(WasmLowerError::Unsupported(
                "relocatable transform: program needs memory/data/element relocations (not yet)",
            ));
        }
    }

    let sec = |id: u8| {
        secs.iter()
            .find(|s| s.id == id)
            .map(|s| &module[s.start..s.end])
    };

    let imports = sec(2).map(parse_imports).transpose()?.unwrap_or_default();
    let k = imports.len() as u32;
    let exported = sec(7).map(parse_exported_functions).transpose()?.unwrap_or_default();

    let func_body = sec(3).ok_or(WasmLowerError::Unsupported("no function section"))?;
    let mut fi = 0;
    let num_defined = read_uleb(func_body, &mut fi)? as u32; // main + user functions

    // Globals are all DEFINED (our modules import only host functions), indexed 0..num_globals.
    let num_globals = match sec(6) {
        Some(g) => {
            let mut gi = 0;
            read_uleb(g, &mut gi)? as u32
        }
        None => 0,
    };
    let num_func_symbols = k + num_defined;

    // Rewrite the code section: each entry is `bodysize, body`. Produce a new code section and collect
    // relocations as (offset within the code section content, symbol index). Function calls reference
    // symbol index = function index; global.get/set reference symbol index = num_func_symbols + global.
    let code_body = sec(10).ok_or(WasmLowerError::Unsupported("no code section"))?;
    let mut ci = 0;
    let n_bodies = read_uleb(code_body, &mut ci)?;
    if n_bodies != u64::from(num_defined) {
        return Err(WasmLowerError::Unsupported("code/function count mismatch"));
    }
    let mut new_code = Vec::new();
    leb_u(n_bodies, &mut new_code);
    let mut func_relocs: Vec<(u32, u32)> = Vec::new();
    let mut global_relocs: Vec<(u32, u32)> = Vec::new();
    for _ in 0..n_bodies {
        let sz = read_uleb(code_body, &mut ci)? as usize;
        let body = &code_body[ci..ci + sz];
        ci += sz;
        let (new_body, calls, globals) = rewrite_body(body)?;
        leb_u(new_body.len() as u64, &mut new_code);
        let body_start = new_code.len() as u32;
        new_code.extend_from_slice(&new_body);
        for (off, func) in calls {
            func_relocs.push((body_start + off, func));
        }
        for (off, g) in globals {
            global_relocs.push((body_start + off, num_func_symbols + g));
        }
    }

    // Symbol table: one FUNCTION symbol per function index (in order), then one GLOBAL symbol per
    // defined global. A call's reloc references symbol index = function index; a global.get/set's
    // references num_func_symbols + global index.
    const WASM_SYM_UNDEFINED: u32 = 0x10;
    const WASM_SYM_EXPORTED: u32 = 0x04;
    const WASM_SYM_BINDING_LOCAL: u32 = 0x02;
    let mut symtab = Vec::new();
    leb_u(u64::from(num_func_symbols + num_globals), &mut symtab);
    for (idx, name) in imports.iter().enumerate() {
        symtab.push(0x00); // SYMTAB_FUNCTION
        leb_u(u64::from(WASM_SYM_UNDEFINED), &mut symtab);
        leb_u(idx as u64, &mut symtab); // import (function) index
        let _ = name; // name comes from the import
    }
    for d in 0..num_defined {
        let fidx = k + d;
        let (name, flags) = if d == 0 {
            ("main".to_string(), WASM_SYM_EXPORTED)
        } else if let Some(name) = exported.get(&fidx) {
            (name.clone(), 0)
        } else {
            (format!("fn{d}"), 0)
        };
        symtab.push(0x00);
        leb_u(u64::from(flags), &mut symtab);
        leb_u(u64::from(fidx), &mut symtab);
        leb_u(name.len() as u64, &mut symtab);
        symtab.extend_from_slice(name.as_bytes());
    }
    for g in 0..num_globals {
        symtab.push(0x02); // SYMTAB_GLOBAL
        leb_u(u64::from(WASM_SYM_BINDING_LOCAL), &mut symtab); // local: private to this program
        leb_u(u64::from(g), &mut symtab); // global index
        let name = format!("g{g}");
        leb_u(name.len() as u64, &mut symtab);
        symtab.extend_from_slice(name.as_bytes());
    }
    let mut linking = Vec::new();
    leb_u(2, &mut linking); // metadata version
    linking.push(0x08); // WASM_SYMBOL_TABLE
    leb_u(symtab.len() as u64, &mut linking);
    linking.extend_from_slice(&symtab);

    // We re-emit every original non-custom section verbatim except CODE (rewritten) and EXPORT (id 7,
    // dropped): its function index is the pre-link value and would export the wrong function after the
    // shift — `main` is exported instead via its symbol's EXPORTED flag + lld's `--export=main`, and
    // the memory via lld's default. `reloc.CODE`'s target section index is CODE's 0-based position in
    // that emitted order.
    let reemit = |id: u8| id != 0 && id != 7;
    let emitted: Vec<u8> = secs.iter().filter(|s| reemit(s.id)).map(|s| s.id).collect();
    let code_index = emitted
        .iter()
        .position(|&id| id == 10)
        .ok_or(WasmLowerError::Unsupported("no code section"))? as u32;

    const R_WASM_FUNCTION_INDEX_LEB: u8 = 0x00;
    const R_WASM_GLOBAL_INDEX_LEB: u8 = 0x07;
    // Merge function + global relocations and sort by offset (the reloc section is offset-ordered).
    let mut all: Vec<(u32, u8, u32)> = Vec::new();
    all.extend(
        func_relocs
            .iter()
            .map(|&(o, s)| (o, R_WASM_FUNCTION_INDEX_LEB, s)),
    );
    all.extend(
        global_relocs
            .iter()
            .map(|&(o, s)| (o, R_WASM_GLOBAL_INDEX_LEB, s)),
    );
    all.sort_by_key(|&(o, _, _)| o);
    let mut reloc = Vec::new();
    leb_u(u64::from(code_index), &mut reloc);
    leb_u(all.len() as u64, &mut reloc);
    for (off, ty, sym) in &all {
        reloc.push(*ty);
        leb_u(u64::from(*off), &mut reloc);
        leb_u(u64::from(*sym), &mut reloc);
    }

    // Reassemble: header, every original non-custom section (CODE rewritten), then linking + reloc.CODE.
    let mut out = Vec::new();
    out.extend_from_slice(b"\0asm");
    out.extend_from_slice(&[0x01, 0x00, 0x00, 0x00]);
    for s in &secs {
        if !reemit(s.id) {
            continue; // drop original custom sections + the pre-link export section
        }
        if s.id == 10 {
            put_section(&mut out, 10, &new_code);
        } else {
            put_section(&mut out, s.id, &module[s.start..s.end]);
        }
    }
    put_custom(&mut out, "linking", &linking);
    put_custom(&mut out, "reloc.CODE", &reloc);
    Ok(out)
}

#[cfg(test)]
#[path = "../../../../logicaffeine_tests/unit/logicaffeine_compile/vm/wasm/reloc/tests.rs"]
mod tests;
