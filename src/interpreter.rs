//! Tree-walking interpreter for LOGOS imperative code.
//!
//! This module provides runtime execution of parsed LOGOS programs,
//! walking the AST and executing statements/expressions directly.
//!
//! Phase 55: Made async for VFS operations (OPFS on WASM, tokio::fs on native).

use std::collections::HashMap;
use std::sync::Arc;

use async_recursion::async_recursion;

use crate::ast::stmt::{BinaryOpKind, Block, Expr, Literal, MatchArm, ReadSource, Stmt, TypeExpr};
use crate::intern::{Interner, Symbol};

// Phase 55: VFS imports
use logos_core::fs::Vfs;

/// Runtime values during interpretation.
#[derive(Debug, Clone)]
pub enum RuntimeValue {
    Int(i64),
    Float(f64),
    Bool(bool),
    Text(String),
    List(Vec<RuntimeValue>),
    Struct {
        type_name: String,
        fields: HashMap<String, RuntimeValue>,
    },
    Nothing,
}

impl RuntimeValue {
    pub fn type_name(&self) -> &'static str {
        match self {
            RuntimeValue::Int(_) => "Int",
            RuntimeValue::Float(_) => "Float",
            RuntimeValue::Bool(_) => "Bool",
            RuntimeValue::Text(_) => "Text",
            RuntimeValue::List(_) => "List",
            RuntimeValue::Struct { .. } => "Struct",
            RuntimeValue::Nothing => "Nothing",
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

    pub fn to_display_string(&self) -> String {
        match self {
            RuntimeValue::Int(n) => n.to_string(),
            RuntimeValue::Float(f) => format!("{:.6}", f).trim_end_matches('0').trim_end_matches('.').to_string(),
            RuntimeValue::Bool(b) => if *b { "true" } else { "false" }.to_string(),
            RuntimeValue::Text(s) => s.clone(),
            RuntimeValue::List(items) => {
                let parts: Vec<String> = items.iter().map(|v| v.to_display_string()).collect();
                format!("[{}]", parts.join(", "))
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
            RuntimeValue::Nothing => "nothing".to_string(),
        }
    }
}

/// Control flow signals for statement execution.
pub enum ControlFlow {
    Continue,
    Return(RuntimeValue),
    Break,
}

/// Stored function definition for user-defined functions.
pub struct FunctionDef<'a> {
    pub params: Vec<(Symbol, &'a TypeExpr<'a>)>,
    pub body: Block<'a>,
    pub return_type: Option<&'a TypeExpr<'a>>,
}

/// Tree-walking interpreter for LOGOS programs.
///
/// Phase 55: Now async with optional VFS for file operations.
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
        }
    }

    /// Phase 55: Set the VFS for file operations.
    pub fn with_vfs(mut self, vfs: Arc<dyn Vfs>) -> Self {
        self.vfs = Some(vfs);
        self
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

            Stmt::SetIndex { collection, index, value } => {
                let idx_val = self.evaluate_expr(index).await?;
                let new_val = self.evaluate_expr(value).await?;
                let idx = match idx_val {
                    RuntimeValue::Int(n) => n as usize,
                    _ => return Err("Index must be an integer".to_string()),
                };
                if let Expr::Identifier(coll_sym) = collection {
                    let mut coll_val = self.lookup(*coll_sym)?.clone();
                    if let RuntimeValue::List(ref mut items) = coll_val {
                        if idx == 0 || idx > items.len() {
                            return Err(format!("Index {} out of bounds for list of length {}", idx, items.len()));
                        }
                        items[idx - 1] = new_val;
                        self.assign(*coll_sym, coll_val)?;
                    } else {
                        return Err("Can only index into a List".to_string());
                    }
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

            Stmt::Give { .. } => {
                Ok(ControlFlow::Continue)
            }

            Stmt::Show { object, recipient } => {
                let obj_val = self.evaluate_expr(object).await?;
                if let Expr::Identifier(sym) = recipient {
                    let name = self.interner.resolve(*sym);
                    if name == "show" {
                        self.output.push(obj_val.to_display_string());
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

            Stmt::MergeCrdt { .. } => {
                Err("CRDT Merge is not supported in the interpreter. Use compiled Rust.".to_string())
            }

            Stmt::IncreaseCrdt { .. } => {
                Err("CRDT Increase is not supported in the interpreter. Use compiled Rust.".to_string())
            }

            Stmt::Check { .. } => {
                Err("Security Check is not supported in the interpreter. Use compiled Rust.".to_string())
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
    #[async_recursion(?Send)]
    async fn execute_inspect(&mut self, target: &RuntimeValue, arms: &[MatchArm<'a>]) -> Result<(), String> {
        for arm in arms {
            if arm.variant.is_none() {
                self.execute_block(arm.body).await?;
                return Ok(());
            }
            if let RuntimeValue::Struct { type_name, fields } = target {
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
                    (RuntimeValue::Text(s), RuntimeValue::Int(idx)) => {
                        let idx = *idx as usize;
                        if idx == 0 || idx > s.len() {
                            return Err(format!("Index {} out of bounds", idx));
                        }
                        Ok(RuntimeValue::Text(s.chars().nth(idx - 1).unwrap().to_string()))
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
                    RuntimeValue::Text(s) => Ok(RuntimeValue::Int(s.len() as i64)),
                    _ => Err(format!("Cannot get length of {}", coll_val.type_name())),
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

                let mut fields = HashMap::new();
                for (field_sym, field_expr) in init_fields {
                    let field_name = self.interner.resolve(*field_sym).to_string();
                    let field_val = self.evaluate_expr(field_expr).await?;
                    fields.insert(field_name, field_val);
                }
                Ok(RuntimeValue::Struct { type_name: name, fields })
            }

            Expr::NewVariant { variant, fields, .. } => {
                let name = self.interner.resolve(*variant).to_string();
                let mut field_map = HashMap::new();
                for (field_sym, field_expr) in fields {
                    let field_name = self.interner.resolve(*field_sym).to_string();
                    let field_val = self.evaluate_expr(field_expr).await?;
                    field_map.insert(field_name, field_val);
                }
                Ok(RuntimeValue::Struct { type_name: name, fields: field_map })
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
            Literal::Text(sym) => Ok(RuntimeValue::Text(self.interner.resolve(*sym).to_string())),
            Literal::Boolean(b) => Ok(RuntimeValue::Bool(*b)),
            Literal::Nothing => Ok(RuntimeValue::Nothing),
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
            (RuntimeValue::Nothing, RuntimeValue::Nothing) => true,
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
}

/// Result from interpretation.
#[derive(Debug, Clone)]
pub struct InterpreterResult {
    pub lines: Vec<String>,
    pub error: Option<String>,
}
