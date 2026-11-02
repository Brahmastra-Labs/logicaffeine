//! The **binary Silicon ISA** — the register bytecode as a stable, self-contained,
//! chip-decodable byte stream. The text sibling is [`crate::vm::isa`] (`emit_isa`, the
//! human-readable assembly listing); this is the machine form of the SAME opcode set.
//!
//! Two roles, one artifact:
//!
//! 1. **Wire transport.** [`encode_program`] / [`decode_program`] turn a whole
//!    [`CompiledProgram`] into a versioned, checksummed, SELF-CONTAINED frame. The frame
//!    carries its own symbol string-table, so a receiver with a *fresh* [`Interner`]
//!    re-interns and runs it byte-identically. This is what makes "send the language
//!    bytecode over the wire and execute it" real: the raw bytecode's `Symbol`s are
//!    interner-relative (see [`crate::vm::tier_cache`]) and thus NOT portable across
//!    machines; the frame lowers every symbol to a wire-local id into the string table
//!    and the decoder re-interns them on the far side.
//!
//! 2. **Chip ISA.** [`opcode`] is the STABLE opcode number a hardware decoder shares.
//!    It is keyed on [`Op::name`] through an APPEND-ONLY table ([`ISA_OPCODES`]), so the
//!    number for a given op can never drift when the `Op` enum is reordered — the exact
//!    property a fabricated decoder needs (its wired constants outlive the source order).
//!    The reversible-core opcodes ([`RevAdd`](Op::RevAdd) … [`RevSwap`](Op::RevSwap)) sit
//!    at pinned low numbers so the adiabatic datapath's control decode is fixed.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::intern::{Interner, Symbol};
use crate::vm::instruction::{
    BoundaryType, CompiledFunction, CompiledProgram, Constant, EnumTypeDef, FuncIdx, Op,
    StructTypeDef,
};
use crate::vm::native_tier::{ParamKind, SlotKind};

/// The frame magic — `LOGOS Bytecode, format 1`. A receiver rejects any buffer that does
/// not open with these four bytes (a foreign or truncated frame is not a program).
pub const ISA_MAGIC: &[u8; 4] = b"LGB1";

/// The wire-format version. Bumped when the frame layout changes incompatibly; a decoder
/// rejects a version it does not understand rather than mis-parsing it.
pub const ISA_VERSION: u32 = 3;

/// The APPEND-ONLY opcode table: `opcode(op) == position of op.name() in this slice`.
/// Order IS the ABI — a new op appends at the end and keeps every prior number fixed; the
/// totality lock (this module's unit test `opcode_map_is_total_over_the_isa`) forces a newly-added `Op` to
/// be registered here, and the append-only lock pins the reversible-core numbers so a chip
/// decoder built against them can never silently mismatch. Never reorder or delete an entry.
pub const ISA_OPCODES: &[&str] = &[
    // --- reversible / adiabatic core (pinned low, the charge-recovery datapath) ---
    "RevAdd",  // 0
    "RevSub",  // 1
    "RevXor",  // 2
    "RevNeg",  // 3
    "RevSwap", // 4
    // --- data movement ---
    "LoadConst",
    "Move",
    "EnsureOwned",
    // --- integer / arithmetic ALU ---
    "Add",
    "AddAssign",
    "Sub",
    "Mul",
    "Div",
    "ExactDiv",
    "FloorDiv",
    "Mod",
    "DivPow2",
    "MagicDivU",
    "Pow",
    // --- bitwise ---
    "BitXor",
    "BitAnd",
    "BitOr",
    "Shl",
    "Shr",
    "Not",
    // --- comparison ---
    "Lt",
    "Gt",
    "LtEq",
    "GtEq",
    "Eq",
    "ApproxEq",
    "NotEq",
    // --- text / sequence value ops ---
    "Concat",
    "SeqConcat",
    // --- control flow ---
    "Jump",
    "JumpIfFalse",
    "JumpIfTrue",
    "Call",
    "CallBuiltin",
    "CallValue",
    "MakeClosure",
    "Return",
    "ReturnNothing",
    "Halt",
    // --- policy / capability ---
    "CheckPolicy",
    // --- globals ---
    "GlobalGet",
    "GlobalSet",
    // --- collections ---
    "ListPushField",
    "NewList",
    "NewEmptyList",
    "NewEmptyListI32",
    "NewEmptySet",
    "NewEmptyMap",
    "NewRange",
    "ListPush",
    "SetAdd",
    "RemoveFrom",
    "SetIndex",
    "SetIndexUnchecked",
    "Index",
    "IndexUnchecked",
    "RegionBoundsGuard",
    "Length",
    "Contains",
    "FormatValue",
    "SliceOp",
    "DeepClone",
    "NewTuple",
    "UnionOp",
    "IntersectOp",
    "ListPop",
    "DestructureTuple",
    // --- temporal ---
    "LoadToday",
    "LoadNow",
    // --- structs / enums ---
    "NewStruct",
    "StructInsert",
    "GetField",
    "NewInductive",
    "TestArm",
    "BindArm",
    // --- CRDTs ---
    "CrdtBump",
    "CrdtMerge",
    "NewCrdt",
    "CrdtAppend",
    "CrdtResolve",
    // --- iteration ---
    "IterPrepare",
    "IterNext",
    "IterPop",
    // --- misc runtime ---
    "Sleep",
    "Show",
    "Args",
    "FailWith",
    // --- concurrency ---
    "ChanNew",
    "ChanSend",
    "ChanRecv",
    "ChanTrySend",
    "ChanTryRecv",
    "ChanClose",
    "Spawn",
    "SpawnHandle",
    "TaskAwait",
    "TaskAbort",
    "SelectArmRecv",
    "SelectArmTimeout",
    "SelectWait",
    // --- networking ---
    "NetConnect",
    "NetListen",
    "NetSend",
    "NetStream",
    "NetAwait",
    "NetMakePeer",
    "NetSync",
    "Obs",
    // --- CRDT lattice join (appended: the wire map is append-only, so existing opcodes never shift) ---
    "CrdtJoin",
    // --- 256-bit BigInt wide lane (one opcode; the operation rides the chip word's dst high nibble) ---
    "BigIntWide",
    // --- tcp.lg client sockets (one opcode; the operation rides the `kind` field) — an off-chip
    //     coprocessor service like the Net ops ---
    "Tcp",
    // --- the `Serve` HTTP keystone (async-tier): the accept loop lives in the async io driver, which
    //     re-enters the handler function as bytecode; an off-host service like the Net/Tcp ops ---
    "Serve",
    // --- fixed-capacity list allocation (append-only; carries the shared sequence proof) ---
    "NewEmptyListFixed",
    // --- runtime capacity hints (append-only; the hint remains visible to every backend) ---
    "NewEmptyListWithCapacity",
    "NewEmptyMapWithCapacity",
];

/// The stable opcode number for an op — the value a hardware decoder keys on. Total over the
/// ISA (a new op that forgets to register in [`ISA_OPCODES`] panics here, and the totality
/// lock catches it in CI first).
pub fn opcode(op: &Op) -> u16 {
    opcode_of_name(op.name())
        .unwrap_or_else(|| panic!("op {:?} is not registered in ISA_OPCODES", op.name()))
}

/// The stable opcode number for an op *name*, or `None` if the name is not in the ISA table.
pub fn opcode_of_name(name: &str) -> Option<u16> {
    ISA_OPCODES
        .iter()
        .position(|&n| n == name)
        .map(|i| i as u16)
}

/// One compiled function, with every interner-relative [`Symbol`] lowered to a wire-local id
/// into [`WireProgram::symbols`]. The mirror of [`CompiledFunction`] that is serde-clean and
/// portable (the real struct's `name`/`captures` symbols cannot be moved between interners).
#[derive(Serialize, Deserialize)]
struct WireFunction {
    name: u32,
    entry_pc: u64,
    param_count: u16,
    register_count: u64,
    captures: Vec<(u32, Option<u16>)>,
    named_regs: Vec<bool>,
    param_kinds: Vec<Option<ParamKind>>,
    ret_kind: Option<SlotKind>,
    param_types: Vec<Option<BoundaryType>>,
    return_type: Option<BoundaryType>,
    mutable_param_regs: Vec<u16>,
    cow_exempt_regs: Vec<u16>,
}

/// The serde-serialized body of an ISA frame: a whole [`CompiledProgram`] with a self-contained
/// symbol string-table. Every `Symbol` anywhere in the program is replaced by an index into
/// `symbols`; the decoder re-interns those strings against the target interner and rewrites the
/// indices back. Debug-only fields (`reg_names`) are dropped — they never affect execution.
#[derive(Serialize, Deserialize)]
struct WireProgram {
    symbols: Vec<String>,
    constants: Vec<Constant>,
    code: Vec<Op>,
    register_count: u64,
    functions: Vec<WireFunction>,
    fn_index: Vec<(u32, u16)>,
    globals: Vec<String>,
    named_regs: Vec<bool>,
    loop_locals: Vec<(u64, Vec<bool>)>,
    struct_types: Vec<StructTypeDef>,
    enum_types: Vec<EnumTypeDef>,
    global_types: Vec<Option<BoundaryType>>,
}

/// Accumulates the wire symbol table during encoding: each distinct interner symbol gets one
/// dense wire-local id, its string captured once from the source interner.
struct SymTab<'i> {
    interner: &'i Interner,
    strings: Vec<String>,
    map: HashMap<Symbol, u32>,
}

impl<'i> SymTab<'i> {
    fn new(interner: &'i Interner) -> Self {
        SymTab {
            interner,
            strings: Vec::new(),
            map: HashMap::new(),
        }
    }

    fn lower(&mut self, s: Symbol) -> u32 {
        if let Some(&id) = self.map.get(&s) {
            return id;
        }
        let id = self.strings.len() as u32;
        self.strings.push(self.interner.resolve(s).to_string());
        self.map.insert(s, id);
        id
    }
}

/// FNV-1a 64 over the payload — a corruption tripwire, not a cryptographic seal. A decoder
/// rejects a frame whose stored digest does not match its recomputed one.
fn checksum(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Serialize a whole compiled program into a self-contained, portable ISA frame:
/// `MAGIC ‖ version ‖ checksum ‖ bincode(WireProgram)`. The `interner` is the one the program
/// was compiled against — every symbol string is captured from it so the frame stands alone.
pub fn encode_program(program: &CompiledProgram, interner: &Interner) -> Vec<u8> {
    let mut tab = SymTab::new(interner);

    // Code: the ONLY symbol-carrying op is `CheckPolicy` (its policy predicate). Lower it to a
    // wire id; every other op references names through constants / function indices already.
    let mut code = program.code.clone();
    for op in &mut code {
        if let Op::CheckPolicy { predicate, .. } = op {
            *predicate = Symbol::from_index(tab.lower(*predicate) as usize);
        }
    }

    let functions: Vec<WireFunction> = program
        .functions
        .iter()
        .map(|f| WireFunction {
            name: tab.lower(f.name),
            entry_pc: f.entry_pc as u64,
            param_count: f.param_count,
            register_count: f.register_count as u64,
            captures: f
                .captures
                .iter()
                .map(|(s, g)| (tab.lower(*s), *g))
                .collect(),
            named_regs: f.named_regs.clone(),
            param_kinds: f.param_kinds.clone(),
            ret_kind: f.ret_kind,
            param_types: f.param_types.clone(),
            return_type: f.return_type.clone(),
            mutable_param_regs: f.mutable_param_regs.clone(),
            cow_exempt_regs: f.cow_exempt_regs.clone(),
        })
        .collect();

    // `fn_index` is a HashMap — sort by function index so the emitted frame is byte-deterministic
    // regardless of hash iteration order (a chip / cache wants a reproducible artifact).
    let mut fn_index: Vec<(u32, u16)> = program
        .fn_index
        .iter()
        .map(|(s, fi)| (tab.lower(*s), *fi))
        .collect();
    fn_index.sort_unstable_by_key(|&(_, fi)| fi);

    // `loop_locals` is ALSO a HashMap (keyed by absolute pc) — same treatment, sorted by that key.
    let mut loop_locals: Vec<(u64, Vec<bool>)> = program
        .loop_locals
        .iter()
        .map(|(k, v)| (*k as u64, v.clone()))
        .collect();
    loop_locals.sort_unstable_by_key(|&(pc, _)| pc);

    // `struct_types`/`enum_types` are `Vec`s on `CompiledProgram`, but the discovery-pass registry
    // they are built FROM (`TypeRegistry::iter_types`) is itself a `HashMap<Symbol, TypeDef>` whose
    // iteration order is reseeded (and so reordered) every process run — the exact hazard
    // `TypeRegistry`'s own variant-lookup helper already documents and sorts around. Two encodes of
    // the identical source therefore end up with two DIFFERENT type-definition orders unless this
    // boundary re-canonicalizes them: sort by name, since every consumer (the AOT backend, `vm/wasm/
    // kind.rs`'s named-layout resolution) addresses a struct/enum by NAME, never by its position in
    // this list — reordering the list changes nothing else.
    let mut struct_types = program.struct_types.clone();
    struct_types.sort_by(|a, b| a.name.cmp(&b.name));
    let mut enum_types = program.enum_types.clone();
    enum_types.sort_by(|a, b| a.name.cmp(&b.name));

    let wire = WireProgram {
        symbols: tab.strings,
        constants: program.constants.clone(),
        code,
        register_count: program.register_count as u64,
        functions,
        fn_index,
        globals: program.globals.clone(),
        named_regs: program.named_regs.clone(),
        loop_locals,
        struct_types,
        enum_types,
        global_types: program.global_types.clone(),
    };

    let payload = bincode::serialize(&wire).expect("WireProgram serializes");
    let mut out = Vec::with_capacity(payload.len() + 16);
    out.extend_from_slice(ISA_MAGIC);
    out.extend_from_slice(&ISA_VERSION.to_le_bytes());
    out.extend_from_slice(&checksum(&payload).to_le_bytes());
    out.extend_from_slice(&payload);
    out
}

/// Decode an ISA frame back into an executable [`CompiledProgram`], re-interning its symbol
/// table against `interner` (which may be BRAND NEW — the frame is self-contained). Returns
/// `None` on any corruption: bad magic, wrong version, checksum mismatch, or malformed payload
/// — a corrupt frame is not a program and is rejected, never half-executed.
pub fn decode_program(bytes: &[u8], interner: &mut Interner) -> Option<CompiledProgram> {
    if bytes.len() < 16 || &bytes[0..4] != ISA_MAGIC {
        return None;
    }
    let version = u32::from_le_bytes(bytes[4..8].try_into().ok()?);
    if version != ISA_VERSION {
        return None;
    }
    let stored = u64::from_le_bytes(bytes[8..16].try_into().ok()?);
    let payload = &bytes[16..];
    if checksum(payload) != stored {
        return None;
    }
    let wire: WireProgram = bincode::deserialize(payload).ok()?;

    // Re-intern the wire string table → real symbols in the target interner.
    let real: Vec<Symbol> = wire.symbols.iter().map(|s| interner.intern(s)).collect();
    let remap = |wid: usize| -> Symbol { real.get(wid).copied().unwrap_or(Symbol::EMPTY) };

    let mut code = wire.code;
    for op in &mut code {
        if let Op::CheckPolicy { predicate, .. } = op {
            *predicate = remap(predicate.index());
        }
    }

    let functions: Vec<CompiledFunction> = wire
        .functions
        .into_iter()
        .map(|wf| CompiledFunction {
            name: remap(wf.name as usize),
            entry_pc: wf.entry_pc as usize,
            param_count: wf.param_count,
            register_count: wf.register_count as usize,
            captures: wf
                .captures
                .into_iter()
                .map(|(w, g)| (remap(w as usize), g))
                .collect(),
            named_regs: wf.named_regs,
            param_kinds: wf.param_kinds,
            ret_kind: wf.ret_kind,
            param_types: wf.param_types,
            return_type: wf.return_type,
            mutable_param_regs: wf.mutable_param_regs,
            cow_exempt_regs: wf.cow_exempt_regs,
        })
        .collect();

    let fn_index: HashMap<Symbol, FuncIdx> = wire
        .fn_index
        .into_iter()
        .map(|(w, fi)| (remap(w as usize), fi))
        .collect();

    Some(CompiledProgram {
        // Not serialized: a wire form that predates the table has none, and a backend that finds
        // none falls back to tracing a construction exactly as it always did.
        declared_types: Vec::new(),
        constants: wire.constants,
        code,
        register_count: wire.register_count as usize,
        functions,
        fn_index,
        globals: wire.globals,
        named_regs: wire.named_regs,
        loop_locals: wire
            .loop_locals
            .into_iter()
            .map(|(k, v)| (k as usize, v))
            .collect(),
        reg_names: Vec::new(),
        fn_debug: Vec::new(),
        struct_types: wire.struct_types,
        enum_types: wire.enum_types,
        global_types: wire.global_types,
        // Observation is elided before wire serialization (the wire frame carries
        // only the compiled/release program), so a decoded program observes nothing.
        observation_sites: Vec::new(),
        // Not serialized: a wire form that predates the table simply has none.
        site_at: Vec::new(),
    })
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/vm/binisa/tests.rs"]
mod tests;
