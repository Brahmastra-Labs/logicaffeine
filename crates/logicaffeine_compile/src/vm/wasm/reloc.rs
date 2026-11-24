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
//! # Memory
//!
//! A module that defines its own memory declares its static layout in a `logos.static` custom section
//! (written by [`super::module`]): the heap's base, where the text pool starts, and which globals hold
//! the heap's pointer and end. Every address into the static region `[0, heap_base)` in its code is a
//! five-byte LEB, which no other emitter writes, so this pass finds them by their encoding. The object
//! imports `env.__linear_memory` instead of defining a memory; the static region becomes one data
//! segment with a local data symbol, every static address a memory-address relocation against it (the
//! heap's base against the linker's `__heap_base`), each interned Text's pointer to its own bytes a data
//! relocation, and `main` begins by seating the heap at `__heap_base` with its end at memory's end. The
//! allocator grows only past `__heap_end`, so pages another allocator in the link grows are never its.
//!
//! A module with a memory or data and no `logos.static`, or with a table (closures, `call_indirect`),
//! is refused; a five-byte LEB in a module without `logos.static` is left as written.

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

/// Read a signed LEB128 at `*i`, advancing it: the value and how many bytes it took.
fn read_sleb(bytes: &[u8], i: &mut usize) -> R<(i64, usize)> {
    let start = *i;
    let mut r = 0i64;
    let mut shift = 0;
    loop {
        let b = *bytes
            .get(*i)
            .ok_or(WasmLowerError::Unsupported("truncated SLEB"))?;
        *i += 1;
        r |= i64::from(b & 0x7f) << shift;
        shift += 7;
        if b & 0x80 == 0 {
            if shift < 64 && b & 0x40 != 0 {
                r |= -1i64 << shift;
            }
            break;
        }
        if shift >= 64 {
            return Err(WasmLowerError::Unsupported("overlong SLEB"));
        }
    }
    Ok((r, *i - start))
}

/// Append the signed LEB128 of `v`.
fn leb_s(mut v: i64, out: &mut Vec<u8>) {
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if (v == 0 && b & 0x40 == 0) || (v == -1 && b & 0x40 != 0) {
            out.push(b);
            break;
        }
        out.push(b | 0x80);
    }
}

/// Whether a LEB of `len` bytes holding `v` is the five-byte form [`super::encode::leb_u32_static`] and
/// [`super::encode::leb_i32_static`] write for a static address (the same bytes, an address being never
/// negative): longer than `v` needs.
fn padded(v: u64, len: usize) -> bool {
    let mut minimal = Vec::new();
    leb_u(v, &mut minimal);
    len == 5 && minimal.len() < 5
}

/// [`padded`] for a signed LEB (an `i32.const`): five bytes where the value's own signed encoding is shorter.
fn padded_signed(v: i64, len: usize) -> bool {
    let mut minimal = Vec::new();
    leb_s(v, &mut minimal);
    len == 5 && minimal.len() < 5
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

/// A static address in a rewritten body: where its five-byte LEB starts, its value, and whether it is an
/// `i32.const` (a signed LEB) or a load/store offset (unsigned).
#[derive(Clone, Copy)]
struct StaticAddr {
    offset: u32,
    value: u32,
    signed: bool,
}

/// A body rewritten for linking: its bytes, and where each padded call index, global index and static
/// address sits in them.
#[derive(Default)]
struct Rewritten {
    body: Vec<u8>,
    calls: Vec<(u32, u32)>,
    globals: Vec<(u32, u32)>,
    statics: Vec<StaticAddr>,
}

/// Code placed at the start of a body, after its locals, with its global indices and static addresses at
/// offsets from its own start.
#[derive(Default)]
struct Prologue {
    code: Vec<u8>,
    globals: Vec<(u32, u32)>,
    statics: Vec<StaticAddr>,
}

/// Rewrite one function body for linking: `prologue` first, then every `call` and `global.get/set`
/// index re-encoded as a fixed 5-byte LEB and recorded, and every five-byte `i32.const` or load/store
/// offset recorded as a static address. Decodes the whole body (locals + instruction stream) so each
/// site is found exactly, refusing (Err) on any opcode outside the emitter's known set and on
/// `call_indirect` (it needs a type relocation this pass does not emit).
fn rewrite_body(body: &[u8], prologue: &Prologue) -> R<Rewritten> {
    let mut r = Rewritten {
        body: Vec::with_capacity(body.len() + prologue.code.len() + 8),
        ..Rewritten::default()
    };
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
    r.body.extend_from_slice(&body[start..i]);
    let base = r.body.len() as u32;
    r.body.extend_from_slice(&prologue.code);
    r.globals.extend(prologue.globals.iter().map(|&(o, g)| (base + o, g)));
    r.statics.extend(prologue.statics.iter().map(|a| StaticAddr { offset: base + a.offset, ..*a }));

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
                r.body.extend_from_slice(&body[op_start..i]);
            }
            0x05 => r.body.extend_from_slice(&body[op_start..i]), // else
            0x0b => {
                r.body.extend_from_slice(&body[op_start..i]); // end
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            0x0c | 0x0d => {
                read_uleb(body, &mut i)?; // br / br_if : labelidx
                r.body.extend_from_slice(&body[op_start..i]);
            }
            0x0e => {
                let n = read_uleb(body, &mut i)?; // br_table
                for _ in 0..=n {
                    read_uleb(body, &mut i)?;
                }
                r.body.extend_from_slice(&body[op_start..i]);
            }
            0x00 | 0x01 | 0x0f | 0x1a | 0x1b => r.body.extend_from_slice(&body[op_start..i]), // unreachable/nop/return/drop/select
            // call — REWRITE the function index to a padded LEB and record the relocation.
            0x10 => {
                let func = read_uleb(body, &mut i)? as u32;
                r.body.push(0x10);
                r.calls.push((r.body.len() as u32, func));
                leb_u32_padded(func, &mut r.body);
            }
            0x11 => {
                return Err(WasmLowerError::Unsupported(
                    "call_indirect needs a type relocation",
                ))
            }
            // local.get/set/tee — a local index, not relocated.
            0x20 | 0x21 | 0x22 => {
                read_uleb(body, &mut i)?;
                r.body.extend_from_slice(&body[op_start..i]);
            }
            // global.get/set — REWRITE the global index (lld shifts globals by inserting
            // `__stack_pointer`) to a padded LEB and record the relocation.
            0x23 | 0x24 => {
                let g = read_uleb(body, &mut i)? as u32;
                r.body.push(op);
                r.globals.push((r.body.len() as u32, g));
                leb_u32_padded(g, &mut r.body);
            }
            // Memory load/store: memarg = align LEB + offset LEB. A five-byte offset is a static address.
            0x28..=0x3e => {
                read_uleb(body, &mut i)?;
                let at = i;
                let offset = read_uleb(body, &mut i)?;
                if padded(offset, i - at) {
                    r.statics.push(StaticAddr {
                        offset: (r.body.len() + (at - op_start)) as u32,
                        value: offset as u32,
                        signed: false,
                    });
                }
                r.body.extend_from_slice(&body[op_start..i]);
            }
            0x3f | 0x40 => {
                i += 1; // memory.size / grow : reserved byte
                r.body.extend_from_slice(&body[op_start..i]);
            }
            // i32.const: five bytes holding a value that fits fewer is a static address.
            0x41 => {
                let at = i;
                let (value, len) = read_sleb(body, &mut i)?;
                if value >= 0 && padded_signed(value, len) {
                    r.statics.push(StaticAddr {
                        offset: (r.body.len() + (at - op_start)) as u32,
                        value: value as u32,
                        signed: true,
                    });
                }
                r.body.extend_from_slice(&body[op_start..i]);
            }
            0x42 => {
                skip_sleb(body, &mut i)?; // i64.const
                r.body.extend_from_slice(&body[op_start..i]);
            }
            0x43 => {
                i += 4; // f32.const
                r.body.extend_from_slice(&body[op_start..i]);
            }
            0x44 => {
                i += 8; // f64.const
                r.body.extend_from_slice(&body[op_start..i]);
            }
            // Numeric / comparison / conversion ops — no immediate.
            0x45..=0xc4 => r.body.extend_from_slice(&body[op_start..i]),
            // Saturating truncation (0..=7), and `memory.copy` (10: two memory indices) / `memory.fill` (11: one).
            0xfc => {
                let sub = read_uleb(body, &mut i)?;
                match sub {
                    0..=7 => {}
                    10 => i += 2,
                    11 => i += 1,
                    _ => return Err(WasmLowerError::Unsupported("unsupported 0xFC subopcode")),
                }
                r.body.extend_from_slice(&body[op_start..i]);
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
    Ok(r)
}

/// The static layout a module that defines its memory declares in its `logos.static` custom section.
struct StaticLayout {
    /// Where the heap begins: the static region is `[0, heap_base)`.
    heap_base: u32,
    /// Where the text pool (the data section's one segment) begins.
    static_base: u32,
    heap_ptr_global: u32,
    heap_end_global: u32,
}

/// The module's `logos.static` section, if it has one.
fn static_layout(module: &[u8], secs: &[Sec]) -> R<Option<StaticLayout>> {
    for sec in secs.iter().filter(|s| s.id == 0) {
        let body = &module[sec.start..sec.end];
        let mut i = 0;
        let len = read_uleb(body, &mut i)? as usize;
        if body.get(i..i + len) != Some(b"logos.static".as_slice()) {
            continue;
        }
        i += len;
        if body.get(i) != Some(&1) {
            return Err(WasmLowerError::Unsupported("an unknown logos.static version"));
        }
        i += 1;
        let mut field = || read_uleb(body, &mut i).map(|v| v as u32);
        return Ok(Some(StaticLayout {
            heap_base: field()?,
            static_base: field()?,
            heap_ptr_global: field()?,
            heap_end_global: field()?,
        }));
    }
    Ok(None)
}

/// `main`'s first instructions in a linked module: `__heap_ptr = __heap_base`, `__heap_end` = memory's end.
fn heap_prologue(layout: &StaticLayout) -> Prologue {
    let mut code = vec![0x41]; // i32.const __heap_base
    leb_u32_padded(layout.heap_base, &mut code);
    code.push(0x24); // global.set __heap_ptr
    leb_u32_padded(layout.heap_ptr_global, &mut code);
    code.extend_from_slice(&[0x3f, 0x00, 0x41, 0x10, 0x74]); // memory.size; i32.const 16; i32.shl
    code.push(0x24); // global.set __heap_end
    leb_u32_padded(layout.heap_end_global, &mut code);
    Prologue {
        code,
        globals: vec![(7, layout.heap_ptr_global), (18, layout.heap_end_global)],
        statics: vec![StaticAddr { offset: 1, value: layout.heap_base, signed: true }],
    }
}

/// The static region as one data segment's bytes: zeros, with the data section's one segment (the text
/// pool) laid in at `static_base`; and, for each interned Text's pointer to its own bytes, its offset in
/// the region and the address it holds.
fn static_image(data: Option<&[u8]>, layout: &StaticLayout) -> R<(Vec<u8>, Vec<(u32, u32)>)> {
    let mut image = vec![0u8; layout.heap_base as usize];
    let mut pointers = Vec::new();
    let Some(body) = data else {
        return Ok((image, pointers));
    };
    let refuse = || WasmLowerError::Unsupported("a data section other than one text pool at the static base");
    let mut i = 0;
    if read_uleb(body, &mut i)? != 1 || read_uleb(body, &mut i)? != 0 || body.get(i) != Some(&0x41) {
        return Err(refuse());
    }
    i += 1;
    let (at, _) = read_sleb(body, &mut i)?;
    if body.get(i) != Some(&0x0b) || at != i64::from(layout.static_base) {
        return Err(refuse());
    }
    i += 1;
    let len = read_uleb(body, &mut i)? as usize;
    let pool = body.get(i..i + len).ok_or_else(refuse)?;
    let base = layout.static_base as usize;
    if base + len > image.len() {
        return Err(refuse());
    }
    image[base..base + len].copy_from_slice(pool);
    let word = |o: usize| u32::from_le_bytes([pool[o], pool[o + 1], pool[o + 2], pool[o + 3]]);
    let mut o = 0;
    while o < len {
        if o + 16 > len {
            return Err(refuse());
        }
        let bytes = word(o) as usize;
        let addr = (base + o) as u32;
        if word(o + 8) != addr + 16 {
            return Err(WasmLowerError::Unsupported("an interned Text that does not point at its own bytes"));
        }
        pointers.push((addr + 8, addr + 16));
        o += 16 + ((bytes + 7) & !7);
    }
    if o != len {
        return Err(refuse());
    }
    Ok((image, pointers))
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
/// the module exports them by, or `fnN`), then one symbol per global, then, for a module that defines
/// its memory, the static region's data symbol and the undefined `__heap_base`. A call's relocation
/// references symbol index = function index. Refuses (Err) any module whose code uses an opcode the
/// rewriter doesn't understand, and any module this pass cannot relocate (see the module doc).
pub(crate) fn module_to_relocatable(module: &[u8]) -> R<Vec<u8>> {
    let secs = sections(module)?;
    let sec = |id: u8| {
        secs.iter()
            .find(|s| s.id == id)
            .map(|s| &module[s.start..s.end])
    };
    if secs.iter().any(|s| s.id == 4 || s.id == 9) {
        return Err(WasmLowerError::Unsupported(
            "relocatable transform: a table (closures) needs table and type relocations (not yet)",
        ));
    }
    let layout = static_layout(module, &secs)?;
    if layout.is_none() && secs.iter().any(|s| s.id == 5 || s.id == 11) {
        return Err(WasmLowerError::Unsupported(
            "relocatable transform: a memory or data with no logos.static layout",
        ));
    }

    let imports = sec(2).map(parse_imports).transpose()?.unwrap_or_default();
    let k = imports.len() as u32;
    let exported = sec(7).map(parse_exported_functions).transpose()?.unwrap_or_default();

    let func_body = sec(3).ok_or(WasmLowerError::Unsupported("no function section"))?;
    let mut fi = 0;
    let num_defined = read_uleb(func_body, &mut fi)? as u32; // main + user functions

    // Globals are all DEFINED (our modules import only host functions and, linked, a memory),
    // indexed 0..num_globals.
    let num_globals = match sec(6) {
        Some(g) => {
            let mut gi = 0;
            read_uleb(g, &mut gi)? as u32
        }
        None => 0,
    };
    let num_func_symbols = k + num_defined;
    // The data symbols follow the global symbols: the static region, then `__heap_base`.
    let static_symbol = num_func_symbols + num_globals;
    let heap_base_symbol = static_symbol + 1;

    const R_WASM_FUNCTION_INDEX_LEB: u8 = 0x00;
    const R_WASM_MEMORY_ADDR_LEB: u8 = 0x03;
    const R_WASM_MEMORY_ADDR_SLEB: u8 = 0x04;
    const R_WASM_MEMORY_ADDR_I32: u8 = 0x05;
    const R_WASM_GLOBAL_INDEX_LEB: u8 = 0x07;
    let has_addend = |ty: u8| matches!(ty, R_WASM_MEMORY_ADDR_LEB | R_WASM_MEMORY_ADDR_SLEB | R_WASM_MEMORY_ADDR_I32);

    // Rewrite the code section: each entry is `bodysize, body`. Produce a new code section and collect
    // relocations as (offset within the code section content, type, symbol index, addend). Function
    // calls reference symbol index = function index; global.get/set reference num_func_symbols + global;
    // a static address the static region's symbol (or `__heap_base`, for the heap's base itself).
    let code_body = sec(10).ok_or(WasmLowerError::Unsupported("no code section"))?;
    let mut ci = 0;
    let n_bodies = read_uleb(code_body, &mut ci)?;
    if n_bodies != u64::from(num_defined) {
        return Err(WasmLowerError::Unsupported("code/function count mismatch"));
    }
    let prologue = layout.as_ref().map(heap_prologue).unwrap_or_default();
    let no_prologue = Prologue::default();
    let mut new_code = Vec::new();
    leb_u(n_bodies, &mut new_code);
    let mut code_relocs: Vec<(u32, u8, u32, i64)> = Vec::new();
    for d in 0..n_bodies {
        let sz = read_uleb(code_body, &mut ci)? as usize;
        let body = &code_body[ci..ci + sz];
        ci += sz;
        let rewritten = rewrite_body(body, if d == 0 { &prologue } else { &no_prologue })?;
        leb_u(rewritten.body.len() as u64, &mut new_code);
        let body_start = new_code.len() as u32;
        new_code.extend_from_slice(&rewritten.body);
        for (off, func) in rewritten.calls {
            code_relocs.push((body_start + off, R_WASM_FUNCTION_INDEX_LEB, func, 0));
        }
        for (off, g) in rewritten.globals {
            code_relocs.push((body_start + off, R_WASM_GLOBAL_INDEX_LEB, num_func_symbols + g, 0));
        }
        if let Some(layout) = &layout {
            for a in rewritten.statics {
                let ty = if a.signed { R_WASM_MEMORY_ADDR_SLEB } else { R_WASM_MEMORY_ADDR_LEB };
                let (symbol, addend) = if a.value == layout.heap_base {
                    (heap_base_symbol, 0)
                } else if a.value < layout.heap_base {
                    (static_symbol, i64::from(a.value))
                } else {
                    return Err(WasmLowerError::Unsupported("a five-byte address past the static region"));
                };
                code_relocs.push((body_start + a.offset, ty, symbol, addend));
            }
        }
    }

    // The static region, its data section and the relocations of the Texts' pointers within it.
    let mut data_relocs: Vec<(u32, u8, u32, i64)> = Vec::new();
    let new_data = match &layout {
        None => None,
        Some(layout) => {
            let (image, pointers) = static_image(sec(11), layout)?;
            let mut data = Vec::new();
            leb_u(1, &mut data); // one segment
            leb_u(0, &mut data); // active, memory 0
            data.extend_from_slice(&[0x41, 0x00, 0x0b]); // at the segment's own start
            leb_u(image.len() as u64, &mut data);
            let image_start = data.len() as u32;
            data.extend_from_slice(&image);
            for (at, address) in pointers {
                data_relocs.push((image_start + at, R_WASM_MEMORY_ADDR_I32, static_symbol, i64::from(address)));
            }
            Some(data)
        }
    };

    // The imports, with the linker's memory after the host functions when the module brought its own.
    let new_imports = match (&layout, sec(2)) {
        (None, _) => None,
        (Some(_), None) => {
            return Err(WasmLowerError::Unsupported("a module that brings its memory imports nothing"));
        }
        (Some(_), Some(existing)) => {
            let mut body = Vec::new();
            let mut i = 0;
            let count = read_uleb(existing, &mut i)?;
            let entries = &existing[i..];
            leb_u(count + 1, &mut body);
            body.extend_from_slice(entries);
            for name in ["env", "__linear_memory"] {
                leb_u(name.len() as u64, &mut body);
                body.extend_from_slice(name.as_bytes());
            }
            body.extend_from_slice(&[0x02, 0x00, 0x00]); // memory, min 0
            Some(body)
        }
    };

    // Symbol table: one FUNCTION symbol per function index (in order), one GLOBAL symbol per defined
    // global, then the data symbols of a module that brought its memory.
    const WASM_SYM_UNDEFINED: u32 = 0x10;
    const WASM_SYM_EXPORTED: u32 = 0x20;
    const WASM_SYM_BINDING_LOCAL: u32 = 0x02;
    let num_symbols = num_func_symbols + num_globals + if layout.is_some() { 2 } else { 0 };
    let mut symtab = Vec::new();
    leb_u(u64::from(num_symbols), &mut symtab);
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
    if let Some(layout) = &layout {
        symtab.push(0x01); // SYMTAB_DATA: the static region, private to this program
        leb_u(u64::from(WASM_SYM_BINDING_LOCAL), &mut symtab);
        let name = "logos_static";
        leb_u(name.len() as u64, &mut symtab);
        symtab.extend_from_slice(name.as_bytes());
        leb_u(0, &mut symtab); // segment
        leb_u(0, &mut symtab); // offset
        leb_u(u64::from(layout.heap_base), &mut symtab); // size
        symtab.push(0x01); // SYMTAB_DATA: the linker's heap base
        leb_u(u64::from(WASM_SYM_UNDEFINED), &mut symtab);
        let name = "__heap_base";
        leb_u(name.len() as u64, &mut symtab);
        symtab.extend_from_slice(name.as_bytes());
    }
    let mut linking = Vec::new();
    leb_u(2, &mut linking); // metadata version
    linking.push(0x08); // WASM_SYMBOL_TABLE
    leb_u(symtab.len() as u64, &mut linking);
    linking.extend_from_slice(&symtab);
    if layout.is_some() {
        let mut info = Vec::new();
        leb_u(1, &mut info); // one segment
        let name = ".data.logos_static";
        leb_u(name.len() as u64, &mut info);
        info.extend_from_slice(name.as_bytes());
        leb_u(3, &mut info); // aligned to 8, as every heap block is
        leb_u(0, &mut info); // flags
        linking.push(0x05); // WASM_SEGMENT_INFO
        leb_u(info.len() as u64, &mut linking);
        linking.extend_from_slice(&info);
    }

    // We re-emit every original non-custom section verbatim except CODE (rewritten), EXPORT (id 7,
    // dropped: its function index is the pre-link value and would export the wrong function after the
    // shift — `main` is exported instead via its symbol's EXPORTED flag + lld's `--export=main`, and the
    // memory via lld's default) and, for a module that brought its memory, MEMORY (dropped: the object
    // imports the linker's), IMPORT (the memory added) and DATA (the whole static region, emitted after
    // CODE even when the module had none). A relocation section's target is its section's 0-based
    // position in that emitted order.
    let mut emitted: Vec<(u8, Vec<u8>)> = Vec::new();
    for s in &secs {
        let body = &module[s.start..s.end];
        match s.id {
            0 | 7 => {}
            5 if layout.is_some() => {}
            11 if layout.is_some() => {}
            2 if new_imports.is_some() => emitted.push((2, new_imports.clone().expect("checked"))),
            10 => {
                emitted.push((10, new_code.clone()));
                if let Some(data) = &new_data {
                    emitted.push((11, data.clone()));
                }
            }
            id => emitted.push((id, body.to_vec())),
        }
    }
    let position = |id: u8| -> R<u32> {
        emitted
            .iter()
            .position(|(i, _)| *i == id)
            .map(|p| p as u32)
            .ok_or(WasmLowerError::Unsupported("a relocated section is missing"))
    };
    let reloc_section = |target: u32, mut relocs: Vec<(u32, u8, u32, i64)>| {
        relocs.sort_by_key(|&(o, _, _, _)| o);
        let mut out = Vec::new();
        leb_u(u64::from(target), &mut out);
        leb_u(relocs.len() as u64, &mut out);
        for (off, ty, sym, addend) in relocs {
            out.push(ty);
            leb_u(u64::from(off), &mut out);
            leb_u(u64::from(sym), &mut out);
            if has_addend(ty) {
                leb_s(addend, &mut out);
            }
        }
        out
    };

    // Reassemble: header, the emitted sections, then linking, reloc.CODE and (with a static region) reloc.DATA.
    let mut out = Vec::new();
    out.extend_from_slice(b"\0asm");
    out.extend_from_slice(&[0x01, 0x00, 0x00, 0x00]);
    for (id, body) in &emitted {
        put_section(&mut out, *id, body);
    }
    put_custom(&mut out, "linking", &linking);
    put_custom(&mut out, "reloc.CODE", &reloc_section(position(10)?, code_relocs));
    if new_data.is_some() {
        put_custom(&mut out, "reloc.DATA", &reloc_section(position(11)?, data_relocs));
    }
    Ok(out)
}

#[cfg(test)]
#[path = "../../../../logicaffeine_tests/unit/logicaffeine_compile/vm/wasm/reloc/tests.rs"]
mod tests;
