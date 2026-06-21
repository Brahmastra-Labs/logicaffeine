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

use crate::ast::stmt::{BinaryOpKind, Block, ClosureBody, Expr, Literal, MatchArm, ReadSource, Stmt, TypeExpr};
use crate::intern::{Interner, Symbol};
use crate::analysis::{PolicyRegistry, PolicyCondition};

// VFS imports for async file operations
use logicaffeine_system::fs::Vfs;

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
}

/// The Map payload behind `RuntimeValue::Map`. FxHash instead of the standard
/// library's SipHash: map-heavy programs hash on every get/insert, and the
/// keys here are small values (ints, short texts) where Fx is several times
/// faster — with no DoS-resistance requirement (a single-program interpreter
/// hashing its own program's keys). Both engines share this alias, so map
/// iteration order — arbitrary but deterministic per build — is identical
/// between the VM and the tree-walker.
pub type MapStorage = rustc_hash::FxHashMap<RuntimeValue, RuntimeValue>;

/// The List payload behind `RuntimeValue::List`: homogeneous all-Int and
/// all-Float lists store UNBOXED vectors (cache-dense, and the JIT can pin a
/// raw pointer to them); anything else boxes. The repr lives INSIDE the
/// `Rc<RefCell<…>>`, so promotion re-tags the payload in place and every
/// alias observes it — reference semantics and Rc identity are untouched.
/// An EMPTY list is vacuously `Ints` and re-tags freely on its first push.
///
/// Hot paths take `&self`/`&mut self` borrows only — no Rc refcount traffic.
/// `Clone` snapshots a buffer's contents for the region deopt-rollback of an
/// in-place-mutated array (see [`crate::vm::native_tier::ArrayPin::mutated`]).
#[derive(Debug, Clone)]
pub enum ListRepr {
    Boxed(Vec<RuntimeValue>),
    Ints(Vec<i64>),
    Floats(Vec<f64>),
    Bools(Vec<bool>),
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
        } else {
            ListRepr::Boxed(values)
        }
    }

    pub fn len(&self) -> usize {
        match self {
            ListRepr::Boxed(v) => v.len(),
            ListRepr::Ints(v) => v.len(),
            ListRepr::Floats(v) => v.len(),
            ListRepr::Bools(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Drop every element past `n` (no-op when already `<= n`). The region
    /// deopt path uses this to roll a pushed buffer back to its entry length
    /// so discard-and-replay re-pushes cleanly instead of duplicating.
    pub fn truncate(&mut self, n: usize) {
        match self {
            ListRepr::Boxed(v) => v.truncate(n),
            ListRepr::Ints(v) => v.truncate(n),
            ListRepr::Floats(v) => v.truncate(n),
            ListRepr::Bools(v) => v.truncate(n),
        }
    }

    /// 0-based read; boxes the scalar (stack-only — no heap, no Rc traffic).
    pub fn get(&self, i: usize) -> Option<RuntimeValue> {
        match self {
            ListRepr::Boxed(v) => v.get(i).cloned(),
            ListRepr::Ints(v) => v.get(i).map(|&n| RuntimeValue::Int(n)),
            ListRepr::Floats(v) => v.get(i).map(|&f| RuntimeValue::Float(f)),
            ListRepr::Bools(v) => v.get(i).map(|&b| RuntimeValue::Bool(b)),
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
            ListRepr::Bools(v) => {
                let boxed = v.drain(..).map(RuntimeValue::Bool).collect();
                *self = ListRepr::Boxed(boxed);
                match self {
                    ListRepr::Boxed(v) => v,
                    _ => unreachable!(),
                }
            }
        }
    }

    /// 0-based write (bounds already validated by the caller); promotes on a
    /// kind mismatch.
    pub fn set(&mut self, i: usize, value: RuntimeValue) {
        match (&mut *self, &value) {
            (ListRepr::Ints(v), RuntimeValue::Int(n)) => v[i] = *n,
            (ListRepr::Floats(v), RuntimeValue::Float(f)) => v[i] = *f,
            (ListRepr::Bools(v), RuntimeValue::Bool(b)) => v[i] = *b,
            (ListRepr::Boxed(v), _) => v[i] = value,
            _ => self.make_boxed()[i] = value,
        }
    }

    pub fn push(&mut self, value: RuntimeValue) {
        match (&mut *self, &value) {
            (ListRepr::Ints(v), RuntimeValue::Int(n)) => v.push(*n),
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
            ListRepr::Floats(v) => v.pop().map(RuntimeValue::Float),
            ListRepr::Bools(v) => v.pop().map(RuntimeValue::Bool),
        }
    }

    pub fn insert(&mut self, i: usize, value: RuntimeValue) {
        match (&mut *self, &value) {
            (ListRepr::Ints(v), RuntimeValue::Int(n)) => v.insert(i, *n),
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
            ListRepr::Floats(v) => RuntimeValue::Float(v.remove(i)),
            ListRepr::Bools(v) => RuntimeValue::Bool(v.remove(i)),
        }
    }

    /// Index of the first element `values_equal` to `needle` (the kernel's
    /// equality: epsilon floats, cross-type never equal).
    pub fn position(&self, needle: &RuntimeValue) -> Option<usize> {
        match (self, needle) {
            (ListRepr::Ints(v), RuntimeValue::Int(n)) => v.iter().position(|x| x == n),
            (ListRepr::Ints(_), _) => None,
            (ListRepr::Floats(v), RuntimeValue::Float(f)) => {
                v.iter().position(|x| (x - f).abs() < f64::EPSILON)
            }
            (ListRepr::Floats(_), _) => None,
            (ListRepr::Bools(v), RuntimeValue::Bool(b)) => v.iter().position(|x| x == b),
            (ListRepr::Bools(_), _) => None,
            (ListRepr::Boxed(v), _) => {
                v.iter().position(|x| crate::semantics::compare::values_equal(x, needle))
            }
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
            ListRepr::Floats(v) => v.iter().map(|&f| RuntimeValue::Float(f)).collect(),
            ListRepr::Bools(v) => v.iter().map(|&b| RuntimeValue::Bool(b)).collect(),
        }
    }

    /// 0-based inclusive-range slice as a fresh payload of the same repr.
    pub fn slice(&self, start: usize, end: usize) -> ListRepr {
        match self {
            ListRepr::Boxed(v) => ListRepr::Boxed(v[start..=end].to_vec()),
            ListRepr::Ints(v) => ListRepr::Ints(v[start..=end].to_vec()),
            ListRepr::Floats(v) => ListRepr::Floats(v[start..=end].to_vec()),
            ListRepr::Bools(v) => ListRepr::Bools(v[start..=end].to_vec()),
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
}

impl PartialEq for RuntimeValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (RuntimeValue::Int(a), RuntimeValue::Int(b)) => a == b,
            (RuntimeValue::Float(a), RuntimeValue::Float(b)) => a.to_bits() == b.to_bits(),
            (RuntimeValue::Bool(a), RuntimeValue::Bool(b)) => a == b,
            (RuntimeValue::Text(a), RuntimeValue::Text(b)) => **a == **b,
            (RuntimeValue::Char(a), RuntimeValue::Char(b)) => a == b,
            (RuntimeValue::Nothing, RuntimeValue::Nothing) => true,
            (RuntimeValue::Duration(a), RuntimeValue::Duration(b)) => a == b,
            (RuntimeValue::Date(a), RuntimeValue::Date(b)) => a == b,
            (RuntimeValue::Moment(a), RuntimeValue::Moment(b)) => a == b,
            (RuntimeValue::Span { months: m1, days: d1 }, RuntimeValue::Span { months: m2, days: d2 }) => {
                m1 == m2 && d1 == d2
            }
            (RuntimeValue::Time(a), RuntimeValue::Time(b)) => a == b,
            (RuntimeValue::Function(a), RuntimeValue::Function(b)) => a.body_index == b.body_index,
            _ => false,
        }
    }
}

impl Eq for RuntimeValue {}

impl std::hash::Hash for RuntimeValue {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            RuntimeValue::Int(n) => n.hash(state),
            RuntimeValue::Float(f) => f.to_bits().hash(state),
            RuntimeValue::Bool(b) => b.hash(state),
            RuntimeValue::Text(s) => s.hash(state),
            RuntimeValue::Char(c) => c.hash(state),
            RuntimeValue::Nothing => {}
            RuntimeValue::Duration(d) => d.hash(state),
            RuntimeValue::Date(d) => d.hash(state),
            RuntimeValue::Moment(m) => m.hash(state),
            RuntimeValue::Span { months, days } => { months.hash(state); days.hash(state); }
            RuntimeValue::Time(t) => t.hash(state),
            // Collections are not meaningfully hashable — hash by identity/length
            RuntimeValue::List(items) => items.borrow().len().hash(state),
            RuntimeValue::Tuple(items) => items.len().hash(state),
            RuntimeValue::Set(items) => items.borrow().len().hash(state),
            RuntimeValue::Map(m) => m.borrow().len().hash(state),
            RuntimeValue::Struct(s) => s.type_name.hash(state),
            RuntimeValue::Inductive(i) => { i.inductive_type.hash(state); i.constructor.hash(state); }
            RuntimeValue::Function(f) => f.body_index.hash(state),
        }
    }
}

impl RuntimeValue {
    /// Returns the type name of this value as a string slice.
    ///
    /// Used for error messages and type checking at runtime.
    pub fn type_name(&self) -> &str {
        match self {
            RuntimeValue::Int(_) => "Int",
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
                }))
            }
            other => other.clone(),
        }
    }

    pub fn is_truthy(&self) -> bool {
        match self {
            RuntimeValue::Bool(b) => *b,
            RuntimeValue::Int(n) => *n != 0,
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
            RuntimeValue::Float(f) => format!("{:.6}", f).trim_end_matches('0').trim_end_matches('.').to_string(),
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
                    let field_strs: Vec<String> = s.fields
                        .iter()
                        .map(|(k, v)| format!("{}: {}", k, v.to_display_string()))
                        .collect();
                    format!("{} {{ {} }}", s.type_name, field_strs.join(", "))
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
                // Convert nanoseconds from midnight to HH:MM format
                let total_seconds = *nanos / 1_000_000_000;
                let hours = total_seconds / 3600;
                let minutes = (total_seconds % 3600) / 60;
                format!("{:02}:{:02}", hours, minutes)
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
pub enum ClosureBodyRef<'a> {
    Expression(&'a Expr<'a>),
    Block(Block<'a>),
}

pub struct Interpreter<'a> {
    interner: &'a Interner,
    env: Environment,
    functions: HashMap<Symbol, FunctionDef<'a>>,
    struct_defs: HashMap<Symbol, Vec<(Symbol, Symbol, bool)>>,
    pub output: Vec<String>,
    vfs: Option<Arc<dyn Vfs>>,
    kernel_ctx: Option<Arc<crate::kernel::Context>>,
    policy_registry: Option<PolicyRegistry>,
    output_callback: Option<OutputCallback>,
    /// Side-table for closure body AST references.
    /// Indexed by `ClosureValue::body_index`.
    closure_bodies: Vec<ClosureBodyRef<'a>>,
    /// Live LOGOS call depth, bounded by `semantics::MAX_CALL_DEPTH`.
    call_depth: usize,
    /// The user function whose body the SYNC path is currently executing. A
    /// `Return self(args)` (or the `Set/Let x to self(args); Return x` pair) of
    /// THIS function is a self-tail-call: `call_function_sync` reassigns the
    /// parameters and loops to the body's start instead of recursing, so tail
    /// recursion runs in constant stack — matching the VM and the AOT TCE.
    /// Saved and restored across each nested call so it always names the
    /// innermost active function.
    tco_fn_sync: Option<Symbol>,
    /// Set by a recognized self-tail-call: the already-evaluated arguments for
    /// the next loop iteration, consumed by `call_function_sync`.
    pending_tail_call: Option<Vec<RuntimeValue>>,
    /// `Repeat` (for-each) nesting on the SYNC path within the current function
    /// body. A `Repeat` owns a live iterator, so — exactly like the VM's
    /// `is_repeat` guard — a self-tail-call detected inside one stays an ordinary
    /// recursive call (jumping to the body start would abandon the iterator),
    /// keeping the two engines bit-identical. Reset to 0 at each call boundary.
    repeat_depth_sync: usize,
    /// `tco_fn_sync` for the ASYNC execution path (used when a program contains
    /// async constructs); same constant-stack TCO semantics on that path.
    tco_fn_async: Option<Symbol>,
    /// `pending_tail_call` for the ASYNC path.
    pending_tail_call_async: Option<Vec<RuntimeValue>>,
    /// `repeat_depth_sync` for the ASYNC path.
    repeat_depth_async: usize,
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

impl<'a> Interpreter<'a> {
    pub fn new(interner: &'a Interner) -> Self {
        Interpreter {
            interner,
            env: Environment::new(),
            functions: HashMap::new(),
            struct_defs: HashMap::new(),
            output: Vec::new(),
            vfs: None,
            kernel_ctx: None,
            policy_registry: None,
            output_callback: None,
            closure_bodies: Vec::new(),
            call_depth: 0,
            tco_fn_sync: None,
            pending_tail_call: None,
            repeat_depth_sync: 0,
            tco_fn_async: None,
            pending_tail_call_async: None,
            repeat_depth_async: 0,
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
        }
    }

    /// Supply the program arguments read by the `args()` system native. The
    /// vector is the full argv (index 0 is the program name), matching the
    /// compiled binary's `env::args()`.
    pub fn with_program_args(mut self, args: Vec<String>) -> Self {
        self.program_args = args;
        self
    }

    /// Phase 55: Set the VFS for file operations.
    pub fn with_vfs(mut self, vfs: Arc<dyn Vfs>) -> Self {
        self.vfs = Some(vfs);
        self
    }

    /// Phase 102: Set the kernel context for inductive type support.
    ///
    /// When set, the interpreter can query the kernel for inductive types
    /// and constructors, enabling unified type system.
    pub fn with_kernel(mut self, ctx: Arc<crate::kernel::Context>) -> Self {
        self.kernel_ctx = Some(ctx);
        self
    }

    /// Set the policy registry for security checks.
    pub fn with_policies(mut self, registry: PolicyRegistry) -> Self {
        self.policy_registry = Some(registry);
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
                self.struct_defs.insert(*name_sym, field_defs);
            }
        }
        self
    }

    /// Set a callback for streaming output.
    /// The callback is called each time `Show` executes, with the output line.
    pub fn with_output_callback(mut self, callback: OutputCallback) -> Self {
        self.output_callback = Some(callback);
        self
    }

    /// Internal helper to emit output (calls callback if set, always adds to output vec)
    fn emit_output(&mut self, line: String) {
        if let Some(ref callback) = self.output_callback {
            (callback.borrow_mut())(line.clone());
        }
        self.output.push(line);
    }

    /// Phase 102: Check if a name is a kernel inductive type.
    pub fn is_kernel_inductive(&self, name: &str) -> bool {
        self.kernel_ctx
            .as_ref()
            .map(|ctx| ctx.is_inductive(name))
            .unwrap_or(false)
    }

    /// Phase 102: Get constructors for a kernel inductive type.
    ///
    /// Returns a vector of (constructor_name, arity) pairs.
    pub fn get_kernel_constructors(&self, name: &str) -> Vec<(String, usize)> {
        self.kernel_ctx
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
        for stmt in stmts {
            match self.execute_stmt(stmt).await? {
                ControlFlow::Return(_) => break,
                ControlFlow::Break => break,
                ControlFlow::Continue => {}
            }
        }
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
                self.repeat_depth_async += 1;
                for item in items {
                    // Bind variables according to pattern
                    match pattern {
                        Pattern::Identifier(sym) => {
                            self.define(*sym, item);
                        }
                        Pattern::Tuple(syms) => {
                            if let RuntimeValue::Tuple(ref tuple_vals) = item {
                                for (sym, val) in syms.iter().zip(tuple_vals.iter()) {
                                    self.define(*sym, val.clone());
                                }
                            } else {
                                self.repeat_depth_async -= 1;
                                return Err(format!("Expected tuple for pattern, got {}", item.type_name()));
                            }
                        }
                    }

                    match self.execute_block(body).await? {
                        ControlFlow::Break => break,
                        ControlFlow::Return(v) => {
                            self.repeat_depth_async -= 1;
                            self.pop_scope();
                            return Ok(ControlFlow::Return(v));
                        }
                        ControlFlow::Continue => {}
                    }
                }
                self.repeat_depth_async -= 1;
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
                        self.pending_tail_call_async = Some(vals);
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
                self.functions.insert(*name, func);
                Ok(ControlFlow::Continue)
            }

            Stmt::StructDef { name, fields, .. } => {
                self.struct_defs.insert(*name, fields.clone());
                Ok(ControlFlow::Continue)
            }

            Stmt::SetField { object, field, value } => {
                let new_val = self.evaluate_expr(value).await?;
                if let Expr::Identifier(obj_sym) = object {
                    let mut obj_val = self.lookup(*obj_sym)?.clone();
                    if let RuntimeValue::Struct(ref mut s) = obj_val {
                        let field_name = self.interner.resolve(*field).to_string();
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
                    let coll_val = self.lookup(*coll_sym)?;
                    crate::semantics::collections::list_push(&coll_val, val)?;
                } else if let Expr::FieldAccess { object, field } = collection {
                    if let Expr::Identifier(obj_sym) = *object {
                        let obj_val = self.lookup(*obj_sym)?;
                        let field_name = self.interner.resolve(*field);
                        crate::semantics::collections::push_to_struct_field(&obj_val, field_name, val)?;
                    } else {
                        return Err("Push to nested field access not supported".to_string());
                    }
                } else {
                    return Err("Push collection must be an identifier or field access".to_string());
                }
                Ok(ControlFlow::Continue)
            }

            Stmt::Pop { collection, into } => {
                if let Expr::Identifier(coll_sym) = collection {
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
                    let coll_val = self.lookup(*coll_sym)?;
                    crate::semantics::collections::set_add(&coll_val, val)?;
                } else {
                    return Err("Add collection must be an identifier".to_string());
                }
                Ok(ControlFlow::Continue)
            }

            Stmt::Remove { value, collection } => {
                let val = self.evaluate_expr(value).await?;
                if let Expr::Identifier(coll_sym) = collection {
                    let coll_val = self.lookup(*coll_sym)?;
                    crate::semantics::collections::remove_from(&coll_val, &val)?;
                } else {
                    return Err("Remove collection must be an identifier".to_string());
                }
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
                    let coll_val = self.lookup(*coll_sym)?;
                    crate::semantics::collections::index_set(&coll_val, &idx_val, new_val)?;
                } else {
                    return Err("SetIndex collection must be an identifier".to_string());
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

            Stmt::RuntimeAssert { condition } => {
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
                    let name = self.interner.resolve(*sym);
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
                        match &self.vfs {
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
                match &self.vfs {
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

            Stmt::SendMessage { .. } => {
                Ok(ControlFlow::Continue)
            }

            Stmt::AwaitMessage { into, .. } => {
                self.define(*into, RuntimeValue::Nothing);
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
                        let field_name = self.interner.resolve(*field).to_string();
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
                        let field_name = self.interner.resolve(*field).to_string();
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

            Stmt::AppendToSequence { .. } => {
                Err("Append to sequence is not supported in the interpreter. Use compiled Rust.".to_string())
            }

            Stmt::ResolveConflict { .. } => {
                Err("Resolve conflict is not supported in the interpreter. Use compiled Rust.".to_string())
            }

            Stmt::Check { subject, predicate, is_capability, object, source_text, .. } => {
                // Get the policy registry
                let registry = match &self.policy_registry {
                    Some(r) => r,
                    None => return Err("Security Check requires policies. Use compiled Rust or add ## Policy block.".to_string()),
                };

                let subj_val = self.lookup(*subject)?.clone();
                let subj_type_name = match &subj_val {
                    RuntimeValue::Struct(s) => s.type_name.clone(),
                    _ => return Err(format!("Check subject must be a struct, got {}", subj_val.type_name())),
                };

                // Find the subject type symbol
                let subj_type_sym = match self.interner.lookup(&subj_type_name) {
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
                            let pred_name = self.interner.resolve(*predicate);
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
                            let pred_name = self.interner.resolve(*predicate);
                            return Err(format!("No predicate '{}' defined for type '{}'", pred_name, subj_type_name));
                        }
                    }
                };

                if !passed {
                    return Err(format!("Security Check Failed: {}", source_text));
                }
                Ok(ControlFlow::Continue)
            }

            Stmt::Listen { .. } => {
                Err("Listen is not supported in the interpreter. Use compiled Rust.".to_string())
            }
            Stmt::ConnectTo { .. } => {
                Err("Connect is not supported in the interpreter. Use compiled Rust.".to_string())
            }
            Stmt::LetPeerAgent { .. } => {
                Err("PeerAgent is not supported in the interpreter. Use compiled Rust.".to_string())
            }
            Stmt::Sleep { milliseconds } => {
                let val = self.evaluate_expr(milliseconds).await?;
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
            Stmt::Sync { .. } => {
                Err("Sync is not supported in the interpreter. Use compiled Rust.".to_string())
            }
            // Phase 55: Mount now supported via VFS
            Stmt::Mount { var, path } => {
                let path_val = self.evaluate_expr(path).await?.to_display_string();
                match &self.vfs {
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

            // Phase 54: Go-like concurrency - not supported in interpreter
            // These are compile-to-Rust only features
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

            // Phase 63: Theorems are verified at compile-time, not executed
            Stmt::Theorem(_) => {
                // Theorems don't execute - they're processed by compile_theorem()
                Ok(ControlFlow::Continue)
            }
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
                        let variant_name = self.interner.resolve(variant);
                        if s.type_name == variant_name {
                            self.push_scope();
                            for (field_name, binding_name) in &arm.bindings {
                                let field_str = self.interner.resolve(*field_name);
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
                        let variant_name = self.interner.resolve(variant);
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
        Ok(ControlFlow::Continue)
    }

    /// Evaluate an expression to a runtime value.
    /// Phase 55: Now async.
    #[async_recursion(?Send)]
    async fn evaluate_expr(&mut self, expr: &Expr<'a>) -> Result<RuntimeValue, String> {
        match expr {
            Expr::Literal(lit) => self.evaluate_literal(lit),

            Expr::Identifier(sym) => {
                let name = self.interner.resolve(*sym);
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
                        if matches!(left_val, RuntimeValue::Int(_)) {
                            let right_val = self.evaluate_expr(right).await?;
                            return self.apply_binary_op(*op, left_val, right_val);
                        }
                        if !left_val.is_truthy() {
                            return Ok(RuntimeValue::Bool(false));
                        }
                        let right_val = self.evaluate_expr(right).await?;
                        Ok(RuntimeValue::Bool(right_val.is_truthy()))
                    }
                    BinaryOpKind::Or => {
                        let left_val = self.evaluate_expr(left).await?;
                        if matches!(left_val, RuntimeValue::Int(_)) {
                            let right_val = self.evaluate_expr(right).await?;
                            return self.apply_binary_op(*op, left_val, right_val);
                        }
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
                        let field_name = self.interner.resolve(*field);
                        s.fields.get(field_name).cloned()
                            .ok_or_else(|| format!("Field '{}' not found", field_name))
                    }
                    _ => Err(format!("Cannot access field on {}", obj_val.type_name())),
                }
            }

            Expr::New { type_name, init_fields, .. } => {
                let name = self.interner.resolve(*type_name).to_string();

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
                    let field_name = self.interner.resolve(*field_sym).to_string();
                    let field_val = self.evaluate_expr(field_expr).await?;
                    fields.insert(field_name, field_val);
                }

                if let Some(def) = self.struct_defs.get(type_name) {
                    for (field_sym, type_sym, _) in def {
                        let field_name = self.interner.resolve(*field_sym).to_string();
                        if !fields.contains_key(&field_name) {
                            let type_name_str = self.interner.resolve(*type_sym).to_string();
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
                let inductive_type = self.interner.resolve(*enum_name).to_string();
                let constructor = self.interner.resolve(*variant).to_string();

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
                            result.push_str(self.interner.resolve(*sym));
                        }
                        crate::ast::stmt::StringPart::Expr { value, format_spec, debug } => {
                            let val = self.evaluate_expr(value).await?;
                            if *debug {
                                let prefix = match value {
                                    Expr::Identifier(sym) => self.interner.resolve(*sym).to_string(),
                                    _ => "expr".to_string(),
                                };
                                result.push_str(&prefix);
                                result.push('=');
                            }
                            if let Some(spec_sym) = format_spec {
                                let spec = self.interner.resolve(*spec_sym);
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
                    if let Some(val) = self.env.lookup(*sym) {
                        captured_env.insert(*sym, val.deep_clone());
                    }
                }

                let body_index = self.closure_bodies.len();
                match body {
                    ClosureBody::Expression(expr) => {
                        self.closure_bodies.push(ClosureBodyRef::Expression(expr));
                    }
                    ClosureBody::Block(block) => {
                        self.closure_bodies.push(ClosureBodyRef::Block(block));
                    }
                }

                let param_names: Vec<Symbol> = params.iter().map(|(name, _)| *name).collect();

                Ok(RuntimeValue::Function(Box::new(ClosureValue {
                    body_index,
                    captured_env,
                    param_names,
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
            Literal::Text(sym) => Ok(RuntimeValue::Text(Rc::new(self.interner.resolve(*sym).to_string()))),
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
        if func_sym == self.sym_show {
            for arg in args {
                let val = self.evaluate_expr(arg).await?;
                self.emit_output(val.to_display_string());
            }
            return Ok(RuntimeValue::Nothing);
        } else if func_sym == self.sym_args {
            // `args()` system native: the stored argv as a `Seq of Text`,
            // mirroring the compiled binary's `env::args()`. Intercepted BEFORE
            // the empty native-decl body would be reached, like `show`.
            let items: Vec<RuntimeValue> = self
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
        if let Some(func) = self.functions.get(&function) {
            let param_count = func.params.len();
            let body = func.body;

            if args.len() != param_count {
                return Err(format!(
                    "Function {} expects {} arguments, got {}",
                    self.interner.resolve(function),
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
            if self.call_depth >= crate::semantics::MAX_CALL_DEPTH {
            return Err(crate::semantics::CALL_DEPTH_ERR.to_string());
        }
        self.call_depth += 1;
        self.env.push_frame();
            for i in 0..param_count {
                let param_name = self.functions[&function].params[i].0;
                self.env.define(param_name, std::mem::replace(&mut arg_values[i], RuntimeValue::Nothing));
            }

            // Execute function body
            // TCO on the async path — mirror of the sync twin in
            // `call_function_sync` (constant-stack self-tail-calls).
            let prev_tco = self.tco_fn_async.replace(function);
            let prev_repeat = std::mem::replace(&mut self.repeat_depth_async, 0);
            let mut return_value = RuntimeValue::Nothing;
            let mut body_err = None;
            'tco: loop {
                self.pending_tail_call_async = None;
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
                                None => self.pending_tail_call_async = Some(vals),
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
                match self.pending_tail_call_async.take() {
                    Some(new_args) => {
                        self.env.pop_frame();
                        self.env.push_frame();
                        for (i, v) in new_args.into_iter().enumerate() {
                            let param_name = self.functions[&function].params[i].0;
                            self.env.define(param_name, v);
                        }
                        continue 'tco;
                    }
                    None => break 'tco,
                }
            }
            self.repeat_depth_async = prev_repeat;
            self.tco_fn_async = prev_tco;

            self.env.pop_frame();
        self.call_depth -= 1;
            match body_err {
                Some(e) => Err(e),
                None => Ok(return_value),
            }
        } else {
            // Fallback: check if the function name is a variable holding a closure
            let maybe_closure = self.env.lookup(function)
                .and_then(|v| if let RuntimeValue::Function(c) = v { Some((**c).clone()) } else { None });

            if let Some(closure) = maybe_closure {
                let mut arg_values = Vec::with_capacity(args.len());
                for arg in args {
                    arg_values.push(self.evaluate_expr(arg).await?);
                }
                self.call_closure_value(&closure, arg_values).await
            } else {
                Err(format!("Unknown function: {}", self.interner.resolve(function)))
            }
        }
    }

    /// Call a function with pre-evaluated RuntimeValue arguments.
    /// Used by Give and Show statements where the object is already evaluated.
    #[async_recursion(?Send)]
    async fn call_function_with_values(&mut self, function: Symbol, mut args: Vec<RuntimeValue>) -> Result<RuntimeValue, String> {
        // Handle built-in "show" via Symbol comparison
        if Some(function) == self.sym_show {
            for val in args {
                self.emit_output(val.to_display_string());
            }
            return Ok(RuntimeValue::Nothing);
        }

        if let Some(func) = self.functions.get(&function) {
            let param_count = func.params.len();
            let body = func.body;

            if args.len() != param_count {
                return Err(format!(
                    "Function {} expects {} arguments, got {}",
                    self.interner.resolve(function), param_count, args.len()
                ));
            }

            if self.call_depth >= crate::semantics::MAX_CALL_DEPTH {
            return Err(crate::semantics::CALL_DEPTH_ERR.to_string());
        }
        self.call_depth += 1;
        self.env.push_frame();
            for i in 0..param_count {
                let param_name = self.functions[&function].params[i].0;
                self.env.define(param_name, std::mem::replace(&mut args[i], RuntimeValue::Nothing));
            }

            // TCO on the async path — mirror of the sync twin in
            // `call_function_sync` (constant-stack self-tail-calls).
            let prev_tco = self.tco_fn_async.replace(function);
            let prev_repeat = std::mem::replace(&mut self.repeat_depth_async, 0);
            let mut return_value = RuntimeValue::Nothing;
            let mut body_err = None;
            'tco: loop {
                self.pending_tail_call_async = None;
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
                                None => self.pending_tail_call_async = Some(vals),
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
                match self.pending_tail_call_async.take() {
                    Some(new_args) => {
                        self.env.pop_frame();
                        self.env.push_frame();
                        for (i, v) in new_args.into_iter().enumerate() {
                            let param_name = self.functions[&function].params[i].0;
                            self.env.define(param_name, v);
                        }
                        continue 'tco;
                    }
                    None => break 'tco,
                }
            }
            self.repeat_depth_async = prev_repeat;
            self.tco_fn_async = prev_tco;

            self.env.pop_frame();
        self.call_depth -= 1;
            match body_err {
                Some(e) => Err(e),
                None => Ok(return_value),
            }
        } else {
            let maybe_closure = self.env.lookup(function)
                .and_then(|v| if let RuntimeValue::Function(c) = v { Some((**c).clone()) } else { None });

            if let Some(closure) = maybe_closure {
                self.call_closure_value(&closure, args).await
            } else {
                Err(format!("Unknown function: {}", self.interner.resolve(function)))
            }
        }
    }

    /// Map a function symbol to its kernel builtin, via the cached symbols.
    fn builtin_id(&self, f: Symbol) -> Option<crate::semantics::builtins::BuiltinId> {
        use crate::semantics::builtins::BuiltinId as B;
        let s = Some(f);
        if s == self.sym_length {
            Some(B::Length)
        } else if s == self.sym_format {
            Some(B::Format)
        } else if s == self.sym_parse_int {
            Some(B::ParseInt)
        } else if s == self.sym_parse_float {
            Some(B::ParseFloat)
        } else if s == self.sym_chr {
            Some(B::Chr)
        } else if s == self.sym_abs {
            Some(B::Abs)
        } else if s == self.sym_sqrt {
            Some(B::Sqrt)
        } else if s == self.sym_min {
            Some(B::Min)
        } else if s == self.sym_max {
            Some(B::Max)
        } else if s == self.sym_floor {
            Some(B::Floor)
        } else if s == self.sym_ceil {
            Some(B::Ceil)
        } else if s == self.sym_round {
            Some(B::Round)
        } else if s == self.sym_pow {
            Some(B::Pow)
        } else if s == self.sym_copy {
            Some(B::Copy)
        } else if s == self.sym_count_ones {
            Some(B::CountOnes)
        } else {
            None
        }
    }

    // Scope management

    fn push_scope(&mut self) {
        self.env.push_scope();
    }

    fn pop_scope(&mut self) {
        self.env.pop_scope();
    }

    fn define(&mut self, name: Symbol, value: RuntimeValue) {
        self.env.define(name, value);
    }

    fn assign(&mut self, name: Symbol, value: RuntimeValue) -> Result<(), String> {
        if self.env.assign(name, value) {
            Ok(())
        } else {
            Err(format!("Undefined variable: {}", self.interner.resolve(name)))
        }
    }

    fn lookup(&self, name: Symbol) -> Result<&RuntimeValue, String> {
        self.env.lookup(name)
            .ok_or_else(|| format!("Undefined variable: {}", self.interner.resolve(name)))
    }

    /// Evaluate a policy condition against a subject value.
    fn evaluate_policy_condition(
        &self,
        condition: &PolicyCondition,
        subject: &RuntimeValue,
        object: Option<&RuntimeValue>,
    ) -> bool {
        crate::semantics::policy::evaluate_policy_condition(
            self.policy_registry.as_ref(),
            self.interner,
            condition,
            subject,
            object,
        )
    }

    // =========================================================================
    // Sync execution path — eliminates async/Future overhead for pure programs
    // =========================================================================

    /// Execute a program synchronously (no async/Future allocation).
    /// Use when `needs_async(stmts)` returns false.
    pub fn run_sync(&mut self, stmts: &[Stmt<'a>]) -> Result<(), String> {
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
                self.repeat_depth_sync += 1;
                for item in items {
                    match pattern {
                        Pattern::Identifier(sym) => {
                            self.define(*sym, item);
                        }
                        Pattern::Tuple(syms) => {
                            if let RuntimeValue::Tuple(ref tuple_vals) = item {
                                for (sym, val) in syms.iter().zip(tuple_vals.iter()) {
                                    self.define(*sym, val.clone());
                                }
                            } else {
                                self.repeat_depth_sync -= 1;
                                return Err(format!("Expected tuple for pattern, got {}", item.type_name()));
                            }
                        }
                    }

                    match self.execute_block_sync(body)? {
                        ControlFlow::Break => break,
                        ControlFlow::Return(v) => {
                            self.repeat_depth_sync -= 1;
                            self.pop_scope();
                            return Ok(ControlFlow::Return(v));
                        }
                        ControlFlow::Continue => {}
                    }
                }
                self.repeat_depth_sync -= 1;
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
                        self.pending_tail_call = Some(vals);
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
                self.functions.insert(*name, func);
                Ok(ControlFlow::Continue)
            }

            Stmt::StructDef { name, fields, .. } => {
                self.struct_defs.insert(*name, fields.clone());
                Ok(ControlFlow::Continue)
            }

            Stmt::SetField { object, field, value } => {
                let new_val = self.evaluate_expr_sync(value)?;
                if let Expr::Identifier(obj_sym) = object {
                    let mut obj_val = self.lookup(*obj_sym)?.clone();
                    if let RuntimeValue::Struct(ref mut s) = obj_val {
                        let field_name = self.interner.resolve(*field).to_string();
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
                    let coll_val = self.lookup(*coll_sym)?;
                    crate::semantics::collections::list_push(&coll_val, val)?;
                } else if let Expr::FieldAccess { object, field } = collection {
                    if let Expr::Identifier(obj_sym) = *object {
                        let obj_val = self.lookup(*obj_sym)?;
                        let field_name = self.interner.resolve(*field);
                        crate::semantics::collections::push_to_struct_field(&obj_val, field_name, val)?;
                    } else {
                        return Err("Push to nested field access not supported".to_string());
                    }
                } else {
                    return Err("Push collection must be an identifier or field access".to_string());
                }
                Ok(ControlFlow::Continue)
            }

            Stmt::Pop { collection, into } => {
                if let Expr::Identifier(coll_sym) = collection {
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
                    let coll_val = self.lookup(*coll_sym)?;
                    crate::semantics::collections::set_add(&coll_val, val)?;
                } else {
                    return Err("Add collection must be an identifier".to_string());
                }
                Ok(ControlFlow::Continue)
            }

            Stmt::Remove { value, collection } => {
                let val = self.evaluate_expr_sync(value)?;
                if let Expr::Identifier(coll_sym) = collection {
                    let coll_val = self.lookup(*coll_sym)?;
                    crate::semantics::collections::remove_from(&coll_val, &val)?;
                } else {
                    return Err("Remove collection must be an identifier".to_string());
                }
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
                    let coll_val = self.lookup(*coll_sym)?;
                    crate::semantics::collections::index_set(&coll_val, &idx_val, new_val)?;
                } else {
                    return Err("SetIndex collection must be an identifier".to_string());
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

            Stmt::RuntimeAssert { condition } => {
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
                    let name = self.interner.resolve(*sym);
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
                Ok(ControlFlow::Continue)
            }

            Stmt::AwaitMessage { into, .. } => {
                self.define(*into, RuntimeValue::Nothing);
                Ok(ControlFlow::Continue)
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
                        let field_name = self.interner.resolve(*field).to_string();
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
                        let field_name = self.interner.resolve(*field).to_string();
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

            Stmt::AppendToSequence { .. } => {
                Err("Append to sequence is not supported in the interpreter. Use compiled Rust.".to_string())
            }

            Stmt::ResolveConflict { .. } => {
                Err("Resolve conflict is not supported in the interpreter. Use compiled Rust.".to_string())
            }

            Stmt::Check { subject, predicate, is_capability, object, source_text, .. } => {
                let registry = match &self.policy_registry {
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
                    self.interner,
                    &subj_val,
                    *predicate,
                    *is_capability,
                    obj_val.as_ref(),
                    source_text,
                )?;
                Ok(ControlFlow::Continue)
            }

            Stmt::Listen { .. } => {
                Err("Listen is not supported in the interpreter. Use compiled Rust.".to_string())
            }
            Stmt::ConnectTo { .. } => {
                Err("Connect is not supported in the interpreter. Use compiled Rust.".to_string())
            }
            Stmt::LetPeerAgent { .. } => {
                Err("PeerAgent is not supported in the interpreter. Use compiled Rust.".to_string())
            }
            Stmt::Sleep { .. } => {
                Err("Sleep requires async execution path".to_string())
            }
            Stmt::Sync { .. } => {
                Err("Sync is not supported in the interpreter. Use compiled Rust.".to_string())
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

            Stmt::Theorem(_) => {
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
                        let variant_name = self.interner.resolve(variant);
                        if s.type_name == variant_name {
                            self.push_scope();
                            for (field_name, binding_name) in &arm.bindings {
                                let field_str = self.interner.resolve(*field_name);
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
                        let variant_name = self.interner.resolve(variant);
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
        Ok(ControlFlow::Continue)
    }

    fn evaluate_expr_sync(&mut self, expr: &Expr<'a>) -> Result<RuntimeValue, String> {
        match expr {
            Expr::Literal(lit) => self.evaluate_literal(lit),

            Expr::Identifier(sym) => {
                let name = self.interner.resolve(*sym);
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
                        if matches!(left_val, RuntimeValue::Int(_)) {
                            let right_val = self.evaluate_expr_sync(right)?;
                            return self.apply_binary_op(*op, left_val, right_val);
                        }
                        if !left_val.is_truthy() {
                            return Ok(RuntimeValue::Bool(false));
                        }
                        let right_val = self.evaluate_expr_sync(right)?;
                        Ok(RuntimeValue::Bool(right_val.is_truthy()))
                    }
                    BinaryOpKind::Or => {
                        let left_val = self.evaluate_expr_sync(left)?;
                        if matches!(left_val, RuntimeValue::Int(_)) {
                            let right_val = self.evaluate_expr_sync(right)?;
                            return self.apply_binary_op(*op, left_val, right_val);
                        }
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
                        let field_name = self.interner.resolve(*field);
                        s.fields.get(field_name).cloned()
                            .ok_or_else(|| format!("Field '{}' not found", field_name))
                    }
                    _ => Err(format!("Cannot access field on {}", obj_val.type_name())),
                }
            }

            Expr::New { type_name, init_fields, .. } => {
                let name = self.interner.resolve(*type_name).to_string();

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
                    let field_name = self.interner.resolve(*field_sym).to_string();
                    let field_val = self.evaluate_expr_sync(field_expr)?;
                    fields.insert(field_name, field_val);
                }

                if let Some(def) = self.struct_defs.get(type_name) {
                    for (field_sym, type_sym, _) in def {
                        let field_name = self.interner.resolve(*field_sym).to_string();
                        if !fields.contains_key(&field_name) {
                            let type_name_str = self.interner.resolve(*type_sym).to_string();
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
                                _ => RuntimeValue::Nothing,
                            };
                            fields.insert(field_name, default);
                        }
                    }
                }

                Ok(RuntimeValue::Struct(Box::new(StructValue { type_name: name, fields })))
            }

            Expr::NewVariant { enum_name, variant, fields } => {
                let inductive_type = self.interner.resolve(*enum_name).to_string();
                let constructor = self.interner.resolve(*variant).to_string();

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
                            result.push_str(self.interner.resolve(*sym));
                        }
                        crate::ast::stmt::StringPart::Expr { value, format_spec, debug } => {
                            let val = self.evaluate_expr_sync(value)?;
                            if *debug {
                                let prefix = match value {
                                    Expr::Identifier(sym) => self.interner.resolve(*sym).to_string(),
                                    _ => "expr".to_string(),
                                };
                                result.push_str(&prefix);
                                result.push('=');
                            }
                            if let Some(spec_sym) = format_spec {
                                let spec = self.interner.resolve(*spec_sym);
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
                    if let Some(val) = self.env.lookup(*sym) {
                        captured_env.insert(*sym, val.deep_clone());
                    }
                }

                let body_index = self.closure_bodies.len();
                match body {
                    ClosureBody::Expression(expr) => {
                        self.closure_bodies.push(ClosureBodyRef::Expression(expr));
                    }
                    ClosureBody::Block(block) => {
                        self.closure_bodies.push(ClosureBodyRef::Block(block));
                    }
                }

                let param_names: Vec<Symbol> = params.iter().map(|(name, _)| *name).collect();

                Ok(RuntimeValue::Function(Box::new(ClosureValue {
                    body_index,
                    captured_env,
                    param_names,
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
        if self.repeat_depth_sync != 0 {
            return None;
        }
        let cur = self.tco_fn_sync?;
        let param_count = self.functions.get(&cur)?.params.len();
        crate::tail_call::direct_self_tail_args(expr, cur, param_count)
    }

    /// ASYNC twin of [`Self::self_tail_call_args_sync`].
    fn self_tail_call_args_async(&self, expr: &'a Expr<'a>) -> Option<&'a [&'a Expr<'a>]> {
        if self.repeat_depth_async != 0 {
            return None;
        }
        let cur = self.tco_fn_async?;
        let param_count = self.functions.get(&cur)?.params.len();
        crate::tail_call::direct_self_tail_args(expr, cur, param_count)
    }

    fn call_function_sync(&mut self, function: Symbol, args: &[&Expr<'a>]) -> Result<RuntimeValue, String> {
        // Built-in functions — Symbol comparison (integer) instead of string matching
        let func_sym = Some(function);
        if func_sym == self.sym_show {
            for arg in args {
                let val = self.evaluate_expr_sync(arg)?;
                self.emit_output(val.to_display_string());
            }
            return Ok(RuntimeValue::Nothing);
        } else if func_sym == self.sym_args {
            // `args()` system native: the stored argv as a `Seq of Text`,
            // mirroring the compiled binary's `env::args()`. Must match the
            // async path AND the VM (the shadow oracle asserts VM ≡ tree-walker).
            let items: Vec<RuntimeValue> = self
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
        if let Some(func) = self.functions.get(&function) {
            let param_count = func.params.len();
            let body = func.body;

            if args.len() != param_count {
                return Err(format!(
                    "Function {} expects {} arguments, got {}",
                    self.interner.resolve(function),
                    param_count,
                    args.len()
                ));
            }

            let mut arg_values = Vec::with_capacity(param_count);
            for arg in args {
                arg_values.push(self.evaluate_expr_sync(arg)?);
            }

            if self.call_depth >= crate::semantics::MAX_CALL_DEPTH {
            return Err(crate::semantics::CALL_DEPTH_ERR.to_string());
        }
        self.call_depth += 1;
        self.env.push_frame();
            for i in 0..param_count {
                let param_name = self.functions[&function].params[i].0;
                self.env.define(param_name, std::mem::replace(&mut arg_values[i], RuntimeValue::Nothing));
            }

            // TCO: while executing THIS function's body, a self-tail-call is a
            // loop-back (reassign params + restart the body) rather than a real
            // recursive call. `tco_fn_sync`/`repeat_depth_sync` are per-activation,
            // so save the caller's and reset for this body.
            let prev_tco = self.tco_fn_sync.replace(function);
            let prev_repeat = std::mem::replace(&mut self.repeat_depth_sync, 0);
            let mut return_value = RuntimeValue::Nothing;
            let mut body_err = None;
            'tco: loop {
                self.pending_tail_call = None;
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
                                None => self.pending_tail_call = Some(vals),
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
                match self.pending_tail_call.take() {
                    Some(new_args) => {
                        // Loop-back: a fresh frame (no stale locals) with the
                        // reassigned parameters — constant stack, no depth bump.
                        self.env.pop_frame();
                        self.env.push_frame();
                        for (i, v) in new_args.into_iter().enumerate() {
                            let param_name = self.functions[&function].params[i].0;
                            self.env.define(param_name, v);
                        }
                        continue 'tco;
                    }
                    None => break 'tco,
                }
            }
            self.repeat_depth_sync = prev_repeat;
            self.tco_fn_sync = prev_tco;

            self.env.pop_frame();
        self.call_depth -= 1;
            match body_err {
                Some(e) => Err(e),
                None => Ok(return_value),
            }
        } else {
            // Fallback: check if the function name is a variable holding a closure
            let maybe_closure = self.env.lookup(function)
                .and_then(|v| if let RuntimeValue::Function(c) = v { Some((**c).clone()) } else { None });

            if let Some(closure) = maybe_closure {
                let mut arg_values = Vec::with_capacity(args.len());
                for arg in args {
                    arg_values.push(self.evaluate_expr_sync(arg)?);
                }
                self.call_closure_value_sync(&closure, arg_values)
            } else {
                Err(format!("Unknown function: {}", self.interner.resolve(function)))
            }
        }
    }

    fn call_function_with_values_sync(&mut self, function: Symbol, mut args: Vec<RuntimeValue>) -> Result<RuntimeValue, String> {
        // Handle built-in "show" via Symbol comparison
        if Some(function) == self.sym_show {
            for val in args {
                self.emit_output(val.to_display_string());
            }
            return Ok(RuntimeValue::Nothing);
        }

        if let Some(func) = self.functions.get(&function) {
            let param_count = func.params.len();
            let body = func.body;

            if args.len() != param_count {
                return Err(format!(
                    "Function {} expects {} arguments, got {}",
                    self.interner.resolve(function), param_count, args.len()
                ));
            }

            if self.call_depth >= crate::semantics::MAX_CALL_DEPTH {
            return Err(crate::semantics::CALL_DEPTH_ERR.to_string());
        }
        self.call_depth += 1;
        self.env.push_frame();
            for i in 0..param_count {
                let param_name = self.functions[&function].params[i].0;
                self.env.define(param_name, std::mem::replace(&mut args[i], RuntimeValue::Nothing));
            }

            // TCO: while executing THIS function's body, a self-tail-call is a
            // loop-back (reassign params + restart the body) rather than a real
            // recursive call. `tco_fn_sync`/`repeat_depth_sync` are per-activation,
            // so save the caller's and reset for this body.
            let prev_tco = self.tco_fn_sync.replace(function);
            let prev_repeat = std::mem::replace(&mut self.repeat_depth_sync, 0);
            let mut return_value = RuntimeValue::Nothing;
            let mut body_err = None;
            'tco: loop {
                self.pending_tail_call = None;
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
                                None => self.pending_tail_call = Some(vals),
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
                match self.pending_tail_call.take() {
                    Some(new_args) => {
                        // Loop-back: a fresh frame (no stale locals) with the
                        // reassigned parameters — constant stack, no depth bump.
                        self.env.pop_frame();
                        self.env.push_frame();
                        for (i, v) in new_args.into_iter().enumerate() {
                            let param_name = self.functions[&function].params[i].0;
                            self.env.define(param_name, v);
                        }
                        continue 'tco;
                    }
                    None => break 'tco,
                }
            }
            self.repeat_depth_sync = prev_repeat;
            self.tco_fn_sync = prev_tco;

            self.env.pop_frame();
        self.call_depth -= 1;
            match body_err {
                Some(e) => Err(e),
                None => Ok(return_value),
            }
        } else {
            let maybe_closure = self.env.lookup(function)
                .and_then(|v| if let RuntimeValue::Function(c) = v { Some((**c).clone()) } else { None });

            if let Some(closure) = maybe_closure {
                self.call_closure_value_sync(&closure, args)
            } else {
                Err(format!("Unknown function: {}", self.interner.resolve(function)))
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
                Stmt::RuntimeAssert { condition } => {
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

        // Extract body reference from side-table (breaks borrow on self)
        let body_index = closure.body_index;
        let is_block = matches!(self.closure_bodies.get(body_index), Some(ClosureBodyRef::Block(_)));

        // A closure body is a fresh frame (lexical barrier): it sees its
        // captures, its parameters, and globals — never the caller's locals.
        if self.call_depth >= crate::semantics::MAX_CALL_DEPTH {
            return Err(crate::semantics::CALL_DEPTH_ERR.to_string());
        }
        self.call_depth += 1;
        self.env.push_frame();

        // Bind captured environment
        for (sym, val) in &closure.captured_env {
            self.env.define(*sym, val.deep_clone());
        }

        // Bind parameters
        for (i, param_sym) in closure.param_names.iter().enumerate() {
            self.env.define(*param_sym, std::mem::replace(&mut arg_values[i], RuntimeValue::Nothing));
        }

        let result = if is_block {
            let block = match &self.closure_bodies[body_index] {
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
            let expr = match &self.closure_bodies[body_index] {
                ClosureBodyRef::Expression(e) => *e,
                _ => unreachable!(),
            };
            self.evaluate_expr(expr).await
        };

        self.env.pop_frame();
        self.call_depth -= 1;
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

        let body_index = closure.body_index;
        let is_block = matches!(self.closure_bodies.get(body_index), Some(ClosureBodyRef::Block(_)));

        // A closure body is a fresh frame (lexical barrier); see the async twin.
        if self.call_depth >= crate::semantics::MAX_CALL_DEPTH {
            return Err(crate::semantics::CALL_DEPTH_ERR.to_string());
        }
        self.call_depth += 1;
        self.env.push_frame();

        for (sym, val) in &closure.captured_env {
            self.env.define(*sym, val.deep_clone());
        }

        for (i, param_sym) in closure.param_names.iter().enumerate() {
            self.env.define(*param_sym, std::mem::replace(&mut arg_values[i], RuntimeValue::Nothing));
        }

        let result = if is_block {
            let block = match &self.closure_bodies[body_index] {
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
            let expr = match &self.closure_bodies[body_index] {
                ClosureBodyRef::Expression(e) => *e,
                _ => unreachable!(),
            };
            self.evaluate_expr_sync(expr)
        };

        self.env.pop_frame();
        self.call_depth -= 1;
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
