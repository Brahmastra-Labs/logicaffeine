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

use async_recursion::async_recursion;

use crate::ast::stmt::{BinaryOpKind, Block, Expr, Literal, MatchArm, ReadSource, Stmt, TypeExpr};
use crate::intern::{Interner, Symbol};
use crate::analysis::{PolicyRegistry, PolicyCondition};

// VFS imports for async file operations
use logicaffeine_system::fs::Vfs;

/// Runtime values during LOGOS interpretation.
///
/// Represents all possible values that can exist at runtime when executing
/// a LOGOS program. Includes primitives, collections, user-defined structs,
/// and kernel inductive types.
#[derive(Debug, Clone, PartialEq)]
pub enum RuntimeValue {
    /// Signed 64-bit integer.
    Int(i64),
    /// 64-bit floating-point number.
    Float(f64),
    /// Boolean value.
    Bool(bool),
    /// UTF-8 string.
    Text(String),
    /// Single Unicode character.
    Char(char),
    /// Ordered, indexable collection of values.
    List(Vec<RuntimeValue>),
    /// Fixed-size heterogeneous tuple.
    Tuple(Vec<RuntimeValue>),
    /// Unordered collection with unique values.
    Set(Vec<RuntimeValue>),
    /// Key-value mapping with string keys.
    Map(HashMap<String, RuntimeValue>),
    /// User-defined struct with named fields.
    Struct {
        /// The struct's type name.
        type_name: String,
        /// Field name to value mapping.
        fields: HashMap<String, RuntimeValue>,
    },
    /// Kernel inductive value.
    ///
    /// Represents a value of a kernel-defined inductive type. At compile time,
    /// the kernel verifies exhaustiveness and type correctness. At runtime,
    /// this is a thin wrapper holding the constructor and arguments.
    ///
    /// This follows the "Dual Life" architecture:
    /// - **Soul (Kernel)**: Full inductive type with proofs
    /// - **Body (Rust)**: Efficient runtime representation
    Inductive {
        /// The inductive type name (e.g., "Nat", "List", "Color")
        inductive_type: String,
        /// The constructor name (e.g., "Zero", "Succ", "Red")
        constructor: String,
        /// Constructor arguments (e.g., Succ has one Nat argument)
        args: Vec<RuntimeValue>,
    },
    /// Unit/void value representing absence of a meaningful value.
    Nothing,
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
            RuntimeValue::Struct { .. } => "Struct",
            RuntimeValue::Inductive { inductive_type, .. } => inductive_type.as_str(),
            RuntimeValue::Nothing => "Nothing",
        }
    }

    /// Checks if this value evaluates to true in a boolean context.
    ///
    /// - `Bool(true)` → true
    /// - `Int(n)` → true if n ≠ 0
    /// - `Nothing` → false
    /// - All other values → true
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
            RuntimeValue::Text(s) => s.clone(),
            RuntimeValue::Char(c) => c.to_string(),
            RuntimeValue::List(items) => {
                let parts: Vec<String> = items.iter().map(|v| v.to_display_string()).collect();
                format!("[{}]", parts.join(", "))
            }
            RuntimeValue::Tuple(items) => {
                let parts: Vec<String> = items.iter().map(|v| v.to_display_string()).collect();
                format!("({})", parts.join(", "))
            }
            RuntimeValue::Set(items) => {
                let parts: Vec<String> = items.iter().map(|v| v.to_display_string()).collect();
                format!("{{{}}}", parts.join(", "))
            }
            RuntimeValue::Map(m) => {
                let pairs: Vec<String> = m.iter()
                    .map(|(k, v)| format!("{}: {}", k, v.to_display_string()))
                    .collect();
                format!("{{{}}}", pairs.join(", "))
            }
            RuntimeValue::Struct { type_name, fields } => {
                if fields.is_empty() {
                    // Unit variant - just show the name
                    type_name.clone()
                } else {
                    let field_strs: Vec<String> = fields
                        .iter()
                        .map(|(k, v)| format!("{}: {}", k, v.to_display_string()))
                        .collect();
                    format!("{} {{ {} }}", type_name, field_strs.join(", "))
                }
            }
            RuntimeValue::Inductive { constructor, args, .. } => {
                if args.is_empty() {
                    // Nullary constructor (e.g., Zero, Nil, Red)
                    constructor.clone()
                } else {
                    // Constructor with arguments (e.g., Succ(Zero), Cons(1, Nil))
                    let arg_strs: Vec<String> = args
                        .iter()
                        .map(|v| v.to_display_string())
                        .collect();
                    format!("{}({})", constructor, arg_strs.join(", "))
                }
            }
            RuntimeValue::Nothing => "nothing".to_string(),
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
pub struct Interpreter<'a> {
    interner: &'a Interner,
    /// Scope stack - each HashMap is a scope level
    env: Vec<HashMap<Symbol, RuntimeValue>>,
    /// User-defined functions
    functions: HashMap<Symbol, FunctionDef<'a>>,
    /// Struct type definitions (for constructor validation)
    struct_defs: HashMap<Symbol, Vec<(Symbol, Symbol, bool)>>,
    /// Output lines from show() calls
    pub output: Vec<String>,
    /// Phase 55: VFS for file operations (OPFS on WASM, NativeVfs on native)
    vfs: Option<Arc<dyn Vfs>>,
    /// Phase 102: Kernel context for inductive type lookup.
    /// When set, the interpreter can query the kernel for inductive types
    /// and their constructors, enabling the "Dual Life" architecture.
    kernel_ctx: Option<Arc<crate::kernel::Context>>,
    /// Policy registry for security predicate/capability checks.
    policy_registry: Option<PolicyRegistry>,
}

impl<'a> Interpreter<'a> {
    pub fn new(interner: &'a Interner) -> Self {
        Interpreter {
            interner,
            env: vec![HashMap::new()], // Global scope
            functions: HashMap::new(),
            struct_defs: HashMap::new(),
            output: Vec::new(),
            vfs: None,
            kernel_ctx: None,
            policy_registry: None,
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

            Stmt::Repeat { var, iterable, body } => {
                let iter_val = self.evaluate_expr(iterable).await?;
                let items = match iter_val {
                    RuntimeValue::List(list) => list,
                    RuntimeValue::Set(set) => set,
                    RuntimeValue::Text(s) => {
                        s.chars().map(|c| RuntimeValue::Text(c.to_string())).collect()
                    }
                    _ => return Err(format!("Cannot iterate over {}", iter_val.type_name())),
                };

                self.push_scope();
                for item in items {
                    self.define(*var, item);
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
                    if let RuntimeValue::Struct { fields, .. } = &mut obj_val {
                        let field_name = self.interner.resolve(*field).to_string();
                        fields.insert(field_name, new_val);
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
                    let mut coll_val = self.lookup(*coll_sym)?.clone();
                    if let RuntimeValue::List(ref mut items) = coll_val {
                        items.push(val);
                        self.assign(*coll_sym, coll_val)?;
                    } else {
                        return Err("Can only push to a List".to_string());
                    }
                } else {
                    return Err("Push collection must be an identifier".to_string());
                }
                Ok(ControlFlow::Continue)
            }

            Stmt::Pop { collection, into } => {
                if let Expr::Identifier(coll_sym) = collection {
                    let mut coll_val = self.lookup(*coll_sym)?.clone();
                    if let RuntimeValue::List(ref mut items) = coll_val {
                        let popped = items.pop().unwrap_or(RuntimeValue::Nothing);
                        self.assign(*coll_sym, coll_val)?;
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
                    let mut coll_val = self.lookup(*coll_sym)?.clone();
                    if let RuntimeValue::Set(ref mut items) = coll_val {
                        // Only add if not already present
                        if !items.iter().any(|x| self.values_equal(x, &val)) {
                            items.push(val);
                        }
                        self.assign(*coll_sym, coll_val)?;
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
                    let mut coll_val = self.lookup(*coll_sym)?.clone();
                    if let RuntimeValue::Set(ref mut items) = coll_val {
                        items.retain(|x| !self.values_equal(x, &val));
                        self.assign(*coll_sym, coll_val)?;
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
                    let mut coll_val = self.lookup(*coll_sym)?.clone();
                    match (&mut coll_val, &idx_val) {
                        (RuntimeValue::List(ref mut items), RuntimeValue::Int(n)) => {
                            let idx = *n as usize;
                            if idx == 0 || idx > items.len() {
                                return Err(format!("Index {} out of bounds for list of length {}", idx, items.len()));
                            }
                            items[idx - 1] = new_val;
                        }
                        (RuntimeValue::Map(ref mut map), RuntimeValue::Text(key)) => {
                            map.insert(key.clone(), new_val);
                        }
                        (RuntimeValue::List(_), _) => {
                            return Err("List index must be an integer".to_string());
                        }
                        (RuntimeValue::Map(_), _) => {
                            return Err("Map key must be a string".to_string());
                        }
                        _ => {
                            return Err(format!("Cannot index into {}", coll_val.type_name()));
                        }
                    }
                    self.assign(*coll_sym, coll_val)?;
                } else {
                    return Err("SetIndex collection must be an identifier".to_string());
                }
                Ok(ControlFlow::Continue)
            }

            Stmt::Inspect { target, arms, .. } => {
                let target_val = self.evaluate_expr(target).await?;
                self.execute_inspect(&target_val, arms).await?;
                Ok(ControlFlow::Continue)
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
                        self.output.push(obj_val.to_display_string());
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
                self.define(*var, RuntimeValue::Text(content));
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
                // Evaluate source (the struct to merge from)
                let source_val = self.evaluate_expr(source).await?;
                let source_fields = match &source_val {
                    RuntimeValue::Struct { fields, .. } => fields.clone(),
                    _ => return Err("Merge source must be a struct".to_string()),
                };

                // Target must be an identifier so we can mutate it
                if let Expr::Identifier(target_sym) = target {
                    let mut target_val = self.lookup(*target_sym)?.clone();

                    if let RuntimeValue::Struct { ref mut fields, .. } = target_val {
                        // For each field in source, merge into target
                        for (field_name, source_field_val) in source_fields {
                            let current = fields.get(&field_name)
                                .cloned()
                                .unwrap_or(RuntimeValue::Int(0));

                            // Merge counters by adding values
                            let merged = match (&current, &source_field_val) {
                                (RuntimeValue::Int(a), RuntimeValue::Int(b)) => {
                                    RuntimeValue::Int(a + b)
                                }
                                _ => source_field_val, // Non-counter fields: just take source value
                            };
                            fields.insert(field_name, merged);
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
                // Evaluate the amount expression
                let amount_val = self.evaluate_expr(amount).await?;
                let amount_int = match amount_val {
                    RuntimeValue::Int(n) => n,
                    _ => return Err("CRDT increment amount must be an integer".to_string()),
                };

                // Get the object (must be an identifier for mutation)
                if let Expr::Identifier(obj_sym) = object {
                    let mut obj_val = self.lookup(*obj_sym)?.clone();

                    // Mutate the field
                    if let RuntimeValue::Struct { ref mut fields, .. } = obj_val {
                        let field_name = self.interner.resolve(*field).to_string();
                        let current = fields.get(&field_name)
                            .cloned()
                            .unwrap_or(RuntimeValue::Int(0));

                        let new_val = match current {
                            RuntimeValue::Int(n) => RuntimeValue::Int(n + amount_int),
                            _ => return Err(format!("Field '{}' is not a counter", field_name)),
                        };
                        fields.insert(field_name, new_val);
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
                // Evaluate the amount expression
                let amount_val = self.evaluate_expr(amount).await?;
                let amount_int = match amount_val {
                    RuntimeValue::Int(n) => n,
                    _ => return Err("CRDT decrement amount must be an integer".to_string()),
                };

                // Get the object (must be an identifier for mutation)
                if let Expr::Identifier(obj_sym) = object {
                    let mut obj_val = self.lookup(*obj_sym)?.clone();

                    // Mutate the field
                    if let RuntimeValue::Struct { ref mut fields, .. } = obj_val {
                        let field_name = self.interner.resolve(*field).to_string();
                        let current = fields.get(&field_name)
                            .cloned()
                            .unwrap_or(RuntimeValue::Int(0));

                        let new_val = match current {
                            RuntimeValue::Int(n) => RuntimeValue::Int(n - amount_int),
                            _ => return Err(format!("Field '{}' is not a counter", field_name)),
                        };
                        fields.insert(field_name, new_val);
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

                // Get the subject value
                let subj_val = self.lookup(*subject)?.clone();
                let subj_type_name = match &subj_val {
                    RuntimeValue::Struct { type_name, .. } => type_name.clone(),
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
            Stmt::Sleep { .. } => {
                // Phase 55: Sleep could be implemented with gloo-timers on WASM
                Err("Sleep is not yet supported in the interpreter.".to_string())
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
                        // Store as a simple value for now (full Persistent<T> requires more work)
                        self.define(*var, RuntimeValue::Text(content));
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
    async fn execute_inspect(&mut self, target: &RuntimeValue, arms: &[MatchArm<'a>]) -> Result<(), String> {
        for arm in arms {
            // Handle Otherwise (wildcard) case
            if arm.variant.is_none() {
                self.execute_block(arm.body).await?;
                return Ok(());
            }

            match target {
                // Original Struct handling (for backward compatibility during transition)
                RuntimeValue::Struct { type_name, fields } => {
                    if let Some(variant) = arm.variant {
                        let variant_name = self.interner.resolve(variant);
                        if type_name == variant_name {
                            self.push_scope();
                            for (field_name, binding_name) in &arm.bindings {
                                let field_str = self.interner.resolve(*field_name);
                                if let Some(val) = fields.get(field_str) {
                                    self.define(*binding_name, val.clone());
                                }
                            }
                            let result = self.execute_block(arm.body).await;
                            self.pop_scope();
                            result?;
                            return Ok(());
                        }
                    }
                }

                // Phase 102: Kernel inductive handling
                RuntimeValue::Inductive { constructor, args, .. } => {
                    if let Some(variant) = arm.variant {
                        let variant_name = self.interner.resolve(variant);
                        if constructor == variant_name {
                            self.push_scope();
                            // Bind args positionally to binding names
                            // arm.bindings is Vec<(field_name, binding_name)>
                            // For inductives, we use bindings positionally
                            for (i, (_, binding_name)) in arm.bindings.iter().enumerate() {
                                if i < args.len() {
                                    self.define(*binding_name, args[i].clone());
                                }
                            }
                            let result = self.execute_block(arm.body).await;
                            self.pop_scope();
                            result?;
                            return Ok(());
                        }
                    }
                }

                _ => {}
            }
        }
        Ok(())
    }

    /// Evaluate an expression to a runtime value.
    /// Phase 55: Now async.
    #[async_recursion(?Send)]
    async fn evaluate_expr(&mut self, expr: &Expr<'a>) -> Result<RuntimeValue, String> {
        match expr {
            Expr::Literal(lit) => self.evaluate_literal(lit),

            Expr::Identifier(sym) => {
                self.lookup(*sym).cloned()
            }

            Expr::BinaryOp { op, left, right } => {
                let left_val = self.evaluate_expr(left).await?;
                let right_val = self.evaluate_expr(right).await?;
                self.apply_binary_op(*op, left_val, right_val)
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
                        if idx == 0 || idx > s.len() {
                            return Err(format!("Index {} out of bounds", idx));
                        }
                        Ok(RuntimeValue::Text(s.chars().nth(idx - 1).unwrap().to_string()))
                    }
                    (RuntimeValue::Map(map), RuntimeValue::Text(key)) => {
                        match map.get(key) {
                            Some(val) => Ok(val.clone()),
                            None => Err(format!("Key '{}' not found in map", key)),
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
                        let start = (*s as usize).saturating_sub(1);
                        let end = *e as usize;
                        let slice: Vec<RuntimeValue> = items.get(start..end).unwrap_or(&[]).to_vec();
                        Ok(RuntimeValue::List(slice))
                    }
                    _ => Err("Slice requires List and Int indices".to_string()),
                }
            }

            Expr::Copy { expr: inner } => {
                self.evaluate_expr(inner).await
            }

            Expr::Length { collection } => {
                let coll_val = self.evaluate_expr(collection).await?;
                match &coll_val {
                    RuntimeValue::List(items) => Ok(RuntimeValue::Int(items.len() as i64)),
                    RuntimeValue::Tuple(items) => Ok(RuntimeValue::Int(items.len() as i64)),
                    RuntimeValue::Set(items) => Ok(RuntimeValue::Int(items.len() as i64)),
                    RuntimeValue::Text(s) => Ok(RuntimeValue::Int(s.len() as i64)),
                    _ => Err(format!("Cannot get length of {}", coll_val.type_name())),
                }
            }

            Expr::Contains { collection, value } => {
                let coll_val = self.evaluate_expr(collection).await?;
                let val = self.evaluate_expr(value).await?;
                match &coll_val {
                    RuntimeValue::Set(items) => {
                        let found = items.iter().any(|item| self.values_equal(item, &val));
                        Ok(RuntimeValue::Bool(found))
                    }
                    RuntimeValue::List(items) => {
                        let found = items.iter().any(|item| self.values_equal(item, &val));
                        Ok(RuntimeValue::Bool(found))
                    }
                    RuntimeValue::Map(entries) => {
                        // For maps, check if key exists (keys are Strings)
                        if let RuntimeValue::Text(key) = &val {
                            Ok(RuntimeValue::Bool(entries.contains_key(key)))
                        } else {
                            Err(format!("Map key must be Text, got {}", val.type_name()))
                        }
                    }
                    RuntimeValue::Text(s) => {
                        // For text, check if substring exists
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
                        let mut result = a.clone();
                        for item in b.iter() {
                            if !result.iter().any(|x| self.values_equal(x, item)) {
                                result.push(item.clone());
                            }
                        }
                        Ok(RuntimeValue::Set(result))
                    }
                    _ => Err(format!("Cannot union {} and {}", left_val.type_name(), right_val.type_name())),
                }
            }

            Expr::Intersection { left, right } => {
                let left_val = self.evaluate_expr(left).await?;
                let right_val = self.evaluate_expr(right).await?;
                match (&left_val, &right_val) {
                    (RuntimeValue::Set(a), RuntimeValue::Set(b)) => {
                        let result: Vec<RuntimeValue> = a.iter()
                            .filter(|item| b.iter().any(|x| self.values_equal(x, item)))
                            .cloned()
                            .collect();
                        Ok(RuntimeValue::Set(result))
                    }
                    _ => Err(format!("Cannot intersect {} and {}", left_val.type_name(), right_val.type_name())),
                }
            }

            Expr::List(items) => {
                // Can't use .map() with async, so manual loop
                let mut values = Vec::with_capacity(items.len());
                for e in items.iter() {
                    values.push(self.evaluate_expr(e).await?);
                }
                Ok(RuntimeValue::List(values))
            }

            Expr::Tuple(items) => {
                let mut values = Vec::with_capacity(items.len());
                for e in items.iter() {
                    values.push(self.evaluate_expr(e).await?);
                }
                Ok(RuntimeValue::Tuple(values))
            }

            Expr::Range { start, end } => {
                let start_val = self.evaluate_expr(start).await?;
                let end_val = self.evaluate_expr(end).await?;
                match (&start_val, &end_val) {
                    (RuntimeValue::Int(s), RuntimeValue::Int(e)) => {
                        let range: Vec<RuntimeValue> = (*s..=*e)
                            .map(RuntimeValue::Int)
                            .collect();
                        Ok(RuntimeValue::List(range))
                    }
                    _ => Err("Range requires Int bounds".to_string()),
                }
            }

            Expr::FieldAccess { object, field } => {
                let obj_val = self.evaluate_expr(object).await?;
                match &obj_val {
                    RuntimeValue::Struct { fields, .. } => {
                        let field_name = self.interner.resolve(*field);
                        fields.get(field_name).cloned()
                            .ok_or_else(|| format!("Field '{}' not found", field_name))
                    }
                    _ => Err(format!("Cannot access field on {}", obj_val.type_name())),
                }
            }

            Expr::New { type_name, init_fields, .. } => {
                let name = self.interner.resolve(*type_name).to_string();

                if name == "Seq" || name == "List" {
                    return Ok(RuntimeValue::List(vec![]));
                }

                if name == "Set" || name == "HashSet" {
                    return Ok(RuntimeValue::Set(vec![]));
                }

                if name == "Map" || name == "HashMap" {
                    return Ok(RuntimeValue::Map(HashMap::new()));
                }

                let mut fields = HashMap::new();
                for (field_sym, field_expr) in init_fields {
                    let field_name = self.interner.resolve(*field_sym).to_string();
                    let field_val = self.evaluate_expr(field_expr).await?;
                    fields.insert(field_name, field_val);
                }
                Ok(RuntimeValue::Struct { type_name: name, fields })
            }

            // Phase 102: Enum variant constructor
            // Now creates RuntimeValue::Inductive for unified kernel types
            Expr::NewVariant { enum_name, variant, fields } => {
                let inductive_type = self.interner.resolve(*enum_name).to_string();
                let constructor = self.interner.resolve(*variant).to_string();

                // Evaluate field values in order (positional for inductives)
                let mut args = Vec::new();
                for (_, field_expr) in fields {
                    let field_val = self.evaluate_expr(field_expr).await?;
                    args.push(field_val);
                }

                // Create unified inductive value
                Ok(RuntimeValue::Inductive {
                    inductive_type,
                    constructor,
                    args,
                })
            }

            Expr::ManifestOf { .. } => {
                Ok(RuntimeValue::List(vec![]))
            }

            Expr::ChunkAt { .. } => {
                Ok(RuntimeValue::Nothing)
            }
        }
    }

    /// Evaluate a literal to a runtime value.
    fn evaluate_literal(&self, lit: &Literal) -> Result<RuntimeValue, String> {
        match lit {
            Literal::Number(n) => Ok(RuntimeValue::Int(*n)),
            Literal::Float(f) => Ok(RuntimeValue::Float(*f)),
            Literal::Text(sym) => Ok(RuntimeValue::Text(self.interner.resolve(*sym).to_string())),
            Literal::Boolean(b) => Ok(RuntimeValue::Bool(*b)),
            Literal::Nothing => Ok(RuntimeValue::Nothing),
            Literal::Char(c) => Ok(RuntimeValue::Char(*c)),
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
            (RuntimeValue::Text(a), RuntimeValue::Text(b)) => Ok(RuntimeValue::Text(format!("{}{}", a, b))),
            (RuntimeValue::Text(a), other) => Ok(RuntimeValue::Text(format!("{}{}", a, other.to_display_string()))),
            (other, RuntimeValue::Text(b)) => Ok(RuntimeValue::Text(format!("{}{}", other.to_display_string(), b))),
            _ => Err(format!("Cannot add {} and {}", left.type_name(), right.type_name())),
        }
    }

    /// Phase 53: String concatenation ("combined with")
    fn apply_concat(&self, left: RuntimeValue, right: RuntimeValue) -> Result<RuntimeValue, String> {
        Ok(RuntimeValue::Text(format!("{}{}", left.to_display_string(), right.to_display_string())))
    }

    fn apply_subtract(&self, left: RuntimeValue, right: RuntimeValue) -> Result<RuntimeValue, String> {
        match (&left, &right) {
            (RuntimeValue::Int(a), RuntimeValue::Int(b)) => Ok(RuntimeValue::Int(a - b)),
            (RuntimeValue::Float(a), RuntimeValue::Float(b)) => Ok(RuntimeValue::Float(a - b)),
            (RuntimeValue::Int(a), RuntimeValue::Float(b)) => Ok(RuntimeValue::Float(*a as f64 - b)),
            (RuntimeValue::Float(a), RuntimeValue::Int(b)) => Ok(RuntimeValue::Float(a - *b as f64)),
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

    fn apply_comparison<F>(&self, left: RuntimeValue, right: RuntimeValue, cmp: F) -> Result<RuntimeValue, String>
    where
        F: Fn(i64, i64) -> bool,
    {
        match (&left, &right) {
            (RuntimeValue::Int(a), RuntimeValue::Int(b)) => Ok(RuntimeValue::Bool(cmp(*a, *b))),
            _ => Err(format!("Cannot compare {} and {}", left.type_name(), right.type_name())),
        }
    }

    fn values_equal(&self, left: &RuntimeValue, right: &RuntimeValue) -> bool {
        match (left, right) {
            (RuntimeValue::Int(a), RuntimeValue::Int(b)) => a == b,
            (RuntimeValue::Float(a), RuntimeValue::Float(b)) => (a - b).abs() < f64::EPSILON,
            (RuntimeValue::Bool(a), RuntimeValue::Bool(b)) => a == b,
            (RuntimeValue::Text(a), RuntimeValue::Text(b)) => a == b,
            (RuntimeValue::Char(a), RuntimeValue::Char(b)) => a == b,
            (RuntimeValue::Nothing, RuntimeValue::Nothing) => true,
            // Phase 102: Inductive equality - same type, same constructor, equal args
            (RuntimeValue::Inductive { inductive_type: t1, constructor: c1, args: a1 },
             RuntimeValue::Inductive { inductive_type: t2, constructor: c2, args: a2 }) => {
                t1 == t2 && c1 == c2 && a1.len() == a2.len() &&
                a1.iter().zip(a2.iter()).all(|(x, y)| self.values_equal(x, y))
            }
            _ => false,
        }
    }

    /// Call a function (built-in or user-defined).
    #[async_recursion(?Send)]
    async fn call_function(&mut self, function: Symbol, args: &[&'async_recursion Expr<'a>]) -> Result<RuntimeValue, String> {
        let func_name = self.interner.resolve(function);

        // Built-in functions
        match func_name {
            "show" => {
                for arg in args {
                    let val = self.evaluate_expr(arg).await?;
                    self.output.push(val.to_display_string());
                }
                return Ok(RuntimeValue::Nothing);
            }
            "length" => {
                if args.len() != 1 {
                    return Err("length() takes exactly 1 argument".to_string());
                }
                let val = self.evaluate_expr(args[0]).await?;
                return match &val {
                    RuntimeValue::List(items) => Ok(RuntimeValue::Int(items.len() as i64)),
                    RuntimeValue::Text(s) => Ok(RuntimeValue::Int(s.len() as i64)),
                    _ => Err(format!("Cannot get length of {}", val.type_name())),
                };
            }
            "format" => {
                if args.is_empty() {
                    return Ok(RuntimeValue::Text(String::new()));
                }
                let val = self.evaluate_expr(args[0]).await?;
                return Ok(RuntimeValue::Text(val.to_display_string()));
            }
            "abs" => {
                if args.len() != 1 {
                    return Err("abs() takes exactly 1 argument".to_string());
                }
                let val = self.evaluate_expr(args[0]).await?;
                return match val {
                    RuntimeValue::Int(n) => Ok(RuntimeValue::Int(n.abs())),
                    RuntimeValue::Float(f) => Ok(RuntimeValue::Float(f.abs())),
                    _ => Err(format!("abs() requires a number, got {}", val.type_name())),
                };
            }
            "min" => {
                if args.len() != 2 {
                    return Err("min() takes exactly 2 arguments".to_string());
                }
                let a = self.evaluate_expr(args[0]).await?;
                let b = self.evaluate_expr(args[1]).await?;
                return match (&a, &b) {
                    (RuntimeValue::Int(x), RuntimeValue::Int(y)) => Ok(RuntimeValue::Int(*x.min(y))),
                    _ => Err("min() requires integers".to_string()),
                };
            }
            "max" => {
                if args.len() != 2 {
                    return Err("max() takes exactly 2 arguments".to_string());
                }
                let a = self.evaluate_expr(args[0]).await?;
                let b = self.evaluate_expr(args[1]).await?;
                return match (&a, &b) {
                    (RuntimeValue::Int(x), RuntimeValue::Int(y)) => Ok(RuntimeValue::Int(*x.max(y))),
                    _ => Err("max() requires integers".to_string()),
                };
            }
            "copy" => {
                if args.len() != 1 {
                    return Err("copy() takes exactly 1 argument".to_string());
                }
                let val = self.evaluate_expr(args[0]).await?;
                return Ok(val.clone());
            }
            _ => {}
        }

        // User-defined function lookup
        // Need to get the function separately to avoid borrow conflicts
        let func_data = self.functions.get(&function)
            .map(|f| (f.params.clone(), f.body))
            .ok_or_else(|| format!("Unknown function: {}", func_name))?;

        let (params, body) = func_data;

        if args.len() != params.len() {
            return Err(format!(
                "Function {} expects {} arguments, got {}",
                func_name,
                params.len(),
                args.len()
            ));
        }

        // Evaluate arguments before pushing scope
        let mut arg_values = Vec::new();
        for arg in args {
            arg_values.push(self.evaluate_expr(arg).await?);
        }

        // Push new scope and bind parameters
        self.push_scope();
        for ((param_name, _), arg_val) in params.iter().zip(arg_values) {
            self.define(*param_name, arg_val);
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
    }

    /// Call a function with pre-evaluated RuntimeValue arguments.
    /// Used by Give and Show statements where the object is already evaluated.
    #[async_recursion(?Send)]
    async fn call_function_with_values(&mut self, function: Symbol, args: Vec<RuntimeValue>) -> Result<RuntimeValue, String> {
        let func_name = self.interner.resolve(function);

        // Handle built-in "show"
        if func_name == "show" {
            for val in args {
                self.output.push(val.to_display_string());
            }
            return Ok(RuntimeValue::Nothing);
        }

        // User-defined function lookup
        let func_data = self.functions.get(&function)
            .map(|f| (f.params.clone(), f.body))
            .ok_or_else(|| format!("Unknown function: {}", func_name))?;

        let (params, body) = func_data;

        if args.len() != params.len() {
            return Err(format!(
                "Function {} expects {} arguments, got {}",
                func_name, params.len(), args.len()
            ));
        }

        // Push new scope and bind parameters
        self.push_scope();
        for ((param_name, _), arg_val) in params.iter().zip(args) {
            self.define(*param_name, arg_val);
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
    }

    // Scope management

    fn push_scope(&mut self) {
        self.env.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        if self.env.len() > 1 {
            self.env.pop();
        }
    }

    fn define(&mut self, name: Symbol, value: RuntimeValue) {
        if let Some(scope) = self.env.last_mut() {
            scope.insert(name, value);
        }
    }

    fn assign(&mut self, name: Symbol, value: RuntimeValue) -> Result<(), String> {
        // Search from innermost to outermost scope
        for scope in self.env.iter_mut().rev() {
            if scope.contains_key(&name) {
                scope.insert(name, value);
                return Ok(());
            }
        }
        Err(format!("Undefined variable: {}", self.interner.resolve(name)))
    }

    fn lookup(&self, name: Symbol) -> Result<&RuntimeValue, String> {
        // Search from innermost to outermost scope
        for scope in self.env.iter().rev() {
            if let Some(value) = scope.get(&name) {
                return Ok(value);
            }
        }
        Err(format!("Undefined variable: {}", self.interner.resolve(name)))
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
                // subject's field equals value
                if let RuntimeValue::Struct { fields, .. } = subject {
                    let field_name = self.interner.resolve(*field);
                    if let Some(field_val) = fields.get(field_name) {
                        let expected = self.interner.resolve(*value);
                        // Compare based on type
                        match field_val {
                            RuntimeValue::Text(s) => s == expected,
                            RuntimeValue::Int(n) => {
                                if *is_string_literal {
                                    false // Can't compare int to string
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
                if let RuntimeValue::Struct { fields, .. } = subject {
                    let field_name = self.interner.resolve(*field);
                    if let Some(RuntimeValue::Bool(b)) = fields.get(field_name) {
                        *b == *value
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            PolicyCondition::Predicate { predicate, .. } => {
                // Recursively evaluate another predicate
                if let Some(registry) = &self.policy_registry {
                    if let RuntimeValue::Struct { type_name, .. } = subject {
                        if let Some(subj_type_sym) = self.interner.lookup(type_name) {
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
                // subject's subj_field equals object's field
                let obj = match object {
                    Some(o) => o,
                    None => return false,
                };
                if let (RuntimeValue::Struct { fields: subj_fields, .. },
                        RuntimeValue::Struct { fields: obj_fields, .. }) = (subject, obj) {
                    let subj_field_name = self.interner.resolve(*subj_field);
                    let obj_field_name = self.interner.resolve(*field);
                    if let (Some(subj_val), Some(obj_val)) = (subj_fields.get(subj_field_name), obj_fields.get(obj_field_name)) {
                        self.values_equal(subj_val, obj_val)
                    } else {
                        // Check if comparing the whole subject/object
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
