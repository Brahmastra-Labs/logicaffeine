//! Tree-walking interpreter for LOGOS imperative code.
//!
//! This module provides runtime execution of parsed LOGOS programs by
//! walking the AST and executing statements/expressions directly. The
//! interpreter is async-capable to support VFS operations.
//!
//! # Architecture
//!
//! ```text
//! LOGOS AST
//!     │
//!     ▼
//! ┌────────────┐
//! │ Interpreter│ ──▶ Evaluate expressions
//! │            │ ──▶ Execute statements
//! │            │ ──▶ Manage scopes
//! └────────────┘
//!     │
//!     ▼
//! RuntimeValue results
//! ```
//!
//! # Runtime Values
//!
//! The interpreter uses [`RuntimeValue`] to represent all values at runtime:
//! - Primitives: `Int`, `Float`, `Bool`, `Text`, `Char`
//! - Collections: `List`, `Tuple`, `Set`, `Map`
//! - User types: `Struct`, `Inductive` (kernel-defined types)
//!
//! # Async Support
//!
//! The interpreter is async to support VFS file operations (OPFS on WASM,
//! `tokio::fs` on native). All statement execution is `async fn`.

use std::collections::HashMap;
use std::sync::Arc;
use std::rc::Rc;
use std::cell::RefCell;

use async_recursion::async_recursion;

use crate::ast::stmt::{BinaryOpKind, Block, ClosureBody, CompressionCodec, Expr, Literal, MatchArm, ReadSource, Stmt, TypeExpr};
use crate::intern::{Interner, Symbol};

/// Map a surface `Send compressed with <codec>` choice to the wire codec.
fn wire_compression_of(codec: CompressionCodec) -> crate::concurrency::marshal::WireCompression {
    use crate::concurrency::marshal::WireCompression;
    match codec {
        CompressionCodec::Deflate => WireCompression::Deflate,
        CompressionCodec::Lz4 => WireCompression::Lz4,
        CompressionCodec::Zstd => WireCompression::Zstd,
    }
}
use crate::analysis::{PolicyRegistry, PolicyCondition};

// VFS imports for async file operations
use logicaffeine_system::fs::Vfs;
use logicaffeine_runtime::{ChanId, RtPayload, SelectArm, TaskId};
use crate::concurrency::bridge::{BlockingRequest, Yield, YieldFuture, YieldState};
use crate::concurrency::driver::{ErrSink, InterpreterTask};
use crate::concurrency::marshal;

/// Callback type for streaming output from the interpreter.
/// Called each time `Show` executes with the output line.
pub type OutputCallback = Rc<RefCell<dyn FnMut(String)>>;

/// Runtime values during LOGOS interpretation.
///
/// Represents all possible values that can exist at runtime when executing
/// a LOGOS program. Includes primitives, collections, user-defined structs,
/// and kernel inductive types.
/// User-defined struct with named fields (boxed to reduce enum size).
#[derive(Debug, Clone)]
pub struct StructValue {
    pub type_name: String,
    pub fields: HashMap<String, RuntimeValue>,
}

/// Kernel inductive value (boxed to reduce enum size).
#[derive(Debug, Clone)]
pub struct InductiveValue {
    pub inductive_type: String,
    pub constructor: String,
    pub args: Vec<RuntimeValue>,
}

/// First-class closure value (boxed to reduce enum size).
#[derive(Debug, Clone)]
pub struct ClosureValue {
    pub body_index: usize,
    pub captured_env: HashMap<Symbol, RuntimeValue>,
    pub param_names: Vec<Symbol>,
    /// A SHIPPED pure function carries its sandboxed generator body directly (self-
    /// contained — no `body_index` into any arena), so it can cross the wire and be
    /// invoked on a receiver that never compiled it. `None` for ordinary closures, whose
    /// body lives in `closure_bodies[body_index]`. Set only by `T_FUNC` decode (and the
    /// gated `Send computed` lowering); the call path evaluates it via the sandbox.
    pub generated: Option<std::rc::Rc<crate::concurrency::marshal::GenExpr>>,
}

/// The Map payload behind `RuntimeValue::Map`. INSERTION-ORDERED (`IndexMap`):
/// iteration, display, and marshaling follow the order keys were first
/// inserted — the LOGOS `Map` contract, identical across the tree-walker, the
/// VM, the AOT `LogosMap`, and the direct-WASM linear map. FxHash instead of
/// the standard library's SipHash: map-heavy programs hash on every
/// get/insert, and the keys here are small values (ints, short texts) where
/// Fx is several times faster — with no DoS-resistance requirement (a
/// single-program interpreter hashing its own program's keys). NOTE: removal
/// must go through `shift_remove` (order-preserving), never `swap_remove`.
pub type MapStorage =
    indexmap::IndexMap<RuntimeValue, RuntimeValue, std::hash::BuildHasherDefault<rustc_hash::FxHasher>>;

/// The List payload behind `RuntimeValue::List`: homogeneous all-Int and
/// all-Float lists store UNBOXED vectors (cache-dense, and the JIT can pin a
/// raw pointer to them); anything else boxes. The repr lives INSIDE the
/// `Rc<RefCell<…>>`, so promotion re-tags the payload in place and every
/// alias observes it — reference semantics and Rc identity are untouched.
/// An EMPTY list is vacuously `Ints` and re-tags freely on its first push.
///
/// Hot paths take `&self`/`&mut self` borrows only — no Rc refcount traffic.
/// `Clone` snapshots a buffer's contents for the region deopt-rollback of an
/// in-place-mutated array (see `crate::vm::native_tier::ArrayPin::mutated`).
#[derive(Debug, Clone)]
pub enum ListRepr {
    Boxed(Vec<RuntimeValue>),
    Ints(Vec<i64>),
    /// A proven-narrowable Int buffer stored half-width: every element fits
    /// `i32` (the narrowing proof in `codegen::narrow`), so the buffer is
    /// `Vec<i32>` — half the footprint and cache pressure. Reads SIGN-EXTEND
    /// (`x as i64`, lossless); writes TRUNCATE (`x as i32`, lossless *because*
    /// of the proof, debug-asserted in range). A write outside `i32` range
    /// PROMOTES the buffer back to a full-width `Ints` (the proof was wrong —
    /// soundness over speed), so observable values never differ from `Ints`.
    /// Created only behind `LOGOS_NARROW_VM` for narrowable declarations.
    IntsI32(Vec<i32>),
    Floats(Vec<f64>),
    Bools(Vec<bool>),
    /// A flat string array (Arrow-style): all element bytes concatenated in one
    /// `data` buffer, with `ends[i]` the exclusive end offset of element `i` (so
    /// element `i` is `data[ends[i-1]..ends[i]]`, `ends.len()` == the count). This
    /// is the columnar layout that lets a string list pack and load as two bulk
    /// copies instead of one heap allocation per string — the same treatment the
    /// numeric variants already get. It is read-optimized: an element materializes
    /// to `Text` only when accessed, and any mutation promotes the buffer to
    /// `Boxed`. The wire decoder builds it directly; normal code paths are
    /// unaffected (a string *literal* stays `Boxed`).
    ///
    /// `cache` is a LAZY memo: empty until the first `get`, so ship/load/iterate
    /// pay nothing, and repeated indexing of the same element returns a cheap `Rc`
    /// clone instead of re-materializing — best of both worlds (flat to move,
    /// boxed-cheap to re-read).
    Strings { data: Vec<u8>, ends: Vec<u32>, cache: RefCell<Vec<Option<Rc<String>>>> },
    /// A homogeneous struct list stored COLUMNAR (struct-of-arrays): instead of N
    /// boxed `StructValue`s (each a heap `Box` + a `HashMap`), the schema is held
    /// once (`type_name` + canonical sorted `field_names`) and each field becomes a
    /// packed column — itself a `ListRepr`, so an int field is a `Vec<i64>`, a bool
    /// field is bit-packed, a nested struct field is recursively columnar. Zero
    /// per-row `Box`/`HashMap`; field access is a column index; the wire encoder
    /// memcpy-streams the columns. Reads reconstruct a `StructValue` on demand
    /// (`get`); ANY mutation de-columnarizes to `Boxed` first (`make_boxed`), so
    /// reference semantics are exactly those of a boxed list. Built by `from_values`
    /// for genuinely-homogeneous struct lists (same type, same field set); ragged
    /// lists stay `Boxed`. `columns` are all the same length (the row count).
    Structs { type_name: String, field_names: Vec<String>, columns: Vec<ListRepr> },
    /// A homogeneous inductive (enum/ADT) list stored COLUMNAR as a tagged union:
    /// instead of N boxed `InductiveValue`s, the type name is held once, the distinct
    /// constructor names are dictionaried (`ctor_dict`), each row carries a small
    /// constructor index (`ctors`), and the constructor ARGUMENTS are packed DENSE
    /// per constructor — `arg_cols[c][j]` is the column of argument `j` across only
    /// the rows whose constructor is `c` (so `arg_cols[c].len()` is constructor `c`'s
    /// arity, and each inner column's length is the count of rows with constructor
    /// `c`). `ranks[i]` is row `i`'s rank within its constructor, so `get` is O(1).
    /// A nullary enum collapses to just the dictionary + index column (no arg cols).
    /// Built by `from_values` for same-type enum lists; ragged/mixed-type lists stay
    /// `Boxed`. Any mutation de-columnarizes via `make_boxed`.
    Inductives {
        inductive_type: String,
        ctor_dict: Vec<String>,
        ctors: Vec<u32>,
        ranks: Vec<u32>,
        arg_cols: Vec<Vec<ListRepr>>,
    },
    /// A received record-list held as RAW WIRE BYTES, decoded LAZILY — the production zero-copy
    /// receive (Cap'n Proto's "read in place, only what you touch"). `bytes` is the full received
    /// frame whose top-level value is a `T_STRUCTS_VIEW` record list; the schema (`type_name`,
    /// `field_names`, `len`) is read ONCE from the header, so `len` and shape are O(1) with ZERO
    /// rows decoded. Field access reads a single cell in place via `WireView::structs_row_field`
    /// (O(1), no allocation for the untouched rows/fields); `get(i)` reconstructs one `StructValue`
    /// on demand. ANY mutation de-lazies to `Boxed` first (`make_boxed`), so reference semantics are
    /// exactly those of a boxed list. Built only by the `view` receive path; normal code is
    /// unaffected.
    WireStructs {
        bytes: Rc<Vec<u8>>,
        type_name: String,
        field_names: Vec<String>,
        len: usize,
    },
    /// A received NUMERIC column (`T_INTS_ALIGNED`/`T_FLOATS_ALIGNED`) held as RAW WIRE BYTES and
    /// read ZERO-COPY: the 8-byte-aligned blob is the array. `len` is O(1) from the header (no
    /// decode); `get(i)` reads element `i` straight out of the borrowed `&[i64]`/`&[f64]` (capnp's
    /// `List<i64>` read-in-place); a partial read touches only what it reads. ANY mutation
    /// materializes to `Ints`/`Floats` first. Built only by the `view` receive path for an aligned
    /// column whose blob is 8-aligned in the received buffer (else the caller decodes eagerly).
    WireColumn {
        bytes: Rc<Vec<u8>>,
        len: usize,
        floats: bool,
    },
}

impl ListRepr {
    /// Wrap a received record-list frame (`T_STRUCTS_VIEW` top-level) as a lazy zero-copy backing.
    /// `None` if the bytes are not a self-describing record-list view (the caller decodes eagerly).
    pub fn from_record_list_view(bytes: Rc<Vec<u8>>) -> Option<ListRepr> {
        let (type_name, field_names, len) = {
            let view = crate::concurrency::marshal::view_message(&bytes)?;
            view.structs_schema()?
        };
        Some(ListRepr::WireStructs { bytes, type_name, field_names, len })
    }

    /// Wrap a received aligned numeric column (`T_INTS_ALIGNED`/`T_FLOATS_ALIGNED`) as a lazy
    /// zero-copy backing. `None` unless the blob reads as an 8-aligned `&[i64]`/`&[f64]` in this
    /// buffer (so every later read is a sound slice cast); the caller then decodes eagerly.
    pub fn from_aligned_column_view(bytes: Rc<Vec<u8>>) -> Option<ListRepr> {
        let view = crate::concurrency::marshal::view_message(&bytes)?;
        if let Some(s) = view.as_i64_slice() {
            return Some(ListRepr::WireColumn { len: s.len(), floats: false, bytes });
        }
        if let Some(s) = view.as_f64_slice() {
            return Some(ListRepr::WireColumn { len: s.len(), floats: true, bytes });
        }
        None
    }

    /// Wrap ANY received self-describing view (record list OR aligned numeric column) as a lazy
    /// zero-copy backing, or `None` for anything else (the caller decodes eagerly).
    pub fn from_received_view(bytes: Rc<Vec<u8>>) -> Option<ListRepr> {
        Self::from_record_list_view(bytes.clone()).or_else(|| Self::from_aligned_column_view(bytes))
    }

    fn wire_column_get(bytes: &[u8], floats: bool, i: usize) -> Option<RuntimeValue> {
        let view = crate::concurrency::marshal::view_message(bytes)?;
        if floats {
            view.as_f64_slice()?.get(i).map(|&f| RuntimeValue::Float(f))
        } else {
            view.as_i64_slice()?.get(i).map(|&n| RuntimeValue::Int(n))
        }
    }

    fn wire_column_to_values(bytes: &[u8], floats: bool) -> Vec<RuntimeValue> {
        let Some(view) = crate::concurrency::marshal::view_message(bytes) else { return Vec::new() };
        if floats {
            view.as_f64_slice().map(|s| s.iter().map(|&f| RuntimeValue::Float(f)).collect()).unwrap_or_default()
        } else {
            view.as_i64_slice().map(|s| s.iter().map(|&n| RuntimeValue::Int(n)).collect()).unwrap_or_default()
        }
    }

    /// Materialize a single row of a lazy `WireStructs` into an owned `StructValue` by reading each
    /// field cell in place. Shared by `get` and `make_boxed`.
    fn wire_struct_row(
        bytes: &[u8],
        type_name: &str,
        field_names: &[String],
        len: usize,
        i: usize,
    ) -> Option<RuntimeValue> {
        if i >= len {
            return None;
        }
        let view = crate::concurrency::marshal::view_message(bytes)?;
        let mut fields = HashMap::with_capacity(field_names.len());
        for name in field_names {
            let cell = view.structs_row_field_value(i, name)?;
            fields.insert(name.clone(), cell);
        }
        Some(RuntimeValue::Struct(Box::new(StructValue { type_name: type_name.to_string(), fields })))
    }
}

impl ListRepr {
    pub fn from_values(values: Vec<RuntimeValue>) -> ListRepr {
        if values.iter().all(|v| matches!(v, RuntimeValue::Int(_))) {
            ListRepr::Ints(
                values
                    .into_iter()
                    .map(|v| match v {
                        RuntimeValue::Int(n) => n,
                        _ => unreachable!(),
                    })
                    .collect(),
            )
        } else if values.iter().all(|v| matches!(v, RuntimeValue::Float(_))) {
            ListRepr::Floats(
                values
                    .into_iter()
                    .map(|v| match v {
                        RuntimeValue::Float(f) => f,
                        _ => unreachable!(),
                    })
                    .collect(),
            )
        } else if values.iter().all(|v| matches!(v, RuntimeValue::Bool(_))) {
            ListRepr::Bools(
                values
                    .into_iter()
                    .map(|v| match v {
                        RuntimeValue::Bool(b) => b,
                        _ => unreachable!(),
                    })
                    .collect(),
            )
        } else if !values.is_empty() && values.iter().all(|v| matches!(v, RuntimeValue::Text(_))) {
            // A homogeneous string list de-boxes to one flat contiguous buffer (bytes + end
            // offsets). It encodes as a single memcpy of the bytes — and the wire form is
            // byte-identical to the boxed path — instead of one scattered copy per string.
            let mut data = Vec::new();
            let mut ends = Vec::with_capacity(values.len());
            for v in &values {
                if let RuntimeValue::Text(s) = v {
                    data.extend_from_slice(s.as_bytes());
                    ends.push(data.len() as u32);
                }
            }
            ListRepr::strings(data, ends)
        } else if let Some((type_name, field_names)) = Self::struct_schema(&values) {
            // A homogeneous struct list de-boxes to columns: one packed `ListRepr`
            // per field (recursively, so nested structs stay columnar too).
            let columns = field_names
                .iter()
                .map(|fname| {
                    ListRepr::from_values(
                        values
                            .iter()
                            .map(|v| match v {
                                RuntimeValue::Struct(sv) => sv.fields.get(fname).cloned().unwrap(),
                                _ => unreachable!("struct_schema guaranteed all-struct"),
                            })
                            .collect(),
                    )
                })
                .collect();
            ListRepr::Structs { type_name, field_names, columns }
        } else if let Some(inductives) = Self::build_inductives(&values) {
            inductives
        } else {
            ListRepr::Boxed(values)
        }
    }

    /// Build a columnar [`ListRepr::Inductives`] from a homogeneous enum list (all
    /// the same `inductive_type`, each constructor used at a consistent arity).
    /// `None` if not a uniform enum list (the list stays boxed). Arguments are
    /// grouped DENSE per constructor and each group packed via `from_values`.
    pub(crate) fn build_inductives(values: &[RuntimeValue]) -> Option<ListRepr> {
        let inductive_type = match values.first()? {
            RuntimeValue::Inductive(i) => i.inductive_type.clone(),
            _ => return None,
        };
        let mut ctor_dict: Vec<String> = Vec::new();
        let mut ctors: Vec<u32> = Vec::with_capacity(values.len());
        let mut ranks: Vec<u32> = Vec::with_capacity(values.len());
        let mut counts: Vec<u32> = Vec::new();
        // grouped[c][j] = the j-th argument across rows whose constructor is `c`.
        let mut grouped: Vec<Vec<Vec<RuntimeValue>>> = Vec::new();
        for v in values {
            let iv = match v {
                RuntimeValue::Inductive(i) if i.inductive_type == inductive_type => i,
                _ => return None,
            };
            let c = match ctor_dict.iter().position(|n| n == &iv.constructor) {
                Some(c) => {
                    if grouped[c].len() != iv.args.len() {
                        return None; // a constructor used at inconsistent arity
                    }
                    c
                }
                None => {
                    ctor_dict.push(iv.constructor.clone());
                    counts.push(0);
                    grouped.push(vec![Vec::new(); iv.args.len()]);
                    ctor_dict.len() - 1
                }
            };
            ctors.push(c as u32);
            ranks.push(counts[c]);
            counts[c] += 1;
            for (j, a) in iv.args.iter().enumerate() {
                grouped[c][j].push(a.clone());
            }
        }
        let arg_cols: Vec<Vec<ListRepr>> = grouped
            .into_iter()
            .map(|cols| cols.into_iter().map(ListRepr::from_values).collect())
            .collect();
        Some(ListRepr::Inductives { inductive_type, ctor_dict, ctors, ranks, arg_cols })
    }

    /// Reconstruct row `i` of a columnar enum store as a boxed `InductiveValue`.
    fn inductive_row(
        inductive_type: &str,
        ctor_dict: &[String],
        ctors: &[u32],
        ranks: &[u32],
        arg_cols: &[Vec<ListRepr>],
        i: usize,
    ) -> Option<RuntimeValue> {
        let c = *ctors.get(i)? as usize;
        let r = ranks[i] as usize;
        let mut args = Vec::with_capacity(arg_cols[c].len());
        for col in &arg_cols[c] {
            args.push(col.get(r)?);
        }
        Some(RuntimeValue::Inductive(Box::new(InductiveValue {
            inductive_type: inductive_type.to_string(),
            constructor: ctor_dict[c].clone(),
            args,
        })))
    }

    /// If `values` is a non-empty run of structs that all share one `type_name` and
    /// the same field-name set, return `(type_name, sorted_field_names)` — the schema
    /// for a columnar [`ListRepr::Structs`]. `None` otherwise (the list stays boxed).
    /// Fields are sorted so the columnar order is canonical and stable.
    fn struct_schema(values: &[RuntimeValue]) -> Option<(String, Vec<String>)> {
        let first = match values.first()? {
            RuntimeValue::Struct(s) => s,
            _ => return None,
        };
        let mut names: Vec<String> = first.fields.keys().cloned().collect();
        names.sort();
        // A columnar store needs ≥1 column to carry the row count — a zero-field
        // struct list stays boxed.
        if names.is_empty() {
            return None;
        }
        for item in values {
            match item {
                RuntimeValue::Struct(s)
                    if s.type_name == first.type_name
                        && s.fields.len() == names.len()
                        && names.iter().all(|n| s.fields.contains_key(n)) => {}
                _ => return None,
            }
        }
        Some((first.type_name.clone(), names))
    }

    /// Reconstruct row `i` of a columnar struct store as a boxed `StructValue`.
    fn struct_row(type_name: &str, field_names: &[String], columns: &[ListRepr], i: usize) -> Option<RuntimeValue> {
        if i >= columns.first().map_or(0, |c| c.len()) {
            return None;
        }
        let mut fields = std::collections::HashMap::with_capacity(field_names.len());
        for (j, fname) in field_names.iter().enumerate() {
            fields.insert(fname.clone(), columns[j].get(i)?);
        }
        Some(RuntimeValue::Struct(Box::new(StructValue { type_name: type_name.to_string(), fields })))
    }

    /// A flat string buffer with an empty (lazy) materialization cache.
    pub fn strings(data: Vec<u8>, ends: Vec<u32>) -> ListRepr {
        ListRepr::Strings { data, ends, cache: RefCell::new(Vec::new()) }
    }

    /// Element `i` of a flat `Strings` buffer as an owned `String` (UTF-8 was
    /// validated when the buffer was built, but we re-check rather than risk UB).
    fn string_at(data: &[u8], ends: &[u32], i: usize) -> Option<String> {
        let end = *ends.get(i)? as usize;
        let start = if i == 0 { 0 } else { ends[i - 1] as usize };
        std::str::from_utf8(data.get(start..end)?).ok().map(str::to_string)
    }

    pub fn len(&self) -> usize {
        match self {
            ListRepr::Boxed(v) => v.len(),
            ListRepr::Ints(v) => v.len(),
            ListRepr::IntsI32(v) => v.len(),
            ListRepr::Floats(v) => v.len(),
            ListRepr::Bools(v) => v.len(),
            ListRepr::Strings { ends, .. } => ends.len(),
            ListRepr::Structs { columns, .. } => columns.first().map_or(0, |c| c.len()),
            ListRepr::Inductives { ctors, .. } => ctors.len(),
            ListRepr::WireStructs { len, .. } | ListRepr::WireColumn { len, .. } => *len,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// A human label for the underlying storage layout — for the debugger's memory
    /// view. It teaches that a list of ints is a *packed* `Vec<i64>` (not boxed
    /// values), a struct list is columnar (struct-of-arrays), and so on: the dense
    /// representations the VM picks automatically.
    pub fn storage_label(&self) -> &'static str {
        match self {
            ListRepr::Boxed(_) => "boxed values",
            ListRepr::Ints(_) => "packed Vec<i64>",
            ListRepr::IntsI32(_) => "packed Vec<i32> (narrowed)",
            ListRepr::Floats(_) => "packed Vec<f64>",
            ListRepr::Bools(_) => "packed Vec<bool>",
            ListRepr::Strings { .. } => "flat string buffer",
            ListRepr::Structs { .. } => "columnar (struct-of-arrays)",
            ListRepr::Inductives { .. } => "columnar tagged-union",
            ListRepr::WireStructs { .. } | ListRepr::WireColumn { .. } => "wire view (lazy)",
        }
    }

    /// Drop every element past `n` (no-op when already `<= n`). The region
    /// deopt path uses this to roll a pushed buffer back to its entry length
    /// so discard-and-replay re-pushes cleanly instead of duplicating.
    pub fn truncate(&mut self, n: usize) {
        match self {
            ListRepr::Boxed(v) => v.truncate(n),
            ListRepr::Ints(v) => v.truncate(n),
            ListRepr::IntsI32(v) => v.truncate(n),
            ListRepr::Floats(v) => v.truncate(n),
            ListRepr::Bools(v) => v.truncate(n),
            ListRepr::Strings { data, ends, cache } => {
                if n < ends.len() {
                    let cut = if n == 0 { 0 } else { ends[n - 1] as usize };
                    data.truncate(cut);
                    ends.truncate(n);
                    let mut c = cache.borrow_mut();
                    if !c.is_empty() {
                        c.truncate(n);
                    }
                }
            }
            ListRepr::Structs { columns, .. } => {
                for c in columns.iter_mut() {
                    c.truncate(n);
                }
            }
            // The union's arg columns are dense (not row-aligned), so a row-range
            // truncate de-columnarizes first (a rare path — region rollback).
            ListRepr::Inductives { .. } => {
                if n < self.len() {
                    self.make_boxed().truncate(n);
                }
            }
            // A structural mutation de-lazies the received view first.
            ListRepr::WireStructs { .. } | ListRepr::WireColumn { .. } => {
                if n < self.len() {
                    self.make_boxed().truncate(n);
                }
            }
        }
    }

    /// 0-based read; boxes the scalar (stack-only — no heap, no Rc traffic).
    pub fn get(&self, i: usize) -> Option<RuntimeValue> {
        match self {
            ListRepr::Boxed(v) => v.get(i).cloned(),
            ListRepr::Ints(v) => v.get(i).map(|&n| RuntimeValue::Int(n)),
            ListRepr::IntsI32(v) => v.get(i).map(|&n| RuntimeValue::Int(n as i64)),
            ListRepr::Floats(v) => v.get(i).map(|&f| RuntimeValue::Float(f)),
            ListRepr::Bools(v) => v.get(i).map(|&b| RuntimeValue::Bool(b)),
            ListRepr::Strings { data, ends, cache } => {
                if i >= ends.len() {
                    return None;
                }
                let mut c = cache.borrow_mut();
                // Lazily size the memo on first access — ship/load/iterate never
                // touch it, so they pay nothing.
                if c.is_empty() {
                    c.resize(ends.len(), None);
                }
                if c[i].is_none() {
                    c[i] = Some(Rc::new(Self::string_at(data, ends, i)?));
                }
                c[i].clone().map(RuntimeValue::Text)
            }
            ListRepr::Structs { type_name, field_names, columns } => {
                Self::struct_row(type_name, field_names, columns, i)
            }
            ListRepr::Inductives { inductive_type, ctor_dict, ctors, ranks, arg_cols } => {
                Self::inductive_row(inductive_type, ctor_dict, ctors, ranks, arg_cols, i)
            }
            ListRepr::WireStructs { bytes, type_name, field_names, len } => {
                Self::wire_struct_row(bytes, type_name, field_names, *len, i)
            }
            ListRepr::WireColumn { bytes, floats, .. } => Self::wire_column_get(bytes, *floats, i),
        }
    }

    /// Read field `name` of row `i` from a columnar struct list by indexing ONE
    /// column directly — no `StructValue` reconstruction. `None` for a non-columnar
    /// repr or a missing field/row. This is the zero-alloc read path that makes a
    /// field scan over a columnar struct list run at array speed.
    pub fn get_field(&self, i: usize, name: &str) -> Option<RuntimeValue> {
        match self {
            ListRepr::Structs { field_names, columns, .. } => {
                let j = field_names.iter().position(|f| f == name)?;
                columns[j].get(i)
            }
            // The lazy zero-copy receive: locate and decode JUST this cell in place — no row
            // reconstruction, no decode of the other rows/fields.
            ListRepr::WireStructs { bytes, .. } => {
                crate::concurrency::marshal::view_message(bytes)?.structs_row_field_value(i, name)
            }
            _ => None,
        }
    }

    /// Direct access to a struct field's whole packed column (the array behind a
    /// field), for aggregating one field across the list at array speed. `None` for
    /// a non-columnar repr or a missing field.
    pub fn column(&self, name: &str) -> Option<&ListRepr> {
        match self {
            ListRepr::Structs { field_names, columns, .. } => {
                let j = field_names.iter().position(|f| f == name)?;
                Some(&columns[j])
            }
            _ => None,
        }
    }

    /// Re-tag to Boxed in place (aliases see it — same Rc).
    fn make_boxed(&mut self) -> &mut Vec<RuntimeValue> {
        match self {
            ListRepr::Boxed(v) => v,
            ListRepr::Ints(v) => {
                let boxed = v.drain(..).map(RuntimeValue::Int).collect();
                *self = ListRepr::Boxed(boxed);
                match self {
                    ListRepr::Boxed(v) => v,
                    _ => unreachable!(),
                }
            }
            ListRepr::Floats(v) => {
                let boxed = v.drain(..).map(RuntimeValue::Float).collect();
                *self = ListRepr::Boxed(boxed);
                match self {
                    ListRepr::Boxed(v) => v,
                    _ => unreachable!(),
                }
            }
            ListRepr::IntsI32(v) => {
                let boxed = v.drain(..).map(|n| RuntimeValue::Int(n as i64)).collect();
                *self = ListRepr::Boxed(boxed);
                match self {
                    ListRepr::Boxed(v) => v,
                    _ => unreachable!(),
                }
            }
            ListRepr::Bools(v) => {
                let boxed = v.drain(..).map(RuntimeValue::Bool).collect();
                *self = ListRepr::Boxed(boxed);
                match self {
                    ListRepr::Boxed(v) => v,
                    _ => unreachable!(),
                }
            }
            ListRepr::Strings { data, ends, .. } => {
                let boxed = (0..ends.len())
                    .filter_map(|i| Self::string_at(data, ends, i).map(|s| RuntimeValue::Text(Rc::new(s))))
                    .collect();
                *self = ListRepr::Boxed(boxed);
                match self {
                    ListRepr::Boxed(v) => v,
                    _ => unreachable!(),
                }
            }
            ListRepr::Structs { type_name, field_names, columns } => {
                let n = columns.first().map_or(0, |c| c.len());
                // Invariant (held by `from_values` and the wire decoder): every column
                // is the same length, so no row is silently dropped on de-columnarize.
                debug_assert!(columns.iter().all(|c| c.len() == n), "columnar struct columns must share one length");
                let boxed = (0..n)
                    .filter_map(|i| Self::struct_row(type_name, field_names, columns, i))
                    .collect();
                *self = ListRepr::Boxed(boxed);
                match self {
                    ListRepr::Boxed(v) => v,
                    _ => unreachable!(),
                }
            }
            ListRepr::Inductives { inductive_type, ctor_dict, ctors, ranks, arg_cols } => {
                let n = ctors.len();
                debug_assert_eq!(ranks.len(), n, "ranks and ctors must agree");
                let boxed = (0..n)
                    .filter_map(|i| Self::inductive_row(inductive_type, ctor_dict, ctors, ranks, arg_cols, i))
                    .collect();
                *self = ListRepr::Boxed(boxed);
                match self {
                    ListRepr::Boxed(v) => v,
                    _ => unreachable!(),
                }
            }
            // A mutation forces the lazy receive to fully decode (read every row in place) and
            // re-tag to `Boxed`, so reference semantics match a boxed list from that point on.
            ListRepr::WireStructs { bytes, type_name, field_names, len } => {
                let len = *len;
                let boxed: Vec<RuntimeValue> = {
                    let view = crate::concurrency::marshal::view_message(bytes);
                    (0..len)
                        .filter_map(|i| {
                            view.as_ref().and_then(|v| {
                                let mut fields = HashMap::with_capacity(field_names.len());
                                for name in field_names.iter() {
                                    let cell = v.structs_row_field_value(i, name)?;
                                    fields.insert(name.clone(), cell);
                                }
                                Some(RuntimeValue::Struct(Box::new(StructValue {
                                    type_name: type_name.clone(),
                                    fields,
                                })))
                            })
                        })
                        .collect()
                };
                *self = ListRepr::Boxed(boxed);
                match self {
                    ListRepr::Boxed(v) => v,
                    _ => unreachable!(),
                }
            }
            ListRepr::WireColumn { bytes, floats, .. } => {
                let boxed = Self::wire_column_to_values(bytes, *floats);
                *self = ListRepr::Boxed(boxed);
                match self {
                    ListRepr::Boxed(v) => v,
                    _ => unreachable!(),
                }
            }
        }
    }

    /// Re-tag a half-width `IntsI32` buffer to full-width `Ints` in place — the
    /// soundness fallback when a value outside `i32` range reaches a narrowed
    /// buffer (the narrowing proof was unsound for that store). Sign-extends
    /// every existing element losslessly. After this the buffer behaves exactly
    /// like a buffer that was never narrowed.
    fn widen_to_ints(&mut self) -> &mut Vec<i64> {
        match self {
            ListRepr::IntsI32(v) => {
                let wide: Vec<i64> = v.drain(..).map(|n| n as i64).collect();
                *self = ListRepr::Ints(wide);
                match self {
                    ListRepr::Ints(v) => v,
                    _ => unreachable!(),
                }
            }
            ListRepr::Ints(v) => v,
            _ => unreachable!("widen_to_ints called on a non-Int buffer"),
        }
    }

    /// 0-based write (bounds already validated by the caller); promotes on a
    /// kind mismatch.
    pub fn set(&mut self, i: usize, value: RuntimeValue) {
        match (&mut *self, &value) {
            (ListRepr::Ints(v), RuntimeValue::Int(n)) => v[i] = *n,
            (ListRepr::IntsI32(v), RuntimeValue::Int(n)) => {
                if let Ok(narrow) = i32::try_from(*n) {
                    v[i] = narrow;
                } else {
                    // The proof said every store fits i32; this one did not.
                    // Widen the whole buffer rather than truncate (which would
                    // silently change the observable value). Soundness wins.
                    self.widen_to_ints()[i] = *n;
                }
            }
            (ListRepr::Floats(v), RuntimeValue::Float(f)) => v[i] = *f,
            (ListRepr::Bools(v), RuntimeValue::Bool(b)) => v[i] = *b,
            (ListRepr::Boxed(v), _) => v[i] = value,
            _ => self.make_boxed()[i] = value,
        }
    }

    pub fn push(&mut self, value: RuntimeValue) {
        match (&mut *self, &value) {
            (ListRepr::Ints(v), RuntimeValue::Int(n)) => v.push(*n),
            (ListRepr::IntsI32(v), RuntimeValue::Int(n)) => match i32::try_from(*n) {
                Ok(narrow) => v.push(narrow),
                Err(_) => self.widen_to_ints().push(*n),
            },
            (ListRepr::Floats(v), RuntimeValue::Float(f)) => v.push(*f),
            (ListRepr::Bools(v), RuntimeValue::Bool(b)) => v.push(*b),
            (ListRepr::Boxed(v), _) => v.push(value),
            (ListRepr::Ints(v), RuntimeValue::Float(f)) if v.is_empty() => {
                *self = ListRepr::Floats(vec![*f]);
            }
            (ListRepr::Ints(v), RuntimeValue::Bool(b)) if v.is_empty() => {
                *self = ListRepr::Bools(vec![*b]);
            }
            (ListRepr::Floats(v), RuntimeValue::Int(n)) if v.is_empty() => {
                *self = ListRepr::Ints(vec![*n]);
            }
            (ListRepr::Floats(v), RuntimeValue::Bool(b)) if v.is_empty() => {
                *self = ListRepr::Bools(vec![*b]);
            }
            (ListRepr::Bools(v), RuntimeValue::Int(n)) if v.is_empty() => {
                *self = ListRepr::Ints(vec![*n]);
            }
            (ListRepr::Bools(v), RuntimeValue::Float(f)) if v.is_empty() => {
                *self = ListRepr::Floats(vec![*f]);
            }
            _ => self.make_boxed().push(value),
        }
    }

    pub fn pop(&mut self) -> Option<RuntimeValue> {
        match self {
            ListRepr::Boxed(v) => v.pop(),
            ListRepr::Ints(v) => v.pop().map(RuntimeValue::Int),
            ListRepr::IntsI32(v) => v.pop().map(|n| RuntimeValue::Int(n as i64)),
            ListRepr::Floats(v) => v.pop().map(RuntimeValue::Float),
            ListRepr::Bools(v) => v.pop().map(RuntimeValue::Bool),
            ListRepr::Strings { data, ends, cache } => {
                let last = ends.len().checked_sub(1)?;
                let s = Self::string_at(data, ends, last)?;
                let start = if last == 0 { 0 } else { ends[last - 1] as usize };
                data.truncate(start);
                ends.pop();
                let mut c = cache.borrow_mut();
                if !c.is_empty() {
                    c.pop();
                }
                Some(RuntimeValue::Text(Rc::new(s)))
            }
            // Removing from a columnar store: de-columnarize, then pop.
            ListRepr::Structs { .. } => self.make_boxed().pop(),
            ListRepr::Inductives { .. } => self.make_boxed().pop(),
            ListRepr::WireStructs { .. } | ListRepr::WireColumn { .. } => self.make_boxed().pop(),
        }
    }

    pub fn insert(&mut self, i: usize, value: RuntimeValue) {
        match (&mut *self, &value) {
            (ListRepr::Ints(v), RuntimeValue::Int(n)) => v.insert(i, *n),
            (ListRepr::IntsI32(v), RuntimeValue::Int(n)) => match i32::try_from(*n) {
                Ok(narrow) => v.insert(i, narrow),
                Err(_) => self.widen_to_ints().insert(i, *n),
            },
            (ListRepr::Floats(v), RuntimeValue::Float(f)) => v.insert(i, *f),
            (ListRepr::Bools(v), RuntimeValue::Bool(b)) => v.insert(i, *b),
            (ListRepr::Boxed(v), _) => v.insert(i, value),
            _ => self.make_boxed().insert(i, value),
        }
    }

    pub fn remove_at(&mut self, i: usize) -> RuntimeValue {
        match self {
            ListRepr::Boxed(v) => v.remove(i),
            ListRepr::Ints(v) => RuntimeValue::Int(v.remove(i)),
            ListRepr::IntsI32(v) => RuntimeValue::Int(v.remove(i) as i64),
            ListRepr::Floats(v) => RuntimeValue::Float(v.remove(i)),
            ListRepr::Bools(v) => RuntimeValue::Bool(v.remove(i)),
            // Removal in the middle is O(n) on a flat buffer; promote and remove.
            ListRepr::Strings { .. } => self.make_boxed().remove(i),
            ListRepr::Structs { .. } => self.make_boxed().remove(i),
            ListRepr::Inductives { .. } => self.make_boxed().remove(i),
            ListRepr::WireStructs { .. } | ListRepr::WireColumn { .. } => self.make_boxed().remove(i),
        }
    }

    /// Index of the first element `values_equal` to `needle` (the kernel's
    /// equality: epsilon floats, cross-type never equal).
    pub fn position(&self, needle: &RuntimeValue) -> Option<usize> {
        match (self, needle) {
            (ListRepr::Ints(v), RuntimeValue::Int(n)) => v.iter().position(|x| x == n),
            (ListRepr::Ints(_), _) => None,
            (ListRepr::IntsI32(v), RuntimeValue::Int(n)) => {
                i32::try_from(*n).ok().and_then(|nn| v.iter().position(|x| *x == nn))
            }
            (ListRepr::IntsI32(_), _) => None,
            (ListRepr::Floats(v), RuntimeValue::Float(f)) => {
                v.iter().position(|x| (x - f).abs() < f64::EPSILON)
            }
            (ListRepr::Floats(_), _) => None,
            (ListRepr::Bools(v), RuntimeValue::Bool(b)) => v.iter().position(|x| x == b),
            (ListRepr::Bools(_), _) => None,
            (ListRepr::Strings { data, ends, .. }, RuntimeValue::Text(t)) => {
                (0..ends.len()).find(|&i| Self::string_at(data, ends, i).as_deref() == Some(t.as_str()))
            }
            (ListRepr::Strings { .. }, _) => None,
            (ListRepr::Boxed(v), _) => {
                v.iter().position(|x| crate::semantics::compare::values_equal(x, needle))
            }
            // A struct never equals a scalar needle, but a needle could be a struct;
            // reconstruct row-by-row and compare (rare path — search over structs).
            (ListRepr::Structs { .. }, _) => (0..self.len())
                .find(|&i| self.get(i).is_some_and(|v| crate::semantics::compare::values_equal(&v, needle))),
            (ListRepr::Inductives { .. }, _) => (0..self.len())
                .find(|&i| self.get(i).is_some_and(|v| crate::semantics::compare::values_equal(&v, needle))),
            // The lazy receive: reconstruct rows/elements on demand and compare.
            (ListRepr::WireStructs { .. }, _) | (ListRepr::WireColumn { .. }, _) => (0..self.len())
                .find(|&i| self.get(i).is_some_and(|v| crate::semantics::compare::values_equal(&v, needle))),
        }
    }

    pub fn contains(&self, needle: &RuntimeValue) -> bool {
        self.position(needle).is_some()
    }

    /// Materialize boxed values (snapshots, display, deep clones).
    pub fn to_values(&self) -> Vec<RuntimeValue> {
        match self {
            ListRepr::Boxed(v) => v.clone(),
            ListRepr::Ints(v) => v.iter().map(|&n| RuntimeValue::Int(n)).collect(),
            ListRepr::IntsI32(v) => v.iter().map(|&n| RuntimeValue::Int(n as i64)).collect(),
            ListRepr::Floats(v) => v.iter().map(|&f| RuntimeValue::Float(f)).collect(),
            ListRepr::Bools(v) => v.iter().map(|&b| RuntimeValue::Bool(b)).collect(),
            ListRepr::Strings { data, ends, .. } => (0..ends.len())
                .filter_map(|i| Self::string_at(data, ends, i).map(|s| RuntimeValue::Text(Rc::new(s))))
                .collect(),
            ListRepr::Structs { type_name, field_names, columns } => {
                let n = columns.first().map_or(0, |c| c.len());
                (0..n).filter_map(|i| Self::struct_row(type_name, field_names, columns, i)).collect()
            }
            ListRepr::Inductives { inductive_type, ctor_dict, ctors, ranks, arg_cols } => (0..ctors.len())
                .filter_map(|i| Self::inductive_row(inductive_type, ctor_dict, ctors, ranks, arg_cols, i))
                .collect(),
            ListRepr::WireStructs { bytes, type_name, field_names, len } => (0..*len)
                .filter_map(|i| Self::wire_struct_row(bytes, type_name, field_names, *len, i))
                .collect(),
            ListRepr::WireColumn { bytes, floats, .. } => Self::wire_column_to_values(bytes, *floats),
        }
    }

    /// 0-based inclusive-range slice as a fresh payload of the same repr.
    pub fn slice(&self, start: usize, end: usize) -> ListRepr {
        match self {
            ListRepr::Boxed(v) => ListRepr::Boxed(v[start..=end].to_vec()),
            ListRepr::Ints(v) => ListRepr::Ints(v[start..=end].to_vec()),
            ListRepr::IntsI32(v) => ListRepr::IntsI32(v[start..=end].to_vec()),
            ListRepr::Floats(v) => ListRepr::Floats(v[start..=end].to_vec()),
            ListRepr::Bools(v) => ListRepr::Bools(v[start..=end].to_vec()),
            ListRepr::Strings { data, ends, .. } => ListRepr::Boxed(
                (start..=end)
                    .filter_map(|i| Self::string_at(data, ends, i).map(|s| RuntimeValue::Text(Rc::new(s))))
                    .collect(),
            ),
            // Slicing stays columnar — slice each column to the same range.
            ListRepr::Structs { type_name, field_names, columns } => ListRepr::Structs {
                type_name: type_name.clone(),
                field_names: field_names.clone(),
                columns: columns.iter().map(|c| c.slice(start, end)).collect(),
            },
            // The union's arg columns are dense, so re-columnarize the sliced rows.
            ListRepr::Inductives { .. } => {
                ListRepr::from_values((start..=end).filter_map(|i| self.get(i)).collect())
            }
            // Reconstruct just the sliced rows/elements from the received view.
            ListRepr::WireStructs { .. } | ListRepr::WireColumn { .. } => {
                ListRepr::from_values((start..=end).filter_map(|i| self.get(i)).collect())
            }
        }
    }

    /// Direct unboxed views for the JIT's region pinning.
    pub fn as_ints_mut(&mut self) -> Option<&mut Vec<i64>> {
        match self {
            ListRepr::Ints(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_floats_mut(&mut self) -> Option<&mut Vec<f64>> {
        match self {
            ListRepr::Floats(v) => Some(v),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum RuntimeValue {
    Int(i64),
    /// An exact integer that does NOT fit `i64` — the overflow-safe continuation of
    /// `Int`. INVARIANT: `b.to_i64().is_none()` always holds (build via
    /// [`RuntimeValue::from_bigint`], which downsizes any in-range result back to
    /// `Int`), so there is exactly one representation per integer value and `Eq`/
    /// `Hash`/ordering never need a cross-`Int` arm. `Rc` keeps `Clone` O(1).
    BigInt(Rc<logicaffeine_base::BigInt>),
    /// An exact rational number — the result of an integer division that does NOT
    /// divide evenly (`7 / 2 → 7/2`), the way `Int` "overflows" into `BigInt`.
    /// INVARIANT: never a whole number — build via [`RuntimeValue::from_rational`],
    /// which downsizes an integer-valued rational to `Int`/`BigInt`, so a value has
    /// one canonical representation and `Eq`/`Hash` need no cross-`Int` arm.
    Rational(Rc<logicaffeine_base::Rational>),
    /// An exact base-10 fixed-point number — money's type. Distinct from `Rational`:
    /// it carries a *scale* (decimal places) for faithful display (`19.99`, not
    /// `1999/100`), and unlike `Int`/`BigInt`/`Rational` it does NOT downsize on a
    /// whole value (`20.00` stays `Decimal`, not `Int`), because the scale is meaning.
    /// `+ − ×` are exact and keep it `Decimal`; `÷` and a `Rational` operand promote to
    /// the exact `Rational` (base-10 division need not terminate). `Rc` keeps `Clone` O(1).
    Decimal(Rc<logicaffeine_base::Decimal>),
    /// An exact complex number `re + im·i`, each part a `Rational`. The field that closes
    /// the tower for `√` of a negative and for EE/signal math: `i·i = −1` exactly. NOT
    /// ordered (complex numbers have no total order), so it never appears in `compare`.
    Complex(Rc<logicaffeine_base::Complex>),
    /// An element of the ring ℤ/nℤ — an integer modulo a fixed modulus (the arbitrary-modulus
    /// generalisation of `Word`). Arithmetic wraps into `[0, modulus)`; the crypto/number-theory
    /// substrate (modular exponentiation, inverse). Equal only at the same value AND modulus.
    Modular(Rc<logicaffeine_base::Modular>),
    Float(f64),
    Bool(bool),
    Text(Rc<String>),
    Char(char),
    List(Rc<RefCell<ListRepr>>),
    Tuple(Rc<Vec<RuntimeValue>>),
    Set(Rc<RefCell<Vec<RuntimeValue>>>),
    Map(Rc<RefCell<MapStorage>>),
    Struct(Box<StructValue>),
    Inductive(Box<InductiveValue>),
    Function(Box<ClosureValue>),
    Nothing,
    Duration(i64),
    Date(i32),
    Moment(i64),
    Span { months: i32, days: i32 },
    Time(i64),
    /// A channel handle (a `Pipe`) — an opaque token into the scheduler.
    Chan(ChanId),
    /// A spawned-task handle — an opaque token into the scheduler.
    TaskHandle(TaskId),
    /// A remote peer handle — its canonical relay topic. `Send … to <peer>`
    /// publishes on this topic; the peer receives it on its own inbox.
    Peer(Rc<String>),
    /// A live CRDT (observed-remove set, replicated sequence, or multi-value register)
    /// held by the tree-walker. Wraps the real `logicaffeine_data` type the compiled tier
    /// uses, so merge converges identically across tiers. `Rc<RefCell<_>>` gives the same
    /// interior-mutation/aliasing semantics as `Set`/`List`/`Map`, so mutating a struct's
    /// CRDT field through a field access updates the shared value in place.
    Crdt(Rc<RefCell<crate::semantics::crdt::CrdtValue>>),
    /// A fixed-width wrapping integer (`Word32`/`Word64`) — the ring ℤ/2ᵏ the bit-twiddling
    /// primitives (ChaCha20 over `Word32`, Keccak over `Word64`) compute over. Distinct from
    /// `Int`: its arithmetic wraps and it never promotes to `BigInt`.
    Word(logicaffeine_base::WordVal),
    /// A SIMD lane vector (`Lanes8Word32` = 8×`Word32` = one `__m256i`) — a fixed-width vector over
    /// the Word ring. The tree-walker carries the scalar-lane representation and computes each op as
    /// independent scalar lanes (the spec); AOT lowers the same op to an AVX2 intrinsic. Boxed in `Rc`
    /// so the 256-bit lane payload stays out of the 16-byte `RuntimeValue` (the NaN-box invariant).
    Lanes(Rc<logicaffeine_base::LanesVal>),
    /// A physical quantity — an exact magnitude carrying a `Dimension` and a display unit
    /// (`2 inches`, `9.8 m/s²`). The magnitude rides the exact rational tower, so unit conversion
    /// is lossless (`2 inches + 5 cm in feet = 42/127 ft`); `+ −` and comparison require the SAME
    /// dimension (else a typed error, like Word width-mismatch), `× ÷` combine dimensions. The
    /// display unit travels with the value so `Show` renders it faithfully.
    Quantity(Rc<QuantityValue>),
    /// An exact monetary amount in a currency (`19.99 USD`). The amount rides the Decimal tower so it
    /// never float-drifts; `+ −` and comparison require the SAME currency (else a typed error, like a
    /// dimension mismatch), `× ÷` scale by a number. The currency travels with the value.
    Money(Rc<logicaffeine_base::Money>),
    /// A 128-bit UUID (RFC 9562). `Ord` by bytes — so v6/v7 ids sort chronologically — and a stable
    /// canonical text form. `Rc`-boxed to keep `RuntimeValue` at 16 bytes (the value itself is a
    /// `Copy [u8;16]`; the compiled tier carries it unboxed).
    Uuid(Rc<logicaffeine_base::Uuid>),
}

/// The payload of a [`RuntimeValue::Quantity`]: the physical quantity (magnitude in SI base +
/// dimension) plus the unit it should be displayed in. Equality/hashing are by physical value
/// (SI magnitude + dimension) — the display unit is presentation only, so `2 inches` equals
/// `5.08 centimetres`.
#[derive(Clone, Debug)]
pub struct QuantityValue {
    pub q: logicaffeine_base::Quantity,
    pub unit: logicaffeine_base::Unit,
}

impl QuantityValue {
    /// The faithful display: the magnitude expressed in the carried unit, then its symbol —
    /// `42/127 ft`, `2 in`, `20 °C`. A synthetic SI unit (empty symbol, produced by a
    /// dimension-combining `× ÷`) shows the dimension signature instead (`12 L^2`).
    pub fn display(&self) -> String {
        let magnitude = self
            .q
            .in_unit(&self.unit)
            .expect("a Quantity's display unit always shares its dimension");
        if self.unit.symbol.is_empty() {
            format!("{} {}", magnitude, self.q.dimension())
        } else {
            format!("{} {}", magnitude, self.unit.symbol)
        }
    }
}

impl PartialEq for RuntimeValue {
    /// ONE equality: delegates to [`crate::semantics::compare::values_equal`],
    /// so map-key lookup, set membership, and the language's `==` can never
    /// disagree. Structural for collections/structs, EXACT across numeric
    /// types (`1 == 1.0`), IEEE for floats — coherent with the unified
    /// numeric `Hash` below (equal values hash equal).
    fn eq(&self, other: &Self) -> bool {
        crate::semantics::compare::values_equal(self, other)
    }
}

/// NOTE: `eq` is IEEE on floats, so `NaN != NaN` — strictly this bends `Eq`'s
/// reflexivity for the one value IEEE defines as not equal to itself. The
/// trade is deliberate: map keys behave exactly like the language's own `==`
/// (a NaN key is unfindable, as IEEE intends) instead of maps and `==`
/// silently disagreeing about float identity. Everything else is a total
/// equivalence.
impl Eq for RuntimeValue {}

impl std::hash::Hash for RuntimeValue {
    /// The hash/equality coherence law: values that compare equal MUST hash
    /// equal. Numeric types are cross-type equal (`1 == 1.0 == 1/1`), so they
    /// share ONE hash stream — the unified numeric hash (value mod 2^61 − 1,
    /// `base::numeric`) with NO discriminant prefix. Everything else keeps
    /// its discriminant-prefixed per-type hash (collisions between UNEQUAL
    /// values are always allowed; only equal ⇒ equal-hash is required).
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        use logicaffeine_base::numeric;
        match self {
            // ── The unified numeric stream (no discriminant) ─────────────
            RuntimeValue::Int(n) => state.write_u64(numeric::numeric_hash_i64(*n)),
            RuntimeValue::BigInt(b) => state.write_u64(numeric::numeric_hash_bigint(b)),
            RuntimeValue::Float(f) => state.write_u64(numeric::numeric_hash_f64(*f)),
            RuntimeValue::Rational(r) => state.write_u64(numeric::numeric_hash_rational(r)),
            // ── Discriminant-prefixed per-type hashes ────────────────────
            other => {
                std::mem::discriminant(other).hash(state);
                match other {
                    RuntimeValue::Int(_)
                    | RuntimeValue::BigInt(_)
                    | RuntimeValue::Float(_)
                    | RuntimeValue::Rational(_) => unreachable!("handled above"),
                    RuntimeValue::Decimal(d) => d.hash(state),
                    RuntimeValue::Complex(c) => c.hash(state),
                    RuntimeValue::Modular(m) => m.hash(state),
                    RuntimeValue::Bool(b) => b.hash(state),
                    RuntimeValue::Text(s) => s.hash(state),
                    RuntimeValue::Char(c) => c.hash(state),
                    RuntimeValue::Nothing => {}
                    RuntimeValue::Duration(d) => d.hash(state),
                    RuntimeValue::Date(d) => d.hash(state),
                    RuntimeValue::Moment(m) => m.hash(state),
                    RuntimeValue::Span { months, days } => { months.hash(state); days.hash(state); }
                    RuntimeValue::Time(t) => t.hash(state),
                    // Tuples are VALUE keys: content-hashed, in order —
                    // coherent with their structural equality.
                    RuntimeValue::Tuple(items) => {
                        items.len().hash(state);
                        for v in items.iter() {
                            v.hash(state);
                        }
                    }
                    // Structs are VALUE keys: type + an ORDER-INSENSITIVE
                    // field fold (the fields map iterates nondeterministically,
                    // and equal structs must hash equal).
                    RuntimeValue::Struct(s) => {
                        s.type_name.hash(state);
                        let mut fold: u64 = 0;
                        for (k, v) in &s.fields {
                            let mut h = rustc_hash::FxHasher::default();
                            std::hash::Hash::hash(k, &mut h);
                            std::hash::Hash::hash(v, &mut h);
                            fold = fold.wrapping_add(std::hash::Hasher::finish(&h));
                        }
                        state.write_u64(fold);
                    }
                    // Mutable containers hash by LENGTH — consistent with
                    // structural equality (equal ⇒ equal length), and they
                    // are rejected as map keys at insert anyway.
                    RuntimeValue::List(items) => items.borrow().len().hash(state),
                    RuntimeValue::Set(items) => items.borrow().len().hash(state),
                    RuntimeValue::Map(m) => m.borrow().len().hash(state),
                    RuntimeValue::Inductive(i) => { i.inductive_type.hash(state); i.constructor.hash(state); }
                    RuntimeValue::Function(f) => f.body_index.hash(state),
                    RuntimeValue::Chan(c) => c.0.hash(state),
                    RuntimeValue::TaskHandle(t) => t.0.hash(state),
                    RuntimeValue::Peer(topic) => topic.hash(state),
                    RuntimeValue::Crdt(c) => c.borrow().len().hash(state),
                    RuntimeValue::Word(w) => w.hash(state),
                    RuntimeValue::Lanes(v) => v.hash(state),
                    // Hash the physical value (SI magnitude + dimension), consistent with `eq`
                    // ignoring the display unit.
                    RuntimeValue::Quantity(qv) => {
                        qv.q.magnitude_si().hash(state);
                        qv.q.dimension().hash(state);
                    }
                    // Hash by value (currency + amount), consistent with `eq`.
                    RuntimeValue::Money(m) => m.hash(state),
                    RuntimeValue::Uuid(u) => u.hash(state),
                }
            }
        }
    }
}

impl RuntimeValue {
    /// Build an integer value from a `BigInt`, DOWNSIZING to [`RuntimeValue::Int`]
    /// whenever the value fits `i64`. This is the single chokepoint that maintains
    /// the `BigInt`-is-always-out-of-range invariant, so every integer has one
    /// canonical representation — the "downsize when it provably fits" rule, applied
    /// unconditionally on every result.
    pub fn from_bigint(b: logicaffeine_base::BigInt) -> RuntimeValue {
        match b.to_i64() {
            Some(i) => RuntimeValue::Int(i),
            None => RuntimeValue::BigInt(Rc::new(b)),
        }
    }

    /// Build a number from a `Rational`, DOWNSIZING to an exact integer
    /// (`Int`/`BigInt`) whenever the denominator reduces to `1`. This is the single
    /// chokepoint that maintains the `Rational`-is-never-whole invariant, so an
    /// integer-valued result (`6 / 2 → 3`) is an `Int`, not a `Rational` — exactly the
    /// "downsize when it provably fits" rule `from_bigint` applies for integers.
    pub fn from_rational(r: logicaffeine_base::Rational) -> RuntimeValue {
        match r.to_bigint() {
            Some(whole) => RuntimeValue::from_bigint(whole),
            None => RuntimeValue::Rational(Rc::new(r)),
        }
    }

    /// Returns the type name of this value as a string slice.
    ///
    /// Used for error messages and type checking at runtime.
    pub fn type_name(&self) -> &str {
        match self {
            RuntimeValue::Int(_) => "Int",
            // A BigInt is an exact integer too — same logical type, wider repr — so it
            // reports "Int", keeping the type stable across promotion/downsizing.
            RuntimeValue::BigInt(_) => "Int",
            RuntimeValue::Rational(_) => "Rational",
            RuntimeValue::Decimal(_) => "Decimal",
            RuntimeValue::Complex(_) => "Complex",
            RuntimeValue::Modular(_) => "Modular",
            RuntimeValue::Float(_) => "Float",
            RuntimeValue::Bool(_) => "Bool",
            RuntimeValue::Text(_) => "Text",
            RuntimeValue::Char(_) => "Char",
            RuntimeValue::List(_) => "List",
            RuntimeValue::Tuple(_) => "Tuple",
            RuntimeValue::Set(_) => "Set",
            RuntimeValue::Map(_) => "Map",
            RuntimeValue::Struct(s) => &s.type_name,
            RuntimeValue::Inductive(ind) => ind.inductive_type.as_str(),
            RuntimeValue::Function(_) => "Function",
            RuntimeValue::Nothing => "Nothing",
            RuntimeValue::Duration(_) => "Duration",
            RuntimeValue::Date(_) => "Date",
            RuntimeValue::Moment(_) => "Moment",
            RuntimeValue::Span { .. } => "Span",
            RuntimeValue::Time(_) => "Time",
            RuntimeValue::Chan(_) => "Channel",
            RuntimeValue::TaskHandle(_) => "Task",
            RuntimeValue::Peer(_) => "PeerAgent",
            RuntimeValue::Crdt(c) => c.borrow().kind(),
            RuntimeValue::Word(w) => {
                if w.width() == 32 {
                    "Word32"
                } else {
                    "Word64"
                }
            }
            RuntimeValue::Lanes(v) => v.type_name(),
            RuntimeValue::Quantity(_) => "Quantity",
            RuntimeValue::Money(_) => "Money",
            RuntimeValue::Uuid(_) => "Uuid",
        }
    }

    /// Checks if this value evaluates to true in a boolean context.
    ///
    /// - `Bool(true)` → true
    /// - `Int(n)` → true if n ≠ 0
    /// - `Nothing` → false
    /// - All other values → true
    pub fn deep_clone(&self) -> RuntimeValue {
        match self {
            RuntimeValue::List(items) => {
                let cloned: Vec<RuntimeValue> =
                    items.borrow().to_values().iter().map(|v| v.deep_clone()).collect();
                RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(cloned))))
            }
            RuntimeValue::Set(items) => {
                let cloned = items.borrow().iter().map(|v| v.deep_clone()).collect();
                RuntimeValue::Set(Rc::new(RefCell::new(cloned)))
            }
            RuntimeValue::Map(m) => {
                let cloned = m.borrow().iter().map(|(k, v)| (k.deep_clone(), v.deep_clone())).collect();
                RuntimeValue::Map(Rc::new(RefCell::new(cloned)))
            }
            RuntimeValue::Tuple(items) => {
                let cloned = items.iter().map(|v| v.deep_clone()).collect();
                RuntimeValue::Tuple(Rc::new(cloned))
            }
            RuntimeValue::Struct(s) => {
                let cloned_fields = s.fields.iter().map(|(k, v)| (k.clone(), v.deep_clone())).collect();
                RuntimeValue::Struct(Box::new(StructValue {
                    type_name: s.type_name.clone(),
                    fields: cloned_fields,
                }))
            }
            RuntimeValue::Inductive(ind) => {
                let cloned_args = ind.args.iter().map(|v| v.deep_clone()).collect();
                RuntimeValue::Inductive(Box::new(InductiveValue {
                    inductive_type: ind.inductive_type.clone(),
                    constructor: ind.constructor.clone(),
                    args: cloned_args,
                }))
            }
            RuntimeValue::Function(f) => {
                let cloned_env = f.captured_env.iter()
                    .map(|(k, v)| (k.clone(), v.deep_clone()))
                    .collect();
                RuntimeValue::Function(Box::new(ClosureValue {
                    body_index: f.body_index,
                    captured_env: cloned_env,
                    param_names: f.param_names.clone(),
                    generated: f.generated.clone(),
                }))
            }
            // A value-copy of a CRDT is an INDEPENDENT replica — deep-copy the inner state
            // so a later mutation of one copy does not alias the other (a shallow `Rc`
            // share would make a struct copy mutate the original's CRDT field).
            RuntimeValue::Crdt(c) => {
                RuntimeValue::Crdt(Rc::new(RefCell::new(c.borrow().clone())))
            }
            other => other.clone(),
        }
    }

    /// Falsy: `false`, numeric zero (Int/Float/BigInt/Rational/Decimal/Complex/Word),
    /// `nothing`, and empty Text/List/Set/Map. Everything else is truthy.
    /// (`-0.0` is zero; NaN is nonzero and therefore truthy.)
    pub fn is_truthy(&self) -> bool {
        match self {
            RuntimeValue::Bool(b) => *b,
            RuntimeValue::Int(n) => *n != 0,
            RuntimeValue::Float(f) => *f != 0.0,
            RuntimeValue::BigInt(b) => !b.is_zero(),
            RuntimeValue::Rational(r) => !r.is_zero(),
            RuntimeValue::Decimal(d) => !d.is_zero(),
            RuntimeValue::Complex(c) => !c.is_zero(),
            RuntimeValue::Word(w) => w.to_u64() != 0,
            RuntimeValue::Text(s) => !s.is_empty(),
            RuntimeValue::List(l) => l.borrow().len() != 0,
            RuntimeValue::Set(s) => !s.borrow().is_empty(),
            RuntimeValue::Map(m) => !m.borrow().is_empty(),
            RuntimeValue::Nothing => false,
            _ => true,
        }
    }

    /// Converts this value to a human-readable string for display.
    ///
    /// Used by the `show()` built-in function and for debugging output.
    /// Formats collections with brackets, structs with field names, and
    /// inductive values with constructor notation.
    pub fn to_display_string(&self) -> String {
        match self {
            RuntimeValue::Int(n) => n.to_string(),
            RuntimeValue::Word(w) => w.to_string(),
            // A lane vector renders like a Seq of its lanes — `[l0, l1, ...]` of unsigned values.
            RuntimeValue::Lanes(v) => {
                let parts: Vec<String> =
                    (0..v.lanes()).map(|i| v.lane(i).to_string()).collect();
                format!("[{}]", parts.join(", "))
            }
            RuntimeValue::BigInt(b) => b.to_string(),
            RuntimeValue::Rational(r) => r.to_string(),
            RuntimeValue::Decimal(d) => d.to_string(),
            RuntimeValue::Complex(c) => c.to_string(),
            RuntimeValue::Modular(m) => m.to_string(),
            RuntimeValue::Quantity(qv) => qv.display(),
            RuntimeValue::Money(m) => m.to_string(),
            RuntimeValue::Uuid(u) => u.to_string(),
            RuntimeValue::Float(f) => logicaffeine_data::fmt::fmt_f64(*f),
            RuntimeValue::Bool(b) => if *b { "true" } else { "false" }.to_string(),
            RuntimeValue::Text(s) => s.as_str().to_string(),
            RuntimeValue::Char(c) => c.to_string(),
            RuntimeValue::List(items) => {
                let items = items.borrow();
                let parts: Vec<String> =
                    items.to_values().iter().map(|v| v.to_display_string()).collect();
                format!("[{}]", parts.join(", "))
            }
            RuntimeValue::Tuple(items) => {
                let parts: Vec<String> = items.iter().map(|v| v.to_display_string()).collect();
                format!("({})", parts.join(", "))
            }
            RuntimeValue::Set(items) => {
                let items = items.borrow();
                let parts: Vec<String> = items.iter().map(|v| v.to_display_string()).collect();
                format!("{{{}}}", parts.join(", "))
            }
            RuntimeValue::Map(m) => {
                let m = m.borrow();
                let pairs: Vec<String> = m.iter()
                    .map(|(k, v)| format!("{}: {}", k.to_display_string(), v.to_display_string()))
                    .collect();
                format!("{{{}}}", pairs.join(", "))
            }
            RuntimeValue::Struct(s) => {
                if s.fields.is_empty() {
                    s.type_name.clone()
                } else {
                    // `fields` is a `HashMap` (random iteration order), so sort by field NAME to make
                    // the display DETERMINISTIC — otherwise `TypeName { … }` order varies per run and
                    // can't be a byte-identical target for the VM/AOT tiers.
                    let mut field_strs: Vec<(&str, String)> = s
                        .fields
                        .iter()
                        .map(|(k, v)| (k.as_str(), v.to_display_string()))
                        .collect();
                    field_strs.sort_by(|a, b| a.0.cmp(b.0));
                    let joined: Vec<String> = field_strs.iter().map(|(k, v)| format!("{k}: {v}")).collect();
                    format!("{} {{ {} }}", s.type_name, joined.join(", "))
                }
            }
            RuntimeValue::Inductive(ind) => {
                if ind.args.is_empty() {
                    ind.constructor.clone()
                } else {
                    let arg_strs: Vec<String> = ind.args
                        .iter()
                        .map(|v| v.to_display_string())
                        .collect();
                    format!("{}({})", ind.constructor, arg_strs.join(", "))
                }
            }
            RuntimeValue::Function(_) => "<closure>".to_string(),
            RuntimeValue::Chan(_) => "<channel>".to_string(),
            RuntimeValue::TaskHandle(_) => "<task>".to_string(),
            RuntimeValue::Peer(topic) => format!("<peer {topic}>"),
            RuntimeValue::Crdt(c) => c.borrow().render(),
            RuntimeValue::Nothing => "nothing".to_string(),
            RuntimeValue::Duration(nanos) => {
                // Format durations nicely based on magnitude
                let abs_nanos = nanos.unsigned_abs();
                let sign = if *nanos < 0 { "-" } else { "" };
                if abs_nanos >= 3_600_000_000_000 {
                    // Hours
                    format!("{}{}h", sign, abs_nanos / 3_600_000_000_000)
                } else if abs_nanos >= 60_000_000_000 {
                    // Minutes
                    format!("{}{}min", sign, abs_nanos / 60_000_000_000)
                } else if abs_nanos >= 1_000_000_000 {
                    // Seconds
                    format!("{}{}s", sign, abs_nanos / 1_000_000_000)
                } else if abs_nanos >= 1_000_000 {
                    // Milliseconds
                    format!("{}{}ms", sign, abs_nanos / 1_000_000)
                } else if abs_nanos >= 1_000 {
                    // Microseconds
                    format!("{}{}μs", sign, abs_nanos / 1_000)
                } else {
                    // Nanoseconds
                    format!("{}{}ns", sign, abs_nanos)
                }
            }
            RuntimeValue::Date(days) => {
                // Convert days since epoch to YYYY-MM-DD format
                // Using Howard Hinnant's algorithm
                let z = *days as i64 + 719468; // shift epoch
                let era = if z >= 0 { z } else { z - 146096 } / 146097;
                let doe = z - era * 146097;
                let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
                let y = yoe + era * 400;
                let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
                let mp = (5 * doy + 2) / 153;
                let d = doy - (153 * mp + 2) / 5 + 1;
                let m = mp + if mp < 10 { 3 } else { -9 };
                let year = y + if m <= 2 { 1 } else { 0 };
                format!("{:04}-{:02}-{:02}", year, m, d)
            }
            RuntimeValue::Moment(nanos) => {
                // Convert nanoseconds since epoch to ISO-8601-like datetime.
                // Use floored (Euclidean) division so a pre-epoch (negative)
                // Moment yields the correct date and a 0..86399 time-of-day,
                // not a negative hour/minute.
                let total_seconds = nanos.div_euclid(1_000_000_000);
                let days = total_seconds.div_euclid(86400) as i32;
                let day_seconds = total_seconds.rem_euclid(86400);
                let hours = day_seconds / 3600;
                let minutes = (day_seconds % 3600) / 60;

                // Convert days since epoch to YYYY-MM-DD using Howard Hinnant's algorithm
                let z = days as i64 + 719468;
                let era = if z >= 0 { z } else { z - 146096 } / 146097;
                let doe = z - era * 146097;
                let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
                let y = yoe + era * 400;
                let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
                let mp = (5 * doy + 2) / 153;
                let d = doy - (153 * mp + 2) / 5 + 1;
                let m = mp + if mp < 10 { 3 } else { -9 };
                let year = y + if m <= 2 { 1 } else { 0 };

                format!("{:04}-{:02}-{:02} {:02}:{:02}", year, m, d, hours, minutes)
            }
            RuntimeValue::Span { months, days } => {
                // Format span with years, months, and days
                let mut parts = Vec::new();

                // Extract years from months
                let years = *months / 12;
                let remaining_months = *months % 12;

                if years != 0 {
                    parts.push(if years.abs() == 1 {
                        format!("{} year", years)
                    } else {
                        format!("{} years", years)
                    });
                }

                if remaining_months != 0 {
                    parts.push(if remaining_months.abs() == 1 {
                        format!("{} month", remaining_months)
                    } else {
                        format!("{} months", remaining_months)
                    });
                }

                if *days != 0 || parts.is_empty() {
                    parts.push(if days.abs() == 1 {
                        format!("{} day", days)
                    } else {
                        format!("{} days", days)
                    });
                }

                parts.join(" and ")
            }
            RuntimeValue::Time(nanos) => {
                // The wall-clock time-of-day, lossless to the nanosecond (HH:MM:SS[.frac]).
                logicaffeine_base::temporal::format_time_of_day(*nanos)
            }
        }
    }
}

/// Control flow signals returned from statement execution.
///
/// These signals allow the interpreter to handle early exits from blocks,
/// function returns, and loop breaks without exceptions.
pub enum ControlFlow {
    /// Continue normal execution to the next statement.
    Continue,
    /// Return from the current function with a value.
    Return(RuntimeValue),
    /// Break out of the current loop.
    Break,
}

/// Stored function definition for user-defined functions.
///
/// Captures the parameter list, body statements, and optional return type
/// for later invocation when the function is called.
#[derive(Clone)]
pub struct FunctionDef<'a> {
    /// Parameter names paired with their type expressions.
    pub params: Vec<(Symbol, &'a TypeExpr<'a>)>,
    /// Statements comprising the function body.
    pub body: Block<'a>,
    /// Optional declared return type.
    pub return_type: Option<&'a TypeExpr<'a>>,
}

/// Tree-walking interpreter for LOGOS programs.
///
/// Phase 55: Now async with optional VFS for file operations.
/// Phase 102: Kernel context for inductive type support.
/// Flat environment with O(1) lookup and undo-log scoping.
/// LEXICALLY scoped environment.
///
/// Main's top-level bindings are globals, visible (and assignable) everywhere.
/// Each function call swaps in a fresh `locals` frame — a callee sees its
/// parameters, its own bindings, and the globals, but NEVER its caller's
/// locals. Block scopes (If/While/Repeat bodies, Zone, Inspect arms) are
/// undo-logged: `define`s inside a block are reverted when it ends, while
/// `assign`s persist (mutation is not binding).
struct Environment {
    /// Main TOP-LEVEL bindings — the program's globals, visible everywhere.
    globals: HashMap<Symbol, RuntimeValue>,
    /// Main BLOCK-scoped bindings (Let inside an If/While/Repeat at Main
    /// level). Lexically NOT visible to called functions.
    main_block: HashMap<Symbol, RuntimeValue>,
    /// The current function frame's bindings.
    locals: HashMap<Symbol, RuntimeValue>,
    save_stack: Vec<Vec<(Symbol, Option<RuntimeValue>)>>,
    // Shelved (locals, save_stack) of each caller. The save stack is shelved
    // WITH the locals so a callee's defines can never be recorded into a
    // caller's undo frame.
    frame_stack: Vec<(HashMap<Symbol, RuntimeValue>, Vec<Vec<(Symbol, Option<RuntimeValue>)>>)>,
}

impl Environment {
    fn new() -> Self {
        Environment {
            globals: HashMap::new(),
            main_block: HashMap::new(),
            locals: HashMap::new(),
            save_stack: Vec::new(),
            frame_stack: Vec::new(),
        }
    }

    fn in_function(&self) -> bool {
        !self.frame_stack.is_empty()
    }

    /// The map `define` writes to in the current context: function locals, or
    /// at Main level the block map (inside a block) vs the globals (at root).
    fn define_map(&mut self) -> &mut HashMap<Symbol, RuntimeValue> {
        if !self.frame_stack.is_empty() {
            &mut self.locals
        } else if !self.save_stack.is_empty() {
            &mut self.main_block
        } else {
            &mut self.globals
        }
    }

    /// Enter a function frame: the caller's locals and undo log are shelved;
    /// the callee starts with neither (the lexical barrier).
    fn push_frame(&mut self) {
        self.frame_stack.push((
            std::mem::take(&mut self.locals),
            std::mem::take(&mut self.save_stack),
        ));
    }

    /// Leave a function frame, restoring the caller's locals and undo log.
    fn pop_frame(&mut self) {
        let (locals, saves) = self.frame_stack.pop().unwrap_or_default();
        self.locals = locals;
        self.save_stack = saves;
    }

    fn push_scope(&mut self) {
        self.save_stack.push(Vec::new());
    }

    fn pop_scope(&mut self) {
        if let Some(saves) = self.save_stack.pop() {
            let map = if !self.frame_stack.is_empty() {
                &mut self.locals
            } else {
                // Main level: block-scoped defines live in main_block (a
                // define at Main ROOT records no undo — save_stack was empty).
                &mut self.main_block
            };
            for (sym, old_val) in saves.into_iter().rev() {
                match old_val {
                    Some(val) => { map.insert(sym, val); }
                    None => { map.remove(&sym); }
                }
            }
        }
    }

    fn define(&mut self, name: Symbol, value: RuntimeValue) {
        let map = self.define_map();
        let old = map.insert(name, value);
        if let Some(frame) = self.save_stack.last_mut() {
            frame.push((name, old));
        }
    }

    fn lookup(&self, name: Symbol) -> Option<&RuntimeValue> {
        if self.in_function() {
            self.locals.get(&name).or_else(|| self.globals.get(&name))
        } else {
            self.main_block.get(&name).or_else(|| self.globals.get(&name))
        }
    }

    fn assign(&mut self, name: Symbol, value: RuntimeValue) -> bool {
        if self.in_function() {
            if self.locals.contains_key(&name) {
                self.locals.insert(name, value);
                return true;
            }
        } else if self.main_block.contains_key(&name) {
            self.main_block.insert(name, value);
            return true;
        }
        if self.globals.contains_key(&name) {
            self.globals.insert(name, value);
            true
        } else {
            false
        }
    }
}

/// Side-table entry storing a closure body AST reference.
/// The index into the `closure_bodies` Vec on the interpreter is stored
/// in `ClosureValue::body_index`.
#[derive(Clone)]
pub enum ClosureBodyRef<'a> {
    Expression(&'a Expr<'a>),
    Block(Block<'a>),
}

/// `Send redundant` FEC parameters: split into `REDUNDANT_K` data shards plus
/// `REDUNDANT_N − REDUNDANT_K` parity shards, so a receiver reconstructs from any
/// `REDUNDANT_K` and tolerates losing up to `REDUNDANT_N − REDUNDANT_K` of the `REDUNDANT_N`
/// (here 2 of 6 — a 33% loss budget at 1.5× bandwidth).
const REDUNDANT_K: usize = 4;
const REDUNDANT_N: usize = 6;

pub struct Interpreter<'a> {
    /// Shared, mostly-immutable context — interner, function/struct tables,
    /// platform handles, pre-interned builtin symbols. Held directly for the
    /// single-task case; wrapped in `Rc<SharedCtx>` and shared across per-task
    /// `Interpreter` instances once the scheduler spawns concurrent tasks.
    ctx: SharedCtx<'a>,
    /// Per-task execution state — owned per task so the cooperative scheduler can
    /// run multiple task continuations without aliasing the interpreter.
    task: TaskState,
    /// The program's output lines. Public API consumed by `ui_bridge`.
    pub output: Vec<String>,
    /// Set when this interpreter is a scheduled concurrent task: the side-channel
    /// to the scheduler. `None` for ordinary single-task execution.
    yield_state: Option<crate::concurrency::bridge::Yield<'a>>,
    /// All peer-messaging state — the relay handle, this node's inbox topic, the received-message
    /// buffer, the wire schema caches, and the FEC shard buffer — lifted into one shared
    /// [`crate::concurrency::net_inbox::NetInbox`] so the bytecode VM's task driver owns the SAME
    /// inbox and networking runs byte-identically on both tiers (no tier silently differs).
    netbox: crate::concurrency::net_inbox::NetInbox,
}

/// The shared interpreter context: function definitions, type metadata, platform
/// handles, and pre-interned builtin symbols. Immutable-after-setup, so multiple
/// per-task [`Interpreter`]s can share one `Rc<SharedCtx>` while each owns its own
/// [`TaskState`] — the basis of the tree-walker's re-entrancy for concurrency.
#[derive(Clone)]
struct SharedCtx<'a> {
    interner: &'a Interner,
    functions: HashMap<Symbol, FunctionDef<'a>>,
    struct_defs: HashMap<Symbol, Vec<(Symbol, Symbol, bool)>>,
    /// Enum type → its constructor names in declaration order. Feeds the wire type
    /// registry so `Send shared` elides enum type/constructor names (T_INDUCTIVE_TID),
    /// the enum analog of struct name elision.
    enum_defs: HashMap<Symbol, Vec<Symbol>>,
    vfs: Option<Arc<dyn Vfs>>,
    kernel_ctx: Option<Arc<crate::kernel::Context>>,
    policy_registry: Option<PolicyRegistry>,
    output_callback: Option<OutputCallback>,
    /// Side-table for closure body AST references.
    /// Indexed by `ClosureValue::body_index`.
    closure_bodies: Vec<ClosureBodyRef<'a>>,
    // Pre-interned builtin function symbols for O(1) dispatch
    sym_show: Option<Symbol>,
    sym_length: Option<Symbol>,
    sym_format: Option<Symbol>,
    sym_parse_int: Option<Symbol>,
    sym_parse_float: Option<Symbol>,
    sym_abs: Option<Symbol>,
    sym_sqrt: Option<Symbol>,
    sym_min: Option<Symbol>,
    sym_max: Option<Symbol>,
    sym_floor: Option<Symbol>,
    sym_ceil: Option<Symbol>,
    sym_round: Option<Symbol>,
    sym_pow: Option<Symbol>,
    sym_copy: Option<Symbol>,
    sym_chr: Option<Symbol>,
    sym_count_ones: Option<Symbol>,
    sym_args: Option<Symbol>,
    /// Program arguments for the `args()` system native — full argv, index 0 is
    /// the program name (mirrors the compiled binary's `env::args()`).
    program_args: Vec<String>,
}

/// The per-task execution state the cooperative scheduler owns for each task.
/// Splitting it out of [`Interpreter`] lets multiple task continuations coexist —
/// each with its own `&mut TaskState` over one shared interpreter context — which
/// is what makes the tree-walker re-entrant for concurrency.
struct TaskState {
    /// Variable bindings / scopes for this task.
    env: Environment,
    /// Live LOGOS call depth, bounded by `semantics::MAX_CALL_DEPTH`.
    call_depth: usize,
    /// The user function whose body the SYNC path is currently executing. A
    /// `Return self(args)` (or the `Set/Let x to self(args); Return x` pair) of
    /// THIS function is a self-tail-call: `call_function_sync` reassigns the
    /// parameters and loops to the body's start instead of recursing, so tail
    /// recursion runs in constant stack — matching the VM and the AOT TCE.
    tco_fn_sync: Option<Symbol>,
    /// Set by a recognized self-tail-call: the already-evaluated arguments for
    /// the next loop iteration, consumed by `call_function_sync`.
    pending_tail_call: Option<Vec<RuntimeValue>>,
    /// `Repeat` (for-each) nesting on the SYNC path within the current function
    /// body. A `Repeat` owns a live iterator, so — exactly like the VM's
    /// `is_repeat` guard — a self-tail-call detected inside one stays an ordinary
    /// recursive call, keeping the two engines bit-identical. Reset at call boundaries.
    repeat_depth_sync: usize,
    /// `tco_fn_sync` for the ASYNC execution path; same constant-stack TCO semantics.
    tco_fn_async: Option<Symbol>,
    /// `pending_tail_call` for the ASYNC path.
    pending_tail_call_async: Option<Vec<RuntimeValue>>,
    /// `repeat_depth_sync` for the ASYNC path.
    repeat_depth_async: usize,
}

impl TaskState {
    fn new() -> Self {
        TaskState {
            env: Environment::new(),
            call_depth: 0,
            tco_fn_sync: None,
            pending_tail_call: None,
            repeat_depth_sync: 0,
            tco_fn_async: None,
            pending_tail_call_async: None,
            repeat_depth_async: 0,
        }
    }
}

impl<'a> Interpreter<'a> {
    pub fn new(interner: &'a Interner) -> Self {
        Interpreter {
            ctx: SharedCtx {
                interner,
                functions: HashMap::new(),
                struct_defs: HashMap::new(),
                enum_defs: HashMap::new(),
                vfs: None,
                kernel_ctx: None,
                policy_registry: None,
                output_callback: None,
                closure_bodies: Vec::new(),
                sym_show: interner.lookup("show"),
                sym_length: interner.lookup("length"),
                sym_format: interner.lookup("format"),
                sym_parse_int: interner.lookup("parseInt"),
                sym_parse_float: interner.lookup("parseFloat"),
                sym_abs: interner.lookup("abs"),
                sym_sqrt: interner.lookup("sqrt"),
                sym_min: interner.lookup("min"),
                sym_max: interner.lookup("max"),
                sym_floor: interner.lookup("floor"),
                sym_ceil: interner.lookup("ceil"),
                sym_round: interner.lookup("round"),
                sym_pow: interner.lookup("pow"),
                sym_copy: interner.lookup("copy"),
                sym_chr: interner.lookup("chr"),
                sym_count_ones: interner.lookup("count_ones"),
                sym_args: interner.lookup("args"),
                program_args: Vec::new(),
            },
            task: TaskState::new(),
            output: Vec::new(),
            yield_state: None,
            netbox: crate::concurrency::net_inbox::NetInbox::new(),
        }
    }

    /// Supply the program arguments read by the `args()` system native. The
    /// vector is the full argv (index 0 is the program name), matching the
    /// compiled binary's `env::args()`.
    pub fn with_program_args(mut self, args: Vec<String>) -> Self {
        self.ctx.program_args = args;
        self
    }

    /// Phase 55: Set the VFS for file operations.
    pub fn with_vfs(mut self, vfs: Arc<dyn Vfs>) -> Self {
        self.ctx.vfs = Some(vfs);
        self
    }

    /// Phase 102: Set the kernel context for inductive type support.
    ///
    /// When set, the interpreter can query the kernel for inductive types
    /// and constructors, enabling unified type system.
    pub fn with_kernel(mut self, ctx: Arc<crate::kernel::Context>) -> Self {
        self.ctx.kernel_ctx = Some(ctx);
        self
    }

    /// Set the policy registry for security checks.
    pub fn with_policies(mut self, registry: PolicyRegistry) -> Self {
        self.ctx.policy_registry = Some(registry);
        self
    }

    /// Populate struct_defs from a TypeRegistry (DiscoveryPass results).
    /// This allows the interpreter to initialize default field values for
    /// structs created with `new Point` (no explicit fields).
    pub fn with_type_registry(mut self, registry: &crate::analysis::TypeRegistry) -> Self {
        use crate::analysis::registry::{TypeDef, FieldType};
        for (name_sym, type_def) in registry.iter_types() {
            if let TypeDef::Struct { fields, .. } = type_def {
                let field_defs: Vec<(Symbol, Symbol, bool)> = fields.iter().map(|f| {
                    let type_sym = match &f.ty {
                        FieldType::Primitive(s) | FieldType::Named(s) | FieldType::TypeParam(s) => *s,
                        FieldType::Generic { base, .. } => *base,
                    };
                    (f.name, type_sym, f.is_public)
                }).collect();
                self.ctx.struct_defs.insert(*name_sym, field_defs);
            } else if let TypeDef::Enum { variants, .. } = type_def {
                // Constructor names in declaration order — the order is the wire's ctor
                // index, so both peers (deriving from the same program) agree.
                let ctors: Vec<Symbol> = variants.iter().map(|v| v.name).collect();
                self.ctx.enum_defs.insert(*name_sym, ctors);
            }
        }
        self
    }

    /// Set a callback for streaming output.
    /// The callback is called each time `Show` executes, with the output line.
    pub fn with_output_callback(mut self, callback: OutputCallback) -> Self {
        self.ctx.output_callback = Some(callback);
        self
    }

    /// Install the scheduler side-channel, marking this interpreter as a scheduled
    /// concurrent task (used by the scheduler-driven run path).
    pub(crate) fn install_yield_state(&mut self, ys: crate::concurrency::bridge::Yield<'a>) {
        self.yield_state = Some(ys);
    }

    /// Internal helper to emit output (calls callback if set, always adds to output vec)
    fn emit_output(&mut self, line: String) {
        if let Some(ref callback) = self.ctx.output_callback {
            (callback.borrow_mut())(line.clone());
        }
        self.output.push(line);
    }

    /// Phase 102: Check if a name is a kernel inductive type.
    pub fn is_kernel_inductive(&self, name: &str) -> bool {
        self.ctx.kernel_ctx
            .as_ref()
            .map(|ctx| ctx.is_inductive(name))
            .unwrap_or(false)
    }

    /// Phase 102: Get constructors for a kernel inductive type.
    ///
    /// Returns a vector of (constructor_name, arity) pairs.
    pub fn get_kernel_constructors(&self, name: &str) -> Vec<(String, usize)> {
        self.ctx.kernel_ctx
            .as_ref()
            .map(|ctx| {
                ctx.get_constructors(name)
                    .iter()
                    .map(|(ctor_name, ty)| {
                        // Count Pi types to determine arity
                        let arity = count_pi_args(ty);
                        (ctor_name.to_string(), arity)
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Execute a program (list of statements).
    /// Phase 55: Now async for VFS operations.
    pub async fn run(&mut self, stmts: &[Stmt<'a>]) -> Result<(), String> {
        // A program begins with no ambient exchange rates in scope — the same clean slate an
        // AOT-compiled binary gets from a fresh process. Conversion reads what the program installs.
        logicaffeine_base::money::clear_ambient_rates();
        for stmt in stmts {
            match self.execute_stmt(stmt).await? {
                ControlFlow::Return(_) => break,
                ControlFlow::Break => break,
                ControlFlow::Continue => {}
            }
        }
        Ok(())
    }

    /// Activate the PNP one-time-pad session for a `Connect`/`Listen` `with pad "<path>" as <role>`
    /// clause: read the pad file, build the quality-gated pool, and install the directional session on
    /// the channel so every subsequent `Send`/receive on this thread is sealed. Fail-closed on any
    /// error (unreadable or non-random pad) — the caller propagates it as a program error, never
    /// proceeding to send plaintext.
    #[cfg(not(target_arch = "wasm32"))]
    async fn activate_pnp_session(&mut self, bind: &crate::ast::SecurePad<'a>) -> Result<(), String> {
        let path = self.evaluate_expr(bind.pad).await?.to_display_string();
        let bytes = std::fs::read(&path)
            .map_err(|e| format!("one-time pad '{path}' could not be read: {e}"))?;
        let pool = crate::concurrency::pnp::PadPool::shared(bytes)
            .map_err(|e| format!("one-time pad '{path}' rejected (not truly random / too small): {e:?}"))?;
        let role = match bind.role {
            crate::ast::SecureRole::Initiator => crate::concurrency::pnp::Role::Initiator,
            crate::ast::SecureRole::Responder => crate::concurrency::pnp::Role::Responder,
        };
        let session: std::rc::Rc<dyn crate::concurrency::channel::ActiveSession> =
            std::rc::Rc::new(pool.session(role));
        crate::concurrency::channel::install_session(Some(session));
        Ok(())
    }

    /// On wasm the pad is provisioned through the VFS handle rather than host files (a future wiring);
    /// the clause is accepted but installs no session, matching the offline single-node tiers.
    #[cfg(target_arch = "wasm32")]
    async fn activate_pnp_session(&mut self, _bind: &crate::ast::SecurePad<'a>) -> Result<(), String> {
        Ok(())
    }

    /// Execute a single statement.
    /// Phase 55: Now async for VFS operations.
    #[async_recursion(?Send)]
    async fn execute_stmt(&mut self, stmt: &Stmt<'a>) -> Result<ControlFlow, String> {
        match stmt {
            Stmt::Let { var, value, .. } => {
                let val = self.evaluate_expr(value).await?;
                self.define(*var, val);
                Ok(ControlFlow::Continue)
            }

            Stmt::Set { target, value } => {
                let val = self.evaluate_expr(value).await?;
                self.assign(*target, val)?;
                Ok(ControlFlow::Continue)
            }

            Stmt::Call { function, args } => {
                self.call_function(*function, args).await?;
                Ok(ControlFlow::Continue)
            }

            Stmt::If { cond, then_block, else_block } => {
                let condition = self.evaluate_expr(cond).await?;
                if condition.is_truthy() {
                    let flow = self.execute_block(then_block).await?;
                    if !matches!(flow, ControlFlow::Continue) {
                        return Ok(flow);
                    }
                } else if let Some(else_stmts) = else_block {
                    let flow = self.execute_block(else_stmts).await?;
                    if !matches!(flow, ControlFlow::Continue) {
                        return Ok(flow);
                    }
                }
                Ok(ControlFlow::Continue)
            }

            Stmt::While { cond, body, .. } => {
                loop {
                    let condition = self.evaluate_expr(cond).await?;
                    if !condition.is_truthy() {
                        break;
                    }
                    match self.execute_block(body).await? {
                        ControlFlow::Break => break,
                        ControlFlow::Return(v) => return Ok(ControlFlow::Return(v)),
                        ControlFlow::Continue => {}
                    }
                }
                Ok(ControlFlow::Continue)
            }

            Stmt::Repeat { pattern, iterable, body } => {
                use crate::ast::stmt::Pattern;

                let iter_val = self.evaluate_expr(iterable).await?;
                let items = crate::semantics::collections::iteration_snapshot(&iter_val)?;

                self.push_scope();
                // Suppress TCO inside a `Repeat` (live iterator) — see SYNC twin.
                self.task.repeat_depth_async += 1;
                for item in items {
                    // Bind variables according to pattern
                    match pattern {
                        Pattern::Identifier(sym) => {
                            self.define(*sym, item);
                        }
                        Pattern::Tuple(syms) => {
                            if let RuntimeValue::Tuple(ref tuple_vals) = item {
                                if syms.len() != tuple_vals.len() {
                                    self.task.repeat_depth_async -= 1;
                                    return Err(format!(
                                        "Cannot bind a {}-tuple to {} names",
                                        tuple_vals.len(),
                                        syms.len()
                                    ));
                                }
                                for (sym, val) in syms.iter().zip(tuple_vals.iter()) {
                                    self.define(*sym, val.clone());
                                }
                            } else {
                                self.task.repeat_depth_async -= 1;
                                return Err(format!("Expected tuple for pattern, got {}", item.type_name()));
                            }
                        }
                    }

                    match self.execute_block(body).await? {
                        ControlFlow::Break => break,
                        ControlFlow::Return(v) => {
                            self.task.repeat_depth_async -= 1;
                            self.pop_scope();
                            return Ok(ControlFlow::Return(v));
                        }
                        ControlFlow::Continue => {}
                    }
                }
                self.task.repeat_depth_async -= 1;
                self.pop_scope();
                Ok(ControlFlow::Continue)
            }

            Stmt::Return { value } => {
                // Direct self-tail-call → loop-back in `call_function` (see the
                // SYNC twin for the full rationale).
                if let Some(expr) = value {
                    if let Some(call_args) = self.self_tail_call_args_async(*expr) {
                        let mut vals = Vec::with_capacity(call_args.len());
                        for a in call_args {
                            vals.push(self.evaluate_expr(a).await?);
                        }
                        self.task.pending_tail_call_async = Some(vals);
                        return Ok(ControlFlow::Return(RuntimeValue::Nothing));
                    }
                }
                let ret_val = match value {
                    Some(expr) => self.evaluate_expr(expr).await?,
                    None => RuntimeValue::Nothing,
                };
                Ok(ControlFlow::Return(ret_val))
            }

            Stmt::Break => Ok(ControlFlow::Break),

            Stmt::FunctionDef { name, params, body, return_type, .. } => {
                let func = FunctionDef {
                    params: params.clone(),
                    body: *body,
                    return_type: *return_type,
                };
                self.ctx.functions.insert(*name, func);
                Ok(ControlFlow::Continue)
            }

            Stmt::StructDef { name, fields, .. } => {
                self.ctx.struct_defs.insert(*name, fields.clone());
                Ok(ControlFlow::Continue)
            }

            Stmt::SetField { object, field, value } => {
                let new_val = self.evaluate_expr(value).await?;
                if let Expr::Identifier(obj_sym) = object {
                    let mut obj_val = self.lookup(*obj_sym)?.clone();
                    if let RuntimeValue::Struct(ref mut s) = obj_val {
                        let field_name = self.ctx.interner.resolve(*field).to_string();
                        s.fields.insert(field_name, new_val);
                        self.assign(*obj_sym, obj_val)?;
                    } else {
                        return Err(format!("Cannot set field on non-struct value"));
                    }
                } else {
                    return Err("SetField target must be an identifier".to_string());
                }
                Ok(ControlFlow::Continue)
            }

            Stmt::Push { value, collection } => {
                let val = self.evaluate_expr(value).await?;
                if let Expr::Identifier(coll_sym) = collection {
                    self.ensure_collection_owned(*coll_sym);
                    let coll_val = self.lookup(*coll_sym)?;
                    crate::semantics::collections::list_push(&coll_val, val)?;
                } else if let Expr::FieldAccess { object, field } = collection {
                    if let Expr::Identifier(obj_sym) = *object {
                        let obj_val = self.lookup(*obj_sym)?;
                        let field_name = self.ctx.interner.resolve(*field);
                        crate::semantics::collections::push_to_struct_field(&obj_val, field_name, val)?;
                    } else {
                        return Err("Push to nested field access not supported".to_string());
                    }
                } else {
                    // Any place expression is an l-value: `Push 5 to item i of
                    // grid`. Collections are shared handles, so pushing through
                    // the evaluated handle mutates in place — the same aliasing
                    // model as `Add`/`Remove` below.
                    let coll_val = self.evaluate_expr(collection).await?;
                    crate::semantics::collections::list_push(&coll_val, val)?;
                }
                Ok(ControlFlow::Continue)
            }

            Stmt::Pop { collection, into } => {
                if let Expr::Identifier(coll_sym) = collection {
                    self.ensure_collection_owned(*coll_sym);
                    let coll_val = self.lookup(*coll_sym)?;
                    let popped = crate::semantics::collections::list_pop(&coll_val)?;
                    if let Some(into_var) = into {
                        self.define(*into_var, popped);
                    }
                } else {
                    return Err("Pop collection must be an identifier".to_string());
                }
                Ok(ControlFlow::Continue)
            }

            Stmt::Add { value, collection } => {
                let val = self.evaluate_expr(value).await?;
                if let Expr::Identifier(coll_sym) = collection {
                    self.ensure_collection_owned(*coll_sym);
                }
                // The collection may be a bare variable OR a (CRDT/Set) struct field —
                // `Add "Alice" to p's guests`. Field reads are shallow `Rc` clones, so
                // mutating the resolved collection updates the value stored in the struct.
                let coll_val = self.evaluate_expr(collection).await?;
                crate::semantics::collections::set_add(&coll_val, val)?;
                Ok(ControlFlow::Continue)
            }

            Stmt::Remove { value, collection } => {
                let val = self.evaluate_expr(value).await?;
                if let Expr::Identifier(coll_sym) = collection {
                    self.ensure_collection_owned(*coll_sym);
                }
                let coll_val = self.evaluate_expr(collection).await?;
                crate::semantics::collections::remove_from(&coll_val, &val)?;
                Ok(ControlFlow::Continue)
            }

            Stmt::SetIndex { collection, index, value } => {
                let idx_val = self.evaluate_expr(index).await?;
                let new_val = self.evaluate_expr(value).await?;
                if let Expr::Identifier(coll_sym) = collection {
                    // Struct field set via index syntax: `Set item "field" of structVar to v`.
                    // Mirrors the read side (`item "field" of struct`) so struct-field
                    // mutation round-trips through the decompiler's CMapSet rendering.
                    if let RuntimeValue::Text(field) = &idx_val {
                        let cur = self.lookup(*coll_sym)?.clone();
                        if let RuntimeValue::Struct(mut s) = cur {
                            s.fields.insert(field.to_string(), new_val);
                            self.assign(*coll_sym, RuntimeValue::Struct(s))?;
                            return Ok(ControlFlow::Continue);
                        }
                    }
                    self.ensure_collection_owned(*coll_sym);
                    let coll_val = self.lookup(*coll_sym)?;
                    crate::semantics::collections::index_set(&coll_val, &idx_val, new_val)?;
                } else {
                    // Any place expression is an l-value: `Set item j of
                    // (item i of grid) to v` writes the inner collection
                    // through its shared handle.
                    let coll_val = self.evaluate_expr(collection).await?;
                    crate::semantics::collections::index_set(&coll_val, &idx_val, new_val)?;
                }
                Ok(ControlFlow::Continue)
            }

            Stmt::Splice { body } => {
                // Scope-TRANSPARENT by contract (see the AST doc): no block
                // scoping — the gensym'd desugar temporaries live in the
                // enclosing scope.
                for s in body.iter() {
                    let flow = self.execute_stmt(s).await?;
                    if !matches!(flow, ControlFlow::Continue) {
                        return Ok(flow);
                    }
                }
                Ok(ControlFlow::Continue)
            }

            Stmt::Inspect { target, arms, .. } => {
                let target_val = self.evaluate_expr(target).await?;
                self.execute_inspect(&target_val, arms).await
            }

            Stmt::Zone { name, body, .. } => {
                self.push_scope();
                self.define(*name, RuntimeValue::Nothing);
                let result = self.execute_block(body).await;
                self.pop_scope();
                result?;
                Ok(ControlFlow::Continue)
            }

            Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => {
                // In WASM, execute sequentially (no threads)
                for task in tasks.iter() {
                    self.execute_stmt(task).await?;
                }
                Ok(ControlFlow::Continue)
            }

            Stmt::Assert { .. } | Stmt::Trust { .. } => {
                Ok(ControlFlow::Continue)
            }

            Stmt::RuntimeAssert { condition, .. } => {
                let val = self.evaluate_expr(condition).await?;
                if !val.is_truthy() {
                    return Err("Assertion failed".to_string());
                }
                Ok(ControlFlow::Continue)
            }

            Stmt::Give { object, recipient } => {
                let obj_val = self.evaluate_expr(object).await?;
                if let Expr::Identifier(sym) = recipient {
                    self.call_function_with_values(*sym, vec![obj_val]).await?;
                }
                Ok(ControlFlow::Continue)
            }

            Stmt::Show { object, recipient } => {
                let obj_val = self.evaluate_expr(object).await?;
                if let Expr::Identifier(sym) = recipient {
                    let name = self.ctx.interner.resolve(*sym);
                    if name == "show" {
                        self.emit_output(obj_val.to_display_string());
                    } else {
                        self.call_function_with_values(*sym, vec![obj_val]).await?;
                    }
                }
                Ok(ControlFlow::Continue)
            }

            // Phase 55: VFS operations now supported
            Stmt::ReadFrom { var, source } => {
                let content = match source {
                    ReadSource::Console => {
                        // Console read not available in WASM interpreter
                        String::new()
                    }
                    ReadSource::File(path_expr) => {
                        let path = self.evaluate_expr(path_expr).await?.to_display_string();
                        match &self.ctx.vfs {
                            Some(vfs) => {
                                vfs.read_to_string(&path).await
                                    .map_err(|e| format!("Read error: {}", e))?
                            }
                            None => return Err("VFS not initialized. Use Interpreter::with_vfs()".to_string()),
                        }
                    }
                };
                self.define(*var, RuntimeValue::Text(Rc::new(content)));
                Ok(ControlFlow::Continue)
            }

            Stmt::WriteFile { content, path } => {
                let content_val = self.evaluate_expr(content).await?.to_display_string();
                let path_val = self.evaluate_expr(path).await?.to_display_string();
                match &self.ctx.vfs {
                    Some(vfs) => {
                        vfs.write(&path_val, content_val.as_bytes()).await
                            .map_err(|e| format!("Write error: {}", e))?;
                    }
                    None => return Err("VFS not initialized. Use Interpreter::with_vfs()".to_string()),
                }
                Ok(ControlFlow::Continue)
            }

            Stmt::Spawn { name, .. } => {
                self.define(*name, RuntimeValue::Nothing);
                Ok(ControlFlow::Continue)
            }

            // `Send <message> to <peer>` — publish the message on the peer's inbox
            // topic over the relay, tagged with our own inbox as the sender so the
            // recipient's `Await … from us` can match it.
            Stmt::SendMessage { message, destination, compression, cached, unchecked, layout, shared, computed, indexed, deduped } => {
                use crate::concurrency::marshal::{
                    default_integrity, message_to_wire_best, message_to_wire_cached, message_to_wire_with,
                    with_compression_codec, with_dedup, with_integrity, with_numerics, with_structure,
                    with_struct_view, with_type_registry, WireCodec, WireGoal, WireIntegrity, WireNumerics,
                    WireSchemaCache, WireStructure,
                };
                use logicaffeine_language::ast::SendLayout;
                let dest = self.evaluate_expr(destination).await?;
                let topic = Self::peer_topic_of(&dest)?;
                let msg = self.evaluate_expr(message).await?;
                // `Send computed f` — COMPUTE-SHIPPING: lower a pure single-argument function
                // into the sandboxed generator so the COMPUTATION crosses the wire (as a
                // callable the receiver evaluates in its bounded sandbox), not data. The
                // lowering is the safety gate: only a total arithmetic expression over the
                // one argument lowers; anything else (I/O, calls, a block, >1 param) is
                // refused here, never shipped.
                let msg = if *computed {
                    match msg {
                        RuntimeValue::Function(c) if c.generated.is_some() => RuntimeValue::Function(c),
                        RuntimeValue::Function(c) => {
                            if c.param_names.len() != 1 {
                                return Err(
                                    "Send computed requires a single-argument pure function".to_string()
                                );
                            }
                            let expr = match self.ctx.closure_bodies.get(c.body_index) {
                                Some(ClosureBodyRef::Expression(e)) => *e,
                                _ => {
                                    return Err(
                                        "Send computed requires a pure expression-bodied function".to_string()
                                    )
                                }
                            };
                            match crate::concurrency::marshal::lower_expr_to_genexpr(expr, c.param_names[0]) {
                                Some(gen) => RuntimeValue::Function(Box::new(ClosureValue {
                                    body_index: usize::MAX,
                                    captured_env: HashMap::default(),
                                    param_names: c.param_names.clone(),
                                    generated: Some(std::rc::Rc::new(gen)),
                                })),
                                None => {
                                    return Err(
                                        "Send computed: the function is not a pure arithmetic computation over its argument"
                                            .to_string(),
                                    )
                                }
                            }
                        }
                        _ => return Err("Send computed requires a function value".to_string()),
                    }
                } else {
                    msg
                };
                // Owned so the schema-cache borrow below doesn't alias the inbox topic.
                let from = self.netbox.inbox.as_ref().map(|t| t.to_string()).unwrap_or_default();
                // Advertise our type-registry epoch (set before the first-contact handshake below), so a
                // same-program peer with a matching epoch negotiates type-id name elision.
                self.netbox.my_profile.registry_epoch = self.build_wire_type_registry().epoch();
                // The message is any language value, encoded faithfully for the wire. The
                // sender's modifiers are the knobs for their link: `fast|compact|packed`
                // is the size↔speed LAYOUT (fixed-memcpy / varint / group-varint);
                // `compressed [with <codec>]` shrinks the body (kept only if it helps);
                // `cached` references a once-sent struct schema by id; `unchecked` drops
                // the integrity checksum (latency↔safety). The wire is self-describing,
                // so any peer decodes it regardless of which knobs the sender turned.
                let integrity = if *unchecked { WireIntegrity::Raw } else { default_integrity() };
                let numerics = match layout {
                    Some(SendLayout::Fast) => WireNumerics::Fixed,
                    Some(SendLayout::Packed) => WireNumerics::GroupVarint,
                    // `smallest`/`best` lets the per-column menu pick each column's own
                    // form, so the numeric dial stays the varint baseline it builds on.
                    Some(SendLayout::Smallest) => WireNumerics::Varint,
                    // `redundant` adds FEC framing OVER the default encoding.
                    Some(SendLayout::Redundant) => WireNumerics::Varint,
                    Some(SendLayout::Compact) | None => WireNumerics::Varint,
                };
                // `smallest` turns on the per-column compression menu (delta / DoD /
                // frame-of-reference / RLE / dictionary, auto-selected, never worse than
                // varint); every other layout leaves structural analysis off.
                let structure = match layout {
                    Some(SendLayout::Smallest) => WireStructure::Auto,
                    _ => WireStructure::Off,
                };
                // `shared` opts into type-id elision: install the program's type registry
                // so structs/enums ship a small id instead of their NAMES. OFF by default
                // (an empty registry → byte-identical self-describing encode) because a
                // relay or a different-program peer would not share the type ids.
                let registry = if *shared {
                    self.build_wire_type_registry()
                } else {
                    crate::concurrency::marshal::WireTypeRegistry::new(Vec::new())
                };
                // `indexed`/`addressable` opts the record list into the random-access struct-view
                // LAYOUT (row + field offset tables) so the receiver reaches any (row, field) in
                // O(1). It wraps EVERY path below, so it composes with `compressed`, `cached`,
                // `shared`, and `unchecked`. OFF by default (the dense columnar form is smaller).
                // A PLAIN send (no explicit codec knob) routes through the SHARED negotiated encoder
                // both tiers call, so the tree-walker and the VM net path stay byte-identical (the
                // cross-tier lock holds). Any explicit knob — `fast`/`packed`/`smallest`/`cached`/
                // `shared`/`indexed`/`compressed`/`computed`/`unchecked`/`redundant` — takes its own
                // override path below.
                let plain = !*computed
                    && !*unchecked
                    && !*shared
                    && !*indexed
                    && !*cached
                    && !*deduped
                    && compression.is_none()
                    && matches!(layout, None | Some(SendLayout::Compact));
                let bytes = if plain {
                    // Pass the program's registry; type-id fires only when the peer's epoch matched
                    // (negotiated), so a raw / different-program peer still gets the plain encoding.
                    self.netbox.encode_negotiated(&from, &msg, &topic, self.build_wire_type_registry())?
                } else {
                    // `deduped` wraps the WHOLE encode: a subtree the same value reaches more than once
                    // ships once + backrefs, and the receiver rebuilds the sharing. A no-op when the
                    // knob is off, so the path below is byte-identical without it.
                    with_dedup(*deduped, || with_struct_view(*indexed, || with_type_registry(registry, || -> Result<Vec<u8>, String> {
                    // `smallest`/`best`: the message-level auto-tuner measures every dial
                    // combination and ships the PROVABLY smallest encoding (never larger than
                    // any single knob). It subsumes the numerics/structure/compression knobs,
                    // and composes with `shared` (the registry is active here) and `redundant`
                    // (FEC shards the result below). The cross-message schema cache (`cached`)
                    // is a separate optimization and keeps its own path. `deduped` is excluded here:
                    // the auto-tuner runs MANY encode passes, but the dedup id-table is per-encode, so
                    // dedup takes the single-pass general path below instead.
                    if matches!(layout, Some(SendLayout::Smallest)) && !*cached && !*deduped {
                        return with_integrity(integrity, || {
                            message_to_wire_best(&from, &msg, WireGoal::Smallest)
                        });
                    }
                    if *cached {
                        let cache = self
                            .netbox
                            .send_schema
                            .entry(topic.clone())
                            .or_insert_with(WireSchemaCache::content_addressed);
                        let mut encode = || message_to_wire_cached(&from, &msg, WireCodec::Native, integrity, cache);
                        with_structure(structure, || with_numerics(numerics, || match compression {
                            Some(codec) => with_compression_codec(wire_compression_of(*codec), &mut encode),
                            None => encode(),
                        }))
                    } else {
                        let mut encode = || message_to_wire_with(&from, &msg, WireCodec::Native, integrity);
                        with_structure(structure, || with_numerics(numerics, || match compression {
                            Some(codec) => with_compression_codec(wire_compression_of(*codec), &mut encode),
                            None => encode(),
                        }))
                    }
                })))?
                };
                // Seal the encoded message under the active crypto session/suite — identity /
                // byte-identical when none is engaged — before FEC/publish (compress → encrypt → FEC).
                // A keyed session may fail closed (a one-time pad is exhausted): refuse the send rather
                // than transmit plaintext.
                let bytes = crate::concurrency::channel::seal_active_checked(bytes)
                    .ok_or_else(|| "one-time pad exhausted — message not sent (PNP fail-closed)".to_string())?;
                // Advertise our surface to this peer on FIRST contact — on its HANDSHAKE topic, never
                // the data topic — so it can negotiate back. Absorbed by the peer's drain.
                if let Some(hs) = self.netbox.first_contact_handshake(&topic) {
                    self.netbox
                        .publish(&crate::concurrency::net_inbox::handshake_topic_for(&topic), hs)?;
                }
                if matches!(layout, Some(SendLayout::Redundant)) {
                    // FEC: split the encoded message into K data + (N−K) parity shards and
                    // publish each as its own packet, so a receiver reconstructs the exact
                    // message from any K even if some are lost on the link.
                    let msg_id = self.netbox.next_msg_id();
                    let shards = crate::concurrency::fec::frame_redundant(
                        msg_id, &bytes, REDUNDANT_K, REDUNDANT_N,
                    )
                    .ok_or_else(|| "redundant framing failed".to_string())?;
                    for shard in shards {
                        self.netbox.publish(&topic, shard)?;
                    }
                } else {
                    self.netbox.publish(&topic, bytes)?;
                }
                Ok(ControlFlow::Continue)
            }

            // `Await … from <peer> into x` — block (cooperatively) until a message
            // from that peer arrives on our inbox, then bind it to `x`. Messages
            // from other peers stay queued for their own `Await`.
            Stmt::AwaitMessage { source, into, view, stream } => {
                let src = self.evaluate_expr(source).await?;
                let want = Self::peer_topic_of(&src)?;
                if self.netbox.inbox.is_none() {
                    return Err("Await requires a prior Listen to establish an inbox".to_string());
                }
                // OFFLINE (no relay): the deterministic oracle reads from our own loopback outbox — a
                // `Send … to <self>` already delivered the message locally, so `Await` resolves it (no
                // real transport needed). A relay-connected node drains the relay instead.
                let msg = if *stream {
                    self.await_stream_from(&want).await?
                } else {
                    self.await_message_from(&want, *view).await?
                };
                self.define(*into, msg);
                Ok(ControlFlow::Continue)
            }

            // `Stream <values> to <peer>` — batch the list into one framed stream message and
            // publish it to the peer's inbox; `Await stream from us` deframes it back into a list.
            Stmt::StreamMessage { values, destination } => {
                let dest = self.evaluate_expr(destination).await?;
                let topic = Self::peer_topic_of(&dest)?;
                let list = self.evaluate_expr(values).await?;
                let items = match &list {
                    RuntimeValue::List(rc) => rc.borrow().to_values(),
                    other => vec![other.clone()],
                };
                let from = self.netbox.inbox.as_ref().map(|t| t.to_string()).unwrap_or_default();
                let registry = self.build_wire_type_registry();
                let blob = crate::concurrency::marshal::with_type_registry(registry, || {
                    crate::concurrency::marshal::frame_stream_message(&from, &items)
                })?;
                // Route through `publish`, which OFFLINE loops the framed stream back into our own inbox
                // (a following `Await stream` deframes it) rather than requiring a relay.
                self.netbox.publish(&topic, blob)?;
                Ok(ControlFlow::Continue)
            }

            Stmt::MergeCrdt { source, target } => {
                let source_val = self.evaluate_expr(source).await?;
                let source_fields = match &source_val {
                    RuntimeValue::Struct(s) => s.fields.clone(),
                    _ => return Err("Merge source must be a struct".to_string()),
                };

                if let Expr::Identifier(target_sym) = target {
                    let mut target_val = self.lookup(*target_sym)?.clone();

                    if let RuntimeValue::Struct(ref mut s) = target_val {
                        for (field_name, source_field_val) in source_fields {
                            let current = s.fields.get(&field_name)
                                .cloned()
                                .unwrap_or(RuntimeValue::Int(0));

                            let merged =
                                crate::semantics::arith::crdt_merge_field(&current, source_field_val);
                            s.fields.insert(field_name, merged);
                        }
                        self.assign(*target_sym, target_val)?;
                    } else {
                        return Err("Merge target must be a struct".to_string());
                    }
                } else {
                    return Err("Merge target must be an identifier".to_string());
                }
                Ok(ControlFlow::Continue)
            }

            Stmt::IncreaseCrdt { object, field, amount } => {
                let amount_val = self.evaluate_expr(amount).await?;
                let amount_int = match amount_val {
                    RuntimeValue::Int(n) => n,
                    _ => return Err("CRDT increment amount must be an integer".to_string()),
                };

                if let Expr::Identifier(obj_sym) = object {
                    let mut obj_val = self.lookup(*obj_sym)?.clone();

                    if let RuntimeValue::Struct(ref mut s) = obj_val {
                        let field_name = self.ctx.interner.resolve(*field).to_string();
                        let current = s.fields.get(&field_name)
                            .cloned()
                            .unwrap_or(RuntimeValue::Int(0));

                        let new_val =
                            crate::semantics::arith::crdt_counter_bump(current, amount_int, &field_name)?;
                        s.fields.insert(field_name, new_val);
                        self.assign(*obj_sym, obj_val)?;
                    } else {
                        return Err("Cannot increase field on non-struct value".to_string());
                    }
                } else {
                    return Err("IncreaseCrdt target must be an identifier".to_string());
                }
                Ok(ControlFlow::Continue)
            }

            Stmt::DecreaseCrdt { object, field, amount } => {
                let amount_val = self.evaluate_expr(amount).await?;
                let amount_int = match amount_val {
                    RuntimeValue::Int(n) => n,
                    _ => return Err("CRDT decrement amount must be an integer".to_string()),
                };

                if let Expr::Identifier(obj_sym) = object {
                    let mut obj_val = self.lookup(*obj_sym)?.clone();

                    if let RuntimeValue::Struct(ref mut s) = obj_val {
                        let field_name = self.ctx.interner.resolve(*field).to_string();
                        let current = s.fields.get(&field_name)
                            .cloned()
                            .unwrap_or(RuntimeValue::Int(0));

                        let new_val = crate::semantics::arith::crdt_counter_bump(
                            current,
                            amount_int.wrapping_neg(),
                            &field_name,
                        )?;
                        s.fields.insert(field_name, new_val);
                        self.assign(*obj_sym, obj_val)?;
                    } else {
                        return Err("Cannot decrease field on non-struct value".to_string());
                    }
                } else {
                    return Err("DecreaseCrdt target must be an identifier".to_string());
                }
                Ok(ControlFlow::Continue)
            }

            Stmt::AppendToSequence { sequence, value } => {
                let val = self.evaluate_expr(value).await?;
                let seq_val = self.evaluate_expr(sequence).await?;
                match &seq_val {
                    RuntimeValue::Crdt(rc) => rc.borrow_mut().append(&val)?,
                    RuntimeValue::List(_) => {
                        crate::semantics::collections::list_push(&seq_val, val)?
                    }
                    _ => return Err(format!("Cannot append to {}", seq_val.type_name())),
                }
                Ok(ControlFlow::Continue)
            }

            Stmt::ResolveConflict { object, field, value } => {
                let val = self.evaluate_expr(value).await?;
                let obj_val = self.evaluate_expr(object).await?;
                let field_name = self.ctx.interner.resolve(*field);
                // A real divergent register resolves in place (its `Rc` is shared with the
                // struct field); a plain field falls back to a direct assignment.
                if let RuntimeValue::Struct(s) = &obj_val {
                    if let Some(RuntimeValue::Crdt(rc)) = s.fields.get(field_name) {
                        rc.borrow_mut().resolve(&val)?;
                        return Ok(ControlFlow::Continue);
                    }
                }
                if let Expr::Identifier(obj_sym) = object {
                    let mut owner = self.lookup(*obj_sym)?.clone();
                    if let RuntimeValue::Struct(ref mut s) = owner {
                        s.fields.insert(field_name.to_string(), val);
                        self.assign(*obj_sym, owner)?;
                        return Ok(ControlFlow::Continue);
                    }
                }
                Err("Resolve target must be a struct field".to_string())
            }

            Stmt::Check { subject, predicate, is_capability, object, source_text, .. } => {
                // Get the policy registry
                let registry = match &self.ctx.policy_registry {
                    Some(r) => r,
                    None => return Err("Security Check requires policies. Use compiled Rust or add ## Policy block.".to_string()),
                };

                let subj_val = self.lookup(*subject)?.clone();
                let subj_type_name = match &subj_val {
                    RuntimeValue::Struct(s) => s.type_name.clone(),
                    _ => return Err(format!("Check subject must be a struct, got {}", subj_val.type_name())),
                };

                // Find the subject type symbol
                let subj_type_sym = match self.ctx.interner.lookup(&subj_type_name) {
                    Some(sym) => sym,
                    None => return Err(format!("Unknown type '{}' in Check statement", subj_type_name)),
                };

                let passed = if *is_capability {
                    // Capability check: "user can publish document"
                    let obj_val = match object {
                        Some(obj_sym) => Some(self.lookup(*obj_sym)?.clone()),
                        None => None,
                    };

                    let caps = registry.get_capabilities(subj_type_sym);
                    let cap = caps
                        .and_then(|caps| caps.iter().find(|c| c.action == *predicate));

                    match cap {
                        Some(cap) => self.evaluate_policy_condition(&cap.condition, &subj_val, obj_val.as_ref()),
                        None => {
                            let pred_name = self.ctx.interner.resolve(*predicate);
                            return Err(format!("No capability '{}' defined for type '{}'", pred_name, subj_type_name));
                        }
                    }
                } else {
                    // Predicate check: "user is admin"
                    let preds = registry.get_predicates(subj_type_sym);
                    let pred_def = preds
                        .and_then(|preds| preds.iter().find(|p| p.predicate_name == *predicate));

                    match pred_def {
                        Some(pred) => self.evaluate_policy_condition(&pred.condition, &subj_val, None),
                        None => {
                            let pred_name = self.ctx.interner.resolve(*predicate);
                            return Err(format!("No predicate '{}' defined for type '{}'", pred_name, subj_type_name));
                        }
                    }
                };

                if !passed {
                    return Err(format!("Security Check Failed: {}", source_text));
                }
                Ok(ControlFlow::Continue)
            }

            // `Connect to "<relay>"` — open the transport: dial the relay and hold
            // the connection for `Sync` and peer messaging. Accepts the same
            // address surface as the compiled path: a libp2p multiaddr
            // (`/ip4/H/tcp/P`) normalizes to the relay's `ws://H:P`; a raw `ws://`
            // URL passes through.
            Stmt::ConnectTo { address, secure } => {
                let raw = self.evaluate_expr(address).await?.to_display_string();
                let url = logicaffeine_system::addr::multiaddr_to_ws_url(&raw)
                    .map_err(|e| format!("Connect address '{raw}' is not a ws:// URL or supported multiaddr: {e}"))?;
                // Activate the one-time-pad session first, so a bad pad fails the connect (fail-closed).
                if let Some(bind) = secure {
                    self.activate_pnp_session(bind).await?;
                }
                // OFFLINE mode (the deterministic tree-walker/VM oracles, no relay transport): `Connect`
                // is a local no-op — nothing to dial, so `net` stays None and the following ops run
                // locally, exactly as `Listen`/`Send`/`Sync` already do offline. A relay-connected driver
                // dials for real. The address is still validated so a malformed one errors on both paths.
                if !crate::concurrency::net_inbox::net_is_offline() {
                    let net = logicaffeine_system::net::Net::connect(&url)
                        .await
                        .map_err(|e| format!("Connect to relay '{url}' failed: {e}"))?;
                    self.netbox.net = Some(net);
                }
                Ok(ControlFlow::Continue)
            }
            // `Listen at "<addr>"` — declare this node's identity: subscribe to its
            // inbox topic so peers can reach it with `Send … to`. The relay is the
            // transport (a browser cannot bind a socket), so this needs a prior
            // `Connect`. The address is canonicalized so `/ip4/H/tcp/P` and the
            // `ws://H:P` form name the same inbox.
            Stmt::Listen { address, secure } => {
                let raw = self.evaluate_expr(address).await?.to_display_string();
                let topic = logicaffeine_system::addr::canonical_topic(&raw);
                let hs_topic = crate::concurrency::net_inbox::handshake_topic_for(&topic);
                if let Some(bind) = secure {
                    self.activate_pnp_session(bind).await?;
                }
                // LOCAL/OFFLINE mode (no relay): declare our inbox identity locally and skip the relay
                // subscribe — a single node listening on its own address needs no transport. A relay-
                // connected node subscribes so peers can reach it. Either way `Listen` never ERRORS.
                if let Some(net) = self.netbox.net.as_mut() {
                    net.subscribe(&topic).await?;
                    // Also receive peers' capability handshakes on our dedicated handshake topic.
                    net.subscribe(&hs_topic).await?;
                }
                self.netbox.inbox = Some(Rc::new(topic));
                Ok(ControlFlow::Continue)
            }
            // `Let r be a PeerAgent at "<addr>"` — a handle to a remote peer; its
            // value is the peer's canonical inbox topic. Pure (no I/O); `Send`
            // and `Await` do the networking.
            Stmt::LetPeerAgent { var, address } => {
                let raw = self.evaluate_expr(address).await?.to_display_string();
                let topic = logicaffeine_system::addr::canonical_topic(&raw);
                self.define(*var, RuntimeValue::Peer(Rc::new(topic)));
                Ok(ControlFlow::Continue)
            }
            Stmt::Sleep { milliseconds } => {
                let val = self.evaluate_expr(milliseconds).await?;

                // Under the deterministic scheduler (any program with tasks/channels), route
                // the sleep through a scheduler timer — the same logical-tick scale as a
                // `Select` `After` arm — so it integrates with task scheduling. Blocking on a
                // raw host timer here would suspend the task with no scheduler request and
                // panic the cooperative driver (and under a non-tokio executor there is no
                // reactor at all).
                if self.yield_state.is_some() {
                    let ticks = match &val {
                        RuntimeValue::Int(n) => (*n).max(0) as u64,
                        RuntimeValue::Duration(d) => (*d).max(0) as u64,
                        _ => return Err(format!("Sleep requires Duration or Int, got {}", val.type_name())),
                    };
                    if ticks > 0 {
                        self.yield_request(BlockingRequest::Sleep(ticks)).await;
                    }
                    return Ok(ControlFlow::Continue);
                }

                let nanos = match val {
                    RuntimeValue::Duration(nanos) => nanos,
                    RuntimeValue::Int(ms) => ms.wrapping_mul(1_000_000), // ms → nanos
                    _ => return Err(format!("Sleep requires Duration or Int, got {}", val.type_name())),
                };

                if nanos > 0 {
                    #[cfg(not(target_arch = "wasm32"))]
                    {
                        // Use tokio re-exported from logicaffeine_system
                        logicaffeine_system::tokio::time::sleep(
                            std::time::Duration::from_nanos(nanos as u64)
                        ).await;
                    }
                    #[cfg(target_arch = "wasm32")]
                    {
                        // On WASM, use gloo-timers for async sleep
                        let millis = (nanos / 1_000_000) as u32;
                        if millis > 0 {
                            gloo_timers::future::TimeoutFuture::new(millis).await;
                        }
                    }
                }
                Ok(ControlFlow::Continue)
            }
            // `Sync x on "topic"` — a CRDT sync POINT over the relay: subscribe,
            // publish the local counter, then merge whatever has arrived (the same
            // `crdt_merge_field` the in-process merge uses). No background task —
            // the merge happens here, keeping the tree-walker's linear model.
            Stmt::Sync { var, topic } => {
                let topic_str = self.evaluate_expr(topic).await?.to_display_string();
                let current = self.lookup(*var)?.clone();
                // Rich CRDT (ORSet / RGA / MVRegister) → δ-STATE sync: publish only what changed since
                // our last sync on this topic (`delta_since`, per field), and merge incoming deltas IN
                // PLACE through the shared `Rc`. The first sync ships the full state (delta since
                // nothing); every later sync ships a handful of bytes for the NEW entries, never the
                // whole collection. Idempotent + commutative, so redelivery / reordering still
                // converges. Covers a bare CrdtValue var (field name "") AND every CrdtValue field of a
                // `Shared` struct — which is how programs actually hold them.
                let crdt_fields: Vec<(String, std::rc::Rc<std::cell::RefCell<crate::semantics::crdt::CrdtValue>>)> =
                    match &current {
                        RuntimeValue::Crdt(rc) => vec![(String::new(), rc.clone())],
                        RuntimeValue::Struct(s) => s
                            .fields
                            .iter()
                            .filter_map(|(k, v)| match v {
                                RuntimeValue::Crdt(rc) => Some((k.clone(), rc.clone())),
                                _ => None,
                            })
                            .collect(),
                        _ => Vec::new(),
                    };
                if !crdt_fields.is_empty() {
                    let field_key = |name: &str| format!("{topic_str}\u{0}{name}");
                    // Each field's delta since the version we last shipped on this topic.
                    let mut frame: Vec<(String, Vec<u8>)> = Vec::new();
                    for (name, rc) in &crdt_fields {
                        let since = self.netbox.sync_versions.get(&field_key(name)).cloned().unwrap_or_default();
                        if let Some(d) = rc.borrow().delta_since_bytes(&since) {
                            frame.push((name.clone(), d));
                        }
                    }
                    let payload = serde_json::to_vec(&frame).unwrap_or_default();
                    // LOCAL/OFFLINE mode (no relay): single-node δ-sync is a no-op — nothing arrives,
                    // the local state stands; we still advance the per-field versions below. A relay-
                    // connected deployment publishes its delta + merges incoming ones.
                    if let Some(net) = self.netbox.net.as_mut() {
                        net.subscribe(&topic_str).await?;
                        if !frame.is_empty() {
                            net.publish(&topic_str, payload)?;
                        }
                        let incoming = net.drain();
                        // Merge every incoming field delta into the matching local CRDT field, in place.
                        for (_t, data) in incoming {
                            let Ok(fields) = serde_json::from_slice::<Vec<(String, Vec<u8>)>>(&data) else {
                                continue;
                            };
                            for (name, delta) in fields {
                                if let Some((_, rc)) = crdt_fields.iter().find(|(n, _)| *n == name) {
                                    rc.borrow_mut().apply_delta_bytes(&delta);
                                }
                            }
                        }
                    }
                    // Record the version we now hold per field, so the next sync ships only later changes.
                    for (name, rc) in &crdt_fields {
                        let v = rc.borrow().version();
                        self.netbox.sync_versions.insert(field_key(name), v);
                    }
                    return Ok(ControlFlow::Continue);
                }
                // Encode the counter (Int) or counter-struct (named Int fields) as
                // the relay wire form; `None` ⇒ nothing to publish yet.
                let publish_bytes = crate::semantics::arith::crdt_to_wire(&current);
                // LOCAL/OFFLINE mode: with no relay connected (the playground/test path, and any
                // program that never `Connect`ed), a `Sync` is a SINGLE-NODE no-op — the local CRDT
                // value stands, deterministically. A real deployment that `Connect`ed to a relay takes
                // the transport branch (publish our delta, merge everyone else's). Either way `Sync`
                // never ERRORS, so a networked program runs identically on tree-walker, VM, and AOT.
                let merged = if let Some(net) = self.netbox.net.as_mut() {
                    net.subscribe(&topic_str).await?;
                    if let Some(bytes) = publish_bytes {
                        net.publish(&topic_str, bytes)?;
                    }
                    // Merge everything that has arrived since the last sync point, field by field —
                    // the same CRDT merge the in-process `Merge` uses.
                    let incoming = net.drain();
                    let mut merged = current;
                    for (_t, data) in incoming {
                        merged = crate::semantics::arith::crdt_merge_wire(merged, &data);
                    }
                    merged
                } else {
                    current
                };
                self.assign(*var, merged)?;
                Ok(ControlFlow::Continue)
            }
            // Phase 55: Mount now supported via VFS
            Stmt::Mount { var, path } => {
                let path_val = self.evaluate_expr(path).await?.to_display_string();
                match &self.ctx.vfs {
                    Some(vfs) => {
                        // Read existing content or create empty
                        let content = match vfs.read_to_string(&path_val).await {
                            Ok(s) => s,
                            Err(_) => String::new(),
                        };
                        self.define(*var, RuntimeValue::Text(Rc::new(content)));
                    }
                    None => return Err("VFS not initialized. Use Interpreter::with_vfs()".to_string()),
                }
                Ok(ControlFlow::Continue)
            }

            // Phase 54 / T6-T7: Go-like concurrency, driven by the deterministic
            // scheduler via the per-task side-channel (`yield_request`).
            Stmt::CreatePipe { var, capacity, .. } => {
                let cap = capacity.map(|c| c as usize);
                let resume = self.yield_request(BlockingRequest::NewChan(cap)).await;
                let ch = resume
                    .into_chan()
                    .ok_or_else(|| "scheduler did not create a channel".to_string())?;
                self.define(*var, RuntimeValue::Chan(ch));
                Ok(ControlFlow::Continue)
            }
            Stmt::SendPipe { value, pipe } => {
                let ch = self.eval_chan(pipe).await?;
                let val = self.evaluate_expr(value).await?;
                let payload = marshal::materialize(&val)
                    .map_err(|e| format!("cannot send value through a channel: {:?}", e))?;
                self.yield_request(BlockingRequest::Send(ch, payload)).await;
                Ok(ControlFlow::Continue)
            }
            Stmt::ReceivePipe { var, pipe } => {
                let ch = self.eval_chan(pipe).await?;
                let resume = self.yield_request(BlockingRequest::Recv(ch)).await;
                let value = marshal::rebuild(resume.into_payload());
                self.define(*var, value);
                Ok(ControlFlow::Continue)
            }
            Stmt::LaunchTask { function, args } => {
                let mut arg_vals = Vec::with_capacity(args.len());
                for a in args.iter() {
                    arg_vals.push(self.evaluate_expr(a).await?);
                }
                let child = self.spawn_child_task(*function, arg_vals);
                self.yield_request(BlockingRequest::Spawn(child)).await;
                Ok(ControlFlow::Continue)
            }
            Stmt::LaunchTaskWithHandle { handle, function, args } => {
                let mut arg_vals = Vec::with_capacity(args.len());
                for a in args.iter() {
                    arg_vals.push(self.evaluate_expr(a).await?);
                }
                let child = self.spawn_child_task(*function, arg_vals);
                let resume = self.yield_request(BlockingRequest::Spawn(child)).await;
                let tid = resume
                    .into_task()
                    .ok_or_else(|| "scheduler did not return a task handle".to_string())?;
                self.define(*handle, RuntimeValue::TaskHandle(tid));
                Ok(ControlFlow::Continue)
            }
            Stmt::StopTask { handle } => {
                let tid = self.eval_task(handle).await?;
                self.yield_request(BlockingRequest::Abort(tid)).await;
                Ok(ControlFlow::Continue)
            }
            Stmt::TrySendPipe { value, pipe, result } => {
                let ch = self.eval_chan(pipe).await?;
                let val = self.evaluate_expr(value).await?;
                let payload = marshal::materialize(&val)
                    .map_err(|e| format!("cannot send value through a channel: {:?}", e))?;
                let resume = self.yield_request(BlockingRequest::TrySend(ch, payload)).await;
                let ok = matches!(resume.into_payload(), RtPayload::Bool(true));
                if let Some(res) = result {
                    self.define(*res, RuntimeValue::Bool(ok));
                }
                Ok(ControlFlow::Continue)
            }
            Stmt::TryReceivePipe { var, pipe } => {
                let ch = self.eval_chan(pipe).await?;
                let resume = self.yield_request(BlockingRequest::TryRecv(ch)).await;
                let value = marshal::rebuild(resume.into_payload());
                self.define(*var, value);
                Ok(ControlFlow::Continue)
            }
            Stmt::Select { branches } => {
                use crate::ast::stmt::SelectBranch;
                // Resolve each branch to a runtime arm in declaration order, so the
                // scheduler's winning index maps straight back to a branch body.
                let mut arms = Vec::with_capacity(branches.len());
                for branch in branches.iter() {
                    match branch {
                        SelectBranch::Receive { pipe, .. } => {
                            let ch = self.eval_chan(pipe).await?;
                            arms.push(SelectArm::Recv(ch));
                        }
                        SelectBranch::Timeout { milliseconds, .. } => {
                            let ticks = self.eval_select_timeout_ticks(milliseconds).await?;
                            arms.push(SelectArm::Timeout(ticks));
                        }
                    }
                }
                let resume = self.yield_request(BlockingRequest::Select(arms)).await;
                let (arm, payload) = resume
                    .into_select()
                    .ok_or_else(|| "scheduler did not resolve the select".to_string())?;
                match &branches[arm] {
                    SelectBranch::Receive { var, body, .. } => {
                        self.define(*var, marshal::rebuild(payload));
                        self.execute_block(*body).await
                    }
                    SelectBranch::Timeout { body, .. } => self.execute_block(*body).await,
                }
            }

            // Escape blocks contain raw Rust code and cannot be interpreted
            Stmt::Escape { .. } => {
                Err(
                    "Escape blocks contain raw Rust code and cannot be interpreted. \
                     Use `largo build` or `largo run` to compile and run this program."
                    .to_string()
                )
            }

            // Dependencies are compilation metadata. No-op in interpreter.
            Stmt::Require { .. } => {
                Ok(ControlFlow::Continue)
            }

            // Theorems and definitions are proof-layer declarations verified at
            // compile-time, not executed.
            Stmt::Theorem(_) | Stmt::Definition(_) | Stmt::Axiom(_) | Stmt::Theory(_) => Ok(ControlFlow::Continue),
        }
    }

    /// Execute a block of statements, returning control flow.
    /// Phase 55: Now async.
    #[async_recursion(?Send)]
    async fn execute_block(&mut self, block: Block<'a>) -> Result<ControlFlow, String> {
        self.push_scope();
        for stmt in block.iter() {
            match self.execute_stmt(stmt).await? {
                ControlFlow::Continue => {}
                flow => {
                    self.pop_scope();
                    return Ok(flow);
                }
            }
        }
        self.pop_scope();
        Ok(ControlFlow::Continue)
    }

    /// The loud message for a non-exhaustive `Inspect` — no arm matched the
    /// scrutinee's actual variant and there was no `Otherwise`. Exhaustiveness
    /// or a wildcard is required; a silent no-op hid the missing arm. Kept
    /// value-agnostic so it is byte-identical to the VM's compile-time
    /// `FailWith` (the VM cannot name the runtime variant at emit time).
    fn inspect_unhandled(&self, _target: &RuntimeValue) -> String {
        "Inspect has no arm for the value and no Otherwise (matches must be exhaustive)".to_string()
    }

    /// Execute Inspect (pattern matching).
    /// Phase 55: Now async.
    /// Phase 102: Extended to handle kernel inductives.
    #[async_recursion(?Send)]
    async fn execute_inspect(&mut self, target: &RuntimeValue, arms: &[MatchArm<'a>]) -> Result<ControlFlow, String> {
        for arm in arms {
            // Handle Otherwise (wildcard) case
            if arm.variant.is_none() {
                let flow = self.execute_block(arm.body).await?;
                return Ok(flow);
            }

            match target {
                RuntimeValue::Struct(s) => {
                    if let Some(variant) = arm.variant {
                        let variant_name = self.ctx.interner.resolve(variant);
                        if s.type_name == variant_name {
                            self.push_scope();
                            for (field_name, binding_name) in &arm.bindings {
                                let field_str = self.ctx.interner.resolve(*field_name);
                                if let Some(val) = s.fields.get(field_str) {
                                    self.define(*binding_name, val.clone());
                                }
                            }
                            let result = self.execute_block(arm.body).await;
                            self.pop_scope();
                            let flow = result?;
                            return Ok(flow);
                        }
                    }
                }

                RuntimeValue::Inductive(ind) => {
                    if let Some(variant) = arm.variant {
                        let variant_name = self.ctx.interner.resolve(variant);
                        if ind.constructor == variant_name {
                            self.push_scope();
                            for (i, (_, binding_name)) in arm.bindings.iter().enumerate() {
                                if i < ind.args.len() {
                                    self.define(*binding_name, ind.args[i].clone());
                                }
                            }
                            let result = self.execute_block(arm.body).await;
                            self.pop_scope();
                            let flow = result?;
                            return Ok(flow);
                        }
                    }
                }

                _ => {}
            }
        }
        Err(self.inspect_unhandled(target))
    }

    /// Resolve a `Send … to <dest>` / `Await … from <src>` operand to a relay
    /// topic: a `PeerAgent` uses its topic; a string is canonicalized as an
    /// address; anything else is a type error.
    fn peer_topic_of(value: &RuntimeValue) -> Result<String, String> {
        match value {
            RuntimeValue::Peer(topic) => Ok((**topic).clone()),
            RuntimeValue::Text(s) => Ok(logicaffeine_system::addr::canonical_topic(s)),
            other => Err(format!(
                "Send/Await expects a PeerAgent or address string, got {}",
                other.type_name()
            )),
        }
    }

    /// Build the wire type registry from the program's struct definitions. A peer running
    /// the SAME program derives the identical (content-addressed) ids, so a `Send shared`
    /// struct can ship its id instead of its field names and the receiver resolves it.
    fn build_wire_type_registry(&self) -> crate::concurrency::marshal::WireTypeRegistry {
        let schemas: Vec<(String, Vec<String>)> = self
            .ctx
            .struct_defs
            .iter()
            .map(|(name_sym, fields)| {
                let type_name = self.ctx.interner.resolve(*name_sym).to_string();
                let field_names = fields
                    .iter()
                    .map(|(fname, _ty, _public)| self.ctx.interner.resolve(*fname).to_string())
                    .collect();
                (type_name, field_names)
            })
            .collect();
        let enums: Vec<(String, Vec<String>)> = self
            .ctx
            .enum_defs
            .iter()
            .map(|(name_sym, ctors)| {
                let type_name = self.ctx.interner.resolve(*name_sym).to_string();
                let ctor_names = ctors.iter().map(|c| self.ctx.interner.resolve(*c).to_string()).collect();
                (type_name, ctor_names)
            })
            .collect();
        crate::concurrency::marshal::WireTypeRegistry::new(schemas).with_enums(enums)
    }

    /// Block (cooperatively) until a message from `want` arrives on our inbox, returning its payload.
    /// Drives the shared [`NetInbox`]: try the buffer, else drain the relay and retry, yielding a
    /// macrotask between polls so the browser event loop (and the relay's delivery) keeps running.
    /// Messages from other senders stay queued for their own `Await`.
    async fn await_message_from(&mut self, want: &str, view: bool) -> Result<RuntimeValue, String> {
        loop {
            if let Some(v) = self.netbox.try_take_message(want, view) {
                return Ok(v);
            }
            let registry = self.build_wire_type_registry();
            self.netbox.drain(registry);
            if let Some(v) = self.netbox.try_take_message(want, view) {
                return Ok(v);
            }
            Self::poll_tick().await;
        }
    }

    /// Block until a batch STREAM from `want` arrives, then deframe it into a list (mirrors
    /// [`await_message_from`] over a `RecvSlot::Stream`).
    async fn await_stream_from(&mut self, want: &str) -> Result<RuntimeValue, String> {
        loop {
            if let Some(v) = self.netbox.try_take_stream(want) {
                return Ok(v);
            }
            let registry = self.build_wire_type_registry();
            self.netbox.drain(registry);
            if let Some(v) = self.netbox.try_take_stream(want) {
                return Ok(v);
            }
            Self::poll_tick().await;
        }
    }

    /// One cooperative poll interval, cross-target: a short async sleep that lets
    /// the relay deliver and (in the browser) the event loop run between polls.
    async fn poll_tick() {
        #[cfg(not(target_arch = "wasm32"))]
        logicaffeine_system::tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        #[cfg(target_arch = "wasm32")]
        gloo_timers::future::TimeoutFuture::new(2).await;
    }

    /// Evaluate an expression to a runtime value.
    /// Phase 55: Now async.
    #[async_recursion(?Send)]
    async fn evaluate_expr(&mut self, expr: &Expr<'a>) -> Result<RuntimeValue, String> {
        match expr {
            Expr::Literal(lit) => self.evaluate_literal(lit),

            Expr::Identifier(sym) => {
                let name = self.ctx.interner.resolve(*sym);
                // Handle temporal builtins (the NAME wins, even when shadowed)
                match name {
                    "today" => {
                        return Ok(crate::semantics::temporal::today());
                    }
                    "now" => {
                        return Ok(crate::semantics::temporal::now());
                    }
                    _ => {}
                }
                self.lookup(*sym).cloned()
            }

            Expr::BinaryOp { op, left, right } => {
                match op {
                    BinaryOpKind::And => {
                        let left_val = self.evaluate_expr(left).await?;
                        if !left_val.is_truthy() {
                            return Ok(RuntimeValue::Bool(false));
                        }
                        let right_val = self.evaluate_expr(right).await?;
                        Ok(RuntimeValue::Bool(right_val.is_truthy()))
                    }
                    BinaryOpKind::Or => {
                        let left_val = self.evaluate_expr(left).await?;
                        if left_val.is_truthy() {
                            return Ok(RuntimeValue::Bool(true));
                        }
                        let right_val = self.evaluate_expr(right).await?;
                        Ok(RuntimeValue::Bool(right_val.is_truthy()))
                    }
                    _ => {
                        let left_val = self.evaluate_expr(left).await?;
                        let right_val = self.evaluate_expr(right).await?;
                        self.apply_binary_op(*op, left_val, right_val)
                    }
                }
            }

            Expr::Call { function, args } => {
                self.call_function(*function, args).await
            }

            Expr::Index { collection, index } => {
                let coll_val = self.evaluate_expr(collection).await?;
                let idx_val = self.evaluate_expr(index).await?;
                crate::semantics::collections::index_get(&coll_val, &idx_val)
            }

            Expr::Slice { collection, start, end } => {
                let coll_val = self.evaluate_expr(collection).await?;
                let start_val = self.evaluate_expr(start).await?;
                let end_val = self.evaluate_expr(end).await?;
                crate::semantics::collections::slice(&coll_val, &start_val, &end_val)
            }

            Expr::Copy { expr: inner } => {
                let val = self.evaluate_expr(inner).await?;
                Ok(val.deep_clone())
            }

            Expr::Give { value } => {
                // In interpreter, Give is just semantic - evaluate the value
                self.evaluate_expr(value).await
            }

            Expr::Length { collection } => {
                let coll_val = self.evaluate_expr(collection).await?;
                crate::semantics::collections::length_of(&coll_val)
            }

            Expr::Contains { collection, value } => {
                let coll_val = self.evaluate_expr(collection).await?;
                let val = self.evaluate_expr(value).await?;
                crate::semantics::collections::contains(&coll_val, &val)
            }

            Expr::Union { left, right } => {
                let left_val = self.evaluate_expr(left).await?;
                let right_val = self.evaluate_expr(right).await?;
                crate::semantics::collections::union(&left_val, &right_val)
            }

            Expr::Intersection { left, right } => {
                let left_val = self.evaluate_expr(left).await?;
                let right_val = self.evaluate_expr(right).await?;
                crate::semantics::collections::intersection(&left_val, &right_val)
            }

            Expr::List(items) => {
                let mut values = Vec::with_capacity(items.len());
                for e in items.iter() {
                    values.push(self.evaluate_expr(e).await?);
                }
                Ok(RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(values)))))
            }

            Expr::Tuple(items) => {
                let mut values = Vec::with_capacity(items.len());
                for e in items.iter() {
                    values.push(self.evaluate_expr(e).await?);
                }
                Ok(RuntimeValue::Tuple(Rc::new(values)))
            }

            Expr::Range { start, end } => {
                let start_val = self.evaluate_expr(start).await?;
                let end_val = self.evaluate_expr(end).await?;
                crate::semantics::collections::range(&start_val, &end_val)
            }

            Expr::FieldAccess { object, field } => {
                let obj_val = self.evaluate_expr(object).await?;
                match &obj_val {
                    RuntimeValue::Struct(s) => {
                        let field_name = self.ctx.interner.resolve(*field);
                        s.fields.get(field_name).cloned()
                            .ok_or_else(|| format!("Field '{}' not found", field_name))
                    }
                    _ => Err(format!("Cannot access field on {}", obj_val.type_name())),
                }
            }

            Expr::New { type_name, init_fields, .. } => {
                let name = self.ctx.interner.resolve(*type_name).to_string();

                if name == "Seq" || name == "List" {
                    return Ok(RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(Vec::new())))));
                }

                if name == "Set" || name == "HashSet" {
                    return Ok(RuntimeValue::Set(Rc::new(RefCell::new(vec![]))));
                }

                if name == "Map" || name == "HashMap" {
                    return Ok(RuntimeValue::Map(Rc::new(RefCell::new(MapStorage::default()))));
                }

                let mut fields = HashMap::new();
                for (field_sym, field_expr) in init_fields {
                    let field_name = self.ctx.interner.resolve(*field_sym).to_string();
                    let field_val = self.evaluate_expr(field_expr).await?;
                    fields.insert(field_name, field_val);
                }

                if let Some(def) = self.ctx.struct_defs.get(type_name) {
                    for (field_sym, type_sym, _) in def {
                        let field_name = self.ctx.interner.resolve(*field_sym).to_string();
                        if !fields.contains_key(&field_name) {
                            let type_name_str = self.ctx.interner.resolve(*type_sym).to_string();
                            let default = match type_name_str.as_str() {
                                "Int" => RuntimeValue::Int(0),
                                "Float" => RuntimeValue::Float(0.0),
                                "Bool" => RuntimeValue::Bool(false),
                                "Text" | "String" => RuntimeValue::Text(Rc::new(String::new())),
                                "Char" => RuntimeValue::Char('\0'),
                                "Byte" => RuntimeValue::Int(0),
                                "Seq" | "List" => RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(Vec::new())))),
                                "Set" | "HashSet" => RuntimeValue::Set(Rc::new(RefCell::new(vec![]))),
                                "Map" | "HashMap" => RuntimeValue::Map(Rc::new(RefCell::new(MapStorage::default()))),
                                // A `Shared` struct's CRDT fields default to an empty live
                                // CRDT, mirroring the compiled tier's `ORSet`/`RGA` field.
                                "SharedSet" | "ORSet" | "SharedSet_AddWins" =>
                                    RuntimeValue::Crdt(Rc::new(RefCell::new(
                                        crate::semantics::crdt::CrdtValue::new_set(
                                            crate::semantics::crdt::next_replica_id())))),
                                "SharedSet_RemoveWins" =>
                                    RuntimeValue::Crdt(Rc::new(RefCell::new(
                                        crate::semantics::crdt::CrdtValue::new_set_remove_wins(
                                            crate::semantics::crdt::next_replica_id())))),
                                "SharedSequence" | "RGA" | "SharedSequence_YATA" | "CollaborativeSequence" =>
                                    RuntimeValue::Crdt(Rc::new(RefCell::new(
                                        crate::semantics::crdt::CrdtValue::new_seq(
                                            crate::semantics::crdt::next_replica_id())))),
                                _ => RuntimeValue::Nothing,
                            };
                            fields.insert(field_name, default);
                        }
                    }
                }

                Ok(RuntimeValue::Struct(Box::new(StructValue { type_name: name, fields })))
            }

            // Phase 102: Enum variant constructor
            // Now creates RuntimeValue::Inductive for unified kernel types
            Expr::NewVariant { enum_name, variant, fields } => {
                let inductive_type = self.ctx.interner.resolve(*enum_name).to_string();
                let constructor = self.ctx.interner.resolve(*variant).to_string();

                let mut args = Vec::new();
                for (_, field_expr) in fields {
                    let field_val = self.evaluate_expr(field_expr).await?;
                    args.push(field_val);
                }

                Ok(RuntimeValue::Inductive(Box::new(InductiveValue {
                    inductive_type,
                    constructor,
                    args,
                })))
            }

            Expr::ManifestOf { .. } => {
                Ok(RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(Vec::new())))))
            }

            Expr::ChunkAt { .. } => {
                Ok(RuntimeValue::Nothing)
            }

            Expr::WithCapacity { value, .. } => {
                self.evaluate_expr(value).await
            }

            Expr::OptionSome { value } => {
                self.evaluate_expr(value).await
            }

            Expr::OptionNone => {
                Ok(RuntimeValue::Nothing)
            }

            Expr::Not { operand } => {
                let val = self.evaluate_expr(operand).await?;
                crate::semantics::arith::not_value(val)
            }

            Expr::InterpolatedString(parts) => {
                let mut result = String::new();
                for part in parts {
                    match part {
                        crate::ast::stmt::StringPart::Literal(sym) => {
                            result.push_str(self.ctx.interner.resolve(*sym));
                        }
                        crate::ast::stmt::StringPart::Expr { value, format_spec, debug } => {
                            let val = self.evaluate_expr(value).await?;
                            if *debug {
                                let prefix = match value {
                                    Expr::Identifier(sym) => self.ctx.interner.resolve(*sym).to_string(),
                                    _ => "expr".to_string(),
                                };
                                result.push_str(&prefix);
                                result.push('=');
                            }
                            if let Some(spec_sym) = format_spec {
                                let spec = self.ctx.interner.resolve(*spec_sym);
                                result.push_str(&apply_format_spec(&val, spec));
                            } else {
                                result.push_str(&val.to_display_string());
                            }
                        }
                    }
                }
                Ok(RuntimeValue::Text(Rc::new(result)))
            }

            Expr::Escape { .. } => {
                Err("Escape expressions contain raw Rust code and cannot be interpreted. \
                     Use `largo build` or `largo run` to compile and run this program.".to_string())
            }

            Expr::Closure { params, body, .. } => {
                let free_vars = self.collect_free_vars_in_closure(params, body);
                let mut captured_env = HashMap::new();
                for sym in &free_vars {
                    if let Some(val) = self.task.env.lookup(*sym) {
                        captured_env.insert(*sym, val.deep_clone());
                    }
                }

                let body_index = self.ctx.closure_bodies.len();
                match body {
                    ClosureBody::Expression(expr) => {
                        self.ctx.closure_bodies.push(ClosureBodyRef::Expression(expr));
                    }
                    ClosureBody::Block(block) => {
                        self.ctx.closure_bodies.push(ClosureBodyRef::Block(block));
                    }
                }

                let param_names: Vec<Symbol> = params.iter().map(|(name, _)| *name).collect();

                Ok(RuntimeValue::Function(Box::new(ClosureValue {
                    body_index,
                    captured_env,
                    param_names,
                    generated: None,
                })))
            }

            Expr::CallExpr { callee, args } => {
                let callee_val = self.evaluate_expr(callee).await?;
                if let RuntimeValue::Function(closure) = callee_val {
                    let mut arg_values = Vec::with_capacity(args.len());
                    for arg in args.iter() {
                        arg_values.push(self.evaluate_expr(arg).await?);
                    }
                    self.call_closure_value(&closure, arg_values).await
                } else {
                    Err(format!("Cannot call value of type {}", callee_val.type_name()))
                }
            }
        }
    }

    /// Evaluate a literal to a runtime value.
    fn evaluate_literal(&self, lit: &Literal) -> Result<RuntimeValue, String> {
        match lit {
            Literal::Number(n) => Ok(RuntimeValue::Int(*n)),
            Literal::Float(f) => Ok(RuntimeValue::Float(*f)),
            Literal::Text(sym) => Ok(RuntimeValue::Text(Rc::new(self.ctx.interner.resolve(*sym).to_string()))),
            Literal::Boolean(b) => Ok(RuntimeValue::Bool(*b)),
            Literal::Nothing => Ok(RuntimeValue::Nothing),
            Literal::Char(c) => Ok(RuntimeValue::Char(*c)),
            Literal::Duration(nanos) => Ok(RuntimeValue::Duration(*nanos)),
            Literal::Date(days) => Ok(RuntimeValue::Date(*days)),
            Literal::Moment(nanos) => Ok(RuntimeValue::Moment(*nanos)),
            Literal::Span { months, days } => Ok(RuntimeValue::Span { months: *months, days: *days }),
            Literal::Time(nanos) => Ok(RuntimeValue::Time(*nanos)),
        }
    }

    /// Apply a binary operator (delegates to the shared semantics kernel).
    fn apply_binary_op(&self, op: BinaryOpKind, left: RuntimeValue, right: RuntimeValue) -> Result<RuntimeValue, String> {
        crate::semantics::arith::binary_op(op, left, right)
    }

    pub fn values_equal_pub(&self, left: &RuntimeValue, right: &RuntimeValue) -> bool {
        self.values_equal(left, right)
    }

    fn values_equal(&self, left: &RuntimeValue, right: &RuntimeValue) -> bool {
        crate::semantics::compare::values_equal(left, right)
    }

    /// Call a function (built-in or user-defined).
    #[async_recursion(?Send)]
    async fn call_function(&mut self, function: Symbol, args: &[&'async_recursion Expr<'a>]) -> Result<RuntimeValue, String> {
        // Built-in functions — Symbol comparison (integer) instead of string matching
        let func_sym = Some(function);
        if func_sym == self.ctx.sym_show {
            for arg in args {
                let val = self.evaluate_expr(arg).await?;
                self.emit_output(val.to_display_string());
            }
            return Ok(RuntimeValue::Nothing);
        } else if func_sym == self.ctx.sym_args {
            // `args()` system native: the stored argv as a `Seq of Text`,
            // mirroring the compiled binary's `env::args()`. Intercepted BEFORE
            // the empty native-decl body would be reached, like `show`.
            let items: Vec<RuntimeValue> = self
                .ctx
                .program_args
                .iter()
                .map(|s| RuntimeValue::Text(Rc::new(s.clone())))
                .collect();
            return Ok(RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(items)))));
        } else if let Some(id) = self.builtin_id(function) {
            // Arity is checked BEFORE evaluating arguments (kernel rule).
            crate::semantics::builtins::check_arity(id, args.len())?;
            // `format` reads only its first argument; preserve its laziness.
            let vals = if id == crate::semantics::builtins::BuiltinId::Format {
                match args.first() {
                    Some(a) => vec![self.evaluate_expr(a).await?],
                    None => Vec::new(),
                }
            } else {
                let mut v = Vec::with_capacity(args.len());
                for arg in args {
                    v.push(self.evaluate_expr(arg).await?);
                }
                v
            };
            return crate::semantics::builtins::call_builtin(id, vals);
        }

        // User-defined function lookup — extract metadata without cloning params
        if let Some(func) = self.ctx.functions.get(&function) {
            let param_count = func.params.len();
            let body = func.body;

            if args.len() != param_count {
                return Err(format!(
                    "Function {} expects {} arguments, got {}",
                    self.ctx.interner.resolve(function),
                    param_count,
                    args.len()
                ));
            }

            // Evaluate arguments before pushing scope
            let mut arg_values = Vec::with_capacity(param_count);
            for arg in args {
                arg_values.push(self.evaluate_expr(arg).await?);
            }

            // Bind parameters in a FRESH frame — the lexical barrier: the body
            // sees params, its own bindings, and globals; never caller locals.
            if self.task.call_depth >= crate::semantics::MAX_CALL_DEPTH {
            return Err(crate::semantics::CALL_DEPTH_ERR.to_string());
        }
        self.task.call_depth += 1;
        self.task.env.push_frame();
            for i in 0..param_count {
                let param_name = self.ctx.functions[&function].params[i].0;
                self.task.env.define(param_name, std::mem::replace(&mut arg_values[i], RuntimeValue::Nothing));
            }

            // Execute function body
            // TCO on the async path — mirror of the sync twin in
            // `call_function_sync` (constant-stack self-tail-calls).
            let prev_tco = self.task.tco_fn_async.replace(function);
            let prev_repeat = std::mem::replace(&mut self.task.repeat_depth_async, 0);
            let mut return_value = RuntimeValue::Nothing;
            let mut body_err = None;
            'tco: loop {
                self.task.pending_tail_call_async = None;
                let mut idx = 0;
                while idx < body.len() {
                    if idx + 1 < body.len() {
                        if let Some(call_args) = crate::tail_call::tail_pair_args(
                            &body[idx],
                            &body[idx + 1],
                            function,
                            param_count,
                        ) {
                            let mut vals = Vec::with_capacity(call_args.len());
                            let mut perr = None;
                            for a in call_args {
                                match self.evaluate_expr(a).await {
                                    Ok(v) => vals.push(v),
                                    Err(e) => {
                                        perr = Some(e);
                                        break;
                                    }
                                }
                            }
                            match perr {
                                Some(e) => body_err = Some(e),
                                None => self.task.pending_tail_call_async = Some(vals),
                            }
                            break;
                        }
                    }
                    match self.execute_stmt(&body[idx]).await {
                        Ok(ControlFlow::Return(val)) => {
                            return_value = val;
                            break;
                        }
                        Ok(ControlFlow::Break) => break,
                        Ok(ControlFlow::Continue) => {}
                        Err(e) => {
                            body_err = Some(e);
                            break;
                        }
                    }
                    idx += 1;
                }
                if body_err.is_some() {
                    break 'tco;
                }
                match self.task.pending_tail_call_async.take() {
                    Some(new_args) => {
                        self.task.env.pop_frame();
                        self.task.env.push_frame();
                        for (i, v) in new_args.into_iter().enumerate() {
                            let param_name = self.ctx.functions[&function].params[i].0;
                            self.task.env.define(param_name, v);
                        }
                        continue 'tco;
                    }
                    None => break 'tco,
                }
            }
            self.task.repeat_depth_async = prev_repeat;
            self.task.tco_fn_async = prev_tco;

            self.task.env.pop_frame();
        self.task.call_depth -= 1;
            match body_err {
                Some(e) => Err(e),
                None => Ok(return_value),
            }
        } else {
            // Fallback: check if the function name is a variable holding a closure
            let maybe_closure = self.task.env.lookup(function)
                .and_then(|v| if let RuntimeValue::Function(c) = v { Some((**c).clone()) } else { None });

            if let Some(closure) = maybe_closure {
                let mut arg_values = Vec::with_capacity(args.len());
                for arg in args {
                    arg_values.push(self.evaluate_expr(arg).await?);
                }
                self.call_closure_value(&closure, arg_values).await
            } else {
                Err(format!("Unknown function: {}", self.ctx.interner.resolve(function)))
            }
        }
    }

    /// Call a function with pre-evaluated RuntimeValue arguments.
    /// Used by Give and Show statements where the object is already evaluated.
    /// Build the suspend-future for a concurrency request. The returned future
    /// owns its `Rc` side-channel, so awaiting it does not borrow `self`.
    fn yield_request(&self, req: BlockingRequest<'a>) -> YieldFuture<'a> {
        let ys = self
            .yield_state
            .clone()
            .expect("concurrency op executed outside a scheduler context");
        ys.borrow_mut().request = Some(req);
        YieldFuture::new(ys)
    }

    /// Evaluate an expression to a channel handle.
    async fn eval_chan(&mut self, expr: &Expr<'a>) -> Result<ChanId, String> {
        match self.evaluate_expr(expr).await? {
            RuntimeValue::Chan(id) => Ok(id),
            other => Err(format!("expected a channel, found {}", other.type_name())),
        }
    }

    /// Evaluate an expression to a task handle.
    async fn eval_task(&mut self, expr: &Expr<'a>) -> Result<TaskId, String> {
        match self.evaluate_expr(expr).await? {
            RuntimeValue::TaskHandle(id) => Ok(id),
            other => Err(format!("expected a task handle, found {}", other.type_name())),
        }
    }

    /// Evaluate a `Select` timeout expression to a non-negative logical tick
    /// count for the scheduler's timer wheel. A bare integer is read as whole
    /// seconds (matching the compiled `Duration::from_secs`), a duration as its
    /// own magnitude, and a calendar span as whole seconds.
    async fn eval_select_timeout_ticks(&mut self, expr: &Expr<'a>) -> Result<u64, String> {
        let ticks = match self.evaluate_expr(expr).await? {
            RuntimeValue::Int(n) => n.max(0) as u64,
            RuntimeValue::Duration(d) => d.max(0) as u64,
            RuntimeValue::Span { months, days } => {
                (((months as i64) * 30 + days as i64) * 86_400).max(0) as u64
            }
            other => {
                return Err(format!(
                    "select timeout must be a number or duration, found {}",
                    other.type_name()
                ))
            }
        };
        Ok(ticks)
    }

    /// Build a child interpreter task that runs `function(args)`, sharing this
    /// interpreter's (cloned) context with a fresh per-task state + side-channel.
    fn spawn_child_task(
        &self,
        function: Symbol,
        args: Vec<RuntimeValue>,
    ) -> Box<dyn logicaffeine_runtime::Task<'a> + 'a> {
        let ys: Yield<'a> = Rc::new(RefCell::new(YieldState::new()));
        let mut child = Interpreter {
            ctx: self.ctx.clone(),
            task: TaskState::new(),
            output: Vec::new(),
            yield_state: Some(ys.clone()),
            netbox: crate::concurrency::net_inbox::NetInbox::new(),
        };
        let fut = Box::pin(async move {
            child.call_function_with_values(function, args).await.map(|_| ())
        });
        Box::new(InterpreterTask::new(fut, ys, None))
    }

    #[async_recursion(?Send)]
    async fn call_function_with_values(&mut self, function: Symbol, mut args: Vec<RuntimeValue>) -> Result<RuntimeValue, String> {
        // Handle built-in "show" via Symbol comparison
        if Some(function) == self.ctx.sym_show {
            for val in args {
                self.emit_output(val.to_display_string());
            }
            return Ok(RuntimeValue::Nothing);
        }

        if let Some(func) = self.ctx.functions.get(&function) {
            let param_count = func.params.len();
            let body = func.body;

            if args.len() != param_count {
                return Err(format!(
                    "Function {} expects {} arguments, got {}",
                    self.ctx.interner.resolve(function), param_count, args.len()
                ));
            }

            if self.task.call_depth >= crate::semantics::MAX_CALL_DEPTH {
            return Err(crate::semantics::CALL_DEPTH_ERR.to_string());
        }
        self.task.call_depth += 1;
        self.task.env.push_frame();
            for i in 0..param_count {
                let param_name = self.ctx.functions[&function].params[i].0;
                self.task.env.define(param_name, std::mem::replace(&mut args[i], RuntimeValue::Nothing));
            }

            // TCO on the async path — mirror of the sync twin in
            // `call_function_sync` (constant-stack self-tail-calls).
            let prev_tco = self.task.tco_fn_async.replace(function);
            let prev_repeat = std::mem::replace(&mut self.task.repeat_depth_async, 0);
            let mut return_value = RuntimeValue::Nothing;
            let mut body_err = None;
            'tco: loop {
                self.task.pending_tail_call_async = None;
                let mut idx = 0;
                while idx < body.len() {
                    if idx + 1 < body.len() {
                        if let Some(call_args) = crate::tail_call::tail_pair_args(
                            &body[idx],
                            &body[idx + 1],
                            function,
                            param_count,
                        ) {
                            let mut vals = Vec::with_capacity(call_args.len());
                            let mut perr = None;
                            for a in call_args {
                                match self.evaluate_expr(a).await {
                                    Ok(v) => vals.push(v),
                                    Err(e) => {
                                        perr = Some(e);
                                        break;
                                    }
                                }
                            }
                            match perr {
                                Some(e) => body_err = Some(e),
                                None => self.task.pending_tail_call_async = Some(vals),
                            }
                            break;
                        }
                    }
                    match self.execute_stmt(&body[idx]).await {
                        Ok(ControlFlow::Return(val)) => {
                            return_value = val;
                            break;
                        }
                        Ok(ControlFlow::Break) => break,
                        Ok(ControlFlow::Continue) => {}
                        Err(e) => {
                            body_err = Some(e);
                            break;
                        }
                    }
                    idx += 1;
                }
                if body_err.is_some() {
                    break 'tco;
                }
                match self.task.pending_tail_call_async.take() {
                    Some(new_args) => {
                        self.task.env.pop_frame();
                        self.task.env.push_frame();
                        for (i, v) in new_args.into_iter().enumerate() {
                            let param_name = self.ctx.functions[&function].params[i].0;
                            self.task.env.define(param_name, v);
                        }
                        continue 'tco;
                    }
                    None => break 'tco,
                }
            }
            self.task.repeat_depth_async = prev_repeat;
            self.task.tco_fn_async = prev_tco;

            self.task.env.pop_frame();
        self.task.call_depth -= 1;
            match body_err {
                Some(e) => Err(e),
                None => Ok(return_value),
            }
        } else {
            let maybe_closure = self.task.env.lookup(function)
                .and_then(|v| if let RuntimeValue::Function(c) = v { Some((**c).clone()) } else { None });

            if let Some(closure) = maybe_closure {
                self.call_closure_value(&closure, args).await
            } else {
                Err(format!("Unknown function: {}", self.ctx.interner.resolve(function)))
            }
        }
    }

    /// Map a function symbol to its kernel builtin, via the cached symbols.
    fn builtin_id(&self, f: Symbol) -> Option<crate::semantics::builtins::BuiltinId> {
        use crate::semantics::builtins::BuiltinId as B;
        let s = Some(f);
        if s == self.ctx.sym_length {
            Some(B::Length)
        } else if s == self.ctx.sym_format {
            Some(B::Format)
        } else if s == self.ctx.sym_parse_int {
            Some(B::ParseInt)
        } else if s == self.ctx.sym_parse_float {
            Some(B::ParseFloat)
        } else if s == self.ctx.sym_chr {
            Some(B::Chr)
        } else if s == self.ctx.sym_abs {
            Some(B::Abs)
        } else if s == self.ctx.sym_sqrt {
            Some(B::Sqrt)
        } else if s == self.ctx.sym_min {
            Some(B::Min)
        } else if s == self.ctx.sym_max {
            Some(B::Max)
        } else if s == self.ctx.sym_floor {
            Some(B::Floor)
        } else if s == self.ctx.sym_ceil {
            Some(B::Ceil)
        } else if s == self.ctx.sym_round {
            Some(B::Round)
        } else if s == self.ctx.sym_pow {
            Some(B::Pow)
        } else if s == self.ctx.sym_copy {
            Some(B::Copy)
        } else if s == self.ctx.sym_count_ones {
            Some(B::CountOnes)
        } else {
            // Fall back to name-based resolution for any builtin NOT pre-interned as a ctx
            // symbol (e.g. `run_accepted`). The pre-interned checks above are a fast path;
            // this keeps the tree-walker resolving exactly the builtin set the VM does via
            // `builtin_from_name` — cross-tier consistent, no silent "Unknown function".
            crate::semantics::builtins::builtin_from_name(self.ctx.interner.resolve(f))
        }
    }

    // Scope management

    fn push_scope(&mut self) {
        self.task.env.push_scope();
    }

    fn pop_scope(&mut self) {
        self.task.env.pop_scope();
    }

    fn define(&mut self, name: Symbol, value: RuntimeValue) {
        self.task.env.define(name, value);
    }

    fn assign(&mut self, name: Symbol, value: RuntimeValue) -> Result<(), String> {
        if self.task.env.assign(name, value) {
            Ok(())
        } else {
            Err(format!("Undefined variable: {}", self.ctx.interner.resolve(name)))
        }
    }

    fn lookup(&self, name: Symbol) -> Result<&RuntimeValue, String> {
        self.task.env.lookup(name)
            .ok_or_else(|| format!("Undefined variable: {}", self.ctx.interner.resolve(name)))
    }

    /// True if `sym` is a `mutable` parameter of the function whose body is
    /// currently executing. Such a parameter passes by reference (Mutable Value
    /// Semantics escape hatch), so its mutations must reach the caller's
    /// collection in place — copy-on-write is suppressed for it.
    fn is_mutable_param(&self, sym: Symbol) -> bool {
        let Some(fn_sym) = self.task.tco_fn_sync.or(self.task.tco_fn_async) else {
            return false;
        };
        let Some(fdef) = self.ctx.functions.get(&fn_sym) else {
            return false;
        };
        fdef.params
            .iter()
            .any(|(p, ty)| *p == sym && matches!(ty, crate::ast::stmt::TypeExpr::Mutable { .. }))
    }

    /// Copy-on-write for value semantics. Before mutating the collection bound to
    /// `sym`, ensure it is uniquely owned: if another binding shares the same
    /// allocation (`Rc` strong count > 1) — from `Let b be a`, a plain parameter,
    /// or storage inside another collection — replace `sym` with a deep copy so
    /// the mutation cannot be observed through that other binding. Sound: it
    /// never UNDER-copies (any alias bumps the count), so it can only ever copy
    /// when it might otherwise alias. A `mutable` parameter is exempt — it
    /// deliberately mutates the caller's collection in place.
    fn ensure_collection_owned(&mut self, sym: Symbol) {
        // MVS migration gate. Copy-on-write value semantics for collections must
        // land in LOCKSTEP across the tree-walker, VM, and AOT: the debug shadow
        // oracle (ui_bridge) cross-checks the VM against the tree-walker on every
        // program, so flipping one engine alone makes every aliasing-observable
        // program diverge. This tree-walker COW is implemented and validated;
        // it stays gated (off by default) until the VM + AOT flip together.
        if !crate::semantics::collections::value_semantics_enabled() {
            return;
        }
        if self.is_mutable_param(sym) {
            return;
        }
        let shared = match self.task.env.lookup(sym) {
            Some(RuntimeValue::List(rc)) => Rc::strong_count(rc) > 1,
            Some(RuntimeValue::Map(rc)) => Rc::strong_count(rc) > 1,
            Some(RuntimeValue::Set(rc)) => Rc::strong_count(rc) > 1,
            _ => false,
        };
        if shared {
            let owned = self.task.env.lookup(sym).map(|v| v.deep_clone());
            if let Some(owned) = owned {
                self.task.env.assign(sym, owned);
            }
        }
    }

    /// Evaluate a policy condition against a subject value.
    fn evaluate_policy_condition(
        &self,
        condition: &PolicyCondition,
        subject: &RuntimeValue,
        object: Option<&RuntimeValue>,
    ) -> bool {
        crate::semantics::policy::evaluate_policy_condition(
            self.ctx.policy_registry.as_ref(),
            self.ctx.interner,
            condition,
            subject,
            object,
        )
    }

    /// The program's global bindings after execution, as sorted
    /// `(name, type, value)` rows — the substrate for the REPL's `:vars`
    /// inspection (see [`crate::repl::ReplSession::vars`]).
    pub fn global_bindings(&self) -> Vec<(String, String, String)> {
        let mut rows: Vec<(String, String, String)> = self
            .task
            .env
            .globals
            .iter()
            .map(|(sym, value)| {
                (
                    self.ctx.interner.resolve(*sym).to_string(),
                    value.type_name().to_string(),
                    value.to_display_string(),
                )
            })
            .collect();
        rows.sort();
        rows
    }

    // =========================================================================
    // Sync execution path — eliminates async/Future overhead for pure programs
    // =========================================================================

    /// Execute a program synchronously (no async/Future allocation).
    /// Use when `needs_async(stmts)` returns false.
    pub fn run_sync(&mut self, stmts: &[Stmt<'a>]) -> Result<(), String> {
        // Hermetic program start: no ambient exchange rates carried in (mirrors a fresh AOT process).
        logicaffeine_base::money::clear_ambient_rates();
        for stmt in stmts {
            match self.execute_stmt_sync(stmt)? {
                ControlFlow::Return(_) => break,
                ControlFlow::Break => break,
                ControlFlow::Continue => {}
            }
        }
        Ok(())
    }

    fn execute_stmt_sync(&mut self, stmt: &Stmt<'a>) -> Result<ControlFlow, String> {
        match stmt {
            Stmt::Let { var, value, .. } => {
                let val = self.evaluate_expr_sync(value)?;
                self.define(*var, val);
                Ok(ControlFlow::Continue)
            }

            Stmt::Set { target, value } => {
                let val = self.evaluate_expr_sync(value)?;
                self.assign(*target, val)?;
                Ok(ControlFlow::Continue)
            }

            Stmt::Call { function, args } => {
                self.call_function_sync(*function, args)?;
                Ok(ControlFlow::Continue)
            }

            Stmt::If { cond, then_block, else_block } => {
                let condition = self.evaluate_expr_sync(cond)?;
                if condition.is_truthy() {
                    let flow = self.execute_block_sync(then_block)?;
                    if !matches!(flow, ControlFlow::Continue) {
                        return Ok(flow);
                    }
                } else if let Some(else_stmts) = else_block {
                    let flow = self.execute_block_sync(else_stmts)?;
                    if !matches!(flow, ControlFlow::Continue) {
                        return Ok(flow);
                    }
                }
                Ok(ControlFlow::Continue)
            }

            Stmt::While { cond, body, .. } => {
                loop {
                    let condition = self.evaluate_expr_sync(cond)?;
                    if !condition.is_truthy() {
                        break;
                    }
                    match self.execute_block_sync(body)? {
                        ControlFlow::Break => break,
                        ControlFlow::Return(v) => return Ok(ControlFlow::Return(v)),
                        ControlFlow::Continue => {}
                    }
                }
                Ok(ControlFlow::Continue)
            }

            Stmt::Repeat { pattern, iterable, body } => {
                use crate::ast::stmt::Pattern;

                let iter_val = self.evaluate_expr_sync(iterable)?;
                let items = crate::semantics::collections::iteration_snapshot(&iter_val)?;

                self.push_scope();
                // A `Repeat` owns a live iterator: suppress TCO of any self-call
                // detected inside it (jumping to the body start would abandon the
                // iterator), matching the VM's `is_repeat` guard exactly.
                self.task.repeat_depth_sync += 1;
                for item in items {
                    match pattern {
                        Pattern::Identifier(sym) => {
                            self.define(*sym, item);
                        }
                        Pattern::Tuple(syms) => {
                            if let RuntimeValue::Tuple(ref tuple_vals) = item {
                                if syms.len() != tuple_vals.len() {
                                    self.task.repeat_depth_sync -= 1;
                                    return Err(format!(
                                        "Cannot bind a {}-tuple to {} names",
                                        tuple_vals.len(),
                                        syms.len()
                                    ));
                                }
                                for (sym, val) in syms.iter().zip(tuple_vals.iter()) {
                                    self.define(*sym, val.clone());
                                }
                            } else {
                                self.task.repeat_depth_sync -= 1;
                                return Err(format!("Expected tuple for pattern, got {}", item.type_name()));
                            }
                        }
                    }

                    match self.execute_block_sync(body)? {
                        ControlFlow::Break => break,
                        ControlFlow::Return(v) => {
                            self.task.repeat_depth_sync -= 1;
                            self.pop_scope();
                            return Ok(ControlFlow::Return(v));
                        }
                        ControlFlow::Continue => {}
                    }
                }
                self.task.repeat_depth_sync -= 1;
                self.pop_scope();
                Ok(ControlFlow::Continue)
            }

            Stmt::Return { value } => {
                // A direct self-tail-call `Return self(args)` becomes a loop-back
                // in `call_function_sync`: signal it via `pending_tail_call` and
                // return control to the body driver (which sees the sentinel and
                // restarts instead of using the value). Args are evaluated here —
                // a nested self-call in an argument stays ordinary recursion.
                if let Some(expr) = value {
                    if let Some(call_args) = self.self_tail_call_args_sync(*expr) {
                        let mut vals = Vec::with_capacity(call_args.len());
                        for a in call_args {
                            vals.push(self.evaluate_expr_sync(a)?);
                        }
                        self.task.pending_tail_call = Some(vals);
                        return Ok(ControlFlow::Return(RuntimeValue::Nothing));
                    }
                }
                let ret_val = match value {
                    Some(expr) => self.evaluate_expr_sync(expr)?,
                    None => RuntimeValue::Nothing,
                };
                Ok(ControlFlow::Return(ret_val))
            }

            Stmt::Break => Ok(ControlFlow::Break),

            Stmt::FunctionDef { name, params, body, return_type, .. } => {
                let func = FunctionDef {
                    params: params.clone(),
                    body: *body,
                    return_type: *return_type,
                };
                self.ctx.functions.insert(*name, func);
                Ok(ControlFlow::Continue)
            }

            Stmt::StructDef { name, fields, .. } => {
                self.ctx.struct_defs.insert(*name, fields.clone());
                Ok(ControlFlow::Continue)
            }

            Stmt::SetField { object, field, value } => {
                let new_val = self.evaluate_expr_sync(value)?;
                if let Expr::Identifier(obj_sym) = object {
                    let mut obj_val = self.lookup(*obj_sym)?.clone();
                    if let RuntimeValue::Struct(ref mut s) = obj_val {
                        let field_name = self.ctx.interner.resolve(*field).to_string();
                        s.fields.insert(field_name, new_val);
                        self.assign(*obj_sym, obj_val)?;
                    } else {
                        return Err(format!("Cannot set field on non-struct value"));
                    }
                } else {
                    return Err("SetField target must be an identifier".to_string());
                }
                Ok(ControlFlow::Continue)
            }

            Stmt::Push { value, collection } => {
                let val = self.evaluate_expr_sync(value)?;
                if let Expr::Identifier(coll_sym) = collection {
                    self.ensure_collection_owned(*coll_sym);
                    let coll_val = self.lookup(*coll_sym)?;
                    crate::semantics::collections::list_push(&coll_val, val)?;
                } else if let Expr::FieldAccess { object, field } = collection {
                    if let Expr::Identifier(obj_sym) = *object {
                        let obj_val = self.lookup(*obj_sym)?;
                        let field_name = self.ctx.interner.resolve(*field);
                        crate::semantics::collections::push_to_struct_field(&obj_val, field_name, val)?;
                    } else {
                        return Err("Push to nested field access not supported".to_string());
                    }
                } else {
                    // Any place expression is an l-value; see the async Push
                    // handler for the aliasing rationale.
                    let coll_val = self.evaluate_expr_sync(collection)?;
                    crate::semantics::collections::list_push(&coll_val, val)?;
                }
                Ok(ControlFlow::Continue)
            }

            Stmt::Pop { collection, into } => {
                if let Expr::Identifier(coll_sym) = collection {
                    self.ensure_collection_owned(*coll_sym);
                    let coll_val = self.lookup(*coll_sym)?;
                    let popped = crate::semantics::collections::list_pop(&coll_val)?;
                    if let Some(into_var) = into {
                        self.define(*into_var, popped);
                    }
                } else {
                    return Err("Pop collection must be an identifier".to_string());
                }
                Ok(ControlFlow::Continue)
            }

            Stmt::Add { value, collection } => {
                let val = self.evaluate_expr_sync(value)?;
                if let Expr::Identifier(coll_sym) = collection {
                    self.ensure_collection_owned(*coll_sym);
                }
                // Resolve the collection generally — a bare variable or a (CRDT/Set)
                // struct field, e.g. `Add "Alice" to p's guests`.
                let coll_val = self.evaluate_expr_sync(collection)?;
                crate::semantics::collections::set_add(&coll_val, val)?;
                Ok(ControlFlow::Continue)
            }

            Stmt::Remove { value, collection } => {
                let val = self.evaluate_expr_sync(value)?;
                if let Expr::Identifier(coll_sym) = collection {
                    self.ensure_collection_owned(*coll_sym);
                }
                let coll_val = self.evaluate_expr_sync(collection)?;
                crate::semantics::collections::remove_from(&coll_val, &val)?;
                Ok(ControlFlow::Continue)
            }

            Stmt::SetIndex { collection, index, value } => {
                let idx_val = self.evaluate_expr_sync(index)?;
                let new_val = self.evaluate_expr_sync(value)?;
                if let Expr::Identifier(coll_sym) = collection {
                    // Struct field set via index syntax (mirrors the read side); see the
                    // async SetIndex handler for rationale.
                    if let RuntimeValue::Text(field) = &idx_val {
                        let cur = self.lookup(*coll_sym)?.clone();
                        if let RuntimeValue::Struct(mut s) = cur {
                            s.fields.insert(field.to_string(), new_val);
                            self.assign(*coll_sym, RuntimeValue::Struct(s))?;
                            return Ok(ControlFlow::Continue);
                        }
                    }
                    self.ensure_collection_owned(*coll_sym);
                    let coll_val = self.lookup(*coll_sym)?;
                    crate::semantics::collections::index_set(&coll_val, &idx_val, new_val)?;
                } else {
                    // Any place expression is an l-value; see the async
                    // SetIndex handler for rationale.
                    let coll_val = self.evaluate_expr_sync(collection)?;
                    crate::semantics::collections::index_set(&coll_val, &idx_val, new_val)?;
                }
                Ok(ControlFlow::Continue)
            }

            Stmt::Splice { body } => {
                // Scope-transparent; see the async Splice handler.
                for s in body.iter() {
                    let flow = self.execute_stmt_sync(s)?;
                    if !matches!(flow, ControlFlow::Continue) {
                        return Ok(flow);
                    }
                }
                Ok(ControlFlow::Continue)
            }

            Stmt::Inspect { target, arms, .. } => {
                let target_val = self.evaluate_expr_sync(target)?;
                self.execute_inspect_sync(&target_val, arms)
            }

            Stmt::Zone { name, body, .. } => {
                self.push_scope();
                self.define(*name, RuntimeValue::Nothing);
                let result = self.execute_block_sync(body);
                self.pop_scope();
                result?;
                Ok(ControlFlow::Continue)
            }

            Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => {
                for task in tasks.iter() {
                    self.execute_stmt_sync(task)?;
                }
                Ok(ControlFlow::Continue)
            }

            Stmt::Assert { .. } | Stmt::Trust { .. } => {
                Ok(ControlFlow::Continue)
            }

            Stmt::RuntimeAssert { condition, .. } => {
                let val = self.evaluate_expr_sync(condition)?;
                if !val.is_truthy() {
                    return Err("Assertion failed".to_string());
                }
                Ok(ControlFlow::Continue)
            }

            Stmt::Give { object, recipient } => {
                let obj_val = self.evaluate_expr_sync(object)?;
                if let Expr::Identifier(sym) = recipient {
                    self.call_function_with_values_sync(*sym, vec![obj_val])?;
                }
                Ok(ControlFlow::Continue)
            }

            Stmt::Show { object, recipient } => {
                let obj_val = self.evaluate_expr_sync(object)?;
                if let Expr::Identifier(sym) = recipient {
                    let name = self.ctx.interner.resolve(*sym);
                    if name == "show" {
                        self.emit_output(obj_val.to_display_string());
                    } else {
                        self.call_function_with_values_sync(*sym, vec![obj_val])?;
                    }
                }
                Ok(ControlFlow::Continue)
            }

            // Async-only operations — unreachable in sync path (checked by needs_async)
            Stmt::ReadFrom { var, source } => {
                match source {
                    ReadSource::Console => {
                        self.define(*var, RuntimeValue::Text(Rc::new(String::new())));
                        Ok(ControlFlow::Continue)
                    }
                    ReadSource::File(_) => {
                        Err("File read requires async execution path".to_string())
                    }
                }
            }

            Stmt::WriteFile { .. } => {
                Err("File write requires async execution path".to_string())
            }

            Stmt::Spawn { name, .. } => {
                self.define(*name, RuntimeValue::Nothing);
                Ok(ControlFlow::Continue)
            }

            Stmt::SendMessage { .. } => {
                Err("Send (peer messaging) requires the async execution path".to_string())
            }
            Stmt::StreamMessage { .. } => {
                Err("Stream (batch peer messaging) requires the async execution path".to_string())
            }

            Stmt::AwaitMessage { .. } => {
                Err("Await (peer messaging) requires the async execution path".to_string())
            }

            Stmt::MergeCrdt { source, target } => {
                let source_val = self.evaluate_expr_sync(source)?;
                let source_fields = match &source_val {
                    RuntimeValue::Struct(s) => s.fields.clone(),
                    _ => return Err("Merge source must be a struct".to_string()),
                };

                if let Expr::Identifier(target_sym) = target {
                    let mut target_val = self.lookup(*target_sym)?.clone();

                    if let RuntimeValue::Struct(ref mut s) = target_val {
                        for (field_name, source_field_val) in source_fields {
                            let current = s.fields.get(&field_name)
                                .cloned()
                                .unwrap_or(RuntimeValue::Int(0));

                            let merged =
                                crate::semantics::arith::crdt_merge_field(&current, source_field_val);
                            s.fields.insert(field_name, merged);
                        }
                        self.assign(*target_sym, target_val)?;
                    } else {
                        return Err("Merge target must be a struct".to_string());
                    }
                } else {
                    return Err("Merge target must be an identifier".to_string());
                }
                Ok(ControlFlow::Continue)
            }

            Stmt::IncreaseCrdt { object, field, amount } => {
                let amount_val = self.evaluate_expr_sync(amount)?;
                let amount_int = match amount_val {
                    RuntimeValue::Int(n) => n,
                    _ => return Err("CRDT increment amount must be an integer".to_string()),
                };

                if let Expr::Identifier(obj_sym) = object {
                    let mut obj_val = self.lookup(*obj_sym)?.clone();

                    if let RuntimeValue::Struct(ref mut s) = obj_val {
                        let field_name = self.ctx.interner.resolve(*field).to_string();
                        let current = s.fields.get(&field_name)
                            .cloned()
                            .unwrap_or(RuntimeValue::Int(0));

                        let new_val =
                            crate::semantics::arith::crdt_counter_bump(current, amount_int, &field_name)?;
                        s.fields.insert(field_name, new_val);
                        self.assign(*obj_sym, obj_val)?;
                    } else {
                        return Err("Cannot increase field on non-struct value".to_string());
                    }
                } else {
                    return Err("IncreaseCrdt target must be an identifier".to_string());
                }
                Ok(ControlFlow::Continue)
            }

            Stmt::DecreaseCrdt { object, field, amount } => {
                let amount_val = self.evaluate_expr_sync(amount)?;
                let amount_int = match amount_val {
                    RuntimeValue::Int(n) => n,
                    _ => return Err("CRDT decrement amount must be an integer".to_string()),
                };

                if let Expr::Identifier(obj_sym) = object {
                    let mut obj_val = self.lookup(*obj_sym)?.clone();

                    if let RuntimeValue::Struct(ref mut s) = obj_val {
                        let field_name = self.ctx.interner.resolve(*field).to_string();
                        let current = s.fields.get(&field_name)
                            .cloned()
                            .unwrap_or(RuntimeValue::Int(0));

                        let new_val = crate::semantics::arith::crdt_counter_bump(
                            current,
                            amount_int.wrapping_neg(),
                            &field_name,
                        )?;
                        s.fields.insert(field_name, new_val);
                        self.assign(*obj_sym, obj_val)?;
                    } else {
                        return Err("Cannot decrease field on non-struct value".to_string());
                    }
                } else {
                    return Err("DecreaseCrdt target must be an identifier".to_string());
                }
                Ok(ControlFlow::Continue)
            }

            Stmt::AppendToSequence { sequence, value } => {
                let val = self.evaluate_expr_sync(value)?;
                let seq_val = self.evaluate_expr_sync(sequence)?;
                match &seq_val {
                    RuntimeValue::Crdt(rc) => rc.borrow_mut().append(&val)?,
                    RuntimeValue::List(_) => {
                        crate::semantics::collections::list_push(&seq_val, val)?
                    }
                    _ => return Err(format!("Cannot append to {}", seq_val.type_name())),
                }
                Ok(ControlFlow::Continue)
            }

            Stmt::ResolveConflict { object, field, value } => {
                let val = self.evaluate_expr_sync(value)?;
                let obj_val = self.evaluate_expr_sync(object)?;
                let field_name = self.ctx.interner.resolve(*field);
                if let RuntimeValue::Struct(s) = &obj_val {
                    if let Some(RuntimeValue::Crdt(rc)) = s.fields.get(field_name) {
                        rc.borrow_mut().resolve(&val)?;
                        return Ok(ControlFlow::Continue);
                    }
                }
                if let Expr::Identifier(obj_sym) = object {
                    let mut owner = self.lookup(*obj_sym)?.clone();
                    if let RuntimeValue::Struct(ref mut s) = owner {
                        s.fields.insert(field_name.to_string(), val);
                        self.assign(*obj_sym, owner)?;
                        return Ok(ControlFlow::Continue);
                    }
                }
                Err("Resolve target must be a struct field".to_string())
            }

            Stmt::Check { subject, predicate, is_capability, object, source_text, .. } => {
                let registry = match &self.ctx.policy_registry {
                    Some(r) => r,
                    None => return Err("Security Check requires policies. Use compiled Rust or add ## Policy block.".to_string()),
                };

                let subj_val = self.lookup(*subject)?.clone();
                // The object is only consulted (and only looked up) for
                // capability checks.
                let obj_val = if *is_capability {
                    match object {
                        Some(obj_sym) => Some(self.lookup(*obj_sym)?.clone()),
                        None => None,
                    }
                } else {
                    None
                };
                crate::semantics::policy::check_policy(
                    registry,
                    self.ctx.interner,
                    &subj_val,
                    *predicate,
                    *is_capability,
                    obj_val.as_ref(),
                    source_text,
                )?;
                Ok(ControlFlow::Continue)
            }

            Stmt::Listen { .. } | Stmt::ConnectTo { .. } => {
                Err("Networking (Connect/Listen) requires the async execution path".to_string())
            }
            // A PeerAgent handle is pure (just its canonical topic), so it works
            // outside the async path; the `Send`/`Await` that use it do not.
            Stmt::LetPeerAgent { var, address } => {
                let raw = self.evaluate_expr_sync(address)?.to_display_string();
                let topic = logicaffeine_system::addr::canonical_topic(&raw);
                self.define(*var, RuntimeValue::Peer(Rc::new(topic)));
                Ok(ControlFlow::Continue)
            }
            Stmt::Sleep { .. } => {
                Err("Sleep requires async execution path".to_string())
            }
            Stmt::Sync { .. } => {
                Err("Sync requires the async execution path".to_string())
            }
            Stmt::Mount { .. } => {
                Err("Mount requires async execution path".to_string())
            }

            Stmt::LaunchTask { .. } |
            Stmt::LaunchTaskWithHandle { .. } |
            Stmt::CreatePipe { .. } |
            Stmt::SendPipe { .. } |
            Stmt::ReceivePipe { .. } |
            Stmt::TrySendPipe { .. } |
            Stmt::TryReceivePipe { .. } |
            Stmt::StopTask { .. } |
            Stmt::Select { .. } => {
                Err("Go-like concurrency (Launch, Pipe, Select) is only supported in compiled mode".to_string())
            }

            Stmt::Escape { .. } => {
                Err(
                    "Escape blocks contain raw Rust code and cannot be interpreted. \
                     Use `largo build` or `largo run` to compile and run this program."
                    .to_string()
                )
            }

            Stmt::Require { .. } => {
                Ok(ControlFlow::Continue)
            }

            Stmt::Theorem(_) | Stmt::Definition(_) | Stmt::Axiom(_) | Stmt::Theory(_) => {
                Ok(ControlFlow::Continue)
            }
        }
    }

    fn execute_block_sync(&mut self, block: Block<'a>) -> Result<ControlFlow, String> {
        self.push_scope();
        for stmt in block.iter() {
            match self.execute_stmt_sync(stmt)? {
                ControlFlow::Continue => {}
                flow => {
                    self.pop_scope();
                    return Ok(flow);
                }
            }
        }
        self.pop_scope();
        Ok(ControlFlow::Continue)
    }

    fn execute_inspect_sync(&mut self, target: &RuntimeValue, arms: &[MatchArm<'a>]) -> Result<ControlFlow, String> {
        for arm in arms {
            if arm.variant.is_none() {
                let flow = self.execute_block_sync(arm.body)?;
                return Ok(flow);
            }

            match target {
                RuntimeValue::Struct(s) => {
                    if let Some(variant) = arm.variant {
                        let variant_name = self.ctx.interner.resolve(variant);
                        if s.type_name == variant_name {
                            self.push_scope();
                            for (field_name, binding_name) in &arm.bindings {
                                let field_str = self.ctx.interner.resolve(*field_name);
                                if let Some(val) = s.fields.get(field_str) {
                                    self.define(*binding_name, val.clone());
                                }
                            }
                            let result = self.execute_block_sync(arm.body);
                            self.pop_scope();
                            let flow = result?;
                            return Ok(flow);
                        }
                    }
                }

                RuntimeValue::Inductive(ind) => {
                    if let Some(variant) = arm.variant {
                        let variant_name = self.ctx.interner.resolve(variant);
                        if ind.constructor == variant_name {
                            self.push_scope();
                            for (i, (_, binding_name)) in arm.bindings.iter().enumerate() {
                                if i < ind.args.len() {
                                    self.define(*binding_name, ind.args[i].clone());
                                }
                            }
                            let result = self.execute_block_sync(arm.body);
                            self.pop_scope();
                            let flow = result?;
                            return Ok(flow);
                        }
                    }
                }

                _ => {}
            }
        }
        Err(self.inspect_unhandled(target))
    }

    fn evaluate_expr_sync(&mut self, expr: &Expr<'a>) -> Result<RuntimeValue, String> {
        match expr {
            Expr::Literal(lit) => self.evaluate_literal(lit),

            Expr::Identifier(sym) => {
                let name = self.ctx.interner.resolve(*sym);
                // Handle temporal builtins (the NAME wins, even when shadowed)
                match name {
                    "today" => {
                        return Ok(crate::semantics::temporal::today());
                    }
                    "now" => {
                        return Ok(crate::semantics::temporal::now());
                    }
                    _ => {}
                }
                self.lookup(*sym).cloned()
            }

            Expr::BinaryOp { op, left, right } => {
                match op {
                    BinaryOpKind::And => {
                        let left_val = self.evaluate_expr_sync(left)?;
                        if !left_val.is_truthy() {
                            return Ok(RuntimeValue::Bool(false));
                        }
                        let right_val = self.evaluate_expr_sync(right)?;
                        Ok(RuntimeValue::Bool(right_val.is_truthy()))
                    }
                    BinaryOpKind::Or => {
                        let left_val = self.evaluate_expr_sync(left)?;
                        if left_val.is_truthy() {
                            return Ok(RuntimeValue::Bool(true));
                        }
                        let right_val = self.evaluate_expr_sync(right)?;
                        Ok(RuntimeValue::Bool(right_val.is_truthy()))
                    }
                    _ => {
                        let left_val = self.evaluate_expr_sync(left)?;
                        let right_val = self.evaluate_expr_sync(right)?;
                        self.apply_binary_op(*op, left_val, right_val)
                    }
                }
            }

            Expr::Call { function, args } => {
                self.call_function_sync(*function, args)
            }

            Expr::Index { collection, index } => {
                let coll_val = self.evaluate_expr_sync(collection)?;
                let idx_val = self.evaluate_expr_sync(index)?;
                crate::semantics::collections::index_get(&coll_val, &idx_val)
            }

            Expr::Slice { collection, start, end } => {
                let coll_val = self.evaluate_expr_sync(collection)?;
                let start_val = self.evaluate_expr_sync(start)?;
                let end_val = self.evaluate_expr_sync(end)?;
                crate::semantics::collections::slice(&coll_val, &start_val, &end_val)
            }

            Expr::Copy { expr: inner } => {
                let val = self.evaluate_expr_sync(inner)?;
                Ok(val.deep_clone())
            }

            Expr::Give { value } => {
                self.evaluate_expr_sync(value)
            }

            Expr::Length { collection } => {
                let coll_val = self.evaluate_expr_sync(collection)?;
                crate::semantics::collections::length_of(&coll_val)
            }

            Expr::Contains { collection, value } => {
                let coll_val = self.evaluate_expr_sync(collection)?;
                let val = self.evaluate_expr_sync(value)?;
                crate::semantics::collections::contains(&coll_val, &val)
            }

            Expr::Union { left, right } => {
                let left_val = self.evaluate_expr_sync(left)?;
                let right_val = self.evaluate_expr_sync(right)?;
                crate::semantics::collections::union(&left_val, &right_val)
            }

            Expr::Intersection { left, right } => {
                let left_val = self.evaluate_expr_sync(left)?;
                let right_val = self.evaluate_expr_sync(right)?;
                crate::semantics::collections::intersection(&left_val, &right_val)
            }

            Expr::List(items) => {
                let mut values = Vec::with_capacity(items.len());
                for e in items.iter() {
                    values.push(self.evaluate_expr_sync(e)?);
                }
                Ok(RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(values)))))
            }

            Expr::Tuple(items) => {
                let mut values = Vec::with_capacity(items.len());
                for e in items.iter() {
                    values.push(self.evaluate_expr_sync(e)?);
                }
                Ok(RuntimeValue::Tuple(Rc::new(values)))
            }

            Expr::Range { start, end } => {
                let start_val = self.evaluate_expr_sync(start)?;
                let end_val = self.evaluate_expr_sync(end)?;
                crate::semantics::collections::range(&start_val, &end_val)
            }

            Expr::FieldAccess { object, field } => {
                let obj_val = self.evaluate_expr_sync(object)?;
                match &obj_val {
                    RuntimeValue::Struct(s) => {
                        let field_name = self.ctx.interner.resolve(*field);
                        s.fields.get(field_name).cloned()
                            .ok_or_else(|| format!("Field '{}' not found", field_name))
                    }
                    _ => Err(format!("Cannot access field on {}", obj_val.type_name())),
                }
            }

            Expr::New { type_name, init_fields, .. } => {
                let name = self.ctx.interner.resolve(*type_name).to_string();

                if name == "Seq" || name == "List" {
                    return Ok(RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(Vec::new())))));
                }

                if name == "Set" || name == "HashSet" {
                    return Ok(RuntimeValue::Set(Rc::new(RefCell::new(vec![]))));
                }

                if name == "Map" || name == "HashMap" {
                    return Ok(RuntimeValue::Map(Rc::new(RefCell::new(MapStorage::default()))));
                }

                let mut fields = HashMap::new();
                for (field_sym, field_expr) in init_fields {
                    let field_name = self.ctx.interner.resolve(*field_sym).to_string();
                    let field_val = self.evaluate_expr_sync(field_expr)?;
                    fields.insert(field_name, field_val);
                }

                if let Some(def) = self.ctx.struct_defs.get(type_name) {
                    for (field_sym, type_sym, _) in def {
                        let field_name = self.ctx.interner.resolve(*field_sym).to_string();
                        if !fields.contains_key(&field_name) {
                            let type_name_str = self.ctx.interner.resolve(*type_sym).to_string();
                            let default = match type_name_str.as_str() {
                                "Int" => RuntimeValue::Int(0),
                                "Float" => RuntimeValue::Float(0.0),
                                "Bool" => RuntimeValue::Bool(false),
                                "Text" | "String" => RuntimeValue::Text(Rc::new(String::new())),
                                "Char" => RuntimeValue::Char('\0'),
                                "Byte" => RuntimeValue::Int(0),
                                "Seq" | "List" => RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(Vec::new())))),
                                "Set" | "HashSet" => RuntimeValue::Set(Rc::new(RefCell::new(vec![]))),
                                "Map" | "HashMap" => RuntimeValue::Map(Rc::new(RefCell::new(MapStorage::default()))),
                                // A `Shared` struct's CRDT fields default to an empty live
                                // CRDT, mirroring the compiled tier's `ORSet`/`RGA` field.
                                "SharedSet" | "ORSet" | "SharedSet_AddWins" =>
                                    RuntimeValue::Crdt(Rc::new(RefCell::new(
                                        crate::semantics::crdt::CrdtValue::new_set(
                                            crate::semantics::crdt::next_replica_id())))),
                                "SharedSet_RemoveWins" =>
                                    RuntimeValue::Crdt(Rc::new(RefCell::new(
                                        crate::semantics::crdt::CrdtValue::new_set_remove_wins(
                                            crate::semantics::crdt::next_replica_id())))),
                                "SharedSequence" | "RGA" | "SharedSequence_YATA" | "CollaborativeSequence" =>
                                    RuntimeValue::Crdt(Rc::new(RefCell::new(
                                        crate::semantics::crdt::CrdtValue::new_seq(
                                            crate::semantics::crdt::next_replica_id())))),
                                _ => RuntimeValue::Nothing,
                            };
                            fields.insert(field_name, default);
                        }
                    }
                }

                Ok(RuntimeValue::Struct(Box::new(StructValue { type_name: name, fields })))
            }

            Expr::NewVariant { enum_name, variant, fields } => {
                let inductive_type = self.ctx.interner.resolve(*enum_name).to_string();
                let constructor = self.ctx.interner.resolve(*variant).to_string();

                let mut args = Vec::new();
                for (_, field_expr) in fields {
                    let field_val = self.evaluate_expr_sync(field_expr)?;
                    args.push(field_val);
                }

                Ok(RuntimeValue::Inductive(Box::new(InductiveValue {
                    inductive_type,
                    constructor,
                    args,
                })))
            }

            Expr::ManifestOf { .. } => {
                Ok(RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(Vec::new())))))
            }

            Expr::ChunkAt { .. } => {
                Ok(RuntimeValue::Nothing)
            }

            Expr::WithCapacity { value, .. } => {
                self.evaluate_expr_sync(value)
            }

            Expr::OptionSome { value } => {
                self.evaluate_expr_sync(value)
            }

            Expr::OptionNone => {
                Ok(RuntimeValue::Nothing)
            }

            Expr::Not { operand } => {
                let val = self.evaluate_expr_sync(operand)?;
                crate::semantics::arith::not_value(val)
            }

            Expr::InterpolatedString(parts) => {
                let mut result = String::new();
                for part in parts {
                    match part {
                        crate::ast::stmt::StringPart::Literal(sym) => {
                            result.push_str(self.ctx.interner.resolve(*sym));
                        }
                        crate::ast::stmt::StringPart::Expr { value, format_spec, debug } => {
                            let val = self.evaluate_expr_sync(value)?;
                            if *debug {
                                let prefix = match value {
                                    Expr::Identifier(sym) => self.ctx.interner.resolve(*sym).to_string(),
                                    _ => "expr".to_string(),
                                };
                                result.push_str(&prefix);
                                result.push('=');
                            }
                            if let Some(spec_sym) = format_spec {
                                let spec = self.ctx.interner.resolve(*spec_sym);
                                result.push_str(&apply_format_spec(&val, spec));
                            } else {
                                result.push_str(&val.to_display_string());
                            }
                        }
                    }
                }
                Ok(RuntimeValue::Text(Rc::new(result)))
            }

            Expr::Escape { .. } => {
                Err("Escape expressions contain raw Rust code and cannot be interpreted. \
                     Use `largo build` or `largo run` to compile and run this program.".to_string())
            }

            Expr::Closure { params, body, .. } => {
                let free_vars = self.collect_free_vars_in_closure(params, body);
                let mut captured_env = HashMap::new();
                for sym in &free_vars {
                    if let Some(val) = self.task.env.lookup(*sym) {
                        captured_env.insert(*sym, val.deep_clone());
                    }
                }

                let body_index = self.ctx.closure_bodies.len();
                match body {
                    ClosureBody::Expression(expr) => {
                        self.ctx.closure_bodies.push(ClosureBodyRef::Expression(expr));
                    }
                    ClosureBody::Block(block) => {
                        self.ctx.closure_bodies.push(ClosureBodyRef::Block(block));
                    }
                }

                let param_names: Vec<Symbol> = params.iter().map(|(name, _)| *name).collect();

                Ok(RuntimeValue::Function(Box::new(ClosureValue {
                    body_index,
                    captured_env,
                    param_names,
                    generated: None,
                })))
            }

            Expr::CallExpr { callee, args } => {
                let callee_val = self.evaluate_expr_sync(callee)?;
                if let RuntimeValue::Function(closure) = callee_val {
                    let mut arg_values = Vec::with_capacity(args.len());
                    for arg in args.iter() {
                        arg_values.push(self.evaluate_expr_sync(arg)?);
                    }
                    self.call_closure_value_sync(&closure, arg_values)
                } else {
                    Err(format!("Cannot call value of type {}", callee_val.type_name()))
                }
            }
        }
    }

    /// If `expr` is a direct self-tail-call — `self(args)` of the function the
    /// SYNC path is currently executing, with matching arity and not inside a
    /// `Repeat` — return its argument expressions for `call_function_sync` to
    /// evaluate and loop on. `None` leaves the `Return` an ordinary one.
    fn self_tail_call_args_sync(&self, expr: &'a Expr<'a>) -> Option<&'a [&'a Expr<'a>]> {
        if self.task.repeat_depth_sync != 0 {
            return None;
        }
        let cur = self.task.tco_fn_sync?;
        let param_count = self.ctx.functions.get(&cur)?.params.len();
        crate::tail_call::direct_self_tail_args(expr, cur, param_count)
    }

    /// ASYNC twin of [`Self::self_tail_call_args_sync`].
    fn self_tail_call_args_async(&self, expr: &'a Expr<'a>) -> Option<&'a [&'a Expr<'a>]> {
        if self.task.repeat_depth_async != 0 {
            return None;
        }
        let cur = self.task.tco_fn_async?;
        let param_count = self.ctx.functions.get(&cur)?.params.len();
        crate::tail_call::direct_self_tail_args(expr, cur, param_count)
    }

    fn call_function_sync(&mut self, function: Symbol, args: &[&Expr<'a>]) -> Result<RuntimeValue, String> {
        // Built-in functions — Symbol comparison (integer) instead of string matching
        let func_sym = Some(function);
        if func_sym == self.ctx.sym_show {
            for arg in args {
                let val = self.evaluate_expr_sync(arg)?;
                self.emit_output(val.to_display_string());
            }
            return Ok(RuntimeValue::Nothing);
        } else if func_sym == self.ctx.sym_args {
            // `args()` system native: the stored argv as a `Seq of Text`,
            // mirroring the compiled binary's `env::args()`. Must match the
            // async path AND the VM (the shadow oracle asserts VM ≡ tree-walker).
            let items: Vec<RuntimeValue> = self
                .ctx
                .program_args
                .iter()
                .map(|s| RuntimeValue::Text(Rc::new(s.clone())))
                .collect();
            return Ok(RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(items)))));
        } else if let Some(id) = self.builtin_id(function) {
            // Arity is checked BEFORE evaluating arguments (kernel rule).
            crate::semantics::builtins::check_arity(id, args.len())?;
            // `format` reads only its first argument; preserve its laziness.
            let vals = if id == crate::semantics::builtins::BuiltinId::Format {
                match args.first() {
                    Some(a) => vec![self.evaluate_expr_sync(a)?],
                    None => Vec::new(),
                }
            } else {
                let mut v = Vec::with_capacity(args.len());
                for arg in args {
                    v.push(self.evaluate_expr_sync(arg)?);
                }
                v
            };
            return crate::semantics::builtins::call_builtin(id, vals);
        }

        // User-defined function lookup — extract metadata without cloning params
        if let Some(func) = self.ctx.functions.get(&function) {
            let param_count = func.params.len();
            let body = func.body;

            if args.len() != param_count {
                return Err(format!(
                    "Function {} expects {} arguments, got {}",
                    self.ctx.interner.resolve(function),
                    param_count,
                    args.len()
                ));
            }

            let mut arg_values = Vec::with_capacity(param_count);
            for arg in args {
                arg_values.push(self.evaluate_expr_sync(arg)?);
            }

            if self.task.call_depth >= crate::semantics::MAX_CALL_DEPTH {
            return Err(crate::semantics::CALL_DEPTH_ERR.to_string());
        }
        self.task.call_depth += 1;
        self.task.env.push_frame();
            for i in 0..param_count {
                let param_name = self.ctx.functions[&function].params[i].0;
                self.task.env.define(param_name, std::mem::replace(&mut arg_values[i], RuntimeValue::Nothing));
            }

            // TCO: while executing THIS function's body, a self-tail-call is a
            // loop-back (reassign params + restart the body) rather than a real
            // recursive call. `tco_fn_sync`/`repeat_depth_sync` are per-activation,
            // so save the caller's and reset for this body.
            let prev_tco = self.task.tco_fn_sync.replace(function);
            let prev_repeat = std::mem::replace(&mut self.task.repeat_depth_sync, 0);
            let mut return_value = RuntimeValue::Nothing;
            let mut body_err = None;
            'tco: loop {
                self.task.pending_tail_call = None;
                let mut idx = 0;
                while idx < body.len() {
                    // Top-level `Set/Let x to self(args); Return x` pair — a tail
                    // call. (A direct `Return self(args)` at any depth is caught
                    // in execute_stmt_sync's Return arm.)
                    if idx + 1 < body.len() {
                        if let Some(call_args) = crate::tail_call::tail_pair_args(
                            &body[idx],
                            &body[idx + 1],
                            function,
                            param_count,
                        ) {
                            let mut vals = Vec::with_capacity(call_args.len());
                            let mut perr = None;
                            for a in call_args {
                                match self.evaluate_expr_sync(a) {
                                    Ok(v) => vals.push(v),
                                    Err(e) => {
                                        perr = Some(e);
                                        break;
                                    }
                                }
                            }
                            match perr {
                                Some(e) => body_err = Some(e),
                                None => self.task.pending_tail_call = Some(vals),
                            }
                            break;
                        }
                    }
                    match self.execute_stmt_sync(&body[idx]) {
                        Ok(ControlFlow::Return(val)) => {
                            return_value = val;
                            break;
                        }
                        Ok(ControlFlow::Break) => break,
                        Ok(ControlFlow::Continue) => {}
                        Err(e) => {
                            body_err = Some(e);
                            break;
                        }
                    }
                    idx += 1;
                }
                if body_err.is_some() {
                    break 'tco;
                }
                match self.task.pending_tail_call.take() {
                    Some(new_args) => {
                        // Loop-back: a fresh frame (no stale locals) with the
                        // reassigned parameters — constant stack, no depth bump.
                        self.task.env.pop_frame();
                        self.task.env.push_frame();
                        for (i, v) in new_args.into_iter().enumerate() {
                            let param_name = self.ctx.functions[&function].params[i].0;
                            self.task.env.define(param_name, v);
                        }
                        continue 'tco;
                    }
                    None => break 'tco,
                }
            }
            self.task.repeat_depth_sync = prev_repeat;
            self.task.tco_fn_sync = prev_tco;

            self.task.env.pop_frame();
        self.task.call_depth -= 1;
            match body_err {
                Some(e) => Err(e),
                None => Ok(return_value),
            }
        } else {
            // Fallback: check if the function name is a variable holding a closure
            let maybe_closure = self.task.env.lookup(function)
                .and_then(|v| if let RuntimeValue::Function(c) = v { Some((**c).clone()) } else { None });

            if let Some(closure) = maybe_closure {
                let mut arg_values = Vec::with_capacity(args.len());
                for arg in args {
                    arg_values.push(self.evaluate_expr_sync(arg)?);
                }
                self.call_closure_value_sync(&closure, arg_values)
            } else {
                Err(format!("Unknown function: {}", self.ctx.interner.resolve(function)))
            }
        }
    }

    fn call_function_with_values_sync(&mut self, function: Symbol, mut args: Vec<RuntimeValue>) -> Result<RuntimeValue, String> {
        // Handle built-in "show" via Symbol comparison
        if Some(function) == self.ctx.sym_show {
            for val in args {
                self.emit_output(val.to_display_string());
            }
            return Ok(RuntimeValue::Nothing);
        }

        if let Some(func) = self.ctx.functions.get(&function) {
            let param_count = func.params.len();
            let body = func.body;

            if args.len() != param_count {
                return Err(format!(
                    "Function {} expects {} arguments, got {}",
                    self.ctx.interner.resolve(function), param_count, args.len()
                ));
            }

            if self.task.call_depth >= crate::semantics::MAX_CALL_DEPTH {
            return Err(crate::semantics::CALL_DEPTH_ERR.to_string());
        }
        self.task.call_depth += 1;
        self.task.env.push_frame();
            for i in 0..param_count {
                let param_name = self.ctx.functions[&function].params[i].0;
                self.task.env.define(param_name, std::mem::replace(&mut args[i], RuntimeValue::Nothing));
            }

            // TCO: while executing THIS function's body, a self-tail-call is a
            // loop-back (reassign params + restart the body) rather than a real
            // recursive call. `tco_fn_sync`/`repeat_depth_sync` are per-activation,
            // so save the caller's and reset for this body.
            let prev_tco = self.task.tco_fn_sync.replace(function);
            let prev_repeat = std::mem::replace(&mut self.task.repeat_depth_sync, 0);
            let mut return_value = RuntimeValue::Nothing;
            let mut body_err = None;
            'tco: loop {
                self.task.pending_tail_call = None;
                let mut idx = 0;
                while idx < body.len() {
                    // Top-level `Set/Let x to self(args); Return x` pair — a tail
                    // call. (A direct `Return self(args)` at any depth is caught
                    // in execute_stmt_sync's Return arm.)
                    if idx + 1 < body.len() {
                        if let Some(call_args) = crate::tail_call::tail_pair_args(
                            &body[idx],
                            &body[idx + 1],
                            function,
                            param_count,
                        ) {
                            let mut vals = Vec::with_capacity(call_args.len());
                            let mut perr = None;
                            for a in call_args {
                                match self.evaluate_expr_sync(a) {
                                    Ok(v) => vals.push(v),
                                    Err(e) => {
                                        perr = Some(e);
                                        break;
                                    }
                                }
                            }
                            match perr {
                                Some(e) => body_err = Some(e),
                                None => self.task.pending_tail_call = Some(vals),
                            }
                            break;
                        }
                    }
                    match self.execute_stmt_sync(&body[idx]) {
                        Ok(ControlFlow::Return(val)) => {
                            return_value = val;
                            break;
                        }
                        Ok(ControlFlow::Break) => break,
                        Ok(ControlFlow::Continue) => {}
                        Err(e) => {
                            body_err = Some(e);
                            break;
                        }
                    }
                    idx += 1;
                }
                if body_err.is_some() {
                    break 'tco;
                }
                match self.task.pending_tail_call.take() {
                    Some(new_args) => {
                        // Loop-back: a fresh frame (no stale locals) with the
                        // reassigned parameters — constant stack, no depth bump.
                        self.task.env.pop_frame();
                        self.task.env.push_frame();
                        for (i, v) in new_args.into_iter().enumerate() {
                            let param_name = self.ctx.functions[&function].params[i].0;
                            self.task.env.define(param_name, v);
                        }
                        continue 'tco;
                    }
                    None => break 'tco,
                }
            }
            self.task.repeat_depth_sync = prev_repeat;
            self.task.tco_fn_sync = prev_tco;

            self.task.env.pop_frame();
        self.task.call_depth -= 1;
            match body_err {
                Some(e) => Err(e),
                None => Ok(return_value),
            }
        } else {
            let maybe_closure = self.task.env.lookup(function)
                .and_then(|v| if let RuntimeValue::Function(c) = v { Some((**c).clone()) } else { None });

            if let Some(closure) = maybe_closure {
                self.call_closure_value_sync(&closure, args)
            } else {
                Err(format!("Unknown function: {}", self.ctx.interner.resolve(function)))
            }
        }
    }

    // =========================================================================
    // Closure support: free variable collection and closure invocation
    // =========================================================================

    /// Collect free variable symbols from a closure body.
    /// Returns all Identifier symbols referenced in the body that are not parameter names.
    /// Shared with the bytecode VM's compiler — both engines MUST agree on the
    /// capture set, so there is exactly one implementation.
    pub(crate) fn collect_free_vars_in_closure(
        &self,
        params: &[(Symbol, &TypeExpr<'a>)],
        body: &ClosureBody<'a>,
    ) -> Vec<Symbol> {
        Self::free_vars_in_closure(params, body)
    }

    /// Static form of [`Self::collect_free_vars_in_closure`] (the VM compiler
    /// has no interpreter instance).
    pub(crate) fn free_vars_in_closure(
        params: &[(Symbol, &TypeExpr<'a>)],
        body: &ClosureBody<'a>,
    ) -> Vec<Symbol> {
        let param_set: std::collections::HashSet<Symbol> = params.iter().map(|(s, _)| *s).collect();
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();

        match body {
            ClosureBody::Expression(expr) => {
                Self::collect_symbols_from_expr(expr, &param_set, &mut out, &mut seen);
            }
            ClosureBody::Block(block) => {
                Self::collect_symbols_from_block(block, &param_set, &mut out, &mut seen);
            }
        }

        out
    }

    fn collect_symbols_from_expr(
        expr: &Expr<'a>,
        exclude: &std::collections::HashSet<Symbol>,
        out: &mut Vec<Symbol>,
        seen: &mut std::collections::HashSet<Symbol>,
    ) {
        match expr {
            Expr::Identifier(sym) => {
                if !exclude.contains(sym) && seen.insert(*sym) {
                    out.push(*sym);
                }
            }
            Expr::Literal(_) | Expr::OptionNone | Expr::Escape { .. } => {}
            Expr::BinaryOp { left, right, .. } => {
                Self::collect_symbols_from_expr(left, exclude, out, seen);
                Self::collect_symbols_from_expr(right, exclude, out, seen);
            }
            Expr::Call { function, args } => {
                if !exclude.contains(function) && seen.insert(*function) {
                    out.push(*function);
                }
                for arg in args {
                    Self::collect_symbols_from_expr(arg, exclude, out, seen);
                }
            }
            Expr::FieldAccess { object, .. } => {
                Self::collect_symbols_from_expr(object, exclude, out, seen);
            }
            Expr::Index { collection, index } => {
                Self::collect_symbols_from_expr(collection, exclude, out, seen);
                Self::collect_symbols_from_expr(index, exclude, out, seen);
            }
            Expr::Slice { collection, start, end } => {
                Self::collect_symbols_from_expr(collection, exclude, out, seen);
                Self::collect_symbols_from_expr(start, exclude, out, seen);
                Self::collect_symbols_from_expr(end, exclude, out, seen);
            }
            Expr::Copy { expr: e } | Expr::Give { value: e } | Expr::Length { collection: e }
            | Expr::Not { operand: e } => {
                Self::collect_symbols_from_expr(e, exclude, out, seen);
            }
            Expr::List(items) | Expr::Tuple(items) => {
                for item in items {
                    Self::collect_symbols_from_expr(item, exclude, out, seen);
                }
            }
            Expr::Range { start, end } => {
                Self::collect_symbols_from_expr(start, exclude, out, seen);
                Self::collect_symbols_from_expr(end, exclude, out, seen);
            }
            Expr::New { init_fields, .. } => {
                for (_, e) in init_fields {
                    Self::collect_symbols_from_expr(e, exclude, out, seen);
                }
            }
            Expr::NewVariant { fields, .. } => {
                for (_, e) in fields {
                    Self::collect_symbols_from_expr(e, exclude, out, seen);
                }
            }
            Expr::Contains { collection, value } | Expr::Union { left: collection, right: value }
            | Expr::Intersection { left: collection, right: value } => {
                Self::collect_symbols_from_expr(collection, exclude, out, seen);
                Self::collect_symbols_from_expr(value, exclude, out, seen);
            }
            Expr::ManifestOf { zone } | Expr::OptionSome { value: zone } => {
                Self::collect_symbols_from_expr(zone, exclude, out, seen);
            }
            Expr::ChunkAt { index, zone } | Expr::WithCapacity { value: index, capacity: zone } => {
                Self::collect_symbols_from_expr(index, exclude, out, seen);
                Self::collect_symbols_from_expr(zone, exclude, out, seen);
            }
            Expr::Closure { params: inner_params, body: inner_body, .. } => {
                // Nested closure: exclude inner params too
                let mut inner_exclude = exclude.clone();
                for (s, _) in inner_params {
                    inner_exclude.insert(*s);
                }
                match inner_body {
                    ClosureBody::Expression(e) => {
                        Self::collect_symbols_from_expr(e, &inner_exclude, out, seen);
                    }
                    ClosureBody::Block(b) => {
                        Self::collect_symbols_from_block(b, &inner_exclude, out, seen);
                    }
                }
            }
            Expr::CallExpr { callee, args } => {
                Self::collect_symbols_from_expr(callee, exclude, out, seen);
                for arg in args {
                    Self::collect_symbols_from_expr(arg, exclude, out, seen);
                }
            }
            Expr::InterpolatedString(parts) => {
                for part in parts {
                    if let crate::ast::stmt::StringPart::Expr { value, .. } = part {
                        Self::collect_symbols_from_expr(value, exclude, out, seen);
                    }
                }
            }
        }
    }

    fn collect_symbols_from_block(
        stmts: &[Stmt<'a>],
        exclude: &std::collections::HashSet<Symbol>,
        out: &mut Vec<Symbol>,
        seen: &mut std::collections::HashSet<Symbol>,
    ) {
        use crate::ast::stmt::Pattern;
        // `bound` = params + the locals introduced so far in THIS block's scope.
        // A `Let` (or `Repeat`/`Inspect` pattern) binding excludes later uses of
        // that name from the FREE-variable set — without this a function's own
        // local that happens to share a name with a Main top-level variable would
        // leak as "free", over-promoting that Main var to a global and blocking
        // the JIT from tiering Main's hot loops. Nested blocks clone `bound`, so a
        // binding never leaks to a sibling block or past its scope.
        let mut bound = exclude.clone();
        for stmt in stmts {
            match stmt {
                Stmt::Let { var, value, .. } => {
                    Self::collect_symbols_from_expr(value, &bound, out, seen);
                    bound.insert(*var);
                }
                Stmt::Set { value, .. } => {
                    Self::collect_symbols_from_expr(value, &bound, out, seen);
                }
                Stmt::Call { function, args } => {
                    if !bound.contains(function) && seen.insert(*function) {
                        out.push(*function);
                    }
                    for arg in args {
                        Self::collect_symbols_from_expr(arg, &bound, out, seen);
                    }
                }
                Stmt::Return { value: Some(e) } => {
                    Self::collect_symbols_from_expr(e, &bound, out, seen);
                }
                Stmt::If { cond, then_block, else_block } => {
                    Self::collect_symbols_from_expr(cond, &bound, out, seen);
                    Self::collect_symbols_from_block(then_block, &bound, out, seen);
                    if let Some(eb) = else_block {
                        Self::collect_symbols_from_block(eb, &bound, out, seen);
                    }
                }
                Stmt::While { cond, body, .. } => {
                    Self::collect_symbols_from_expr(cond, &bound, out, seen);
                    Self::collect_symbols_from_block(body, &bound, out, seen);
                }
                Stmt::Repeat { pattern, iterable, body } => {
                    Self::collect_symbols_from_expr(iterable, &bound, out, seen);
                    let mut body_bound = bound.clone();
                    match pattern {
                        Pattern::Identifier(s) => {
                            body_bound.insert(*s);
                        }
                        Pattern::Tuple(syms) => {
                            for s in syms {
                                body_bound.insert(*s);
                            }
                        }
                    }
                    Self::collect_symbols_from_block(body, &body_bound, out, seen);
                }
                Stmt::Show { object, .. } | Stmt::Give { object, .. } => {
                    Self::collect_symbols_from_expr(object, &bound, out, seen);
                }
                Stmt::Push { value, collection } | Stmt::Add { value, collection }
                | Stmt::Remove { value, collection } => {
                    Self::collect_symbols_from_expr(value, &bound, out, seen);
                    Self::collect_symbols_from_expr(collection, &bound, out, seen);
                }
                Stmt::SetIndex { collection, index, value } => {
                    Self::collect_symbols_from_expr(collection, &bound, out, seen);
                    Self::collect_symbols_from_expr(index, &bound, out, seen);
                    Self::collect_symbols_from_expr(value, &bound, out, seen);
                }
                Stmt::SetField { object, value, .. } => {
                    Self::collect_symbols_from_expr(object, &bound, out, seen);
                    Self::collect_symbols_from_expr(value, &bound, out, seen);
                }
                Stmt::RuntimeAssert { condition, .. } => {
                    Self::collect_symbols_from_expr(condition, &bound, out, seen);
                }
                Stmt::Zone { body, .. } => {
                    Self::collect_symbols_from_block(body, &bound, out, seen);
                }
                Stmt::Inspect { target, arms, .. } => {
                    Self::collect_symbols_from_expr(target, &bound, out, seen);
                    for arm in arms {
                        Self::collect_symbols_from_block(arm.body, &bound, out, seen);
                    }
                }
                Stmt::Pop { collection, .. } => {
                    Self::collect_symbols_from_expr(collection, &bound, out, seen);
                }
                _ => {}
            }
        }
    }

    /// Execute a closure with pre-evaluated argument values (async).
    #[async_recursion(?Send)]
    async fn call_closure_value(
        &mut self,
        closure: &ClosureValue,
        mut arg_values: Vec<RuntimeValue>,
    ) -> Result<RuntimeValue, String> {
        if arg_values.len() != closure.param_names.len() {
            return Err(format!(
                "Closure expects {} arguments, got {}",
                closure.param_names.len(),
                arg_values.len()
            ));
        }

        // A SHIPPED generator function carries its body as a sandboxed `GenExpr` (it crossed
        // the wire — there is no arena AST to run). Evaluate it directly: total, bounded, no
        // frame, no escape. Single-argument arithmetic (the lowerable subset) → an `Int`.
        if let Some(expr) = &closure.generated {
            let i = match arg_values.first() {
                Some(RuntimeValue::Int(n)) => *n,
                _ => 0,
            };
            return Ok(RuntimeValue::Int(crate::concurrency::marshal::gen_eval(expr, i)));
        }

        // Extract body reference from side-table (breaks borrow on self)
        let body_index = closure.body_index;
        let is_block = matches!(self.ctx.closure_bodies.get(body_index), Some(ClosureBodyRef::Block(_)));

        // A closure body is a fresh frame (lexical barrier): it sees its
        // captures, its parameters, and globals — never the caller's locals.
        if self.task.call_depth >= crate::semantics::MAX_CALL_DEPTH {
            return Err(crate::semantics::CALL_DEPTH_ERR.to_string());
        }
        self.task.call_depth += 1;
        self.task.env.push_frame();

        // Bind captured environment
        for (sym, val) in &closure.captured_env {
            self.task.env.define(*sym, val.deep_clone());
        }

        // Bind parameters
        for (i, param_sym) in closure.param_names.iter().enumerate() {
            self.task.env.define(*param_sym, std::mem::replace(&mut arg_values[i], RuntimeValue::Nothing));
        }

        let result = if is_block {
            let block = match &self.ctx.closure_bodies[body_index] {
                ClosureBodyRef::Block(b) => *b,
                _ => unreachable!(),
            };
            let mut outcome = Ok(RuntimeValue::Nothing);
            for stmt in block.iter() {
                match self.execute_stmt(stmt).await {
                    Ok(ControlFlow::Return(val)) => {
                        outcome = Ok(val);
                        break;
                    }
                    Ok(ControlFlow::Break) => break,
                    Ok(ControlFlow::Continue) => {}
                    Err(e) => {
                        outcome = Err(e);
                        break;
                    }
                }
            }
            outcome
        } else {
            let expr = match &self.ctx.closure_bodies[body_index] {
                ClosureBodyRef::Expression(e) => *e,
                _ => unreachable!(),
            };
            self.evaluate_expr(expr).await
        };

        self.task.env.pop_frame();
        self.task.call_depth -= 1;
        result
    }

    /// Execute a closure with pre-evaluated argument values (sync).
    fn call_closure_value_sync(
        &mut self,
        closure: &ClosureValue,
        mut arg_values: Vec<RuntimeValue>,
    ) -> Result<RuntimeValue, String> {
        if arg_values.len() != closure.param_names.len() {
            return Err(format!(
                "Closure expects {} arguments, got {}",
                closure.param_names.len(),
                arg_values.len()
            ));
        }

        // A SHIPPED generator function evaluates its sandboxed body directly (see async twin).
        if let Some(expr) = &closure.generated {
            let i = match arg_values.first() {
                Some(RuntimeValue::Int(n)) => *n,
                _ => 0,
            };
            return Ok(RuntimeValue::Int(crate::concurrency::marshal::gen_eval(expr, i)));
        }

        let body_index = closure.body_index;
        let is_block = matches!(self.ctx.closure_bodies.get(body_index), Some(ClosureBodyRef::Block(_)));

        // A closure body is a fresh frame (lexical barrier); see the async twin.
        if self.task.call_depth >= crate::semantics::MAX_CALL_DEPTH {
            return Err(crate::semantics::CALL_DEPTH_ERR.to_string());
        }
        self.task.call_depth += 1;
        self.task.env.push_frame();

        for (sym, val) in &closure.captured_env {
            self.task.env.define(*sym, val.deep_clone());
        }

        for (i, param_sym) in closure.param_names.iter().enumerate() {
            self.task.env.define(*param_sym, std::mem::replace(&mut arg_values[i], RuntimeValue::Nothing));
        }

        let result = if is_block {
            let block = match &self.ctx.closure_bodies[body_index] {
                ClosureBodyRef::Block(b) => *b,
                _ => unreachable!(),
            };
            let mut outcome = Ok(RuntimeValue::Nothing);
            for stmt in block.iter() {
                match self.execute_stmt_sync(stmt) {
                    Ok(ControlFlow::Return(val)) => {
                        outcome = Ok(val);
                        break;
                    }
                    Ok(ControlFlow::Break) => break,
                    Ok(ControlFlow::Continue) => {}
                    Err(e) => {
                        outcome = Err(e);
                        break;
                    }
                }
            }
            outcome
        } else {
            let expr = match &self.ctx.closure_bodies[body_index] {
                ClosureBodyRef::Expression(e) => *e,
                _ => unreachable!(),
            };
            self.evaluate_expr_sync(expr)
        };

        self.task.env.pop_frame();
        self.task.call_depth -= 1;
        result
    }
}

/// Check whether a program requires async execution.
///
/// Only 4 statement types need async: ReadFrom (file), WriteFile, Sleep, Mount.
/// If none are present, the sync execution path can be used for better performance.
fn apply_format_spec(val: &RuntimeValue, spec: &str) -> String {
    crate::semantics::format::apply_format_spec(val, spec)
}

pub fn needs_async(stmts: &[Stmt]) -> bool {
    stmts.iter().any(|s| stmt_needs_async(s))
}

fn stmt_needs_async(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::ReadFrom { source, .. } => {
            matches!(source, ReadSource::File(_))
        }
        Stmt::WriteFile { .. } | Stmt::Sleep { .. } | Stmt::Mount { .. } => true,
        // Networking over the relay is async (dial + subscribe await).
        Stmt::Sync { .. } | Stmt::Listen { .. } | Stmt::ConnectTo { .. } => true,
        // Peer messaging rides the relay (subscribe/publish/poll) — async only.
        Stmt::SendMessage { .. } | Stmt::AwaitMessage { .. } | Stmt::StreamMessage { .. } => true,
        Stmt::If { then_block, else_block, .. } => {
            needs_async(then_block)
                || else_block.as_ref().map_or(false, |b| needs_async(b))
        }
        Stmt::While { body, .. } | Stmt::Repeat { body, .. } => needs_async(body),
        Stmt::FunctionDef { body, .. } => needs_async(body),
        Stmt::Zone { body, .. } => needs_async(body),
        Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => needs_async(tasks),
        Stmt::Inspect { arms, .. } => arms.iter().any(|arm| needs_async(arm.body)),
        _ => false,
    }
}

/// Phase 102: Count the number of Pi (function) arguments in a kernel Term.
///
/// Used to determine constructor arity for inductive types.
fn count_pi_args(term: &crate::kernel::Term) -> usize {
    use crate::kernel::Term;
    match term {
        Term::Pi { body_type, .. } => 1 + count_pi_args(body_type),
        _ => 0,
    }
}

/// Result from program interpretation.
///
/// Contains both the output produced by `show()` calls and any error
/// that occurred during execution. Used by the UI bridge to display
/// program output to users.
#[derive(Debug, Clone)]
pub struct InterpreterResult {
    /// Output lines from `show()` calls during execution.
    pub lines: Vec<String>,
    /// Error message if execution failed, or `None` on success.
    pub error: Option<String>,
}

#[cfg(test)]
mod ints_i32_repr_tests {
    use super::*;

    fn i32_buf(vals: &[i64]) -> ListRepr {
        let mut r = ListRepr::IntsI32(Vec::new());
        for &v in vals {
            r.push(RuntimeValue::Int(v));
        }
        r
    }

    #[test]
    fn push_and_get_sign_extend() {
        let r = i32_buf(&[-1, 0, 7, i32::MIN as i64, i32::MAX as i64]);
        assert!(matches!(r, ListRepr::IntsI32(_)), "stays half-width when every value fits i32");
        assert_eq!(r.get(0), Some(RuntimeValue::Int(-1)), "negative sign-extends losslessly");
        assert_eq!(r.get(3), Some(RuntimeValue::Int(i32::MIN as i64)));
        assert_eq!(r.get(4), Some(RuntimeValue::Int(i32::MAX as i64)));
        assert_eq!(r.len(), 5);
    }

    #[test]
    fn push_out_of_range_widens_and_preserves_values() {
        // A value just past i32::MAX forces the whole buffer to full width;
        // every earlier element survives, bit-identical to a never-narrowed run.
        let mut r = i32_buf(&[1, -2, 100]);
        r.push(RuntimeValue::Int(i32::MAX as i64 + 1));
        assert!(matches!(r, ListRepr::Ints(_)), "an out-of-range push widens to full-width Ints");
        assert_eq!(r.get(0), Some(RuntimeValue::Int(1)));
        assert_eq!(r.get(1), Some(RuntimeValue::Int(-2)));
        assert_eq!(r.get(2), Some(RuntimeValue::Int(100)));
        assert_eq!(r.get(3), Some(RuntimeValue::Int(i32::MAX as i64 + 1)));
    }

    #[test]
    fn set_out_of_range_widens() {
        let mut r = i32_buf(&[5, 5, 5]);
        r.set(1, RuntimeValue::Int(i64::MIN));
        assert!(matches!(r, ListRepr::Ints(_)), "an out-of-range in-place store widens");
        assert_eq!(r.get(0), Some(RuntimeValue::Int(5)));
        assert_eq!(r.get(1), Some(RuntimeValue::Int(i64::MIN)));
        assert_eq!(r.get(2), Some(RuntimeValue::Int(5)));
    }

    #[test]
    fn set_in_range_truncates_losslessly() {
        let mut r = i32_buf(&[0, 0, 0]);
        r.set(2, RuntimeValue::Int(-12345));
        assert!(matches!(r, ListRepr::IntsI32(_)), "in-range store stays half-width");
        assert_eq!(r.get(2), Some(RuntimeValue::Int(-12345)));
    }

    #[test]
    fn non_int_push_promotes_to_boxed() {
        // Soundness net: a narrowed buffer that somehow receives a non-Int value
        // boxes rather than dropping the type — never silently wrong.
        let mut r = i32_buf(&[1, 2]);
        r.push(RuntimeValue::Float(3.5));
        assert!(matches!(r, ListRepr::Boxed(_)));
        assert_eq!(r.get(2), Some(RuntimeValue::Float(3.5)));
        assert_eq!(r.get(0), Some(RuntimeValue::Int(1)));
    }

    #[test]
    fn clone_round_trips_to_values() {
        let r = i32_buf(&[-7, 42, i32::MIN as i64]);
        let snap = r.clone();
        assert_eq!(snap.to_values(), r.to_values());
        assert_eq!(
            r.to_values(),
            vec![
                RuntimeValue::Int(-7),
                RuntimeValue::Int(42),
                RuntimeValue::Int(i32::MIN as i64)
            ]
        );
    }

    #[test]
    fn pop_truncate_position_match_full_width() {
        let mut r = i32_buf(&[10, 20, 30]);
        assert_eq!(r.position(&RuntimeValue::Int(20)), Some(1));
        assert_eq!(r.position(&RuntimeValue::Int(99)), None);
        assert_eq!(r.pop(), Some(RuntimeValue::Int(30)));
        r.truncate(1);
        assert_eq!(r.len(), 1);
        assert_eq!(r.get(0), Some(RuntimeValue::Int(10)));
    }
}

#[cfg(test)]
mod structs_repr_tests {
    use super::*;
    use std::collections::HashMap;

    fn point(x: i64, y: i64) -> RuntimeValue {
        let mut f = HashMap::new();
        f.insert("x".to_string(), RuntimeValue::Int(x));
        f.insert("y".to_string(), RuntimeValue::Int(y));
        RuntimeValue::Struct(Box::new(StructValue { type_name: "Point".to_string(), fields: f }))
    }
    fn int_field(v: &RuntimeValue, name: &str) -> i64 {
        match v {
            RuntimeValue::Struct(sv) => match sv.fields.get(name) {
                Some(RuntimeValue::Int(n)) => *n,
                other => panic!("field {name} not an int: {other:?}"),
            },
            other => panic!("not a struct: {other:?}"),
        }
    }

    #[test]
    fn from_values_homogeneous_structs_is_columnar() {
        let r = ListRepr::from_values(vec![point(0, 0), point(1, 2), point(2, 4)]);
        assert!(matches!(r, ListRepr::Structs { .. }), "a homogeneous struct list de-boxes to columns");
        assert_eq!(r.len(), 3);
        assert!(!r.is_empty());
    }

    #[test]
    fn structs_get_reconstructs_exact() {
        let r = ListRepr::from_values(vec![point(0, 0), point(1, 2), point(2, 4)]);
        let s = r.get(1).unwrap();
        assert_eq!(int_field(&s, "x"), 1);
        assert_eq!(int_field(&s, "y"), 2);
        assert!(r.get(3).is_none(), "out-of-range index is None");
    }

    #[test]
    fn structs_to_values_reconstructs_all_rows() {
        let r = ListRepr::from_values(vec![point(5, 6), point(7, 8)]);
        let vs = r.to_values();
        assert_eq!(vs.len(), 2);
        assert_eq!(int_field(&vs[0], "x"), 5);
        assert_eq!(int_field(&vs[1], "y"), 8);
    }

    #[test]
    fn structs_truncate_is_columnwise() {
        let mut r = ListRepr::from_values(vec![point(0, 0), point(1, 1), point(2, 2)]);
        r.truncate(2);
        assert!(matches!(r, ListRepr::Structs { .. }), "truncate keeps it columnar");
        assert_eq!(r.len(), 2);
        assert_eq!(int_field(&r.get(1).unwrap(), "x"), 1);
    }

    #[test]
    fn columnar_field_read_is_direct() {
        let r = ListRepr::from_values(vec![point(10, 20), point(30, 40)]);
        // get_field reads one column directly — no StructValue reconstruction.
        assert_eq!(r.get_field(0, "x"), Some(RuntimeValue::Int(10)));
        assert_eq!(r.get_field(1, "y"), Some(RuntimeValue::Int(40)));
        assert_eq!(r.get_field(0, "z"), None, "missing field");
        assert!(r.get_field(5, "x").is_none(), "out of range");
        // the column accessor exposes the raw packed column for array-speed scans.
        match r.column("x") {
            Some(ListRepr::Ints(v)) => assert_eq!(v, &vec![10, 30]),
            other => panic!("expected an Ints column, got {other:?}"),
        }
        // a boxed list has no columns.
        let boxed = ListRepr::Boxed(vec![point(1, 2)]);
        assert!(boxed.get_field(0, "x").is_none());
        assert!(boxed.column("x").is_none());
    }

    #[test]
    fn structs_mutation_decolumnarizes_and_stays_correct() {
        // A set to a non-struct value de-columnarizes (make_boxed) but preserves
        // every prior row exactly — the soundness invariant.
        let mut r = ListRepr::from_values(vec![point(0, 0), point(1, 1), point(2, 2)]);
        r.set(1, RuntimeValue::Int(99));
        assert!(matches!(r, ListRepr::Boxed(_)), "a mutating set de-columnarizes");
        assert_eq!(int_field(&r.get(0).unwrap(), "x"), 0);
        assert_eq!(r.get(1), Some(RuntimeValue::Int(99)));
        assert_eq!(int_field(&r.get(2).unwrap(), "y"), 2);
    }

    #[test]
    fn structs_push_stays_correct() {
        let mut r = ListRepr::from_values(vec![point(0, 0)]);
        r.push(point(1, 1));
        assert_eq!(r.len(), 2);
        assert_eq!(int_field(&r.get(0).unwrap(), "x"), 0);
        assert_eq!(int_field(&r.get(1).unwrap(), "x"), 1);
    }

    #[test]
    fn heterogeneous_structs_stay_boxed() {
        // Ragged field set ⇒ boxed.
        let mut only_x = HashMap::new();
        only_x.insert("x".to_string(), RuntimeValue::Int(9));
        let odd = RuntimeValue::Struct(Box::new(StructValue { type_name: "Point".to_string(), fields: only_x }));
        let r = ListRepr::from_values(vec![point(0, 0), odd]);
        assert!(matches!(r, ListRepr::Boxed(_)), "ragged field sets stay boxed");

        // Mixed type names ⇒ boxed.
        let mut cf = HashMap::new();
        cf.insert("x".to_string(), RuntimeValue::Int(1));
        cf.insert("y".to_string(), RuntimeValue::Int(2));
        let q = RuntimeValue::Struct(Box::new(StructValue { type_name: "Other".to_string(), fields: cf }));
        let r2 = ListRepr::from_values(vec![point(0, 0), q]);
        assert!(matches!(r2, ListRepr::Boxed(_)), "mixed type names stay boxed");
    }

    #[test]
    fn columnar_field_scan_is_faster_than_boxed_and_iteration_is_not_slower() {
        // The in-memory win, measured on the shared `ListRepr` primitive (both the
        // tree-walker and the VM read lists through it). Reported with --nocapture;
        // the asserts are noise-robust (huge margins / not-slower).
        use std::time::Instant;
        const N: usize = 5000;
        const ITERS: u32 = 300;

        let rows: Vec<RuntimeValue> = (0..N as i64).map(|i| point(i, i * 2)).collect();
        let columnar = ListRepr::from_values(rows.clone());
        assert!(matches!(columnar, ListRepr::Structs { .. }), "the columnar baseline must be Structs");
        let boxed = ListRepr::Boxed(rows);

        // (a) FIELD SCAN — sum the "x" field across the list. Columnar reads the raw
        //     Vec<i64> column (array speed); boxed reconstructs/clones each struct and
        //     hashmap-looks-up "x". This is the "arrays not heap" win.
        let scan_columnar = || -> i64 {
            match columnar.column("x") {
                Some(ListRepr::Ints(v)) => v.iter().copied().sum(),
                _ => unreachable!(),
            }
        };
        let scan_boxed = || -> i64 {
            let mut s = 0i64;
            for i in 0..boxed.len() {
                if let Some(RuntimeValue::Struct(sv)) = boxed.get(i) {
                    if let Some(RuntimeValue::Int(x)) = sv.fields.get("x") {
                        s += *x;
                    }
                }
            }
            s
        };
        assert_eq!(scan_columnar(), scan_boxed(), "the two scans must agree");

        let t = Instant::now();
        for _ in 0..ITERS {
            std::hint::black_box(scan_columnar());
        }
        let col_ns = t.elapsed().as_nanos().max(1);
        let t = Instant::now();
        for _ in 0..ITERS {
            std::hint::black_box(scan_boxed());
        }
        let box_ns = t.elapsed().as_nanos().max(1);
        println!(
            "\n[E3] field scan (sum x over {N}, ×{ITERS}): columnar {col_ns} ns vs boxed {box_ns} ns  ({:.1}× faster)",
            box_ns as f64 / col_ns as f64
        );
        assert!(col_ns * 2 <= box_ns, "columnar field scan should be ≥2× faster: columnar {col_ns} vs boxed {box_ns}");

        // (b) FULL-ROW iteration — reconstruct every struct. Roughly a wash (both
        //     reprs box a StructValue per element), reported for honesty; asserted only
        //     "not catastrophically slower".
        let iter_repr = |r: &ListRepr| {
            let mut acc = 0i64;
            for i in 0..r.len() {
                if let Some(RuntimeValue::Struct(sv)) = r.get(i) {
                    if let (Some(RuntimeValue::Int(x)), Some(RuntimeValue::Int(y))) =
                        (sv.fields.get("x"), sv.fields.get("y"))
                    {
                        acc += x + y;
                    }
                }
            }
            acc
        };
        assert_eq!(iter_repr(&columnar), iter_repr(&boxed), "full-row iteration must agree");
        let t = Instant::now();
        for _ in 0..ITERS {
            std::hint::black_box(iter_repr(&columnar));
        }
        let col_it = t.elapsed().as_nanos().max(1);
        let t = Instant::now();
        for _ in 0..ITERS {
            std::hint::black_box(iter_repr(&boxed));
        }
        let box_it = t.elapsed().as_nanos().max(1);
        println!(
            "[E3] full-row iter (×{ITERS}): columnar {col_it} ns vs boxed {box_it} ns  ({:.2}× of boxed)",
            col_it as f64 / box_it as f64
        );
        assert!(col_it <= box_it * 3, "full-row iteration must not be catastrophically slower: {col_it} vs {box_it}");
    }

    #[test]
    fn zero_field_structs_stay_boxed_and_keep_count() {
        // A columnar store has no column to carry the row count when the struct has
        // no fields, so such a list MUST stay boxed (and report the right length).
        let unit = || RuntimeValue::Struct(Box::new(StructValue { type_name: "Unit".to_string(), fields: HashMap::new() }));
        let r = ListRepr::from_values(vec![unit(), unit(), unit()]);
        assert!(matches!(r, ListRepr::Boxed(_)), "zero-field struct list stays boxed");
        assert_eq!(r.len(), 3, "row count is preserved");
    }
}

#[cfg(test)]
mod enums_repr_tests {
    use super::*;

    fn nullary(ty: &str, ctor: &str) -> RuntimeValue {
        RuntimeValue::Inductive(Box::new(InductiveValue { inductive_type: ty.into(), constructor: ctor.into(), args: vec![] }))
    }
    fn with_args(ty: &str, ctor: &str, args: Vec<RuntimeValue>) -> RuntimeValue {
        RuntimeValue::Inductive(Box::new(InductiveValue { inductive_type: ty.into(), constructor: ctor.into(), args }))
    }
    fn ctor_of(v: &RuntimeValue) -> String {
        match v {
            RuntimeValue::Inductive(i) => i.constructor.clone(),
            other => panic!("not an inductive: {other:?}"),
        }
    }
    fn int_arg(v: &RuntimeValue, j: usize) -> i64 {
        match v {
            RuntimeValue::Inductive(i) => match &i.args[j] {
                RuntimeValue::Int(n) => *n,
                other => panic!("arg {j} not an int: {other:?}"),
            },
            other => panic!("not an inductive: {other:?}"),
        }
    }

    #[test]
    fn from_values_nullary_enums_is_columnar() {
        let r = ListRepr::from_values(vec![nullary("Color", "Red"), nullary("Color", "Green"), nullary("Color", "Red")]);
        assert!(matches!(r, ListRepr::Inductives { .. }), "a nullary enum list de-boxes to columns");
        assert_eq!(r.len(), 3);
        assert_eq!(ctor_of(&r.get(0).unwrap()), "Red");
        assert_eq!(ctor_of(&r.get(1).unwrap()), "Green");
        assert_eq!(ctor_of(&r.get(2).unwrap()), "Red");
        assert!(r.get(3).is_none());
    }

    #[test]
    fn from_values_uniform_arg_enums_is_columnar() {
        let r = ListRepr::from_values(vec![
            with_args("Boxed", "B", vec![RuntimeValue::Int(1)]),
            with_args("Boxed", "B", vec![RuntimeValue::Int(2)]),
        ]);
        assert!(matches!(r, ListRepr::Inductives { .. }), "a uniform-arg enum list packs columnar");
        assert_eq!(int_arg(&r.get(0).unwrap(), 0), 1);
        assert_eq!(int_arg(&r.get(1).unwrap(), 0), 2);
    }

    #[test]
    fn from_values_mixed_arity_enums_is_columnar() {
        // Option-like: Some(1), None, Some(2), None, Some(3) — a tagged union, packed
        // as dense per-constructor arg columns.
        let rows = vec![
            with_args("Option", "Some", vec![RuntimeValue::Int(1)]),
            nullary("Option", "None"),
            with_args("Option", "Some", vec![RuntimeValue::Int(2)]),
            nullary("Option", "None"),
            with_args("Option", "Some", vec![RuntimeValue::Int(3)]),
        ];
        let r = ListRepr::from_values(rows);
        assert!(matches!(r, ListRepr::Inductives { .. }), "a mixed-arity enum list packs columnar");
        assert_eq!(r.len(), 5);
        assert_eq!(ctor_of(&r.get(0).unwrap()), "Some");
        assert_eq!(int_arg(&r.get(0).unwrap(), 0), 1);
        assert_eq!(ctor_of(&r.get(1).unwrap()), "None");
        assert_eq!(ctor_of(&r.get(2).unwrap()), "Some");
        assert_eq!(int_arg(&r.get(2).unwrap(), 0), 2);
        assert_eq!(ctor_of(&r.get(3).unwrap()), "None");
        assert_eq!(ctor_of(&r.get(4).unwrap()), "Some");
        assert_eq!(int_arg(&r.get(4).unwrap(), 0), 3);
    }

    #[test]
    fn enums_to_values_reconstructs_all_rows() {
        let rows = vec![with_args("Option", "Some", vec![RuntimeValue::Int(7)]), nullary("Option", "None")];
        let vs = ListRepr::from_values(rows).to_values();
        assert_eq!(vs.len(), 2);
        assert_eq!(ctor_of(&vs[0]), "Some");
        assert_eq!(int_arg(&vs[0], 0), 7);
        assert_eq!(ctor_of(&vs[1]), "None");
    }

    #[test]
    fn enums_mutation_decolumnarizes_and_stays_correct() {
        let mut r = ListRepr::from_values(vec![nullary("Color", "Red"), nullary("Color", "Green")]);
        r.set(1, RuntimeValue::Int(99));
        assert!(matches!(r, ListRepr::Boxed(_)), "a mutating set de-columnarizes");
        assert_eq!(ctor_of(&r.get(0).unwrap()), "Red");
        assert_eq!(r.get(1), Some(RuntimeValue::Int(99)));
    }

    #[test]
    fn heterogeneous_enums_stay_boxed() {
        let r = ListRepr::from_values(vec![nullary("Color", "Red"), nullary("Suit", "Spade")]);
        assert!(matches!(r, ListRepr::Boxed(_)), "mixed inductive types stay boxed");
    }
}

#[cfg(test)]
mod float_comparison_tests {
    use super::*;

    fn expect_bool(r: Result<RuntimeValue, String>, want: bool, label: &str) {
        match r {
            Ok(RuntimeValue::Bool(b)) => assert_eq!(b, want, "{label}"),
            other => panic!("{label}: expected Bool, got {:?}", other.map(|v| v.type_name().to_string())),
        }
    }

    #[test]
    fn float_relational_uses_ieee_semantics() {
        use RuntimeValue::{Float, Int};
        let interner = Interner::new();
        let interp = Interpreter::new(&interner);
        let nan = f64::NAN;

        // -0.0 and +0.0 are equal under IEEE 754.
        expect_bool(interp.apply_binary_op(BinaryOpKind::Lt, Float(-0.0), Float(0.0)), false, "-0.0 < 0.0");
        expect_bool(interp.apply_binary_op(BinaryOpKind::Gt, Float(0.0), Float(-0.0)), false, "0.0 > -0.0");
        expect_bool(interp.apply_binary_op(BinaryOpKind::LtEq, Float(-0.0), Float(0.0)), true, "-0.0 <= 0.0");
        expect_bool(interp.apply_binary_op(BinaryOpKind::GtEq, Float(-0.0), Float(0.0)), true, "-0.0 >= 0.0");

        // NaN is unordered: every relational comparison is false.
        expect_bool(interp.apply_binary_op(BinaryOpKind::Lt, Float(nan), Float(1.0)), false, "NaN < 1");
        expect_bool(interp.apply_binary_op(BinaryOpKind::Gt, Float(nan), Float(1.0)), false, "NaN > 1");
        expect_bool(interp.apply_binary_op(BinaryOpKind::LtEq, Float(nan), Float(nan)), false, "NaN <= NaN");
        expect_bool(interp.apply_binary_op(BinaryOpKind::GtEq, Float(1.0), Float(nan)), false, "1 >= NaN");

        // Ordinary comparisons still work, including mixed Int/Float and pure Int.
        expect_bool(interp.apply_binary_op(BinaryOpKind::Lt, Float(1.5), Float(2.5)), true, "1.5 < 2.5");
        expect_bool(interp.apply_binary_op(BinaryOpKind::Lt, Int(2), Float(2.5)), true, "2 < 2.5");
        expect_bool(interp.apply_binary_op(BinaryOpKind::GtEq, Float(2.5), Int(2)), true, "2.5 >= 2");
        expect_bool(interp.apply_binary_op(BinaryOpKind::Lt, Int(3), Int(5)), true, "3 < 5");
        expect_bool(interp.apply_binary_op(BinaryOpKind::GtEq, Int(5), Int(5)), true, "5 >= 5");
    }
}
