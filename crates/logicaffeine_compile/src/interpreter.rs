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

#[derive(Debug, Clone)]
pub enum RuntimeValue {
    Int(i64),
    Float(f64),
    Bool(bool),
    Text(Rc<String>),
    Char(char),
    List(Rc<RefCell<Vec<RuntimeValue>>>),
    Tuple(Rc<Vec<RuntimeValue>>),
    Set(Rc<RefCell<Vec<RuntimeValue>>>),
    Map(Rc<RefCell<HashMap<RuntimeValue, RuntimeValue>>>),
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
                let cloned = items.borrow().iter().map(|v| v.deep_clone()).collect();
                RuntimeValue::List(Rc::new(RefCell::new(cloned)))
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
                let parts: Vec<String> = items.iter().map(|v| v.to_display_string()).collect();
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
                // Convert nanoseconds since epoch to ISO-8601-like datetime
                let total_seconds = *nanos / 1_000_000_000;
                let days = (total_seconds / 86400) as i32;
                let day_seconds = total_seconds % 86400;
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
struct Environment {
    values: HashMap<Symbol, RuntimeValue>,
    save_stack: Vec<Vec<(Symbol, Option<RuntimeValue>)>>,
}

impl Environment {
    fn new() -> Self {
        Environment {
            values: HashMap::new(),
            save_stack: Vec::new(),
        }
    }

    fn push_scope(&mut self) {
        self.save_stack.push(Vec::new());
    }

    fn pop_scope(&mut self) {
        if let Some(saves) = self.save_stack.pop() {
            for (sym, old_val) in saves.into_iter().rev() {
                match old_val {
                    Some(val) => { self.values.insert(sym, val); }
                    None => { self.values.remove(&sym); }
                }
            }
        }
    }

    fn define(&mut self, name: Symbol, value: RuntimeValue) {
        let old = self.values.insert(name, value);
        if let Some(frame) = self.save_stack.last_mut() {
            frame.push((name, old));
        }
    }

    fn lookup(&self, name: Symbol) -> Option<&RuntimeValue> {
        self.values.get(&name)
    }

    fn assign(&mut self, name: Symbol, value: RuntimeValue) -> bool {
        if self.values.contains_key(&name) {
            self.values.insert(name, value);
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
    // Pre-interned builtin function symbols for O(1) dispatch
    sym_show: Option<Symbol>,
    sym_length: Option<Symbol>,
    sym_format: Option<Symbol>,
    sym_parse_int: Option<Symbol>,
    sym_parse_float: Option<Symbol>,
    sym_abs: Option<Symbol>,
    sym_min: Option<Symbol>,
    sym_max: Option<Symbol>,
    sym_copy: Option<Symbol>,
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
            sym_show: interner.lookup("show"),
            sym_length: interner.lookup("length"),
            sym_format: interner.lookup("format"),
            sym_parse_int: interner.lookup("parseInt"),
            sym_parse_float: interner.lookup("parseFloat"),
            sym_abs: interner.lookup("abs"),
            sym_min: interner.lookup("min"),
            sym_max: interner.lookup("max"),
            sym_copy: interner.lookup("copy"),
        }
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
                let items: Vec<RuntimeValue> = match &iter_val {
                    RuntimeValue::List(list) => list.borrow().clone(),
                    RuntimeValue::Set(set) => set.borrow().clone(),
                    RuntimeValue::Text(s) => {
                        s.chars().map(|c| RuntimeValue::Text(Rc::new(c.to_string()))).collect()
                    }
                    RuntimeValue::Map(map) => {
                        map.borrow().iter()
                            .map(|(k, v)| RuntimeValue::Tuple(Rc::new(vec![k.clone(), v.clone()])))
                            .collect()
                    }
                    _ => return Err(format!("Cannot iterate over {}", iter_val.type_name())),
                };

                self.push_scope();
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
                                return Err(format!("Expected tuple for pattern, got {}", item.type_name()));
                            }
                        }
                    }

                    match self.execute_block(body).await? {
                        ControlFlow::Break => break,
                        ControlFlow::Return(v) => {
                            self.pop_scope();
                            return Ok(ControlFlow::Return(v));
                        }
                        ControlFlow::Continue => {}
                    }
                }
                self.pop_scope();
                Ok(ControlFlow::Continue)
            }

            Stmt::Return { value } => {
                let ret_val = match value {
                    Some(expr) => self.evaluate_expr(expr).await?,
                    None => RuntimeValue::Nothing,
                };
                Ok(ControlFlow::Return(ret_val))
            }

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
                    if let RuntimeValue::List(items) = coll_val {
                        items.borrow_mut().push(val);
                    } else {
                        return Err("Can only push to a List".to_string());
                    }
                } else if let Expr::FieldAccess { object, field } = collection {
                    if let Expr::Identifier(obj_sym) = *object {
                        let obj_val = self.lookup(*obj_sym)?;
                        if let RuntimeValue::Struct(s) = obj_val {
                            let field_name = self.interner.resolve(*field).to_string();
                            if let Some(RuntimeValue::List(items)) = s.fields.get(&field_name) {
                                items.borrow_mut().push(val);
                            } else {
                                return Err(format!("Field '{}' is not a List", field_name));
                            }
                        } else {
                            return Err("Cannot push to field of non-struct".to_string());
                        }
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
                    if let RuntimeValue::List(items) = coll_val {
                        let popped = items.borrow_mut().pop().unwrap_or(RuntimeValue::Nothing);
                        if let Some(into_var) = into {
                            self.define(*into_var, popped);
                        }
                    } else {
                        return Err("Can only pop from a List".to_string());
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
                    if let RuntimeValue::Set(items) = coll_val {
                        let already_present = items.borrow().iter().any(|x| self.values_equal(x, &val));
                        if !already_present {
                            items.borrow_mut().push(val);
                        }
                    } else {
                        return Err("Can only add to a Set".to_string());
                    }
                } else {
                    return Err("Add collection must be an identifier".to_string());
                }
                Ok(ControlFlow::Continue)
            }

            Stmt::Remove { value, collection } => {
                let val = self.evaluate_expr(value).await?;
                if let Expr::Identifier(coll_sym) = collection {
                    let coll_val = self.lookup(*coll_sym)?;
                    if let RuntimeValue::Set(items) = coll_val {
                        items.borrow_mut().retain(|x| !self.values_equal(x, &val));
                    } else {
                        return Err("Can only remove from a Set".to_string());
                    }
                } else {
                    return Err("Remove collection must be an identifier".to_string());
                }
                Ok(ControlFlow::Continue)
            }

            Stmt::SetIndex { collection, index, value } => {
                let idx_val = self.evaluate_expr(index).await?;
                let new_val = self.evaluate_expr(value).await?;
                if let Expr::Identifier(coll_sym) = collection {
                    let coll_val = self.lookup(*coll_sym)?;
                    match (coll_val, &idx_val) {
                        (RuntimeValue::List(items), RuntimeValue::Int(n)) => {
                            let idx = *n as usize;
                            let mut items = items.borrow_mut();
                            if idx == 0 || idx > items.len() {
                                return Err(format!("Index {} out of bounds for list of length {}", idx, items.len()));
                            }
                            items[idx - 1] = new_val;
                        }
                        (RuntimeValue::Map(map), key) => {
                            map.borrow_mut().insert(key.clone(), new_val);
                        }
                        (RuntimeValue::List(_), _) => {
                            return Err("List index must be an integer".to_string());
                        }
                        _ => {
                            return Err(format!("Cannot index into {}", coll_val.type_name()));
                        }
                    }
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

                            let merged = match (&current, &source_field_val) {
                                (RuntimeValue::Int(a), RuntimeValue::Int(b)) => {
                                    RuntimeValue::Int(a + b)
                                }
                                _ => source_field_val,
                            };
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

                        let new_val = match current {
                            RuntimeValue::Int(n) => RuntimeValue::Int(n + amount_int),
                            RuntimeValue::Nothing => RuntimeValue::Int(amount_int),
                            _ => return Err(format!("Field '{}' is not a counter", field_name)),
                        };
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

                        let new_val = match current {
                            RuntimeValue::Int(n) => RuntimeValue::Int(n - amount_int),
                            RuntimeValue::Nothing => RuntimeValue::Int(-amount_int),
                            _ => return Err(format!("Field '{}' is not a counter", field_name)),
                        };
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
                    RuntimeValue::Int(ms) => ms * 1_000_000, // Convert ms to nanos
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
                // Handle temporal builtins
                match name {
                    "today" => {
                        #[cfg(not(target_arch = "wasm32"))]
                        {
                            use std::time::{SystemTime, UNIX_EPOCH};
                            let duration = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
                            let days = (duration.as_secs() / 86400) as i32;
                            return Ok(RuntimeValue::Date(days));
                        }
                        #[cfg(target_arch = "wasm32")]
                        {
                            return Ok(RuntimeValue::Date(0)); // Placeholder for WASM
                        }
                    }
                    "now" => {
                        #[cfg(not(target_arch = "wasm32"))]
                        {
                            use std::time::{SystemTime, UNIX_EPOCH};
                            let duration = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
                            let nanos = duration.as_nanos() as i64;
                            return Ok(RuntimeValue::Moment(nanos));
                        }
                        #[cfg(target_arch = "wasm32")]
                        {
                            return Ok(RuntimeValue::Moment(0)); // Placeholder for WASM
                        }
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
                match (&coll_val, &idx_val) {
                    (RuntimeValue::List(items), RuntimeValue::Int(idx)) => {
                        let idx = *idx as usize;
                        let items = items.borrow();
                        if idx == 0 || idx > items.len() {
                            return Err(format!("Index {} out of bounds", idx));
                        }
                        Ok(items[idx - 1].clone())
                    }
                    (RuntimeValue::Tuple(items), RuntimeValue::Int(idx)) => {
                        let idx = *idx as usize;
                        if idx == 0 || idx > items.len() {
                            return Err(format!("Index {} out of bounds", idx));
                        }
                        Ok(items[idx - 1].clone())
                    }
                    (RuntimeValue::Text(s), RuntimeValue::Int(idx)) => {
                        let idx = *idx as usize;
                        if idx == 0 || idx > s.chars().count() {
                            return Err(format!("Index {} out of bounds", idx));
                        }
                        Ok(RuntimeValue::Text(Rc::new(s.chars().nth(idx - 1).unwrap().to_string())))
                    }
                    (RuntimeValue::Map(map), key) => {
                        let map = map.borrow();
                        match map.get(key) {
                            Some(val) => Ok(val.clone()),
                            None => Err(format!("Key '{}' not found in map", key.to_display_string())),
                        }
                    }
                    _ => Err(format!("Cannot index {} with {}", coll_val.type_name(), idx_val.type_name())),
                }
            }

            Expr::Slice { collection, start, end } => {
                let coll_val = self.evaluate_expr(collection).await?;
                let start_val = self.evaluate_expr(start).await?;
                let end_val = self.evaluate_expr(end).await?;
                match (&coll_val, &start_val, &end_val) {
                    (RuntimeValue::List(items), RuntimeValue::Int(s), RuntimeValue::Int(e)) => {
                        let items = items.borrow();
                        let start = (*s as usize).saturating_sub(1);
                        let end = *e as usize;
                        let slice: Vec<RuntimeValue> = items.get(start..end).unwrap_or(&[]).to_vec();
                        Ok(RuntimeValue::List(Rc::new(RefCell::new(slice))))
                    }
                    _ => Err("Slice requires List and Int indices".to_string()),
                }
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
                match &coll_val {
                    RuntimeValue::List(items) => Ok(RuntimeValue::Int(items.borrow().len() as i64)),
                    RuntimeValue::Tuple(items) => Ok(RuntimeValue::Int(items.len() as i64)),
                    RuntimeValue::Set(items) => Ok(RuntimeValue::Int(items.borrow().len() as i64)),
                    RuntimeValue::Text(s) => Ok(RuntimeValue::Int(s.len() as i64)),
                    RuntimeValue::Map(map) => Ok(RuntimeValue::Int(map.borrow().len() as i64)),
                    _ => Err(format!("Cannot get length of {}", coll_val.type_name())),
                }
            }

            Expr::Contains { collection, value } => {
                let coll_val = self.evaluate_expr(collection).await?;
                let val = self.evaluate_expr(value).await?;
                match &coll_val {
                    RuntimeValue::Set(items) => {
                        let items = items.borrow();
                        let found = items.iter().any(|item| self.values_equal(item, &val));
                        Ok(RuntimeValue::Bool(found))
                    }
                    RuntimeValue::List(items) => {
                        let items = items.borrow();
                        let found = items.iter().any(|item| self.values_equal(item, &val));
                        Ok(RuntimeValue::Bool(found))
                    }
                    RuntimeValue::Map(entries) => {
                        Ok(RuntimeValue::Bool(entries.borrow().contains_key(&val)))
                    }
                    RuntimeValue::Text(s) => {
                        if let RuntimeValue::Text(needle) = &val {
                            Ok(RuntimeValue::Bool(s.contains(needle.as_str())))
                        } else if let RuntimeValue::Char(c) = &val {
                            Ok(RuntimeValue::Bool(s.contains(*c)))
                        } else {
                            Err(format!("Cannot check if Text contains {}", val.type_name()))
                        }
                    }
                    _ => Err(format!("Cannot check contains on {}", coll_val.type_name())),
                }
            }

            Expr::Union { left, right } => {
                let left_val = self.evaluate_expr(left).await?;
                let right_val = self.evaluate_expr(right).await?;
                match (&left_val, &right_val) {
                    (RuntimeValue::Set(a), RuntimeValue::Set(b)) => {
                        let a = a.borrow();
                        let b = b.borrow();
                        let mut result = a.clone();
                        for item in b.iter() {
                            if !result.iter().any(|x| self.values_equal(x, item)) {
                                result.push(item.clone());
                            }
                        }
                        Ok(RuntimeValue::Set(Rc::new(RefCell::new(result))))
                    }
                    _ => Err(format!("Cannot union {} and {}", left_val.type_name(), right_val.type_name())),
                }
            }

            Expr::Intersection { left, right } => {
                let left_val = self.evaluate_expr(left).await?;
                let right_val = self.evaluate_expr(right).await?;
                match (&left_val, &right_val) {
                    (RuntimeValue::Set(a), RuntimeValue::Set(b)) => {
                        let a = a.borrow();
                        let b = b.borrow();
                        let result: Vec<RuntimeValue> = a.iter()
                            .filter(|item| b.iter().any(|x| self.values_equal(x, item)))
                            .cloned()
                            .collect();
                        Ok(RuntimeValue::Set(Rc::new(RefCell::new(result))))
                    }
                    _ => Err(format!("Cannot intersect {} and {}", left_val.type_name(), right_val.type_name())),
                }
            }

            Expr::List(items) => {
                let mut values = Vec::with_capacity(items.len());
                for e in items.iter() {
                    values.push(self.evaluate_expr(e).await?);
                }
                Ok(RuntimeValue::List(Rc::new(RefCell::new(values))))
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
                match (&start_val, &end_val) {
                    (RuntimeValue::Int(s), RuntimeValue::Int(e)) => {
                        let range: Vec<RuntimeValue> = (*s..=*e)
                            .map(RuntimeValue::Int)
                            .collect();
                        Ok(RuntimeValue::List(Rc::new(RefCell::new(range))))
                    }
                    _ => Err("Range requires Int bounds".to_string()),
                }
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
                    return Ok(RuntimeValue::List(Rc::new(RefCell::new(vec![]))));
                }

                if name == "Set" || name == "HashSet" {
                    return Ok(RuntimeValue::Set(Rc::new(RefCell::new(vec![]))));
                }

                if name == "Map" || name == "HashMap" {
                    return Ok(RuntimeValue::Map(Rc::new(RefCell::new(HashMap::new()))));
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
                                "Seq" | "List" => RuntimeValue::List(Rc::new(RefCell::new(vec![]))),
                                "Set" | "HashSet" => RuntimeValue::Set(Rc::new(RefCell::new(vec![]))),
                                "Map" | "HashMap" => RuntimeValue::Map(Rc::new(RefCell::new(HashMap::new()))),
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
                Ok(RuntimeValue::List(Rc::new(RefCell::new(vec![]))))
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

    /// Apply a binary operator.
    fn apply_binary_op(&self, op: BinaryOpKind, left: RuntimeValue, right: RuntimeValue) -> Result<RuntimeValue, String> {
        match op {
            BinaryOpKind::Add => self.apply_add(left, right),
            BinaryOpKind::Subtract => self.apply_subtract(left, right),
            BinaryOpKind::Multiply => self.apply_multiply(left, right),
            BinaryOpKind::Divide => self.apply_divide(left, right),
            BinaryOpKind::Modulo => self.apply_modulo(left, right),
            BinaryOpKind::Eq => Ok(RuntimeValue::Bool(self.values_equal(&left, &right))),
            BinaryOpKind::NotEq => Ok(RuntimeValue::Bool(!self.values_equal(&left, &right))),
            BinaryOpKind::Lt => self.apply_comparison(left, right, |a, b| a < b),
            BinaryOpKind::Gt => self.apply_comparison(left, right, |a, b| a > b),
            BinaryOpKind::LtEq => self.apply_comparison(left, right, |a, b| a <= b),
            BinaryOpKind::GtEq => self.apply_comparison(left, right, |a, b| a >= b),
            BinaryOpKind::And => Ok(RuntimeValue::Bool(left.is_truthy() && right.is_truthy())),
            BinaryOpKind::Or => Ok(RuntimeValue::Bool(left.is_truthy() || right.is_truthy())),
            // Phase 53: String concatenation
            BinaryOpKind::Concat => self.apply_concat(left, right),
        }
    }

    fn apply_add(&self, left: RuntimeValue, right: RuntimeValue) -> Result<RuntimeValue, String> {
        match (&left, &right) {
            (RuntimeValue::Int(a), RuntimeValue::Int(b)) => Ok(RuntimeValue::Int(a + b)),
            (RuntimeValue::Float(a), RuntimeValue::Float(b)) => Ok(RuntimeValue::Float(a + b)),
            (RuntimeValue::Int(a), RuntimeValue::Float(b)) => Ok(RuntimeValue::Float(*a as f64 + b)),
            (RuntimeValue::Float(a), RuntimeValue::Int(b)) => Ok(RuntimeValue::Float(a + *b as f64)),
            (RuntimeValue::Text(a), RuntimeValue::Text(b)) => Ok(RuntimeValue::Text(Rc::new(format!("{}{}", a, b)))),
            (RuntimeValue::Text(a), other) => Ok(RuntimeValue::Text(Rc::new(format!("{}{}", a, other.to_display_string())))),
            (other, RuntimeValue::Text(b)) => Ok(RuntimeValue::Text(Rc::new(format!("{}{}", other.to_display_string(), b)))),
            (RuntimeValue::Duration(a), RuntimeValue::Duration(b)) => Ok(RuntimeValue::Duration(a + b)),
            (RuntimeValue::Date(days), RuntimeValue::Span { months, days: span_days }) => {
                let result_days = Self::date_add_span(*days, *months, *span_days);
                Ok(RuntimeValue::Date(result_days))
            }
            _ => Err(format!("Cannot add {} and {}", left.type_name(), right.type_name())),
        }
    }

    fn apply_concat(&self, left: RuntimeValue, right: RuntimeValue) -> Result<RuntimeValue, String> {
        Ok(RuntimeValue::Text(Rc::new(format!("{}{}", left.to_display_string(), right.to_display_string()))))
    }

    fn apply_subtract(&self, left: RuntimeValue, right: RuntimeValue) -> Result<RuntimeValue, String> {
        match (&left, &right) {
            (RuntimeValue::Int(a), RuntimeValue::Int(b)) => Ok(RuntimeValue::Int(a - b)),
            (RuntimeValue::Float(a), RuntimeValue::Float(b)) => Ok(RuntimeValue::Float(a - b)),
            (RuntimeValue::Int(a), RuntimeValue::Float(b)) => Ok(RuntimeValue::Float(*a as f64 - b)),
            (RuntimeValue::Float(a), RuntimeValue::Int(b)) => Ok(RuntimeValue::Float(a - *b as f64)),
            // Duration subtraction
            (RuntimeValue::Duration(a), RuntimeValue::Duration(b)) => Ok(RuntimeValue::Duration(a - b)),
            // Date - Span → Date (calendar-aware)
            (RuntimeValue::Date(days), RuntimeValue::Span { months, days: span_days }) => {
                let result_days = Self::date_add_span(*days, -*months, -*span_days);
                Ok(RuntimeValue::Date(result_days))
            }
            _ => Err(format!("Cannot subtract {} from {}", right.type_name(), left.type_name())),
        }
    }

    fn apply_multiply(&self, left: RuntimeValue, right: RuntimeValue) -> Result<RuntimeValue, String> {
        match (&left, &right) {
            (RuntimeValue::Int(a), RuntimeValue::Int(b)) => Ok(RuntimeValue::Int(a * b)),
            (RuntimeValue::Float(a), RuntimeValue::Float(b)) => Ok(RuntimeValue::Float(a * b)),
            (RuntimeValue::Int(a), RuntimeValue::Float(b)) => Ok(RuntimeValue::Float(*a as f64 * b)),
            (RuntimeValue::Float(a), RuntimeValue::Int(b)) => Ok(RuntimeValue::Float(a * *b as f64)),
            _ => Err(format!("Cannot multiply {} and {}", left.type_name(), right.type_name())),
        }
    }

    fn apply_divide(&self, left: RuntimeValue, right: RuntimeValue) -> Result<RuntimeValue, String> {
        match (&left, &right) {
            (RuntimeValue::Int(a), RuntimeValue::Int(b)) => {
                if *b == 0 {
                    return Err("Division by zero".to_string());
                }
                Ok(RuntimeValue::Int(a / b))
            }
            (RuntimeValue::Float(a), RuntimeValue::Float(b)) => {
                if *b == 0.0 {
                    return Err("Division by zero".to_string());
                }
                Ok(RuntimeValue::Float(a / b))
            }
            (RuntimeValue::Int(a), RuntimeValue::Float(b)) => {
                if *b == 0.0 {
                    return Err("Division by zero".to_string());
                }
                Ok(RuntimeValue::Float(*a as f64 / b))
            }
            (RuntimeValue::Float(a), RuntimeValue::Int(b)) => {
                if *b == 0 {
                    return Err("Division by zero".to_string());
                }
                Ok(RuntimeValue::Float(a / *b as f64))
            }
            _ => Err(format!("Cannot divide {} by {}", left.type_name(), right.type_name())),
        }
    }

    fn apply_modulo(&self, left: RuntimeValue, right: RuntimeValue) -> Result<RuntimeValue, String> {
        match (&left, &right) {
            (RuntimeValue::Int(a), RuntimeValue::Int(b)) => {
                if *b == 0 {
                    return Err("Modulo by zero".to_string());
                }
                Ok(RuntimeValue::Int(a % b))
            }
            _ => Err(format!("Cannot compute modulo of {} and {}", left.type_name(), right.type_name())),
        }
    }

    /// Add months and days to a date (calendar-aware).
    /// Uses Howard Hinnant's date algorithms for correct month-end handling.
    fn date_add_span(days_since_epoch: i32, months: i32, days: i32) -> i32 {
        // Convert days since epoch to (year, month, day)
        // Using Howard Hinnant's algorithm (same as in to_display_string)
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

        // Add months
        let total_months = (year * 12 + month - 1) + months;
        year = total_months / 12;
        month = total_months % 12 + 1;
        if month <= 0 {
            month += 12;
            year -= 1;
        }

        // Clamp day to valid range for new month
        let days_in_month = Self::days_in_month(year, month);
        if day > days_in_month {
            day = days_in_month;
        }

        // Convert back to days since epoch
        let yp = year - if month <= 2 { 1 } else { 0 };
        let era2 = if yp >= 0 { yp / 400 } else { (yp - 399) / 400 };
        let yoe2 = (yp - era2 * 400) as u32;
        let mp2 = if month > 2 { month as u32 - 3 } else { month as u32 + 9 };
        let doy2 = (153 * mp2 + 2) / 5 + day as u32 - 1;
        let doe2 = yoe2 * 365 + yoe2 / 4 - yoe2 / 100 + doy2;
        let result = era2 * 146097 + doe2 as i32 - 719468;

        // Add days
        result + days
    }

    /// Get the number of days in a given month (1-indexed).
    fn days_in_month(year: i32, month: i32) -> i32 {
        match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 => {
                // Leap year check
                let is_leap = (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0);
                if is_leap { 29 } else { 28 }
            }
            _ => 30, // Fallback
        }
    }

    fn apply_comparison<F>(&self, left: RuntimeValue, right: RuntimeValue, cmp: F) -> Result<RuntimeValue, String>
    where
        F: Fn(i64, i64) -> bool,
    {
        match (&left, &right) {
            (RuntimeValue::Int(a), RuntimeValue::Int(b)) => Ok(RuntimeValue::Bool(cmp(*a, *b))),
            // Duration comparison (nanoseconds)
            (RuntimeValue::Duration(a), RuntimeValue::Duration(b)) => Ok(RuntimeValue::Bool(cmp(*a, *b))),
            // Date comparison (days)
            (RuntimeValue::Date(a), RuntimeValue::Date(b)) => Ok(RuntimeValue::Bool(cmp(*a as i64, *b as i64))),
            // Moment comparison (nanoseconds)
            (RuntimeValue::Moment(a), RuntimeValue::Moment(b)) => Ok(RuntimeValue::Bool(cmp(*a, *b))),
            // Time-of-day comparison (nanoseconds from midnight)
            (RuntimeValue::Time(a), RuntimeValue::Time(b)) => Ok(RuntimeValue::Bool(cmp(*a, *b))),
            // Moment vs Time: extract time-of-day from Moment
            (RuntimeValue::Moment(m), RuntimeValue::Time(t)) => {
                let nanos_per_day = 86_400_000_000_000i64;
                let moment_tod = *m % nanos_per_day;
                Ok(RuntimeValue::Bool(cmp(moment_tod, *t)))
            }
            (RuntimeValue::Time(t), RuntimeValue::Moment(m)) => {
                let nanos_per_day = 86_400_000_000_000i64;
                let moment_tod = *m % nanos_per_day;
                Ok(RuntimeValue::Bool(cmp(*t, moment_tod)))
            }
            _ => Err(format!("Cannot compare {} and {}", left.type_name(), right.type_name())),
        }
    }

    pub fn values_equal_pub(&self, left: &RuntimeValue, right: &RuntimeValue) -> bool {
        self.values_equal(left, right)
    }

    fn values_equal(&self, left: &RuntimeValue, right: &RuntimeValue) -> bool {
        match (left, right) {
            (RuntimeValue::Int(a), RuntimeValue::Int(b)) => a == b,
            (RuntimeValue::Float(a), RuntimeValue::Float(b)) => (a - b).abs() < f64::EPSILON,
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
            (RuntimeValue::Inductive(a), RuntimeValue::Inductive(b)) => {
                a.inductive_type == b.inductive_type && a.constructor == b.constructor &&
                a.args.len() == b.args.len() &&
                a.args.iter().zip(b.args.iter()).all(|(x, y)| self.values_equal(x, y))
            }
            _ => false,
        }
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
        } else if func_sym == self.sym_length {
            if args.len() != 1 {
                return Err("length() takes exactly 1 argument".to_string());
            }
            let val = self.evaluate_expr(args[0]).await?;
            return match &val {
                RuntimeValue::List(items) => Ok(RuntimeValue::Int(items.borrow().len() as i64)),
                RuntimeValue::Text(s) => Ok(RuntimeValue::Int(s.len() as i64)),
                RuntimeValue::Map(map) => Ok(RuntimeValue::Int(map.borrow().len() as i64)),
                _ => Err(format!("Cannot get length of {}", val.type_name())),
            };
        } else if func_sym == self.sym_format {
            if args.is_empty() {
                return Ok(RuntimeValue::Text(Rc::new(String::new())));
            }
            let val = self.evaluate_expr(args[0]).await?;
            return Ok(RuntimeValue::Text(Rc::new(val.to_display_string())));
        } else if func_sym == self.sym_parse_int {
            if args.len() != 1 {
                return Err("parseInt() takes exactly 1 argument".to_string());
            }
            let val = self.evaluate_expr(args[0]).await?;
            if let RuntimeValue::Text(s) = &val {
                return Ok(RuntimeValue::Int(s.trim().parse::<i64>()
                    .map_err(|_| format!("Cannot parse '{}' as Int", s))?));
            }
            return Err("parseInt requires a Text argument".to_string());
        } else if func_sym == self.sym_parse_float {
            if args.len() != 1 {
                return Err("parseFloat() takes exactly 1 argument".to_string());
            }
            let val = self.evaluate_expr(args[0]).await?;
            if let RuntimeValue::Text(s) = &val {
                return Ok(RuntimeValue::Float(s.trim().parse::<f64>()
                    .map_err(|_| format!("Cannot parse '{}' as Float", s))?));
            }
            return Err("parseFloat requires a Text argument".to_string());
        } else if func_sym == self.sym_abs {
            if args.len() != 1 {
                return Err("abs() takes exactly 1 argument".to_string());
            }
            let val = self.evaluate_expr(args[0]).await?;
            return match val {
                RuntimeValue::Int(n) => Ok(RuntimeValue::Int(n.abs())),
                RuntimeValue::Float(f) => Ok(RuntimeValue::Float(f.abs())),
                _ => Err(format!("abs() requires a number, got {}", val.type_name())),
            };
        } else if func_sym == self.sym_min {
            if args.len() != 2 {
                return Err("min() takes exactly 2 arguments".to_string());
            }
            let a = self.evaluate_expr(args[0]).await?;
            let b = self.evaluate_expr(args[1]).await?;
            return match (&a, &b) {
                (RuntimeValue::Int(x), RuntimeValue::Int(y)) => Ok(RuntimeValue::Int(*x.min(y))),
                _ => Err("min() requires integers".to_string()),
            };
        } else if func_sym == self.sym_max {
            if args.len() != 2 {
                return Err("max() takes exactly 2 arguments".to_string());
            }
            let a = self.evaluate_expr(args[0]).await?;
            let b = self.evaluate_expr(args[1]).await?;
            return match (&a, &b) {
                (RuntimeValue::Int(x), RuntimeValue::Int(y)) => Ok(RuntimeValue::Int(*x.max(y))),
                _ => Err("max() requires integers".to_string()),
            };
        } else if func_sym == self.sym_copy {
            if args.len() != 1 {
                return Err("copy() takes exactly 1 argument".to_string());
            }
            let val = self.evaluate_expr(args[0]).await?;
            return Ok(val.deep_clone());
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

            // Bind parameters — re-borrow self.functions for param names (no clone needed)
            self.push_scope();
            for i in 0..param_count {
                let param_name = self.functions[&function].params[i].0;
                self.env.define(param_name, std::mem::replace(&mut arg_values[i], RuntimeValue::Nothing));
            }

            // Execute function body
            let mut return_value = RuntimeValue::Nothing;
            for stmt in body.iter() {
                match self.execute_stmt(stmt).await? {
                    ControlFlow::Return(val) => {
                        return_value = val;
                        break;
                    }
                    ControlFlow::Break => break,
                    ControlFlow::Continue => {}
                }
            }

            self.pop_scope();
            Ok(return_value)
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

            self.push_scope();
            for i in 0..param_count {
                let param_name = self.functions[&function].params[i].0;
                self.env.define(param_name, std::mem::replace(&mut args[i], RuntimeValue::Nothing));
            }

            let mut return_value = RuntimeValue::Nothing;
            for stmt in body.iter() {
                match self.execute_stmt(stmt).await? {
                    ControlFlow::Return(val) => {
                        return_value = val;
                        break;
                    }
                    ControlFlow::Break => break,
                    ControlFlow::Continue => {}
                }
            }

            self.pop_scope();
            Ok(return_value)
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
        match condition {
            PolicyCondition::FieldEquals { field, value, is_string_literal } => {
                if let RuntimeValue::Struct(s) = subject {
                    let field_name = self.interner.resolve(*field);
                    if let Some(field_val) = s.fields.get(field_name) {
                        let expected = self.interner.resolve(*value);
                        match field_val {
                            RuntimeValue::Text(s) => s.as_str() == expected,
                            RuntimeValue::Int(n) => {
                                if *is_string_literal {
                                    false
                                } else {
                                    expected.parse::<i64>().map(|e| *n == e).unwrap_or(false)
                                }
                            }
                            RuntimeValue::Bool(b) => {
                                if *is_string_literal {
                                    false
                                } else {
                                    expected.parse::<bool>().map(|e| *b == e).unwrap_or(false)
                                }
                            }
                            _ => false,
                        }
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            PolicyCondition::FieldBool { field, value } => {
                if let RuntimeValue::Struct(s) = subject {
                    let field_name = self.interner.resolve(*field);
                    if let Some(RuntimeValue::Bool(b)) = s.fields.get(field_name) {
                        *b == *value
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            PolicyCondition::Predicate { predicate, .. } => {
                if let Some(registry) = &self.policy_registry {
                    if let RuntimeValue::Struct(s) = subject {
                        if let Some(subj_type_sym) = self.interner.lookup(&s.type_name) {
                            if let Some(preds) = registry.get_predicates(subj_type_sym) {
                                if let Some(pred) = preds.iter().find(|p| p.predicate_name == *predicate) {
                                    return self.evaluate_policy_condition(&pred.condition, subject, object);
                                }
                            }
                        }
                    }
                }
                false
            }
            PolicyCondition::ObjectFieldEquals { subject: subj_field, object: obj_sym, field } => {
                let obj = match object {
                    Some(o) => o,
                    None => return false,
                };
                if let (RuntimeValue::Struct(subj_s), RuntimeValue::Struct(obj_s)) = (subject, obj) {
                    let subj_field_name = self.interner.resolve(*subj_field);
                    let obj_field_name = self.interner.resolve(*field);
                    if let (Some(subj_val), Some(obj_val)) = (subj_s.fields.get(subj_field_name), obj_s.fields.get(obj_field_name)) {
                        self.values_equal(subj_val, obj_val)
                    } else {
                        let _obj_sym_name = self.interner.resolve(*obj_sym);
                        false
                    }
                } else {
                    false
                }
            }
            PolicyCondition::Or(left, right) => {
                self.evaluate_policy_condition(left, subject, object)
                    || self.evaluate_policy_condition(right, subject, object)
            }
            PolicyCondition::And(left, right) => {
                self.evaluate_policy_condition(left, subject, object)
                    && self.evaluate_policy_condition(right, subject, object)
            }
        }
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
                let items: Vec<RuntimeValue> = match &iter_val {
                    RuntimeValue::List(list) => list.borrow().clone(),
                    RuntimeValue::Set(set) => set.borrow().clone(),
                    RuntimeValue::Text(s) => {
                        s.chars().map(|c| RuntimeValue::Text(Rc::new(c.to_string()))).collect()
                    }
                    RuntimeValue::Map(map) => {
                        map.borrow().iter()
                            .map(|(k, v)| RuntimeValue::Tuple(Rc::new(vec![k.clone(), v.clone()])))
                            .collect()
                    }
                    _ => return Err(format!("Cannot iterate over {}", iter_val.type_name())),
                };

                self.push_scope();
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
                                return Err(format!("Expected tuple for pattern, got {}", item.type_name()));
                            }
                        }
                    }

                    match self.execute_block_sync(body)? {
                        ControlFlow::Break => break,
                        ControlFlow::Return(v) => {
                            self.pop_scope();
                            return Ok(ControlFlow::Return(v));
                        }
                        ControlFlow::Continue => {}
                    }
                }
                self.pop_scope();
                Ok(ControlFlow::Continue)
            }

            Stmt::Return { value } => {
                let ret_val = match value {
                    Some(expr) => self.evaluate_expr_sync(expr)?,
                    None => RuntimeValue::Nothing,
                };
                Ok(ControlFlow::Return(ret_val))
            }

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
                    if let RuntimeValue::List(items) = coll_val {
                        items.borrow_mut().push(val);
                    } else {
                        return Err("Can only push to a List".to_string());
                    }
                } else if let Expr::FieldAccess { object, field } = collection {
                    if let Expr::Identifier(obj_sym) = *object {
                        let obj_val = self.lookup(*obj_sym)?;
                        if let RuntimeValue::Struct(s) = obj_val {
                            let field_name = self.interner.resolve(*field).to_string();
                            if let Some(RuntimeValue::List(items)) = s.fields.get(&field_name) {
                                items.borrow_mut().push(val);
                            } else {
                                return Err(format!("Field '{}' is not a List", field_name));
                            }
                        } else {
                            return Err("Cannot push to field of non-struct".to_string());
                        }
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
                    if let RuntimeValue::List(items) = coll_val {
                        let popped = items.borrow_mut().pop().unwrap_or(RuntimeValue::Nothing);
                        if let Some(into_var) = into {
                            self.define(*into_var, popped);
                        }
                    } else {
                        return Err("Can only pop from a List".to_string());
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
                    if let RuntimeValue::Set(items) = coll_val {
                        let already_present = items.borrow().iter().any(|x| self.values_equal(x, &val));
                        if !already_present {
                            items.borrow_mut().push(val);
                        }
                    } else {
                        return Err("Can only add to a Set".to_string());
                    }
                } else {
                    return Err("Add collection must be an identifier".to_string());
                }
                Ok(ControlFlow::Continue)
            }

            Stmt::Remove { value, collection } => {
                let val = self.evaluate_expr_sync(value)?;
                if let Expr::Identifier(coll_sym) = collection {
                    let coll_val = self.lookup(*coll_sym)?;
                    if let RuntimeValue::Set(items) = coll_val {
                        items.borrow_mut().retain(|x| !self.values_equal(x, &val));
                    } else {
                        return Err("Can only remove from a Set".to_string());
                    }
                } else {
                    return Err("Remove collection must be an identifier".to_string());
                }
                Ok(ControlFlow::Continue)
            }

            Stmt::SetIndex { collection, index, value } => {
                let idx_val = self.evaluate_expr_sync(index)?;
                let new_val = self.evaluate_expr_sync(value)?;
                if let Expr::Identifier(coll_sym) = collection {
                    let coll_val = self.lookup(*coll_sym)?;
                    match (coll_val, &idx_val) {
                        (RuntimeValue::List(items), RuntimeValue::Int(n)) => {
                            let idx = *n as usize;
                            let mut items = items.borrow_mut();
                            if idx == 0 || idx > items.len() {
                                return Err(format!("Index {} out of bounds for list of length {}", idx, items.len()));
                            }
                            items[idx - 1] = new_val;
                        }
                        (RuntimeValue::Map(map), key) => {
                            map.borrow_mut().insert(key.clone(), new_val);
                        }
                        (RuntimeValue::List(_), _) => {
                            return Err("List index must be an integer".to_string());
                        }
                        _ => {
                            return Err(format!("Cannot index into {}", coll_val.type_name()));
                        }
                    }
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

                            let merged = match (&current, &source_field_val) {
                                (RuntimeValue::Int(a), RuntimeValue::Int(b)) => {
                                    RuntimeValue::Int(a + b)
                                }
                                _ => source_field_val,
                            };
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

                        let new_val = match current {
                            RuntimeValue::Int(n) => RuntimeValue::Int(n + amount_int),
                            RuntimeValue::Nothing => RuntimeValue::Int(amount_int),
                            _ => return Err(format!("Field '{}' is not a counter", field_name)),
                        };
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

                        let new_val = match current {
                            RuntimeValue::Int(n) => RuntimeValue::Int(n - amount_int),
                            RuntimeValue::Nothing => RuntimeValue::Int(-amount_int),
                            _ => return Err(format!("Field '{}' is not a counter", field_name)),
                        };
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
                let subj_type_name = match &subj_val {
                    RuntimeValue::Struct(s) => s.type_name.clone(),
                    _ => return Err(format!("Check subject must be a struct, got {}", subj_val.type_name())),
                };

                let subj_type_sym = match self.interner.lookup(&subj_type_name) {
                    Some(sym) => sym,
                    None => return Err(format!("Unknown type '{}' in Check statement", subj_type_name)),
                };

                let passed = if *is_capability {
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
                match name {
                    "today" => {
                        #[cfg(not(target_arch = "wasm32"))]
                        {
                            use std::time::{SystemTime, UNIX_EPOCH};
                            let duration = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
                            let days = (duration.as_secs() / 86400) as i32;
                            return Ok(RuntimeValue::Date(days));
                        }
                        #[cfg(target_arch = "wasm32")]
                        {
                            return Ok(RuntimeValue::Date(0));
                        }
                    }
                    "now" => {
                        #[cfg(not(target_arch = "wasm32"))]
                        {
                            use std::time::{SystemTime, UNIX_EPOCH};
                            let duration = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
                            let nanos = duration.as_nanos() as i64;
                            return Ok(RuntimeValue::Moment(nanos));
                        }
                        #[cfg(target_arch = "wasm32")]
                        {
                            return Ok(RuntimeValue::Moment(0));
                        }
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
                match (&coll_val, &idx_val) {
                    (RuntimeValue::List(items), RuntimeValue::Int(idx)) => {
                        let idx = *idx as usize;
                        let items = items.borrow();
                        if idx == 0 || idx > items.len() {
                            return Err(format!("Index {} out of bounds", idx));
                        }
                        Ok(items[idx - 1].clone())
                    }
                    (RuntimeValue::Tuple(items), RuntimeValue::Int(idx)) => {
                        let idx = *idx as usize;
                        if idx == 0 || idx > items.len() {
                            return Err(format!("Index {} out of bounds", idx));
                        }
                        Ok(items[idx - 1].clone())
                    }
                    (RuntimeValue::Text(s), RuntimeValue::Int(idx)) => {
                        let idx = *idx as usize;
                        if idx == 0 || idx > s.chars().count() {
                            return Err(format!("Index {} out of bounds", idx));
                        }
                        Ok(RuntimeValue::Text(Rc::new(s.chars().nth(idx - 1).unwrap().to_string())))
                    }
                    (RuntimeValue::Map(map), key) => {
                        let map = map.borrow();
                        match map.get(key) {
                            Some(val) => Ok(val.clone()),
                            None => Err(format!("Key '{}' not found in map", key.to_display_string())),
                        }
                    }
                    _ => Err(format!("Cannot index {} with {}", coll_val.type_name(), idx_val.type_name())),
                }
            }

            Expr::Slice { collection, start, end } => {
                let coll_val = self.evaluate_expr_sync(collection)?;
                let start_val = self.evaluate_expr_sync(start)?;
                let end_val = self.evaluate_expr_sync(end)?;
                match (&coll_val, &start_val, &end_val) {
                    (RuntimeValue::List(items), RuntimeValue::Int(s), RuntimeValue::Int(e)) => {
                        let items = items.borrow();
                        let start = (*s as usize).saturating_sub(1);
                        let end = *e as usize;
                        let slice: Vec<RuntimeValue> = items.get(start..end).unwrap_or(&[]).to_vec();
                        Ok(RuntimeValue::List(Rc::new(RefCell::new(slice))))
                    }
                    _ => Err("Slice requires List and Int indices".to_string()),
                }
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
                match &coll_val {
                    RuntimeValue::List(items) => Ok(RuntimeValue::Int(items.borrow().len() as i64)),
                    RuntimeValue::Tuple(items) => Ok(RuntimeValue::Int(items.len() as i64)),
                    RuntimeValue::Set(items) => Ok(RuntimeValue::Int(items.borrow().len() as i64)),
                    RuntimeValue::Text(s) => Ok(RuntimeValue::Int(s.len() as i64)),
                    RuntimeValue::Map(map) => Ok(RuntimeValue::Int(map.borrow().len() as i64)),
                    _ => Err(format!("Cannot get length of {}", coll_val.type_name())),
                }
            }

            Expr::Contains { collection, value } => {
                let coll_val = self.evaluate_expr_sync(collection)?;
                let val = self.evaluate_expr_sync(value)?;
                match &coll_val {
                    RuntimeValue::Set(items) => {
                        let items = items.borrow();
                        let found = items.iter().any(|item| self.values_equal(item, &val));
                        Ok(RuntimeValue::Bool(found))
                    }
                    RuntimeValue::List(items) => {
                        let items = items.borrow();
                        let found = items.iter().any(|item| self.values_equal(item, &val));
                        Ok(RuntimeValue::Bool(found))
                    }
                    RuntimeValue::Map(entries) => {
                        Ok(RuntimeValue::Bool(entries.borrow().contains_key(&val)))
                    }
                    RuntimeValue::Text(s) => {
                        if let RuntimeValue::Text(needle) = &val {
                            Ok(RuntimeValue::Bool(s.contains(needle.as_str())))
                        } else if let RuntimeValue::Char(c) = &val {
                            Ok(RuntimeValue::Bool(s.contains(*c)))
                        } else {
                            Err(format!("Cannot check if Text contains {}", val.type_name()))
                        }
                    }
                    _ => Err(format!("Cannot check contains on {}", coll_val.type_name())),
                }
            }

            Expr::Union { left, right } => {
                let left_val = self.evaluate_expr_sync(left)?;
                let right_val = self.evaluate_expr_sync(right)?;
                match (&left_val, &right_val) {
                    (RuntimeValue::Set(a), RuntimeValue::Set(b)) => {
                        let a = a.borrow();
                        let b = b.borrow();
                        let mut result = a.clone();
                        for item in b.iter() {
                            if !result.iter().any(|x| self.values_equal(x, item)) {
                                result.push(item.clone());
                            }
                        }
                        Ok(RuntimeValue::Set(Rc::new(RefCell::new(result))))
                    }
                    _ => Err(format!("Cannot union {} and {}", left_val.type_name(), right_val.type_name())),
                }
            }

            Expr::Intersection { left, right } => {
                let left_val = self.evaluate_expr_sync(left)?;
                let right_val = self.evaluate_expr_sync(right)?;
                match (&left_val, &right_val) {
                    (RuntimeValue::Set(a), RuntimeValue::Set(b)) => {
                        let a = a.borrow();
                        let b = b.borrow();
                        let result: Vec<RuntimeValue> = a.iter()
                            .filter(|item| b.iter().any(|x| self.values_equal(x, item)))
                            .cloned()
                            .collect();
                        Ok(RuntimeValue::Set(Rc::new(RefCell::new(result))))
                    }
                    _ => Err(format!("Cannot intersect {} and {}", left_val.type_name(), right_val.type_name())),
                }
            }

            Expr::List(items) => {
                let mut values = Vec::with_capacity(items.len());
                for e in items.iter() {
                    values.push(self.evaluate_expr_sync(e)?);
                }
                Ok(RuntimeValue::List(Rc::new(RefCell::new(values))))
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
                match (&start_val, &end_val) {
                    (RuntimeValue::Int(s), RuntimeValue::Int(e)) => {
                        let range: Vec<RuntimeValue> = (*s..=*e)
                            .map(RuntimeValue::Int)
                            .collect();
                        Ok(RuntimeValue::List(Rc::new(RefCell::new(range))))
                    }
                    _ => Err("Range requires Int bounds".to_string()),
                }
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
                    return Ok(RuntimeValue::List(Rc::new(RefCell::new(vec![]))));
                }

                if name == "Set" || name == "HashSet" {
                    return Ok(RuntimeValue::Set(Rc::new(RefCell::new(vec![]))));
                }

                if name == "Map" || name == "HashMap" {
                    return Ok(RuntimeValue::Map(Rc::new(RefCell::new(HashMap::new()))));
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
                                "Seq" | "List" => RuntimeValue::List(Rc::new(RefCell::new(vec![]))),
                                "Set" | "HashSet" => RuntimeValue::Set(Rc::new(RefCell::new(vec![]))),
                                "Map" | "HashMap" => RuntimeValue::Map(Rc::new(RefCell::new(HashMap::new()))),
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
                Ok(RuntimeValue::List(Rc::new(RefCell::new(vec![]))))
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

    fn call_function_sync(&mut self, function: Symbol, args: &[&Expr<'a>]) -> Result<RuntimeValue, String> {
        // Built-in functions — Symbol comparison (integer) instead of string matching
        let func_sym = Some(function);
        if func_sym == self.sym_show {
            for arg in args {
                let val = self.evaluate_expr_sync(arg)?;
                self.emit_output(val.to_display_string());
            }
            return Ok(RuntimeValue::Nothing);
        } else if func_sym == self.sym_length {
            if args.len() != 1 {
                return Err("length() takes exactly 1 argument".to_string());
            }
            let val = self.evaluate_expr_sync(args[0])?;
            return match &val {
                RuntimeValue::List(items) => Ok(RuntimeValue::Int(items.borrow().len() as i64)),
                RuntimeValue::Text(s) => Ok(RuntimeValue::Int(s.len() as i64)),
                RuntimeValue::Map(map) => Ok(RuntimeValue::Int(map.borrow().len() as i64)),
                _ => Err(format!("Cannot get length of {}", val.type_name())),
            };
        } else if func_sym == self.sym_format {
            if args.is_empty() {
                return Ok(RuntimeValue::Text(Rc::new(String::new())));
            }
            let val = self.evaluate_expr_sync(args[0])?;
            return Ok(RuntimeValue::Text(Rc::new(val.to_display_string())));
        } else if func_sym == self.sym_parse_int {
            if args.len() != 1 {
                return Err("parseInt() takes exactly 1 argument".to_string());
            }
            let val = self.evaluate_expr_sync(args[0])?;
            if let RuntimeValue::Text(s) = &val {
                return Ok(RuntimeValue::Int(s.trim().parse::<i64>()
                    .map_err(|_| format!("Cannot parse '{}' as Int", s))?));
            }
            return Err("parseInt requires a Text argument".to_string());
        } else if func_sym == self.sym_parse_float {
            if args.len() != 1 {
                return Err("parseFloat() takes exactly 1 argument".to_string());
            }
            let val = self.evaluate_expr_sync(args[0])?;
            if let RuntimeValue::Text(s) = &val {
                return Ok(RuntimeValue::Float(s.trim().parse::<f64>()
                    .map_err(|_| format!("Cannot parse '{}' as Float", s))?));
            }
            return Err("parseFloat requires a Text argument".to_string());
        } else if func_sym == self.sym_abs {
            if args.len() != 1 {
                return Err("abs() takes exactly 1 argument".to_string());
            }
            let val = self.evaluate_expr_sync(args[0])?;
            return match val {
                RuntimeValue::Int(n) => Ok(RuntimeValue::Int(n.abs())),
                RuntimeValue::Float(f) => Ok(RuntimeValue::Float(f.abs())),
                _ => Err(format!("abs() requires a number, got {}", val.type_name())),
            };
        } else if func_sym == self.sym_min {
            if args.len() != 2 {
                return Err("min() takes exactly 2 arguments".to_string());
            }
            let a = self.evaluate_expr_sync(args[0])?;
            let b = self.evaluate_expr_sync(args[1])?;
            return match (&a, &b) {
                (RuntimeValue::Int(x), RuntimeValue::Int(y)) => Ok(RuntimeValue::Int(*x.min(y))),
                _ => Err("min() requires integers".to_string()),
            };
        } else if func_sym == self.sym_max {
            if args.len() != 2 {
                return Err("max() takes exactly 2 arguments".to_string());
            }
            let a = self.evaluate_expr_sync(args[0])?;
            let b = self.evaluate_expr_sync(args[1])?;
            return match (&a, &b) {
                (RuntimeValue::Int(x), RuntimeValue::Int(y)) => Ok(RuntimeValue::Int(*x.max(y))),
                _ => Err("max() requires integers".to_string()),
            };
        } else if func_sym == self.sym_copy {
            if args.len() != 1 {
                return Err("copy() takes exactly 1 argument".to_string());
            }
            let val = self.evaluate_expr_sync(args[0])?;
            return Ok(val.deep_clone());
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

            self.push_scope();
            for i in 0..param_count {
                let param_name = self.functions[&function].params[i].0;
                self.env.define(param_name, std::mem::replace(&mut arg_values[i], RuntimeValue::Nothing));
            }

            let mut return_value = RuntimeValue::Nothing;
            for stmt in body.iter() {
                match self.execute_stmt_sync(stmt)? {
                    ControlFlow::Return(val) => {
                        return_value = val;
                        break;
                    }
                    ControlFlow::Break => break,
                    ControlFlow::Continue => {}
                }
            }

            self.pop_scope();
            Ok(return_value)
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

            self.push_scope();
            for i in 0..param_count {
                let param_name = self.functions[&function].params[i].0;
                self.env.define(param_name, std::mem::replace(&mut args[i], RuntimeValue::Nothing));
            }

            let mut return_value = RuntimeValue::Nothing;
            for stmt in body.iter() {
                match self.execute_stmt_sync(stmt)? {
                    ControlFlow::Return(val) => {
                        return_value = val;
                        break;
                    }
                    ControlFlow::Break => break,
                    ControlFlow::Continue => {}
                }
            }

            self.pop_scope();
            Ok(return_value)
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
    fn collect_free_vars_in_closure(
        &self,
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
            Expr::Copy { expr: e } | Expr::Give { value: e } | Expr::Length { collection: e } => {
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
        }
    }

    fn collect_symbols_from_block(
        stmts: &[Stmt<'a>],
        exclude: &std::collections::HashSet<Symbol>,
        out: &mut Vec<Symbol>,
        seen: &mut std::collections::HashSet<Symbol>,
    ) {
        for stmt in stmts {
            match stmt {
                Stmt::Let { value, .. } => {
                    Self::collect_symbols_from_expr(value, exclude, out, seen);
                }
                Stmt::Set { value, .. } => {
                    Self::collect_symbols_from_expr(value, exclude, out, seen);
                }
                Stmt::Call { function, args } => {
                    if !exclude.contains(function) && seen.insert(*function) {
                        out.push(*function);
                    }
                    for arg in args {
                        Self::collect_symbols_from_expr(arg, exclude, out, seen);
                    }
                }
                Stmt::Return { value: Some(e) } => {
                    Self::collect_symbols_from_expr(e, exclude, out, seen);
                }
                Stmt::If { cond, then_block, else_block } => {
                    Self::collect_symbols_from_expr(cond, exclude, out, seen);
                    Self::collect_symbols_from_block(then_block, exclude, out, seen);
                    if let Some(eb) = else_block {
                        Self::collect_symbols_from_block(eb, exclude, out, seen);
                    }
                }
                Stmt::While { cond, body, .. } => {
                    Self::collect_symbols_from_expr(cond, exclude, out, seen);
                    Self::collect_symbols_from_block(body, exclude, out, seen);
                }
                Stmt::Repeat { iterable, body, .. } => {
                    Self::collect_symbols_from_expr(iterable, exclude, out, seen);
                    Self::collect_symbols_from_block(body, exclude, out, seen);
                }
                Stmt::Show { object, .. } | Stmt::Give { object, .. } => {
                    Self::collect_symbols_from_expr(object, exclude, out, seen);
                }
                Stmt::Push { value, collection } | Stmt::Add { value, collection }
                | Stmt::Remove { value, collection } => {
                    Self::collect_symbols_from_expr(value, exclude, out, seen);
                    Self::collect_symbols_from_expr(collection, exclude, out, seen);
                }
                Stmt::SetIndex { collection, index, value } => {
                    Self::collect_symbols_from_expr(collection, exclude, out, seen);
                    Self::collect_symbols_from_expr(index, exclude, out, seen);
                    Self::collect_symbols_from_expr(value, exclude, out, seen);
                }
                Stmt::SetField { object, value, .. } => {
                    Self::collect_symbols_from_expr(object, exclude, out, seen);
                    Self::collect_symbols_from_expr(value, exclude, out, seen);
                }
                Stmt::RuntimeAssert { condition } => {
                    Self::collect_symbols_from_expr(condition, exclude, out, seen);
                }
                Stmt::Zone { body, .. } => {
                    Self::collect_symbols_from_block(body, exclude, out, seen);
                }
                Stmt::Inspect { target, arms, .. } => {
                    Self::collect_symbols_from_expr(target, exclude, out, seen);
                    for arm in arms {
                        Self::collect_symbols_from_block(arm.body, exclude, out, seen);
                    }
                }
                Stmt::Pop { collection, .. } => {
                    Self::collect_symbols_from_expr(collection, exclude, out, seen);
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

        self.push_scope();

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
            let mut return_value = RuntimeValue::Nothing;
            for stmt in block.iter() {
                match self.execute_stmt(stmt).await? {
                    ControlFlow::Return(val) => {
                        return_value = val;
                        break;
                    }
                    ControlFlow::Break => break,
                    ControlFlow::Continue => {}
                }
            }
            Ok(return_value)
        } else {
            let expr = match &self.closure_bodies[body_index] {
                ClosureBodyRef::Expression(e) => *e,
                _ => unreachable!(),
            };
            self.evaluate_expr(expr).await
        };

        self.pop_scope();
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

        self.push_scope();

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
            let mut return_value = RuntimeValue::Nothing;
            for stmt in block.iter() {
                match self.execute_stmt_sync(stmt)? {
                    ControlFlow::Return(val) => {
                        return_value = val;
                        break;
                    }
                    ControlFlow::Break => break,
                    ControlFlow::Continue => {}
                }
            }
            Ok(return_value)
        } else {
            let expr = match &self.closure_bodies[body_index] {
                ClosureBodyRef::Expression(e) => *e,
                _ => unreachable!(),
            };
            self.evaluate_expr_sync(expr)
        };

        self.pop_scope();
        result
    }
}

/// Check whether a program requires async execution.
///
/// Only 4 statement types need async: ReadFrom (file), WriteFile, Sleep, Mount.
/// If none are present, the sync execution path can be used for better performance.
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
