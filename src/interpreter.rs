//! Tree-walking interpreter for LOGOS imperative code.
//!
//! This module provides runtime execution of parsed LOGOS programs,
//! walking the AST and executing statements/expressions directly.

use std::collections::HashMap;

use crate::ast::stmt::{BinaryOpKind, Block, Expr, Literal, MatchArm, Stmt, TypeExpr};
use crate::intern::{Interner, Symbol};

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
}

impl<'a> Interpreter<'a> {
    pub fn new(interner: &'a Interner) -> Self {
        Interpreter {
            interner,
            env: vec![HashMap::new()], // Global scope
            functions: HashMap::new(),
            struct_defs: HashMap::new(),
            output: Vec::new(),
        }
    }

    /// Execute a program (list of statements).
    pub fn run(&mut self, stmts: &[Stmt<'a>]) -> Result<(), String> {
        for stmt in stmts {
            match self.execute_stmt(stmt)? {
                ControlFlow::Return(_) => break,
                ControlFlow::Break => break,
                ControlFlow::Continue => {}
            }
        }
        Ok(())
    }

    /// Execute a single statement.
    fn execute_stmt(&mut self, stmt: &Stmt<'a>) -> Result<ControlFlow, String> {
        match stmt {
            Stmt::Let { var, value, .. } => {
                let val = self.evaluate_expr(value)?;
                self.define(*var, val);
                Ok(ControlFlow::Continue)
            }

            Stmt::Set { target, value } => {
                let val = self.evaluate_expr(value)?;
                self.assign(*target, val)?;
                Ok(ControlFlow::Continue)
            }

            Stmt::Call { function, args } => {
                self.call_function(*function, args)?;
                Ok(ControlFlow::Continue)
            }

            Stmt::If { cond, then_block, else_block } => {
                let condition = self.evaluate_expr(cond)?;
                if condition.is_truthy() {
                    let flow = self.execute_block(then_block)?;
                    if !matches!(flow, ControlFlow::Continue) {
                        return Ok(flow); // Propagate Return/Break from If block
                    }
                } else if let Some(else_stmts) = else_block {
                    let flow = self.execute_block(else_stmts)?;
                    if !matches!(flow, ControlFlow::Continue) {
                        return Ok(flow); // Propagate Return/Break from Otherwise block
                    }
                }
                Ok(ControlFlow::Continue)
            }

            Stmt::While { cond, body, .. } => {
                loop {
                    let condition = self.evaluate_expr(cond)?;
                    if !condition.is_truthy() {
                        break;
                    }
                    match self.execute_block(body)? {
                        ControlFlow::Break => break,
                        ControlFlow::Return(v) => return Ok(ControlFlow::Return(v)),
                        ControlFlow::Continue => {}
                    }
                }
                Ok(ControlFlow::Continue)
            }

            Stmt::Repeat { var, iterable, body } => {
                let iter_val = self.evaluate_expr(iterable)?;
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
                    match self.execute_block(body)? {
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
                    Some(expr) => self.evaluate_expr(expr)?,
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
                let new_val = self.evaluate_expr(value)?;
                // Get the object identifier
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
                let val = self.evaluate_expr(value)?;
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
                let idx_val = self.evaluate_expr(index)?;
                let new_val = self.evaluate_expr(value)?;
                let idx = match idx_val {
                    RuntimeValue::Int(n) => n as usize,
                    _ => return Err("Index must be an integer".to_string()),
                };
                if let Expr::Identifier(coll_sym) = collection {
                    let mut coll_val = self.lookup(*coll_sym)?.clone();
                    if let RuntimeValue::List(ref mut items) = coll_val {
                        // 1-indexed
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
                let target_val = self.evaluate_expr(target)?;
                self.execute_inspect(&target_val, arms)?;
                Ok(ControlFlow::Continue)
            }

            Stmt::Zone { name, body, .. } => {
                // Zones create a new scope
                self.push_scope();
                // Define the zone handle (as Nothing for now)
                self.define(*name, RuntimeValue::Nothing);
                let result = self.execute_block(body);
                self.pop_scope();
                result?;
                Ok(ControlFlow::Continue)
            }

            Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => {
                // In WASM, execute sequentially
                for task in tasks.iter() {
                    self.execute_stmt(task)?;
                }
                Ok(ControlFlow::Continue)
            }

            Stmt::Assert { .. } | Stmt::Trust { .. } => {
                // Logic assertions - for now, just continue
                Ok(ControlFlow::Continue)
            }

            Stmt::RuntimeAssert { condition } => {
                let val = self.evaluate_expr(condition)?;
                if !val.is_truthy() {
                    return Err("Assertion failed".to_string());
                }
                Ok(ControlFlow::Continue)
            }

            Stmt::Give { .. } => {
                // Ownership semantics - in interpreter, just continue
                Ok(ControlFlow::Continue)
            }

            Stmt::Show { object, recipient } => {
                // Show statement: "Show x." or "Show x to console."
                // The recipient is the show function by default
                let obj_val = self.evaluate_expr(object)?;

                // Check if recipient is the "show" function (default)
                if let Expr::Identifier(sym) = recipient {
                    let name = self.interner.resolve(*sym);
                    if name == "show" {
                        // Output the value
                        self.output.push(obj_val.to_display_string());
                    }
                }
                Ok(ControlFlow::Continue)
            }

            Stmt::ReadFrom { var, .. } => {
                // No filesystem in WASM - return empty string
                self.define(*var, RuntimeValue::Text(String::new()));
                Ok(ControlFlow::Continue)
            }

            Stmt::WriteFile { .. } => {
                // No filesystem in WASM - just continue
                Ok(ControlFlow::Continue)
            }

            Stmt::Spawn { name, .. } => {
                // No agents in WASM - create a placeholder
                self.define(*name, RuntimeValue::Nothing);
                Ok(ControlFlow::Continue)
            }

            Stmt::SendMessage { .. } => {
                // No agents in WASM
                Ok(ControlFlow::Continue)
            }

            Stmt::AwaitMessage { into, .. } => {
                // No agents in WASM - define the into variable as Nothing
                self.define(*into, RuntimeValue::Nothing);
                Ok(ControlFlow::Continue)
            }

            // Phase 49: CRDT operations - not supported in interpreter (compile-only)
            Stmt::MergeCrdt { .. } => {
                Err("CRDT Merge is not supported in the interpreter. Use compiled Rust.".to_string())
            }

            Stmt::IncreaseCrdt { .. } => {
                Err("CRDT Increase is not supported in the interpreter. Use compiled Rust.".to_string())
            }

            // Phase 50: Security Check - not supported in interpreter (compile-only)
            Stmt::Check { .. } => {
                Err("Security Check is not supported in the interpreter. Use compiled Rust.".to_string())
            }
        }
    }

    /// Execute a block of statements, returning control flow.
    fn execute_block(&mut self, block: Block<'a>) -> Result<ControlFlow, String> {
        self.push_scope();
        for stmt in block.iter() {
            match self.execute_stmt(stmt)? {
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
    fn execute_inspect(&mut self, target: &RuntimeValue, arms: &[MatchArm<'a>]) -> Result<(), String> {
        for arm in arms {
            if arm.variant.is_none() {
                // Otherwise arm - always matches
                self.execute_block(arm.body)?;
                return Ok(());
            }
            // For now, simplified matching - just check type name
            if let RuntimeValue::Struct { type_name, fields } = target {
                if let Some(variant) = arm.variant {
                    let variant_name = self.interner.resolve(variant);
                    if type_name == variant_name {
                        // Bind fields
                        self.push_scope();
                        for (field_name, binding_name) in &arm.bindings {
                            let field_str = self.interner.resolve(*field_name);
                            if let Some(val) = fields.get(field_str) {
                                self.define(*binding_name, val.clone());
                            }
                        }
                        let result = self.execute_block(arm.body);
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
    fn evaluate_expr(&mut self, expr: &Expr<'a>) -> Result<RuntimeValue, String> {
        match expr {
            Expr::Literal(lit) => self.evaluate_literal(lit),

            Expr::Identifier(sym) => {
                self.lookup(*sym).cloned()
            }

            Expr::BinaryOp { op, left, right } => {
                let left_val = self.evaluate_expr(left)?;
                let right_val = self.evaluate_expr(right)?;
                self.apply_binary_op(*op, left_val, right_val)
            }

            Expr::Call { function, args } => {
                self.call_function(*function, args)
            }

            Expr::Index { collection, index } => {
                let coll_val = self.evaluate_expr(collection)?;
                let idx_val = self.evaluate_expr(index)?;
                match (&coll_val, &idx_val) {
                    (RuntimeValue::List(items), RuntimeValue::Int(idx)) => {
                        // 1-indexed
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
                let coll_val = self.evaluate_expr(collection)?;
                let start_val = self.evaluate_expr(start)?;
                let end_val = self.evaluate_expr(end)?;
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
                // Copy just evaluates and clones
                self.evaluate_expr(inner)
            }

            Expr::Length { collection } => {
                let coll_val = self.evaluate_expr(collection)?;
                match &coll_val {
                    RuntimeValue::List(items) => Ok(RuntimeValue::Int(items.len() as i64)),
                    RuntimeValue::Text(s) => Ok(RuntimeValue::Int(s.len() as i64)),
                    _ => Err(format!("Cannot get length of {}", coll_val.type_name())),
                }
            }

            Expr::List(items) => {
                let values: Result<Vec<RuntimeValue>, String> = items
                    .iter()
                    .map(|e| self.evaluate_expr(e))
                    .collect();
                Ok(RuntimeValue::List(values?))
            }

            Expr::Range { start, end } => {
                let start_val = self.evaluate_expr(start)?;
                let end_val = self.evaluate_expr(end)?;
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
                let obj_val = self.evaluate_expr(object)?;
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

                // Check if this is a collection type (Seq or List)
                if name == "Seq" || name == "List" {
                    return Ok(RuntimeValue::List(vec![]));
                }

                // Otherwise create a struct
                let mut fields = HashMap::new();
                for (field_sym, field_expr) in init_fields {
                    let field_name = self.interner.resolve(*field_sym).to_string();
                    let field_val = self.evaluate_expr(field_expr)?;
                    fields.insert(field_name, field_val);
                }
                Ok(RuntimeValue::Struct { type_name: name, fields })
            }

            Expr::NewVariant { variant, fields, .. } => {
                let name = self.interner.resolve(*variant).to_string();
                let mut field_map = HashMap::new();
                for (field_sym, field_expr) in fields {
                    let field_name = self.interner.resolve(*field_sym).to_string();
                    let field_val = self.evaluate_expr(field_expr)?;
                    field_map.insert(field_name, field_val);
                }
                Ok(RuntimeValue::Struct { type_name: name, fields: field_map })
            }

            Expr::ManifestOf { .. } => {
                // Phase 48: Zone manifests not available in WASM
                Ok(RuntimeValue::List(vec![]))
            }

            Expr::ChunkAt { .. } => {
                // Phase 48: Zone chunks not available in WASM
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
    fn call_function(&mut self, function: Symbol, args: &[&Expr<'a>]) -> Result<RuntimeValue, String> {
        let func_name = self.interner.resolve(function);

        // Built-in functions
        match func_name {
            "show" => {
                for arg in args {
                    let val = self.evaluate_expr(arg)?;
                    self.output.push(val.to_display_string());
                }
                return Ok(RuntimeValue::Nothing);
            }
            "length" => {
                if args.len() != 1 {
                    return Err("length() takes exactly 1 argument".to_string());
                }
                let val = self.evaluate_expr(args[0])?;
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
                let val = self.evaluate_expr(args[0])?;
                return Ok(RuntimeValue::Text(val.to_display_string()));
            }
            "abs" => {
                if args.len() != 1 {
                    return Err("abs() takes exactly 1 argument".to_string());
                }
                let val = self.evaluate_expr(args[0])?;
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
                let a = self.evaluate_expr(args[0])?;
                let b = self.evaluate_expr(args[1])?;
                return match (&a, &b) {
                    (RuntimeValue::Int(x), RuntimeValue::Int(y)) => Ok(RuntimeValue::Int(*x.min(y))),
                    _ => Err("min() requires integers".to_string()),
                };
            }
            "max" => {
                if args.len() != 2 {
                    return Err("max() takes exactly 2 arguments".to_string());
                }
                let a = self.evaluate_expr(args[0])?;
                let b = self.evaluate_expr(args[1])?;
                return match (&a, &b) {
                    (RuntimeValue::Int(x), RuntimeValue::Int(y)) => Ok(RuntimeValue::Int(*x.max(y))),
                    _ => Err("max() requires integers".to_string()),
                };
            }
            "copy" => {
                if args.len() != 1 {
                    return Err("copy() takes exactly 1 argument".to_string());
                }
                let val = self.evaluate_expr(args[0])?;
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
            arg_values.push(self.evaluate_expr(arg)?);
        }

        // Push new scope and bind parameters
        self.push_scope();
        for ((param_name, _), arg_val) in params.iter().zip(arg_values) {
            self.define(*param_name, arg_val);
        }

        // Execute function body
        let mut return_value = RuntimeValue::Nothing;
        for stmt in body.iter() {
            match self.execute_stmt(stmt)? {
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
