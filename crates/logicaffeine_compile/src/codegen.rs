//! Code generation from LOGOS AST to Rust source code.
//!
//! This module transforms the parsed and type-checked LOGOS program into
//! idiomatic Rust code. The generated code uses `logicaffeine_data` types
//! for runtime values and integrates with the kernel for proof verification.
//!
//! # Pipeline Position
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────┐
//! │  LOGOS Source → Lexer → Parser → AST → Analysis → HERE │
//! └─────────────────────────────────────────────────────────┘
//!                                                      ↓
//!                                               Rust Source
//! ```
//!
//! # Code Generation Rules
//!
//! | LOGOS Statement | Rust Output |
//! |-----------------|-------------|
//! | `Let x be 5.` | `let x = 5;` |
//! | `Set x to 10.` | `x = 10;` |
//! | `Give x to y.` | `let y = x;` (move) |
//! | `Show x to show.` | `println!("{}", x);` |
//! | `If x > 0 then...` | `if x > 0 { ... }` |
//! | `Repeat while x > 0...` | `while x > 0 { ... }` |
//! | `Zone "name"...` | `{ /* zone scope */ }` |
//! | `Mount x at "path".` | `x.mount(vfs, "path").await;` |
//! | `Sync x on "topic".` | `x.subscribe("topic").await;` |
//!
//! # Key Features
//!
//! - **Refinement Types**: Generates `debug_assert!` for type predicates
//! - **Policy Enforcement**: Emits capability checks for access control
//! - **Zone Safety**: Translates memory zones to Rust scopes
//! - **CRDT Mutability**: Uses `.set()` for LWWRegister/MVRegister fields
//! - **Async Detection**: Adds `#[tokio::main]` when async operations are present
//! - **VFS Detection**: Injects `NativeVfs::new()` for file operations
//! - **Mount+Sync Detection**: Uses `Distributed<T>` for combined persistence/sync
//!
//! # Refinement Context
//!
//! The [`RefinementContext`] tracks type predicates across scopes:
//!
//! ```text
//! Let x: { it: Int | it > 0 } be 5.   ←  Register constraint
//!      ↓
//! debug_assert!(5 > 0);               ←  Check at definition
//!      ↓
//! Set x to 10.                        ←  Re-check on mutation
//!      ↓
//! debug_assert!(10 > 0);              ←  Re-emit assertion
//! ```
//!
//! - When a variable with a refinement type is defined, its constraint is registered
//! - When that variable is mutated, the assertion is re-emitted
//! - Variable types are tracked for capability resolution
//!
//! # Entry Point
//!
//! The main entry point is [`codegen_program`], which generates a complete Rust
//! program from a list of statements.

use std::collections::{HashMap, HashSet};
use std::fmt::Write;

use crate::analysis::registry::{FieldDef, FieldType, TypeDef, TypeRegistry, VariantDef};
use crate::analysis::policy::{PolicyRegistry, PredicateDef, CapabilityDef, PolicyCondition};
use crate::ast::logic::{LogicExpr, NumberKind, Term};
use crate::ast::stmt::{BinaryOpKind, Expr, Literal, ReadSource, Stmt, TypeExpr};
use crate::formatter::RustFormatter;
use crate::intern::{Interner, Symbol};
use crate::registry::SymbolRegistry;

// =============================================================================
// Refinement Type Enforcement
// =============================================================================

/// Tracks refinement type constraints across scopes for mutation enforcement.
///
/// When a variable with a refinement type is defined, its constraint is registered
/// in the current scope. When that variable is mutated via `Set`, the assertion is
/// re-emitted to ensure the invariant is preserved.
///
/// # Scope Management
///
/// The context maintains a stack of scopes to handle nested blocks:
///
/// ```text
/// ┌─────────────────────────────┐
/// │ Global Scope               │ ← x: { it > 0 }
/// │  ┌──────────────────────┐  │
/// │  │ Zone Scope           │  │ ← y: { it < 100 }
/// │  │  ┌────────────────┐  │  │
/// │  │  │ If Block Scope │  │  │ ← z: { it != 0 }
/// │  │  └────────────────┘  │  │
/// │  └──────────────────────┘  │
/// └─────────────────────────────┘
/// ```
///
/// # Variable Type Tracking
///
/// The context also tracks variable types for capability resolution. This allows
/// statements like `Check that user can publish the document` to resolve "the document"
/// to a variable named `doc` of type `Document`.
pub struct RefinementContext<'a> {
    /// Stack of scopes. Each scope maps variable Symbol to (bound_var, predicate).
    scopes: Vec<HashMap<Symbol, (Symbol, &'a LogicExpr<'a>)>>,

    /// Maps variable name Symbol to type name (for capability resolution).
    ///
    /// Example: `doc` → `"Document"` allows resolving "the document" to `&doc`.
    variable_types: HashMap<Symbol, String>,

    /// Stack of scopes tracking which bindings came from boxed enum fields.
    /// When these are used in expressions, they need to be dereferenced with `*`.
    boxed_binding_scopes: Vec<HashSet<Symbol>>,

    /// Tracks variables that are known to be String type.
    /// Used for proper string concatenation codegen (format! vs +).
    string_vars: HashSet<Symbol>,
}

impl<'a> RefinementContext<'a> {
    pub fn new() -> Self {
        Self {
            scopes: vec![HashMap::new()],
            variable_types: HashMap::new(),
            boxed_binding_scopes: vec![HashSet::new()],
            string_vars: HashSet::new(),
        }
    }

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
        self.boxed_binding_scopes.push(HashSet::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
        self.boxed_binding_scopes.pop();
    }

    /// Register a binding that came from a boxed enum field.
    /// These need `*` dereferencing when used in expressions.
    fn register_boxed_binding(&mut self, var: Symbol) {
        if let Some(scope) = self.boxed_binding_scopes.last_mut() {
            scope.insert(var);
        }
    }

    /// Check if a variable is a boxed binding (needs dereferencing).
    fn is_boxed_binding(&self, var: Symbol) -> bool {
        for scope in self.boxed_binding_scopes.iter().rev() {
            if scope.contains(&var) {
                return true;
            }
        }
        false
    }

    /// Register a variable as having String type.
    /// Used for proper string concatenation codegen.
    fn register_string_var(&mut self, var: Symbol) {
        self.string_vars.insert(var);
    }

    /// Check if a variable is known to be a String.
    fn is_string_var(&self, var: Symbol) -> bool {
        self.string_vars.contains(&var)
    }

    /// Get a reference to the string_vars set for expression codegen.
    fn get_string_vars(&self) -> &HashSet<Symbol> {
        &self.string_vars
    }

    fn register(&mut self, var: Symbol, bound_var: Symbol, predicate: &'a LogicExpr<'a>) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(var, (bound_var, predicate));
        }
    }

    fn get_constraint(&self, var: Symbol) -> Option<(Symbol, &'a LogicExpr<'a>)> {
        for scope in self.scopes.iter().rev() {
            if let Some(entry) = scope.get(&var) {
                return Some(*entry);
            }
        }
        None
    }

    /// Phase 50: Register a variable with its type for capability resolution
    fn register_variable_type(&mut self, var: Symbol, type_name: String) {
        self.variable_types.insert(var, type_name);
    }

    /// Get variable type map for expression codegen optimization.
    fn get_variable_types(&self) -> &HashMap<Symbol, String> {
        &self.variable_types
    }

    /// Phase 50: Find a variable name by its type (for resolving "the document" to "doc")
    fn find_variable_by_type(&self, type_name: &str, interner: &Interner) -> Option<String> {
        let type_lower = type_name.to_lowercase();
        for (var_sym, var_type) in &self.variable_types {
            if var_type.to_lowercase() == type_lower {
                return Some(interner.resolve(*var_sym).to_string());
            }
        }
        None
    }
}

/// Emits a debug_assert for a refinement predicate, substituting the bound variable.
fn emit_refinement_check(
    var_name: &str,
    bound_var: Symbol,
    predicate: &LogicExpr,
    interner: &Interner,
    indent_str: &str,
    output: &mut String,
) {
    let assertion = codegen_assertion(predicate, interner);
    let bound = interner.resolve(bound_var);
    let check = if bound == var_name {
        assertion
    } else {
        replace_word(&assertion, bound, var_name)
    };
    writeln!(output, "{}debug_assert!({});", indent_str, check).unwrap();
}

/// Word-boundary replacement to substitute bound variable with actual variable.
fn replace_word(text: &str, from: &str, to: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut word = String::new();
    for c in text.chars() {
        if c.is_alphanumeric() || c == '_' {
            word.push(c);
        } else {
            if !word.is_empty() {
                result.push_str(if word == from { to } else { &word });
                word.clear();
            }
            result.push(c);
        }
    }
    if !word.is_empty() {
        result.push_str(if word == from { to } else { &word });
    }
    result
}

// =============================================================================
// Mount+Sync Detection for Distributed<T>
// =============================================================================

/// Tracks which variables have Mount and/or Sync statements.
///
/// This is used to detect when a variable needs `Distributed<T>` instead of
/// separate persistence and synchronization wrappers. A variable that is both
/// mounted and synced can use the unified `Distributed<T>` type.
///
/// # Detection Flow
///
/// ```text
/// Pre-scan all statements
///       ↓
/// Found "Mount x at path"  →  x.mounted = true, x.mount_path = Some(path)
/// Found "Sync x on topic"  →  x.synced = true, x.sync_topic = Some(topic)
///       ↓
/// If x.mounted && x.synced  →  Use Distributed<T> with both
/// ```
#[derive(Debug, Default)]
pub struct VariableCapabilities {
    /// Variable has a Mount statement (persistence).
    mounted: bool,
    /// Variable has a Sync statement (network synchronization).
    synced: bool,
    /// Path expression for Mount (as generated code string).
    mount_path: Option<String>,
    /// Topic expression for Sync (as generated code string).
    sync_topic: Option<String>,
}

/// Helper to create an empty VariableCapabilities map (for tests).
pub fn empty_var_caps() -> HashMap<Symbol, VariableCapabilities> {
    HashMap::new()
}

/// Pre-scan statements to detect variables that have both Mount and Sync.
/// Returns a map from variable Symbol to its capabilities.
fn analyze_variable_capabilities<'a>(
    stmts: &[Stmt<'a>],
    interner: &Interner,
) -> HashMap<Symbol, VariableCapabilities> {
    let mut caps: HashMap<Symbol, VariableCapabilities> = HashMap::new();
    let empty_synced = HashSet::new();

    for stmt in stmts {
        match stmt {
            Stmt::Mount { var, path } => {
                let entry = caps.entry(*var).or_default();
                entry.mounted = true;
                entry.mount_path = Some(codegen_expr(path, interner, &empty_synced));
            }
            Stmt::Sync { var, topic } => {
                let entry = caps.entry(*var).or_default();
                entry.synced = true;
                entry.sync_topic = Some(codegen_expr(topic, interner, &empty_synced));
            }
            // Recursively check nested blocks (Block<'a> is &[Stmt<'a>])
            Stmt::If { then_block, else_block, .. } => {
                let nested = analyze_variable_capabilities(then_block, interner);
                for (var, cap) in nested {
                    let entry = caps.entry(var).or_default();
                    if cap.mounted { entry.mounted = true; entry.mount_path = cap.mount_path; }
                    if cap.synced { entry.synced = true; entry.sync_topic = cap.sync_topic; }
                }
                if let Some(else_b) = else_block {
                    let nested = analyze_variable_capabilities(else_b, interner);
                    for (var, cap) in nested {
                        let entry = caps.entry(var).or_default();
                        if cap.mounted { entry.mounted = true; entry.mount_path = cap.mount_path; }
                        if cap.synced { entry.synced = true; entry.sync_topic = cap.sync_topic; }
                    }
                }
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } => {
                let nested = analyze_variable_capabilities(body, interner);
                for (var, cap) in nested {
                    let entry = caps.entry(var).or_default();
                    if cap.mounted { entry.mounted = true; entry.mount_path = cap.mount_path; }
                    if cap.synced { entry.synced = true; entry.sync_topic = cap.sync_topic; }
                }
            }
            _ => {}
        }
    }

    caps
}

/// FFI: Detect if any function is exported for WASM.
/// Used to emit `use wasm_bindgen::prelude::*;` preamble.
fn has_wasm_exports(stmts: &[Stmt], interner: &Interner) -> bool {
    stmts.iter().any(|stmt| {
        if let Stmt::FunctionDef { is_exported: true, export_target: Some(target), .. } = stmt {
            interner.resolve(*target).eq_ignore_ascii_case("wasm")
        } else {
            false
        }
    })
}

/// FFI: Detect if any function is exported for C ABI.
/// Used to emit the LogosStatus runtime preamble and CStr/CString imports.
fn has_c_exports(stmts: &[Stmt], interner: &Interner) -> bool {
    stmts.iter().any(|stmt| {
        if let Stmt::FunctionDef { is_exported: true, export_target, .. } = stmt {
            match export_target {
                None => true,
                Some(t) => interner.resolve(*t).eq_ignore_ascii_case("c"),
            }
        } else {
            false
        }
    })
}

/// FFI: Detect if any C-exported function uses Text (String) types.
/// Used to emit `use std::ffi::{CStr, CString};` preamble.
fn has_c_exports_with_text(stmts: &[Stmt], interner: &Interner) -> bool {
    stmts.iter().any(|stmt| {
        if let Stmt::FunctionDef { is_exported: true, export_target, params, return_type, .. } = stmt {
            let is_c = match export_target {
                None => true,
                Some(t) => interner.resolve(*t).eq_ignore_ascii_case("c"),
            };
            if !is_c { return false; }
            let has_text_param = params.iter().any(|(_, ty)| is_text_type(ty, interner));
            let has_text_return = return_type.as_ref().map_or(false, |ty| is_text_type(ty, interner));
            has_text_param || has_text_return
        } else {
            false
        }
    })
}

// =============================================================================
// Universal ABI: Status-Code Error Runtime
// =============================================================================

/// Classification of a LOGOS type for C ABI boundary crossing.
///
/// Value types are passed directly as `#[repr(C)]` values.
/// Reference types are passed as opaque handles with accessor functions.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CAbiClass {
    /// Passed directly by value (primitives, Text, small flat structs).
    ValueType,
    /// Passed as opaque `logos_handle_t` with generated accessors and free function.
    ReferenceType,
}

/// Classify a LOGOS TypeExpr for C ABI boundary crossing.
///
/// Value types: Int, Nat, Real, Bool, Byte, Char, Text, small user structs (all value-type fields).
/// Reference types: Seq, Map, Set, Option of reference, Result, large/recursive user types.
fn classify_type_for_c_abi(ty: &TypeExpr, interner: &Interner, registry: &TypeRegistry) -> CAbiClass {
    match ty {
        TypeExpr::Primitive(sym) | TypeExpr::Named(sym) => {
            let name = interner.resolve(*sym);
            match name {
                "Int" | "Nat" | "Real" | "Float" | "Bool" | "Boolean"
                | "Byte" | "Char" | "Unit" => CAbiClass::ValueType,
                "Text" | "String" => CAbiClass::ValueType,
                _ => {
                    // Check registry for user-defined types
                    if let Some(type_def) = registry.get(*sym) {
                        match type_def {
                            TypeDef::Struct { fields, .. } => {
                                // Small struct with all value-type fields → ValueType
                                let all_value = fields.iter().all(|f| {
                                    is_value_type_field(&f.ty, interner)
                                });
                                if all_value && fields.len() <= 4 {
                                    CAbiClass::ValueType
                                } else {
                                    CAbiClass::ReferenceType
                                }
                            }
                            TypeDef::Enum { .. } => CAbiClass::ReferenceType,
                            TypeDef::Primitive => CAbiClass::ValueType,
                            TypeDef::Generic { .. } => CAbiClass::ReferenceType,
                            TypeDef::Alias { .. } => CAbiClass::ValueType,
                        }
                    } else {
                        CAbiClass::ValueType // Unknown → pass through
                    }
                }
            }
        }
        TypeExpr::Refinement { base, .. } => classify_type_for_c_abi(base, interner, registry),
        TypeExpr::Generic { base, .. } => {
            let base_name = interner.resolve(*base);
            match base_name {
                "Option" | "Maybe" => {
                    // Option of value type → value type (struct { present, value })
                    // Option of reference type → reference type
                    // For simplicity, treat all Options as reference types for now
                    CAbiClass::ReferenceType
                }
                "Result" | "Seq" | "List" | "Vec" | "Map" | "HashMap"
                | "Set" | "HashSet" => CAbiClass::ReferenceType,
                _ => CAbiClass::ReferenceType,
            }
        }
        TypeExpr::Function { .. } => CAbiClass::ReferenceType,
        TypeExpr::Persistent { .. } => CAbiClass::ReferenceType,
    }
}

/// Check if a field type is a C ABI value type (for struct classification).
fn is_value_type_field(ft: &FieldType, interner: &Interner) -> bool {
    match ft {
        FieldType::Primitive(sym) | FieldType::Named(sym) => {
            let name = interner.resolve(*sym);
            matches!(name, "Int" | "Nat" | "Real" | "Float" | "Bool" | "Boolean"
                | "Byte" | "Char" | "Unit")
            // Text/String excluded — String cannot cross C ABI by value
        }
        FieldType::Generic { .. } => false, // Generic fields are reference types
        FieldType::TypeParam(_) => false,
    }
}

/// Check if a name is a Rust keyword (needs `r#` escaping in generated code).
fn is_rust_keyword(name: &str) -> bool {
    matches!(name,
        "as" | "async" | "await" | "break" | "const" | "continue" | "crate" |
        "dyn" | "else" | "enum" | "extern" | "false" | "fn" | "for" | "if" |
        "impl" | "in" | "let" | "loop" | "match" | "mod" | "move" | "mut" |
        "pub" | "ref" | "return" | "self" | "Self" | "static" | "struct" |
        "super" | "trait" | "true" | "type" | "unsafe" | "use" | "where" |
        "while" | "abstract" | "become" | "box" | "do" | "final" | "macro" |
        "override" | "priv" | "try" | "typeof" | "unsized" | "virtual" | "yield"
    )
}

/// Escape a name as a Rust raw identifier if it's a keyword.
/// e.g., "move" → "r#move", "add" → "add"
fn escape_rust_ident(name: &str) -> String {
    if is_rust_keyword(name) {
        format!("r#{}", name)
    } else {
        name.to_string()
    }
}

/// Mangle a Rust type string into a C-safe identifier component.
/// e.g., "i64" → "i64", "Vec<i64>" → "seq_i64", "HashMap<String, i64>" → "map_string_i64"
fn mangle_type_for_c(ty: &TypeExpr, interner: &Interner) -> String {
    match ty {
        TypeExpr::Primitive(sym) | TypeExpr::Named(sym) => {
            let name = interner.resolve(*sym);
            match name {
                "Int" => "i64".to_string(),
                "Nat" => "u64".to_string(),
                "Real" | "Float" => "f64".to_string(),
                "Bool" | "Boolean" => "bool".to_string(),
                "Byte" => "u8".to_string(),
                "Char" => "char".to_string(),
                "Text" | "String" => "string".to_string(),
                other => other.to_lowercase(),
            }
        }
        TypeExpr::Refinement { base, .. } => mangle_type_for_c(base, interner),
        TypeExpr::Generic { base, params } => {
            let base_name = interner.resolve(*base);
            let param_strs: Vec<String> = params.iter()
                .map(|p| mangle_type_for_c(p, interner))
                .collect();
            match base_name {
                "Seq" | "List" | "Vec" => format!("seq_{}", param_strs.join("_")),
                "Map" | "HashMap" => format!("map_{}", param_strs.join("_")),
                "Set" | "HashSet" => format!("set_{}", param_strs.join("_")),
                "Option" | "Maybe" => format!("option_{}", param_strs.join("_")),
                "Result" => format!("result_{}", param_strs.join("_")),
                other => format!("{}_{}", other.to_lowercase(), param_strs.join("_")),
            }
        }
        TypeExpr::Function { .. } => "fn".to_string(),
        TypeExpr::Persistent { inner } => mangle_type_for_c(inner, interner),
    }
}

/// Generate the LogosStatus runtime preamble for C ABI exports.
///
/// Emits:
/// - `LogosStatus` repr(C) enum
/// - Thread-local error storage
/// - `logos_get_last_error()` and `logos_clear_error()` extern C functions
/// - `logos_free_string()` for freeing allocated CStrings
fn codegen_logos_runtime_preamble() -> String {
    let mut out = String::new();

    writeln!(out, "// ═══ LogicAffeine Universal ABI Runtime ═══\n").unwrap();

    // LogosStatus enum
    writeln!(out, "#[repr(C)]").unwrap();
    writeln!(out, "#[derive(Debug, Clone, Copy, PartialEq)]").unwrap();
    writeln!(out, "pub enum LogosStatus {{").unwrap();
    writeln!(out, "    Ok = 0,").unwrap();
    writeln!(out, "    Error = 1,").unwrap();
    writeln!(out, "    RefinementViolation = 2,").unwrap();
    writeln!(out, "    NullPointer = 3,").unwrap();
    writeln!(out, "    OutOfBounds = 4,").unwrap();
    writeln!(out, "    DeserializationFailed = 5,").unwrap();
    writeln!(out, "    InvalidHandle = 6,").unwrap();
    writeln!(out, "    ContainsNullByte = 7,").unwrap();
    writeln!(out, "    ThreadPanic = 8,").unwrap();
    writeln!(out, "    MemoryExhausted = 9,").unwrap();
    writeln!(out, "}}\n").unwrap();

    // Opaque handle type alias
    writeln!(out, "pub type LogosHandle = *mut std::ffi::c_void;\n").unwrap();

    // Thread-safe error storage (keyed by ThreadId)
    writeln!(out, "fn logos_error_store() -> &'static std::sync::Mutex<std::collections::HashMap<std::thread::ThreadId, String>> {{").unwrap();
    writeln!(out, "    use std::sync::OnceLock;").unwrap();
    writeln!(out, "    static STORE: OnceLock<std::sync::Mutex<std::collections::HashMap<std::thread::ThreadId, String>>> = OnceLock::new();").unwrap();
    writeln!(out, "    STORE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))").unwrap();
    writeln!(out, "}}\n").unwrap();

    // set_last_error helper (not exported, internal only)
    writeln!(out, "fn logos_set_last_error(msg: String) {{").unwrap();
    writeln!(out, "    let mut store = logos_error_store().lock().unwrap_or_else(|e| e.into_inner());").unwrap();
    writeln!(out, "    store.insert(std::thread::current().id(), msg);").unwrap();
    writeln!(out, "}}\n").unwrap();

    // logos_last_error (exported) — canonical name
    // Uses a thread-local CString cache to avoid dangling pointers.
    // The returned pointer is valid until the next call to logos_last_error on the same thread.
    writeln!(out, "thread_local! {{").unwrap();
    writeln!(out, "    static LOGOS_ERROR_CACHE: std::cell::RefCell<Option<std::ffi::CString>> = std::cell::RefCell::new(None);").unwrap();
    writeln!(out, "}}\n").unwrap();

    writeln!(out, "#[no_mangle]").unwrap();
    writeln!(out, "pub extern \"C\" fn logos_last_error() -> *const std::os::raw::c_char {{").unwrap();
    writeln!(out, "    let msg = logos_error_store().lock().unwrap_or_else(|e| e.into_inner())").unwrap();
    writeln!(out, "        .get(&std::thread::current().id()).cloned();").unwrap();
    writeln!(out, "    match msg {{").unwrap();
    writeln!(out, "        Some(s) => match std::ffi::CString::new(s) {{").unwrap();
    writeln!(out, "            Ok(cstr) => {{").unwrap();
    writeln!(out, "                let ptr = cstr.as_ptr();").unwrap();
    writeln!(out, "                LOGOS_ERROR_CACHE.with(|cache| {{ cache.borrow_mut().replace(cstr); }});").unwrap();
    writeln!(out, "                LOGOS_ERROR_CACHE.with(|cache| {{").unwrap();
    writeln!(out, "                    cache.borrow().as_ref().map_or(std::ptr::null(), |c| c.as_ptr())").unwrap();
    writeln!(out, "                }})").unwrap();
    writeln!(out, "            }}").unwrap();
    writeln!(out, "            Err(_) => std::ptr::null(),").unwrap();
    writeln!(out, "        }}").unwrap();
    writeln!(out, "        None => std::ptr::null(),").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "}}\n").unwrap();

    // logos_get_last_error (exported) — backwards-compatible alias
    writeln!(out, "#[no_mangle]").unwrap();
    writeln!(out, "pub extern \"C\" fn logos_get_last_error() -> *const std::os::raw::c_char {{").unwrap();
    writeln!(out, "    logos_last_error()").unwrap();
    writeln!(out, "}}\n").unwrap();

    // logos_clear_error (exported)
    writeln!(out, "#[no_mangle]").unwrap();
    writeln!(out, "pub extern \"C\" fn logos_clear_error() {{").unwrap();
    writeln!(out, "    let mut store = logos_error_store().lock().unwrap_or_else(|e| e.into_inner());").unwrap();
    writeln!(out, "    store.remove(&std::thread::current().id());").unwrap();
    writeln!(out, "}}\n").unwrap();

    // logos_free_string (exported) — for freeing CStrings returned by accessors/JSON helpers
    writeln!(out, "#[no_mangle]").unwrap();
    writeln!(out, "pub extern \"C\" fn logos_free_string(ptr: *mut std::os::raw::c_char) {{").unwrap();
    writeln!(out, "    if !ptr.is_null() {{").unwrap();
    writeln!(out, "        unsafe {{ drop(std::ffi::CString::from_raw(ptr)); }}").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "}}\n").unwrap();

    // ABI version constant and introspection functions
    writeln!(out, "pub const LOGOS_ABI_VERSION: u32 = 1;\n").unwrap();

    writeln!(out, "#[no_mangle]").unwrap();
    writeln!(out, "pub extern \"C\" fn logos_version() -> *const std::os::raw::c_char {{").unwrap();
    writeln!(out, "    concat!(env!(\"CARGO_PKG_VERSION\"), \"\\0\").as_ptr() as *const std::os::raw::c_char").unwrap();
    writeln!(out, "}}\n").unwrap();

    writeln!(out, "#[no_mangle]").unwrap();
    writeln!(out, "pub extern \"C\" fn logos_abi_version() -> u32 {{").unwrap();
    writeln!(out, "    LOGOS_ABI_VERSION").unwrap();
    writeln!(out, "}}\n").unwrap();

    // Handle registry with generation counters for use-after-free protection
    writeln!(out, "struct HandleEntry {{").unwrap();
    writeln!(out, "    data: usize,").unwrap();
    writeln!(out, "    generation: u64,").unwrap();
    writeln!(out, "}}\n").unwrap();

    writeln!(out, "struct HandleRegistry {{").unwrap();
    writeln!(out, "    entries: std::collections::HashMap<u64, HandleEntry>,").unwrap();
    writeln!(out, "    counter: u64,").unwrap();
    writeln!(out, "}}\n").unwrap();

    writeln!(out, "impl HandleRegistry {{").unwrap();
    writeln!(out, "    fn new() -> Self {{").unwrap();
    writeln!(out, "        HandleRegistry {{ entries: std::collections::HashMap::new(), counter: 0 }}").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "    fn register(&mut self, ptr: usize) -> (u64, u64) {{").unwrap();
    writeln!(out, "        self.counter += 1;").unwrap();
    writeln!(out, "        let id = self.counter;").unwrap();
    writeln!(out, "        let generation = id;").unwrap();
    writeln!(out, "        self.entries.insert(id, HandleEntry {{ data: ptr, generation }});").unwrap();
    writeln!(out, "        (id, generation)").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "    fn validate_handle(&self, id: u64, generation: u64) -> bool {{").unwrap();
    writeln!(out, "        self.entries.get(&id).map_or(false, |e| e.generation == generation)").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "    fn deref(&self, id: u64) -> Option<usize> {{").unwrap();
    writeln!(out, "        self.entries.get(&id).map(|e| e.data)").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "    fn free(&mut self, id: u64) -> Result<usize, ()> {{").unwrap();
    writeln!(out, "        if let Some(entry) = self.entries.remove(&id) {{ Ok(entry.data) }} else {{ Err(()) }}").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "}}\n").unwrap();

    writeln!(out, "fn logos_handle_registry() -> &'static std::sync::Mutex<HandleRegistry> {{").unwrap();
    writeln!(out, "    use std::sync::OnceLock;").unwrap();
    writeln!(out, "    static REGISTRY: OnceLock<std::sync::Mutex<HandleRegistry>> = OnceLock::new();").unwrap();
    writeln!(out, "    REGISTRY.get_or_init(|| std::sync::Mutex::new(HandleRegistry::new()))").unwrap();
    writeln!(out, "}}\n").unwrap();

    out
}

/// Emit the opening of a catch_unwind panic boundary for an accessor function body.
fn emit_catch_unwind_open(out: &mut String) {
    writeln!(out, "    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {{").unwrap();
}

/// Emit the closing of a catch_unwind panic boundary for an accessor.
/// `default_expr` is the fallback value on panic (e.g., "0", "std::ptr::null_mut()").
fn emit_catch_unwind_close(out: &mut String, default_expr: &str) {
    writeln!(out, "    }})) {{").unwrap();
    writeln!(out, "        Ok(__v) => __v,").unwrap();
    writeln!(out, "        Err(__panic) => {{").unwrap();
    writeln!(out, "            let __msg = if let Some(s) = __panic.downcast_ref::<String>() {{ s.clone() }} else if let Some(s) = __panic.downcast_ref::<&str>() {{ s.to_string() }} else {{ \"Unknown panic\".to_string() }};").unwrap();
    writeln!(out, "            logos_set_last_error(__msg);").unwrap();
    writeln!(out, "            {}", default_expr).unwrap();
    writeln!(out, "        }}").unwrap();
    writeln!(out, "    }}").unwrap();
}

/// Emit a null handle check with early return. Used at the start of every accessor/free body.
fn emit_null_handle_check(out: &mut String, default_expr: &str) {
    writeln!(out, "        if handle.is_null() {{ logos_set_last_error(\"NullPointer: handle is null\".to_string()); return {}; }}", default_expr).unwrap();
}

/// Emit a null out-parameter check with early return. Used before every `*out = ...` write.
fn emit_null_out_check(out: &mut String, default_expr: &str) {
    writeln!(out, "        if out.is_null() {{ logos_set_last_error(\"NullPointer: output parameter is null\".to_string()); return {}; }}", default_expr).unwrap();
}

/// Emit registry handle lookup for an accessor. Returns the pointer or early-returns with error.
fn emit_registry_deref(out: &mut String, default_expr: &str) {
    writeln!(out, "        let __id = handle as u64;").unwrap();
    writeln!(out, "        let __reg = logos_handle_registry().lock().unwrap_or_else(|e| e.into_inner());").unwrap();
    writeln!(out, "        let __ptr = match __reg.deref(__id) {{").unwrap();
    writeln!(out, "            Some(p) => p,").unwrap();
    writeln!(out, "            None => {{ logos_set_last_error(\"InvalidHandle: handle not found in registry\".to_string()); return {}; }}", default_expr).unwrap();
    writeln!(out, "        }};").unwrap();
    writeln!(out, "        drop(__reg);").unwrap();
}

/// Emit a _create body that registers the handle in the registry.
/// `alloc_expr` is like `Vec::<i64>::new()`. `rust_type` is like `Vec<i64>`.
fn emit_registry_create(out: &mut String, alloc_expr: &str, _rust_type: &str) {
    writeln!(out, "        let __data = {};", alloc_expr).unwrap();
    writeln!(out, "        let __ptr = Box::into_raw(Box::new(__data)) as usize;").unwrap();
    writeln!(out, "        let mut __reg = logos_handle_registry().lock().unwrap_or_else(|e| e.into_inner());").unwrap();
    writeln!(out, "        let (__id, _) = __reg.register(__ptr);").unwrap();
    writeln!(out, "        __id as LogosHandle").unwrap();
}

/// Emit a _free body that deregisters and drops the handle.
/// `rust_type` is like `Vec<i64>`.
fn emit_registry_free(out: &mut String, rust_type: &str) {
    writeln!(out, "        if handle.is_null() {{ return; }}").unwrap();
    writeln!(out, "        let __id = handle as u64;").unwrap();
    writeln!(out, "        let mut __reg = logos_handle_registry().lock().unwrap_or_else(|e| e.into_inner());").unwrap();
    writeln!(out, "        match __reg.free(__id) {{").unwrap();
    writeln!(out, "            Ok(__ptr) => {{ unsafe {{ drop(Box::from_raw(__ptr as *mut {})); }} }}", rust_type).unwrap();
    writeln!(out, "            Err(()) => {{ logos_set_last_error(\"InvalidHandle: handle already freed or not found\".to_string()); }}").unwrap();
    writeln!(out, "        }}").unwrap();
}

/// Generate accessor functions for a reference type (Seq, Map, Set, user structs).
/// Returns the Rust source for the accessor/free functions.
fn codegen_c_accessors(ty: &TypeExpr, interner: &Interner, registry: &TypeRegistry) -> String {
    let mut out = String::new();
    let mangled = mangle_type_for_c(ty, interner);

    match ty {
        TypeExpr::Generic { base, params } => {
            let base_name = interner.resolve(*base);
            match base_name {
                "Seq" | "List" | "Vec" if !params.is_empty() => {
                    let inner_rust_type = codegen_type_expr(&params[0], interner);
                    let _inner_mangled = mangle_type_for_c(&params[0], interner);
                    let is_inner_text = is_text_type(&params[0], interner);
                    let vec_type = format!("Vec<{}>", inner_rust_type);

                    // len
                    writeln!(out, "#[no_mangle]").unwrap();
                    writeln!(out, "pub extern \"C\" fn logos_{}_len(handle: LogosHandle) -> usize {{", mangled).unwrap();
                    emit_catch_unwind_open(&mut out);
                    emit_null_handle_check(&mut out, "0");
                    emit_registry_deref(&mut out, "0");
                    writeln!(out, "        let seq = unsafe {{ &*(__ptr as *const {}) }};", vec_type).unwrap();
                    writeln!(out, "        seq.len()").unwrap();
                    emit_catch_unwind_close(&mut out, "0");
                    writeln!(out, "}}\n").unwrap();

                    // at (index access)
                    if is_inner_text {
                        writeln!(out, "#[no_mangle]").unwrap();
                        writeln!(out, "pub extern \"C\" fn logos_{}_at(handle: LogosHandle, index: usize) -> *mut std::os::raw::c_char {{", mangled).unwrap();
                        emit_catch_unwind_open(&mut out);
                        emit_null_handle_check(&mut out, "std::ptr::null_mut()");
                        emit_registry_deref(&mut out, "std::ptr::null_mut()");
                        writeln!(out, "        let seq = unsafe {{ &*(__ptr as *const {}) }};", vec_type).unwrap();
                        writeln!(out, "        if index >= seq.len() {{").unwrap();
                        writeln!(out, "            logos_set_last_error(format!(\"Index {{}} out of bounds (len {{}})\", index, seq.len()));").unwrap();
                        writeln!(out, "            return std::ptr::null_mut();").unwrap();
                        writeln!(out, "        }}").unwrap();
                        writeln!(out, "        match std::ffi::CString::new(seq[index].clone()) {{").unwrap();
                        writeln!(out, "            Ok(cstr) => cstr.into_raw(),").unwrap();
                        writeln!(out, "            Err(_) => {{ logos_set_last_error(\"String contains null byte\".to_string()); std::ptr::null_mut() }}").unwrap();
                        writeln!(out, "        }}").unwrap();
                        emit_catch_unwind_close(&mut out, "std::ptr::null_mut()");
                        writeln!(out, "}}\n").unwrap();
                    } else {
                        writeln!(out, "#[no_mangle]").unwrap();
                        writeln!(out, "pub extern \"C\" fn logos_{}_at(handle: LogosHandle, index: usize, out: *mut {}) -> LogosStatus {{", mangled, inner_rust_type).unwrap();
                        emit_catch_unwind_open(&mut out);
                        emit_null_handle_check(&mut out, "LogosStatus::NullPointer");
                        emit_null_out_check(&mut out, "LogosStatus::NullPointer");
                        emit_registry_deref(&mut out, "LogosStatus::InvalidHandle");
                        writeln!(out, "        let seq = unsafe {{ &*(__ptr as *const {}) }};", vec_type).unwrap();
                        writeln!(out, "        if index >= seq.len() {{").unwrap();
                        writeln!(out, "            logos_set_last_error(format!(\"Index {{}} out of bounds (len {{}})\", index, seq.len()));").unwrap();
                        writeln!(out, "            return LogosStatus::OutOfBounds;").unwrap();
                        writeln!(out, "        }}").unwrap();
                        writeln!(out, "        unsafe {{ *out = seq[index].clone(); }}").unwrap();
                        writeln!(out, "        LogosStatus::Ok").unwrap();
                        emit_catch_unwind_close(&mut out, "LogosStatus::ThreadPanic");
                        writeln!(out, "}}\n").unwrap();
                    }

                    // create
                    writeln!(out, "#[no_mangle]").unwrap();
                    writeln!(out, "pub extern \"C\" fn logos_{}_create() -> LogosHandle {{", mangled).unwrap();
                    emit_catch_unwind_open(&mut out);
                    emit_registry_create(&mut out, &format!("Vec::<{}>::new()", inner_rust_type), &vec_type);
                    emit_catch_unwind_close(&mut out, "std::ptr::null_mut() as LogosHandle");
                    writeln!(out, "}}\n").unwrap();

                    // push
                    if is_inner_text {
                        writeln!(out, "#[no_mangle]").unwrap();
                        writeln!(out, "pub extern \"C\" fn logos_{}_push(handle: LogosHandle, value: *const std::os::raw::c_char) {{", mangled).unwrap();
                        emit_catch_unwind_open(&mut out);
                        emit_null_handle_check(&mut out, "()");
                        emit_registry_deref(&mut out, "()");
                        writeln!(out, "        let seq = unsafe {{ &mut *(__ptr as *mut {}) }};", vec_type).unwrap();
                        writeln!(out, "        let val_str = unsafe {{ std::ffi::CStr::from_ptr(value).to_string_lossy().into_owned() }};").unwrap();
                        writeln!(out, "        seq.push(val_str);").unwrap();
                        emit_catch_unwind_close(&mut out, "()");
                        writeln!(out, "}}\n").unwrap();
                    } else {
                        writeln!(out, "#[no_mangle]").unwrap();
                        writeln!(out, "pub extern \"C\" fn logos_{}_push(handle: LogosHandle, value: {}) {{", mangled, inner_rust_type).unwrap();
                        emit_catch_unwind_open(&mut out);
                        emit_null_handle_check(&mut out, "()");
                        emit_registry_deref(&mut out, "()");
                        writeln!(out, "        let seq = unsafe {{ &mut *(__ptr as *mut {}) }};", vec_type).unwrap();
                        writeln!(out, "        seq.push(value);").unwrap();
                        emit_catch_unwind_close(&mut out, "()");
                        writeln!(out, "}}\n").unwrap();
                    }

                    // pop
                    if is_inner_text {
                        writeln!(out, "#[no_mangle]").unwrap();
                        writeln!(out, "pub extern \"C\" fn logos_{}_pop(handle: LogosHandle) -> *mut std::os::raw::c_char {{", mangled).unwrap();
                        emit_catch_unwind_open(&mut out);
                        emit_null_handle_check(&mut out, "std::ptr::null_mut()");
                        emit_registry_deref(&mut out, "std::ptr::null_mut()");
                        writeln!(out, "        let seq = unsafe {{ &mut *(__ptr as *mut {}) }};", vec_type).unwrap();
                        writeln!(out, "        match seq.pop() {{").unwrap();
                        writeln!(out, "            Some(val) => match std::ffi::CString::new(val) {{").unwrap();
                        writeln!(out, "                Ok(cstr) => cstr.into_raw(),").unwrap();
                        writeln!(out, "                Err(_) => {{ logos_set_last_error(\"Value contains null byte\".to_string()); std::ptr::null_mut() }}").unwrap();
                        writeln!(out, "            }},").unwrap();
                        writeln!(out, "            None => {{ logos_set_last_error(\"Pop from empty sequence\".to_string()); std::ptr::null_mut() }}").unwrap();
                        writeln!(out, "        }}").unwrap();
                        emit_catch_unwind_close(&mut out, "std::ptr::null_mut()");
                        writeln!(out, "}}\n").unwrap();
                    } else {
                        writeln!(out, "#[no_mangle]").unwrap();
                        writeln!(out, "pub extern \"C\" fn logos_{}_pop(handle: LogosHandle, out: *mut {}) -> LogosStatus {{", mangled, inner_rust_type).unwrap();
                        emit_catch_unwind_open(&mut out);
                        emit_null_handle_check(&mut out, "LogosStatus::NullPointer");
                        emit_null_out_check(&mut out, "LogosStatus::NullPointer");
                        emit_registry_deref(&mut out, "LogosStatus::InvalidHandle");
                        writeln!(out, "        let seq = unsafe {{ &mut *(__ptr as *mut {}) }};", vec_type).unwrap();
                        writeln!(out, "        match seq.pop() {{").unwrap();
                        writeln!(out, "            Some(val) => {{ unsafe {{ *out = val; }} LogosStatus::Ok }}").unwrap();
                        writeln!(out, "            None => {{ logos_set_last_error(\"Pop from empty sequence\".to_string()); LogosStatus::Error }}").unwrap();
                        writeln!(out, "        }}").unwrap();
                        emit_catch_unwind_close(&mut out, "LogosStatus::ThreadPanic");
                        writeln!(out, "}}\n").unwrap();
                    }

                    // to_json
                    writeln!(out, "#[no_mangle]").unwrap();
                    writeln!(out, "pub extern \"C\" fn logos_{}_to_json(handle: LogosHandle) -> *mut std::os::raw::c_char {{", mangled).unwrap();
                    emit_catch_unwind_open(&mut out);
                    emit_null_handle_check(&mut out, "std::ptr::null_mut()");
                    emit_registry_deref(&mut out, "std::ptr::null_mut()");
                    writeln!(out, "        let seq = unsafe {{ &*(__ptr as *const {}) }};", vec_type).unwrap();
                    writeln!(out, "        match serde_json::to_string(seq) {{").unwrap();
                    writeln!(out, "            Ok(json) => match std::ffi::CString::new(json) {{").unwrap();
                    writeln!(out, "                Ok(cstr) => cstr.into_raw(),").unwrap();
                    writeln!(out, "                Err(_) => {{ logos_set_last_error(\"JSON contains null byte\".to_string()); std::ptr::null_mut() }}").unwrap();
                    writeln!(out, "            }},").unwrap();
                    writeln!(out, "            Err(e) => {{ logos_set_last_error(e.to_string()); std::ptr::null_mut() }}").unwrap();
                    writeln!(out, "        }}").unwrap();
                    emit_catch_unwind_close(&mut out, "std::ptr::null_mut()");
                    writeln!(out, "}}\n").unwrap();

                    // from_json
                    writeln!(out, "#[no_mangle]").unwrap();
                    writeln!(out, "pub extern \"C\" fn logos_{}_from_json(json: *const std::os::raw::c_char, out: *mut LogosHandle) -> LogosStatus {{", mangled).unwrap();
                    emit_catch_unwind_open(&mut out);
                    writeln!(out, "        if json.is_null() {{ logos_set_last_error(\"Null JSON pointer\".to_string()); return LogosStatus::NullPointer; }}").unwrap();
                    emit_null_out_check(&mut out, "LogosStatus::NullPointer");
                    writeln!(out, "        let json_str = unsafe {{ std::ffi::CStr::from_ptr(json).to_string_lossy() }};").unwrap();
                    writeln!(out, "        match serde_json::from_str::<{}>(&json_str) {{", vec_type).unwrap();
                    writeln!(out, "            Ok(val) => {{").unwrap();
                    writeln!(out, "                let __ptr = Box::into_raw(Box::new(val)) as usize;").unwrap();
                    writeln!(out, "                let mut __reg = logos_handle_registry().lock().unwrap_or_else(|e| e.into_inner());").unwrap();
                    writeln!(out, "                let (__id, _) = __reg.register(__ptr);").unwrap();
                    writeln!(out, "                unsafe {{ *out = __id as LogosHandle; }}").unwrap();
                    writeln!(out, "                LogosStatus::Ok").unwrap();
                    writeln!(out, "            }}").unwrap();
                    writeln!(out, "            Err(e) => {{ logos_set_last_error(e.to_string()); LogosStatus::DeserializationFailed }}").unwrap();
                    writeln!(out, "        }}").unwrap();
                    emit_catch_unwind_close(&mut out, "LogosStatus::ThreadPanic");
                    writeln!(out, "}}\n").unwrap();

                    // free
                    writeln!(out, "#[no_mangle]").unwrap();
                    writeln!(out, "pub extern \"C\" fn logos_{}_free(handle: LogosHandle) {{", mangled).unwrap();
                    emit_catch_unwind_open(&mut out);
                    emit_registry_free(&mut out, &vec_type);
                    emit_catch_unwind_close(&mut out, "()");
                    writeln!(out, "}}\n").unwrap();
                }

                "Map" | "HashMap" if params.len() >= 2 => {
                    let key_rust = codegen_type_expr(&params[0], interner);
                    let val_rust = codegen_type_expr(&params[1], interner);
                    let is_key_text = is_text_type(&params[0], interner);
                    let is_val_text = is_text_type(&params[1], interner);
                    let map_type = format!("std::collections::HashMap<{}, {}>", key_rust, val_rust);

                    // len
                    writeln!(out, "#[no_mangle]").unwrap();
                    writeln!(out, "pub extern \"C\" fn logos_{}_len(handle: LogosHandle) -> usize {{", mangled).unwrap();
                    emit_catch_unwind_open(&mut out);
                    emit_null_handle_check(&mut out, "0");
                    emit_registry_deref(&mut out, "0");
                    writeln!(out, "        let map = unsafe {{ &*(__ptr as *const {}) }};", map_type).unwrap();
                    writeln!(out, "        map.len()").unwrap();
                    emit_catch_unwind_close(&mut out, "0");
                    writeln!(out, "}}\n").unwrap();

                    // get
                    if is_key_text {
                        if is_val_text {
                            writeln!(out, "#[no_mangle]").unwrap();
                            writeln!(out, "pub extern \"C\" fn logos_{}_get(handle: LogosHandle, key: *const std::os::raw::c_char) -> *mut std::os::raw::c_char {{", mangled).unwrap();
                            emit_catch_unwind_open(&mut out);
                            emit_null_handle_check(&mut out, "std::ptr::null_mut()");
                            emit_registry_deref(&mut out, "std::ptr::null_mut()");
                            writeln!(out, "        let map = unsafe {{ &*(__ptr as *const {}) }};", map_type).unwrap();
                            writeln!(out, "        let key_str = unsafe {{ std::ffi::CStr::from_ptr(key).to_string_lossy().into_owned() }};").unwrap();
                            writeln!(out, "        match map.get(&key_str) {{").unwrap();
                            writeln!(out, "            Some(val) => match std::ffi::CString::new(val.clone()) {{").unwrap();
                            writeln!(out, "                Ok(cstr) => cstr.into_raw(),").unwrap();
                            writeln!(out, "                Err(_) => {{ logos_set_last_error(\"Value contains null byte\".to_string()); std::ptr::null_mut() }}").unwrap();
                            writeln!(out, "            }},").unwrap();
                            writeln!(out, "            None => std::ptr::null_mut(),").unwrap();
                            writeln!(out, "        }}").unwrap();
                            emit_catch_unwind_close(&mut out, "std::ptr::null_mut()");
                            writeln!(out, "}}\n").unwrap();
                        } else {
                            writeln!(out, "#[no_mangle]").unwrap();
                            writeln!(out, "pub extern \"C\" fn logos_{}_get(handle: LogosHandle, key: *const std::os::raw::c_char, out: *mut {}) -> LogosStatus {{", mangled, val_rust).unwrap();
                            emit_catch_unwind_open(&mut out);
                            emit_null_handle_check(&mut out, "LogosStatus::NullPointer");
                            emit_null_out_check(&mut out, "LogosStatus::NullPointer");
                            emit_registry_deref(&mut out, "LogosStatus::InvalidHandle");
                            writeln!(out, "        let map = unsafe {{ &*(__ptr as *const {}) }};", map_type).unwrap();
                            writeln!(out, "        let key_str = unsafe {{ std::ffi::CStr::from_ptr(key).to_string_lossy().into_owned() }};").unwrap();
                            writeln!(out, "        match map.get(&key_str) {{").unwrap();
                            writeln!(out, "            Some(val) => {{ unsafe {{ *out = val.clone(); }} LogosStatus::Ok }}").unwrap();
                            writeln!(out, "            None => {{ logos_set_last_error(format!(\"Key not found: {{}}\", key_str)); LogosStatus::Error }}").unwrap();
                            writeln!(out, "        }}").unwrap();
                            emit_catch_unwind_close(&mut out, "LogosStatus::ThreadPanic");
                            writeln!(out, "}}\n").unwrap();
                        }
                    } else {
                        if is_val_text {
                            writeln!(out, "#[no_mangle]").unwrap();
                            writeln!(out, "pub extern \"C\" fn logos_{}_get(handle: LogosHandle, key: {}) -> *mut std::os::raw::c_char {{", mangled, key_rust).unwrap();
                            emit_catch_unwind_open(&mut out);
                            emit_null_handle_check(&mut out, "std::ptr::null_mut()");
                            emit_registry_deref(&mut out, "std::ptr::null_mut()");
                            writeln!(out, "        let map = unsafe {{ &*(__ptr as *const {}) }};", map_type).unwrap();
                            writeln!(out, "        match map.get(&key) {{").unwrap();
                            writeln!(out, "            Some(val) => match std::ffi::CString::new(val.clone()) {{").unwrap();
                            writeln!(out, "                Ok(cstr) => cstr.into_raw(),").unwrap();
                            writeln!(out, "                Err(_) => {{ logos_set_last_error(\"Value contains null byte\".to_string()); std::ptr::null_mut() }}").unwrap();
                            writeln!(out, "            }},").unwrap();
                            writeln!(out, "            None => std::ptr::null_mut(),").unwrap();
                            writeln!(out, "        }}").unwrap();
                            emit_catch_unwind_close(&mut out, "std::ptr::null_mut()");
                            writeln!(out, "}}\n").unwrap();
                        } else {
                            writeln!(out, "#[no_mangle]").unwrap();
                            writeln!(out, "pub extern \"C\" fn logos_{}_get(handle: LogosHandle, key: {}, out: *mut {}) -> LogosStatus {{", mangled, key_rust, val_rust).unwrap();
                            emit_catch_unwind_open(&mut out);
                            emit_null_handle_check(&mut out, "LogosStatus::NullPointer");
                            emit_null_out_check(&mut out, "LogosStatus::NullPointer");
                            emit_registry_deref(&mut out, "LogosStatus::InvalidHandle");
                            writeln!(out, "        let map = unsafe {{ &*(__ptr as *const {}) }};", map_type).unwrap();
                            writeln!(out, "        match map.get(&key) {{").unwrap();
                            writeln!(out, "            Some(val) => {{ unsafe {{ *out = val.clone(); }} LogosStatus::Ok }}").unwrap();
                            writeln!(out, "            None => {{ logos_set_last_error(format!(\"Key not found: {{}}\", key)); LogosStatus::Error }}").unwrap();
                            writeln!(out, "        }}").unwrap();
                            emit_catch_unwind_close(&mut out, "LogosStatus::ThreadPanic");
                            writeln!(out, "}}\n").unwrap();
                        }
                    }

                    // keys
                    writeln!(out, "#[no_mangle]").unwrap();
                    writeln!(out, "pub extern \"C\" fn logos_{}_keys(handle: LogosHandle) -> LogosHandle {{", mangled).unwrap();
                    emit_catch_unwind_open(&mut out);
                    emit_null_handle_check(&mut out, "std::ptr::null_mut() as LogosHandle");
                    emit_registry_deref(&mut out, "std::ptr::null_mut() as LogosHandle");
                    writeln!(out, "        let map = unsafe {{ &*(__ptr as *const {}) }};", map_type).unwrap();
                    writeln!(out, "        let keys: Vec<{}> = map.keys().cloned().collect();", key_rust).unwrap();
                    writeln!(out, "        let __kptr = Box::into_raw(Box::new(keys)) as usize;").unwrap();
                    writeln!(out, "        let mut __reg = logos_handle_registry().lock().unwrap_or_else(|e| e.into_inner());").unwrap();
                    writeln!(out, "        let (__id, _) = __reg.register(__kptr);").unwrap();
                    writeln!(out, "        __id as LogosHandle").unwrap();
                    emit_catch_unwind_close(&mut out, "std::ptr::null_mut() as LogosHandle");
                    writeln!(out, "}}\n").unwrap();

                    // values
                    writeln!(out, "#[no_mangle]").unwrap();
                    writeln!(out, "pub extern \"C\" fn logos_{}_values(handle: LogosHandle) -> LogosHandle {{", mangled).unwrap();
                    emit_catch_unwind_open(&mut out);
                    emit_null_handle_check(&mut out, "std::ptr::null_mut() as LogosHandle");
                    emit_registry_deref(&mut out, "std::ptr::null_mut() as LogosHandle");
                    writeln!(out, "        let map = unsafe {{ &*(__ptr as *const {}) }};", map_type).unwrap();
                    writeln!(out, "        let values: Vec<{}> = map.values().cloned().collect();", val_rust).unwrap();
                    writeln!(out, "        let __vptr = Box::into_raw(Box::new(values)) as usize;").unwrap();
                    writeln!(out, "        let mut __reg = logos_handle_registry().lock().unwrap_or_else(|e| e.into_inner());").unwrap();
                    writeln!(out, "        let (__id, _) = __reg.register(__vptr);").unwrap();
                    writeln!(out, "        __id as LogosHandle").unwrap();
                    emit_catch_unwind_close(&mut out, "std::ptr::null_mut() as LogosHandle");
                    writeln!(out, "}}\n").unwrap();

                    // create
                    writeln!(out, "#[no_mangle]").unwrap();
                    writeln!(out, "pub extern \"C\" fn logos_{}_create() -> LogosHandle {{", mangled).unwrap();
                    emit_catch_unwind_open(&mut out);
                    emit_registry_create(&mut out, &format!("std::collections::HashMap::<{}, {}>::new()", key_rust, val_rust), &map_type);
                    emit_catch_unwind_close(&mut out, "std::ptr::null_mut() as LogosHandle");
                    writeln!(out, "}}\n").unwrap();

                    // insert
                    if is_key_text {
                        let val_param = if is_val_text {
                            "value: *const std::os::raw::c_char".to_string()
                        } else {
                            format!("value: {}", val_rust)
                        };
                        writeln!(out, "#[no_mangle]").unwrap();
                        writeln!(out, "pub extern \"C\" fn logos_{}_insert(handle: LogosHandle, key: *const std::os::raw::c_char, {}) {{", mangled, val_param).unwrap();
                        emit_catch_unwind_open(&mut out);
                        emit_null_handle_check(&mut out, "()");
                        emit_registry_deref(&mut out, "()");
                        writeln!(out, "        let map = unsafe {{ &mut *(__ptr as *mut {}) }};", map_type).unwrap();
                        writeln!(out, "        let key_str = unsafe {{ std::ffi::CStr::from_ptr(key).to_string_lossy().into_owned() }};").unwrap();
                        if is_val_text {
                            writeln!(out, "        let val_str = unsafe {{ std::ffi::CStr::from_ptr(value).to_string_lossy().into_owned() }};").unwrap();
                            writeln!(out, "        map.insert(key_str, val_str);").unwrap();
                        } else {
                            writeln!(out, "        map.insert(key_str, value);").unwrap();
                        }
                        emit_catch_unwind_close(&mut out, "()");
                        writeln!(out, "}}\n").unwrap();
                    } else {
                        let val_param = if is_val_text {
                            "value: *const std::os::raw::c_char".to_string()
                        } else {
                            format!("value: {}", val_rust)
                        };
                        writeln!(out, "#[no_mangle]").unwrap();
                        writeln!(out, "pub extern \"C\" fn logos_{}_insert(handle: LogosHandle, key: {}, {}) {{", mangled, key_rust, val_param).unwrap();
                        emit_catch_unwind_open(&mut out);
                        emit_null_handle_check(&mut out, "()");
                        emit_registry_deref(&mut out, "()");
                        writeln!(out, "        let map = unsafe {{ &mut *(__ptr as *mut {}) }};", map_type).unwrap();
                        if is_val_text {
                            writeln!(out, "        let val_str = unsafe {{ std::ffi::CStr::from_ptr(value).to_string_lossy().into_owned() }};").unwrap();
                            writeln!(out, "        map.insert(key, val_str);").unwrap();
                        } else {
                            writeln!(out, "        map.insert(key, value);").unwrap();
                        }
                        emit_catch_unwind_close(&mut out, "()");
                        writeln!(out, "}}\n").unwrap();
                    }

                    // remove
                    if is_key_text {
                        writeln!(out, "#[no_mangle]").unwrap();
                        writeln!(out, "pub extern \"C\" fn logos_{}_remove(handle: LogosHandle, key: *const std::os::raw::c_char) -> bool {{", mangled).unwrap();
                        emit_catch_unwind_open(&mut out);
                        emit_null_handle_check(&mut out, "false");
                        emit_registry_deref(&mut out, "false");
                        writeln!(out, "        let map = unsafe {{ &mut *(__ptr as *mut {}) }};", map_type).unwrap();
                        writeln!(out, "        let key_str = unsafe {{ std::ffi::CStr::from_ptr(key).to_string_lossy().into_owned() }};").unwrap();
                        writeln!(out, "        map.remove(&key_str).is_some()").unwrap();
                        emit_catch_unwind_close(&mut out, "false");
                        writeln!(out, "}}\n").unwrap();
                    } else {
                        writeln!(out, "#[no_mangle]").unwrap();
                        writeln!(out, "pub extern \"C\" fn logos_{}_remove(handle: LogosHandle, key: {}) -> bool {{", mangled, key_rust).unwrap();
                        emit_catch_unwind_open(&mut out);
                        emit_null_handle_check(&mut out, "false");
                        emit_registry_deref(&mut out, "false");
                        writeln!(out, "        let map = unsafe {{ &mut *(__ptr as *mut {}) }};", map_type).unwrap();
                        writeln!(out, "        map.remove(&key).is_some()").unwrap();
                        emit_catch_unwind_close(&mut out, "false");
                        writeln!(out, "}}\n").unwrap();
                    }

                    // to_json
                    writeln!(out, "#[no_mangle]").unwrap();
                    writeln!(out, "pub extern \"C\" fn logos_{}_to_json(handle: LogosHandle) -> *mut std::os::raw::c_char {{", mangled).unwrap();
                    emit_catch_unwind_open(&mut out);
                    emit_null_handle_check(&mut out, "std::ptr::null_mut()");
                    emit_registry_deref(&mut out, "std::ptr::null_mut()");
                    writeln!(out, "        let map = unsafe {{ &*(__ptr as *const {}) }};", map_type).unwrap();
                    writeln!(out, "        match serde_json::to_string(map) {{").unwrap();
                    writeln!(out, "            Ok(json) => match std::ffi::CString::new(json) {{").unwrap();
                    writeln!(out, "                Ok(cstr) => cstr.into_raw(),").unwrap();
                    writeln!(out, "                Err(_) => {{ logos_set_last_error(\"JSON contains null byte\".to_string()); std::ptr::null_mut() }}").unwrap();
                    writeln!(out, "            }},").unwrap();
                    writeln!(out, "            Err(e) => {{ logos_set_last_error(e.to_string()); std::ptr::null_mut() }}").unwrap();
                    writeln!(out, "        }}").unwrap();
                    emit_catch_unwind_close(&mut out, "std::ptr::null_mut()");
                    writeln!(out, "}}\n").unwrap();

                    // from_json
                    writeln!(out, "#[no_mangle]").unwrap();
                    writeln!(out, "pub extern \"C\" fn logos_{}_from_json(json: *const std::os::raw::c_char, out: *mut LogosHandle) -> LogosStatus {{", mangled).unwrap();
                    emit_catch_unwind_open(&mut out);
                    writeln!(out, "        if json.is_null() {{ logos_set_last_error(\"Null JSON pointer\".to_string()); return LogosStatus::NullPointer; }}").unwrap();
                    emit_null_out_check(&mut out, "LogosStatus::NullPointer");
                    writeln!(out, "        let json_str = unsafe {{ std::ffi::CStr::from_ptr(json).to_string_lossy() }};").unwrap();
                    writeln!(out, "        match serde_json::from_str::<{}>(&json_str) {{", map_type).unwrap();
                    writeln!(out, "            Ok(val) => {{").unwrap();
                    writeln!(out, "                let __ptr = Box::into_raw(Box::new(val)) as usize;").unwrap();
                    writeln!(out, "                let mut __reg = logos_handle_registry().lock().unwrap_or_else(|e| e.into_inner());").unwrap();
                    writeln!(out, "                let (__id, _) = __reg.register(__ptr);").unwrap();
                    writeln!(out, "                unsafe {{ *out = __id as LogosHandle; }}").unwrap();
                    writeln!(out, "                LogosStatus::Ok").unwrap();
                    writeln!(out, "            }}").unwrap();
                    writeln!(out, "            Err(e) => {{ logos_set_last_error(e.to_string()); LogosStatus::DeserializationFailed }}").unwrap();
                    writeln!(out, "        }}").unwrap();
                    emit_catch_unwind_close(&mut out, "LogosStatus::ThreadPanic");
                    writeln!(out, "}}\n").unwrap();

                    // free
                    writeln!(out, "#[no_mangle]").unwrap();
                    writeln!(out, "pub extern \"C\" fn logos_{}_free(handle: LogosHandle) {{", mangled).unwrap();
                    emit_catch_unwind_open(&mut out);
                    emit_registry_free(&mut out, &map_type);
                    emit_catch_unwind_close(&mut out, "()");
                    writeln!(out, "}}\n").unwrap();
                }

                "Set" | "HashSet" if !params.is_empty() => {
                    let inner_rust_type = codegen_type_expr(&params[0], interner);
                    let is_inner_text = is_text_type(&params[0], interner);
                    let set_type = format!("std::collections::HashSet<{}>", inner_rust_type);

                    // len
                    writeln!(out, "#[no_mangle]").unwrap();
                    writeln!(out, "pub extern \"C\" fn logos_{}_len(handle: LogosHandle) -> usize {{", mangled).unwrap();
                    emit_catch_unwind_open(&mut out);
                    emit_null_handle_check(&mut out, "0");
                    emit_registry_deref(&mut out, "0");
                    writeln!(out, "        let set = unsafe {{ &*(__ptr as *const {}) }};", set_type).unwrap();
                    writeln!(out, "        set.len()").unwrap();
                    emit_catch_unwind_close(&mut out, "0");
                    writeln!(out, "}}\n").unwrap();

                    // contains
                    if is_inner_text {
                        writeln!(out, "#[no_mangle]").unwrap();
                        writeln!(out, "pub extern \"C\" fn logos_{}_contains(handle: LogosHandle, value: *const std::os::raw::c_char) -> bool {{", mangled).unwrap();
                        emit_catch_unwind_open(&mut out);
                        emit_null_handle_check(&mut out, "false");
                        emit_registry_deref(&mut out, "false");
                        writeln!(out, "        let set = unsafe {{ &*(__ptr as *const {}) }};", set_type).unwrap();
                        writeln!(out, "        let val_str = unsafe {{ std::ffi::CStr::from_ptr(value).to_string_lossy().into_owned() }};").unwrap();
                        writeln!(out, "        set.contains(&val_str)").unwrap();
                        emit_catch_unwind_close(&mut out, "false");
                        writeln!(out, "}}\n").unwrap();
                    } else {
                        writeln!(out, "#[no_mangle]").unwrap();
                        writeln!(out, "pub extern \"C\" fn logos_{}_contains(handle: LogosHandle, value: {}) -> bool {{", mangled, inner_rust_type).unwrap();
                        emit_catch_unwind_open(&mut out);
                        emit_null_handle_check(&mut out, "false");
                        emit_registry_deref(&mut out, "false");
                        writeln!(out, "        let set = unsafe {{ &*(__ptr as *const {}) }};", set_type).unwrap();
                        writeln!(out, "        set.contains(&value)").unwrap();
                        emit_catch_unwind_close(&mut out, "false");
                        writeln!(out, "}}\n").unwrap();
                    }

                    // create
                    writeln!(out, "#[no_mangle]").unwrap();
                    writeln!(out, "pub extern \"C\" fn logos_{}_create() -> LogosHandle {{", mangled).unwrap();
                    emit_catch_unwind_open(&mut out);
                    emit_registry_create(&mut out, &format!("std::collections::HashSet::<{}>::new()", inner_rust_type), &set_type);
                    emit_catch_unwind_close(&mut out, "std::ptr::null_mut() as LogosHandle");
                    writeln!(out, "}}\n").unwrap();

                    // insert
                    if is_inner_text {
                        writeln!(out, "#[no_mangle]").unwrap();
                        writeln!(out, "pub extern \"C\" fn logos_{}_insert(handle: LogosHandle, value: *const std::os::raw::c_char) {{", mangled).unwrap();
                        emit_catch_unwind_open(&mut out);
                        emit_null_handle_check(&mut out, "()");
                        emit_registry_deref(&mut out, "()");
                        writeln!(out, "        let set = unsafe {{ &mut *(__ptr as *mut {}) }};", set_type).unwrap();
                        writeln!(out, "        let val_str = unsafe {{ std::ffi::CStr::from_ptr(value).to_string_lossy().into_owned() }};").unwrap();
                        writeln!(out, "        set.insert(val_str);").unwrap();
                        emit_catch_unwind_close(&mut out, "()");
                        writeln!(out, "}}\n").unwrap();
                    } else {
                        writeln!(out, "#[no_mangle]").unwrap();
                        writeln!(out, "pub extern \"C\" fn logos_{}_insert(handle: LogosHandle, value: {}) {{", mangled, inner_rust_type).unwrap();
                        emit_catch_unwind_open(&mut out);
                        emit_null_handle_check(&mut out, "()");
                        emit_registry_deref(&mut out, "()");
                        writeln!(out, "        let set = unsafe {{ &mut *(__ptr as *mut {}) }};", set_type).unwrap();
                        writeln!(out, "        set.insert(value);").unwrap();
                        emit_catch_unwind_close(&mut out, "()");
                        writeln!(out, "}}\n").unwrap();
                    }

                    // remove
                    if is_inner_text {
                        writeln!(out, "#[no_mangle]").unwrap();
                        writeln!(out, "pub extern \"C\" fn logos_{}_remove(handle: LogosHandle, value: *const std::os::raw::c_char) -> bool {{", mangled).unwrap();
                        emit_catch_unwind_open(&mut out);
                        emit_null_handle_check(&mut out, "false");
                        emit_registry_deref(&mut out, "false");
                        writeln!(out, "        let set = unsafe {{ &mut *(__ptr as *mut {}) }};", set_type).unwrap();
                        writeln!(out, "        let val_str = unsafe {{ std::ffi::CStr::from_ptr(value).to_string_lossy().into_owned() }};").unwrap();
                        writeln!(out, "        set.remove(&val_str)").unwrap();
                        emit_catch_unwind_close(&mut out, "false");
                        writeln!(out, "}}\n").unwrap();
                    } else {
                        writeln!(out, "#[no_mangle]").unwrap();
                        writeln!(out, "pub extern \"C\" fn logos_{}_remove(handle: LogosHandle, value: {}) -> bool {{", mangled, inner_rust_type).unwrap();
                        emit_catch_unwind_open(&mut out);
                        emit_null_handle_check(&mut out, "false");
                        emit_registry_deref(&mut out, "false");
                        writeln!(out, "        let set = unsafe {{ &mut *(__ptr as *mut {}) }};", set_type).unwrap();
                        writeln!(out, "        set.remove(&value)").unwrap();
                        emit_catch_unwind_close(&mut out, "false");
                        writeln!(out, "}}\n").unwrap();
                    }

                    // to_json
                    writeln!(out, "#[no_mangle]").unwrap();
                    writeln!(out, "pub extern \"C\" fn logos_{}_to_json(handle: LogosHandle) -> *mut std::os::raw::c_char {{", mangled).unwrap();
                    emit_catch_unwind_open(&mut out);
                    emit_null_handle_check(&mut out, "std::ptr::null_mut()");
                    emit_registry_deref(&mut out, "std::ptr::null_mut()");
                    writeln!(out, "        let set = unsafe {{ &*(__ptr as *const {}) }};", set_type).unwrap();
                    writeln!(out, "        match serde_json::to_string(set) {{").unwrap();
                    writeln!(out, "            Ok(json) => match std::ffi::CString::new(json) {{").unwrap();
                    writeln!(out, "                Ok(cstr) => cstr.into_raw(),").unwrap();
                    writeln!(out, "                Err(_) => {{ logos_set_last_error(\"JSON contains null byte\".to_string()); std::ptr::null_mut() }}").unwrap();
                    writeln!(out, "            }},").unwrap();
                    writeln!(out, "            Err(e) => {{ logos_set_last_error(e.to_string()); std::ptr::null_mut() }}").unwrap();
                    writeln!(out, "        }}").unwrap();
                    emit_catch_unwind_close(&mut out, "std::ptr::null_mut()");
                    writeln!(out, "}}\n").unwrap();

                    // free
                    writeln!(out, "#[no_mangle]").unwrap();
                    writeln!(out, "pub extern \"C\" fn logos_{}_free(handle: LogosHandle) {{", mangled).unwrap();
                    emit_catch_unwind_open(&mut out);
                    emit_registry_free(&mut out, &set_type);
                    emit_catch_unwind_close(&mut out, "()");
                    writeln!(out, "}}\n").unwrap();
                }

                "Option" | "Maybe" if !params.is_empty() => {
                    let inner_rust_type = codegen_type_expr(&params[0], interner);
                    let is_inner_text = is_text_type(&params[0], interner);
                    let opt_type = format!("Option<{}>", inner_rust_type);

                    // is_some
                    writeln!(out, "#[no_mangle]").unwrap();
                    writeln!(out, "pub extern \"C\" fn logos_{}_is_some(handle: LogosHandle) -> bool {{", mangled).unwrap();
                    emit_catch_unwind_open(&mut out);
                    emit_null_handle_check(&mut out, "false");
                    emit_registry_deref(&mut out, "false");
                    writeln!(out, "        let opt = unsafe {{ &*(__ptr as *const {}) }};", opt_type).unwrap();
                    writeln!(out, "        opt.is_some()").unwrap();
                    emit_catch_unwind_close(&mut out, "false");
                    writeln!(out, "}}\n").unwrap();

                    // unwrap
                    if is_inner_text {
                        writeln!(out, "#[no_mangle]").unwrap();
                        writeln!(out, "pub extern \"C\" fn logos_{}_unwrap(handle: LogosHandle) -> *mut std::os::raw::c_char {{", mangled).unwrap();
                        emit_catch_unwind_open(&mut out);
                        emit_null_handle_check(&mut out, "std::ptr::null_mut()");
                        emit_registry_deref(&mut out, "std::ptr::null_mut()");
                        writeln!(out, "        let opt = unsafe {{ &*(__ptr as *const {}) }};", opt_type).unwrap();
                        writeln!(out, "        match opt {{").unwrap();
                        writeln!(out, "            Some(val) => match std::ffi::CString::new(val.clone()) {{").unwrap();
                        writeln!(out, "                Ok(cstr) => cstr.into_raw(),").unwrap();
                        writeln!(out, "                Err(_) => {{ logos_set_last_error(\"Value contains null byte\".to_string()); std::ptr::null_mut() }}").unwrap();
                        writeln!(out, "            }},").unwrap();
                        writeln!(out, "            None => {{ logos_set_last_error(\"Unwrap called on None\".to_string()); std::ptr::null_mut() }}").unwrap();
                        writeln!(out, "        }}").unwrap();
                        emit_catch_unwind_close(&mut out, "std::ptr::null_mut()");
                        writeln!(out, "}}\n").unwrap();
                    } else {
                        writeln!(out, "#[no_mangle]").unwrap();
                        writeln!(out, "pub extern \"C\" fn logos_{}_unwrap(handle: LogosHandle, out: *mut {}) -> LogosStatus {{", mangled, inner_rust_type).unwrap();
                        emit_catch_unwind_open(&mut out);
                        emit_null_handle_check(&mut out, "LogosStatus::NullPointer");
                        emit_null_out_check(&mut out, "LogosStatus::NullPointer");
                        emit_registry_deref(&mut out, "LogosStatus::InvalidHandle");
                        writeln!(out, "        let opt = unsafe {{ &*(__ptr as *const {}) }};", opt_type).unwrap();
                        writeln!(out, "        match opt {{").unwrap();
                        writeln!(out, "            Some(val) => {{ unsafe {{ *out = val.clone(); }} LogosStatus::Ok }}").unwrap();
                        writeln!(out, "            None => {{ logos_set_last_error(\"Unwrap called on None\".to_string()); LogosStatus::Error }}").unwrap();
                        writeln!(out, "        }}").unwrap();
                        emit_catch_unwind_close(&mut out, "LogosStatus::ThreadPanic");
                        writeln!(out, "}}\n").unwrap();
                    }

                    // create_some
                    if is_inner_text {
                        writeln!(out, "#[no_mangle]").unwrap();
                        writeln!(out, "pub extern \"C\" fn logos_{}_some(value: *const std::os::raw::c_char) -> LogosHandle {{", mangled).unwrap();
                        emit_catch_unwind_open(&mut out);
                        writeln!(out, "        if value.is_null() {{ logos_set_last_error(\"NullPointer: value is null\".to_string()); return std::ptr::null_mut() as LogosHandle; }}").unwrap();
                        writeln!(out, "        let val_str = unsafe {{ std::ffi::CStr::from_ptr(value).to_string_lossy().into_owned() }};").unwrap();
                        writeln!(out, "        let opt: {} = Some(val_str);", opt_type).unwrap();
                        writeln!(out, "        let __ptr = Box::into_raw(Box::new(opt)) as usize;").unwrap();
                        writeln!(out, "        let mut __reg = logos_handle_registry().lock().unwrap_or_else(|e| e.into_inner());").unwrap();
                        writeln!(out, "        let (__id, _) = __reg.register(__ptr);").unwrap();
                        writeln!(out, "        __id as LogosHandle").unwrap();
                        emit_catch_unwind_close(&mut out, "std::ptr::null_mut() as LogosHandle");
                        writeln!(out, "}}\n").unwrap();
                    } else {
                        writeln!(out, "#[no_mangle]").unwrap();
                        writeln!(out, "pub extern \"C\" fn logos_{}_some(value: {}) -> LogosHandle {{", mangled, inner_rust_type).unwrap();
                        emit_catch_unwind_open(&mut out);
                        writeln!(out, "        let opt: {} = Some(value);", opt_type).unwrap();
                        writeln!(out, "        let __ptr = Box::into_raw(Box::new(opt)) as usize;").unwrap();
                        writeln!(out, "        let mut __reg = logos_handle_registry().lock().unwrap_or_else(|e| e.into_inner());").unwrap();
                        writeln!(out, "        let (__id, _) = __reg.register(__ptr);").unwrap();
                        writeln!(out, "        __id as LogosHandle").unwrap();
                        emit_catch_unwind_close(&mut out, "std::ptr::null_mut() as LogosHandle");
                        writeln!(out, "}}\n").unwrap();
                    }

                    // create_none
                    writeln!(out, "#[no_mangle]").unwrap();
                    writeln!(out, "pub extern \"C\" fn logos_{}_none() -> LogosHandle {{", mangled).unwrap();
                    emit_catch_unwind_open(&mut out);
                    writeln!(out, "        let opt: {} = None;", opt_type).unwrap();
                    writeln!(out, "        let __ptr = Box::into_raw(Box::new(opt)) as usize;").unwrap();
                    writeln!(out, "        let mut __reg = logos_handle_registry().lock().unwrap_or_else(|e| e.into_inner());").unwrap();
                    writeln!(out, "        let (__id, _) = __reg.register(__ptr);").unwrap();
                    writeln!(out, "        __id as LogosHandle").unwrap();
                    emit_catch_unwind_close(&mut out, "std::ptr::null_mut() as LogosHandle");
                    writeln!(out, "}}\n").unwrap();

                    // free
                    writeln!(out, "#[no_mangle]").unwrap();
                    writeln!(out, "pub extern \"C\" fn logos_{}_free(handle: LogosHandle) {{", mangled).unwrap();
                    emit_catch_unwind_open(&mut out);
                    emit_registry_free(&mut out, &opt_type);
                    emit_catch_unwind_close(&mut out, "()");
                    writeln!(out, "}}\n").unwrap();
                }

                _ => {}
            }
        }
        TypeExpr::Primitive(sym) | TypeExpr::Named(sym) => {
            let type_name = interner.resolve(*sym);
            let type_def = registry.get(*sym);

            match type_def {
                Some(TypeDef::Struct { fields, is_portable, .. }) => {
                    let mangled_struct = type_name.to_lowercase();
                    let rust_struct_name = type_name.to_string();

                    for field in fields {
                        let field_name = interner.resolve(field.name);
                        let is_field_text = match &field.ty {
                            FieldType::Primitive(s) | FieldType::Named(s) => {
                                let n = interner.resolve(*s);
                                n == "Text" || n == "String"
                            }
                            _ => false,
                        };

                        writeln!(out, "#[no_mangle]").unwrap();
                        if is_field_text {
                            writeln!(out, "pub extern \"C\" fn logos_{}_{field}(handle: LogosHandle) -> *mut std::os::raw::c_char {{",
                                mangled_struct, field = field_name).unwrap();
                            emit_catch_unwind_open(&mut out);
                            emit_null_handle_check(&mut out, "std::ptr::null_mut()");
                            emit_registry_deref(&mut out, "std::ptr::null_mut()");
                            writeln!(out, "        let obj = unsafe {{ &*(__ptr as *const {}) }};", rust_struct_name).unwrap();
                            writeln!(out, "        match std::ffi::CString::new(obj.{}.clone()) {{", field_name).unwrap();
                            writeln!(out, "            Ok(cstr) => cstr.into_raw(),").unwrap();
                            writeln!(out, "            Err(_) => {{ logos_set_last_error(\"Field contains null byte\".to_string()); std::ptr::null_mut() }}").unwrap();
                            writeln!(out, "        }}").unwrap();
                            emit_catch_unwind_close(&mut out, "std::ptr::null_mut()");
                            writeln!(out, "}}\n").unwrap();
                        } else {
                            let (field_rust_type, is_char) = match &field.ty {
                                FieldType::Primitive(s) | FieldType::Named(s) => {
                                    let n = interner.resolve(*s);
                                    match n {
                                        "Int" => ("i64", false),
                                        "Nat" => ("u64", false),
                                        "Real" | "Float" => ("f64", false),
                                        "Bool" | "Boolean" => ("bool", false),
                                        "Byte" => ("u8", false),
                                        "Char" => ("u32", true),
                                        _ => (n, false),
                                    }
                                }
                                _ => ("LogosHandle", false),
                            };
                            writeln!(out, "pub extern \"C\" fn logos_{}_{field}(handle: LogosHandle) -> {} {{",
                                mangled_struct, field_rust_type, field = field_name).unwrap();
                            emit_catch_unwind_open(&mut out);
                            emit_null_handle_check(&mut out, "Default::default()");
                            emit_registry_deref(&mut out, "Default::default()");
                            writeln!(out, "        let obj = unsafe {{ &*(__ptr as *const {}) }};", rust_struct_name).unwrap();
                            if is_char {
                                writeln!(out, "        obj.{}.clone() as u32", field_name).unwrap();
                            } else {
                                writeln!(out, "        obj.{}.clone()", field_name).unwrap();
                            }
                            emit_catch_unwind_close(&mut out, "Default::default()");
                            writeln!(out, "}}\n").unwrap();
                        }
                    }

                    {
                        // to_json / from_json — always generated for C export reference-type structs
                        writeln!(out, "#[no_mangle]").unwrap();
                        writeln!(out, "pub extern \"C\" fn logos_{}_to_json(handle: LogosHandle) -> *mut std::os::raw::c_char {{", mangled_struct).unwrap();
                        emit_catch_unwind_open(&mut out);
                        emit_null_handle_check(&mut out, "std::ptr::null_mut()");
                        emit_registry_deref(&mut out, "std::ptr::null_mut()");
                        writeln!(out, "        let obj = unsafe {{ &*(__ptr as *const {}) }};", rust_struct_name).unwrap();
                        writeln!(out, "        match serde_json::to_string(obj) {{").unwrap();
                        writeln!(out, "            Ok(json) => match std::ffi::CString::new(json) {{").unwrap();
                        writeln!(out, "                Ok(cstr) => cstr.into_raw(),").unwrap();
                        writeln!(out, "                Err(_) => {{ logos_set_last_error(\"JSON contains null byte\".to_string()); std::ptr::null_mut() }}").unwrap();
                        writeln!(out, "            }},").unwrap();
                        writeln!(out, "            Err(e) => {{ logos_set_last_error(e.to_string()); std::ptr::null_mut() }}").unwrap();
                        writeln!(out, "        }}").unwrap();
                        emit_catch_unwind_close(&mut out, "std::ptr::null_mut()");
                        writeln!(out, "}}\n").unwrap();

                        // from_json — uses registry to register the deserialized handle
                        writeln!(out, "#[no_mangle]").unwrap();
                        writeln!(out, "pub extern \"C\" fn logos_{}_from_json(json: *const std::os::raw::c_char, out: *mut LogosHandle) -> LogosStatus {{", mangled_struct).unwrap();
                        emit_catch_unwind_open(&mut out);
                        writeln!(out, "        if json.is_null() {{ logos_set_last_error(\"Null JSON pointer\".to_string()); return LogosStatus::NullPointer; }}").unwrap();
                        emit_null_out_check(&mut out, "LogosStatus::NullPointer");
                        writeln!(out, "        let json_str = unsafe {{ std::ffi::CStr::from_ptr(json).to_string_lossy() }};").unwrap();
                        writeln!(out, "        match serde_json::from_str::<{}>(&json_str) {{", rust_struct_name).unwrap();
                        writeln!(out, "            Ok(val) => {{").unwrap();
                        writeln!(out, "                let __ptr = Box::into_raw(Box::new(val)) as usize;").unwrap();
                        writeln!(out, "                let mut __reg = logos_handle_registry().lock().unwrap_or_else(|e| e.into_inner());").unwrap();
                        writeln!(out, "                let (__id, _) = __reg.register(__ptr);").unwrap();
                        writeln!(out, "                unsafe {{ *out = __id as LogosHandle; }}").unwrap();
                        writeln!(out, "                LogosStatus::Ok").unwrap();
                        writeln!(out, "            }}").unwrap();
                        writeln!(out, "            Err(e) => {{ logos_set_last_error(e.to_string()); LogosStatus::DeserializationFailed }}").unwrap();
                        writeln!(out, "        }}").unwrap();
                        emit_catch_unwind_close(&mut out, "LogosStatus::ThreadPanic");
                        writeln!(out, "}}\n").unwrap();
                    }

                    // free
                    writeln!(out, "#[no_mangle]").unwrap();
                    writeln!(out, "pub extern \"C\" fn logos_{}_free(handle: LogosHandle) {{", mangled_struct).unwrap();
                    emit_catch_unwind_open(&mut out);
                    emit_registry_free(&mut out, &rust_struct_name);
                    emit_catch_unwind_close(&mut out, "()");
                    writeln!(out, "}}\n").unwrap();
                }
                Some(TypeDef::Enum { variants, .. }) => {
                    let mangled_enum = type_name.to_lowercase();
                    let rust_enum_name = type_name.to_string();

                    // tag accessor
                    writeln!(out, "#[no_mangle]").unwrap();
                    writeln!(out, "pub extern \"C\" fn logos_{}_tag(handle: LogosHandle) -> i32 {{", mangled_enum).unwrap();
                    emit_catch_unwind_open(&mut out);
                    emit_null_handle_check(&mut out, "-1");
                    emit_registry_deref(&mut out, "-1");
                    writeln!(out, "        let obj = unsafe {{ &*(__ptr as *const {}) }};", rust_enum_name).unwrap();
                    writeln!(out, "        match obj {{").unwrap();
                    for (i, variant) in variants.iter().enumerate() {
                        let vname = interner.resolve(variant.name);
                        if variant.fields.is_empty() {
                            writeln!(out, "            {}::{} => {},", rust_enum_name, vname, i).unwrap();
                        } else {
                            writeln!(out, "            {}::{}{{ .. }} => {},", rust_enum_name, vname, i).unwrap();
                        }
                    }
                    writeln!(out, "        }}").unwrap();
                    emit_catch_unwind_close(&mut out, "-1");
                    writeln!(out, "}}\n").unwrap();

                    for variant in variants {
                        let vname = interner.resolve(variant.name);
                        let vname_lower = vname.to_lowercase();
                        for field in &variant.fields {
                            let fname = interner.resolve(field.name);
                            let is_field_text = match &field.ty {
                                FieldType::Primitive(s) | FieldType::Named(s) => {
                                    let n = interner.resolve(*s);
                                    n == "Text" || n == "String"
                                }
                                _ => false,
                            };

                            writeln!(out, "#[no_mangle]").unwrap();
                            if is_field_text {
                                writeln!(out, "pub extern \"C\" fn logos_{}_{}_{fname}(handle: LogosHandle) -> *mut std::os::raw::c_char {{",
                                    mangled_enum, vname_lower, fname = fname).unwrap();
                                emit_catch_unwind_open(&mut out);
                                emit_null_handle_check(&mut out, "std::ptr::null_mut()");
                                emit_registry_deref(&mut out, "std::ptr::null_mut()");
                                writeln!(out, "        let obj = unsafe {{ &*(__ptr as *const {}) }};", rust_enum_name).unwrap();
                                writeln!(out, "        if let {}::{} {{ {fname}, .. }} = obj {{", rust_enum_name, vname, fname = fname).unwrap();
                                writeln!(out, "            match std::ffi::CString::new({fname}.clone()) {{", fname = fname).unwrap();
                                writeln!(out, "                Ok(cstr) => cstr.into_raw(),").unwrap();
                                writeln!(out, "                Err(_) => {{ logos_set_last_error(\"Field contains null byte\".to_string()); std::ptr::null_mut() }}").unwrap();
                                writeln!(out, "            }}").unwrap();
                                writeln!(out, "        }} else {{ logos_set_last_error(\"Wrong variant: expected {}\".to_string()); std::ptr::null_mut() }}", vname).unwrap();
                                emit_catch_unwind_close(&mut out, "std::ptr::null_mut()");
                                writeln!(out, "}}\n").unwrap();
                            } else {
                                let field_rust_type = match &field.ty {
                                    FieldType::Primitive(s) | FieldType::Named(s) => {
                                        let n = interner.resolve(*s);
                                        match n {
                                            "Int" => "i64",
                                            "Nat" => "u64",
                                            "Real" | "Float" => "f64",
                                            "Bool" | "Boolean" => "bool",
                                            "Byte" => "u8",
                                            "Char" => "u32",
                                            _ => n,
                                        }
                                    }
                                    _ => "LogosHandle",
                                };
                                writeln!(out, "pub extern \"C\" fn logos_{}_{}_{fname}(handle: LogosHandle) -> {} {{",
                                    mangled_enum, vname_lower, field_rust_type, fname = fname).unwrap();
                                emit_catch_unwind_open(&mut out);
                                emit_null_handle_check(&mut out, "Default::default()");
                                emit_registry_deref(&mut out, "Default::default()");
                                writeln!(out, "        let obj = unsafe {{ &*(__ptr as *const {}) }};", rust_enum_name).unwrap();
                                writeln!(out, "        if let {}::{} {{ {fname}, .. }} = obj {{", rust_enum_name, vname, fname = fname).unwrap();
                                writeln!(out, "            {fname}.clone()", fname = fname).unwrap();
                                writeln!(out, "        }} else {{ logos_set_last_error(\"Wrong variant: expected {}\".to_string()); Default::default() }}", vname).unwrap();
                                emit_catch_unwind_close(&mut out, "Default::default()");
                                writeln!(out, "}}\n").unwrap();
                            }
                        }
                    }

                    // free
                    writeln!(out, "#[no_mangle]").unwrap();
                    writeln!(out, "pub extern \"C\" fn logos_{}_free(handle: LogosHandle) {{", mangled_enum).unwrap();
                    emit_catch_unwind_open(&mut out);
                    emit_registry_free(&mut out, &rust_enum_name);
                    emit_catch_unwind_close(&mut out, "()");
                    writeln!(out, "}}\n").unwrap();
                }
                _ => {}
            }
        }
        _ => {}
    }

    out
}

/// Collect all unique reference types that appear in C-exported function signatures.
/// Used to emit accessor functions once per type.
fn collect_c_export_reference_types<'a>(
    stmts: &'a [Stmt<'a>],
    interner: &Interner,
    registry: &TypeRegistry,
) -> Vec<&'a TypeExpr<'a>> {
    let mut seen = HashSet::new();
    let mut types = Vec::new();

    for stmt in stmts {
        if let Stmt::FunctionDef { is_exported: true, export_target, params, return_type, .. } = stmt {
            let is_c = match export_target {
                None => true,
                Some(t) => interner.resolve(*t).eq_ignore_ascii_case("c"),
            };
            if !is_c { continue; }

            for (_, ty) in params.iter() {
                if classify_type_for_c_abi(ty, interner, registry) == CAbiClass::ReferenceType {
                    let mangled = mangle_type_for_c(ty, interner);
                    if seen.insert(mangled) {
                        types.push(*ty);
                    }
                }
            }
            if let Some(ty) = return_type {
                if classify_type_for_c_abi(ty, interner, registry) == CAbiClass::ReferenceType {
                    let mangled = mangle_type_for_c(ty, interner);
                    if seen.insert(mangled) {
                        types.push(*ty);
                    }
                }
            }
        }
    }

    types
}

/// Collect all user-defined struct Symbols that are used as C ABI value types in exports.
/// These structs need `#[repr(C)]` for stable field layout.
fn collect_c_export_value_type_structs(
    stmts: &[Stmt],
    interner: &Interner,
    registry: &TypeRegistry,
) -> HashSet<Symbol> {
    let mut value_structs = HashSet::new();

    for stmt in stmts {
        if let Stmt::FunctionDef { is_exported: true, export_target, params, return_type, .. } = stmt {
            let is_c = match export_target {
                None => true,
                Some(t) => interner.resolve(*t).eq_ignore_ascii_case("c"),
            };
            if !is_c { continue; }

            let all_types: Vec<&TypeExpr> = params.iter()
                .map(|(_, ty)| *ty)
                .chain(return_type.iter().copied())
                .collect();

            for ty in all_types {
                if let TypeExpr::Primitive(sym) | TypeExpr::Named(sym) = ty {
                    if classify_type_for_c_abi(ty, interner, registry) == CAbiClass::ValueType {
                        if registry.get(*sym).is_some() {
                            value_structs.insert(*sym);
                        }
                    }
                }
            }
        }
    }

    value_structs
}

/// Collect all user-defined struct Symbols that are used as C ABI reference types in exports.
/// These structs need serde derives for from_json/to_json support.
fn collect_c_export_ref_structs(
    stmts: &[Stmt],
    interner: &Interner,
    registry: &TypeRegistry,
) -> HashSet<Symbol> {
    let mut ref_structs = HashSet::new();
    let ref_types = collect_c_export_reference_types(stmts, interner, registry);
    for ty in ref_types {
        if let TypeExpr::Primitive(sym) | TypeExpr::Named(sym) = ty {
            if registry.get(*sym).map_or(false, |d| matches!(d, TypeDef::Struct { .. })) {
                ref_structs.insert(*sym);
            }
        }
    }
    ref_structs
}

/// Generate the C header (.h) content for all C-exported functions.
///
/// Includes:
/// - Runtime types (logos_status_t, logos_handle_t)
/// - Runtime functions (logos_get_last_error, logos_clear_error, logos_free_string)
/// - Value-type struct definitions
/// - Exported function declarations
/// - Accessor function declarations for reference types
pub fn generate_c_header(
    stmts: &[Stmt],
    module_name: &str,
    interner: &Interner,
    registry: &TypeRegistry,
) -> String {
    let mut out = String::new();
    let guard = module_name.to_uppercase().replace('-', "_");

    writeln!(out, "// Generated from {}.lg — LogicAffeine Universal ABI", module_name).unwrap();
    writeln!(out, "#ifndef {}_H", guard).unwrap();
    writeln!(out, "#define {}_H\n", guard).unwrap();
    writeln!(out, "#include <stdint.h>").unwrap();
    writeln!(out, "#include <stdbool.h>").unwrap();
    writeln!(out, "#include <stddef.h>\n").unwrap();

    writeln!(out, "#ifdef __cplusplus").unwrap();
    writeln!(out, "extern \"C\" {{").unwrap();
    writeln!(out, "#endif\n").unwrap();

    // Runtime types
    writeln!(out, "// ═══ Runtime ═══").unwrap();
    writeln!(out, "typedef enum {{").unwrap();
    writeln!(out, "    LOGOS_STATUS_OK = 0,").unwrap();
    writeln!(out, "    LOGOS_STATUS_ERROR = 1,").unwrap();
    writeln!(out, "    LOGOS_STATUS_REFINEMENT_VIOLATION = 2,").unwrap();
    writeln!(out, "    LOGOS_STATUS_NULL_POINTER = 3,").unwrap();
    writeln!(out, "    LOGOS_STATUS_OUT_OF_BOUNDS = 4,").unwrap();
    writeln!(out, "    LOGOS_STATUS_DESERIALIZATION_FAILED = 5,").unwrap();
    writeln!(out, "    LOGOS_STATUS_INVALID_HANDLE = 6,").unwrap();
    writeln!(out, "    LOGOS_STATUS_CONTAINS_NULL_BYTE = 7,").unwrap();
    writeln!(out, "    LOGOS_STATUS_THREAD_PANIC = 8,").unwrap();
    writeln!(out, "    LOGOS_STATUS_MEMORY_EXHAUSTED = 9,").unwrap();
    writeln!(out, "}} logos_status_t;\n").unwrap();
    writeln!(out, "typedef void* logos_handle_t;\n").unwrap();
    writeln!(out, "const char* logos_last_error(void);").unwrap();
    writeln!(out, "const char* logos_get_last_error(void);").unwrap();
    writeln!(out, "void logos_clear_error(void);").unwrap();
    writeln!(out, "void logos_free_string(char* str);\n").unwrap();

    writeln!(out, "#define LOGOS_ABI_VERSION 1").unwrap();
    writeln!(out, "const char* logos_version(void);").unwrap();
    writeln!(out, "uint32_t logos_abi_version(void);\n").unwrap();

    // Collect value-type user structs used in exports
    let mut emitted_structs = HashSet::new();
    for stmt in stmts {
        if let Stmt::FunctionDef { is_exported: true, export_target, params, return_type, .. } = stmt {
            let is_c = match export_target {
                None => true,
                Some(t) => interner.resolve(*t).eq_ignore_ascii_case("c"),
            };
            if !is_c { continue; }

            // Check params and return for user struct types
            let all_types: Vec<&TypeExpr> = params.iter()
                .map(|(_, ty)| *ty)
                .chain(return_type.iter().copied())
                .collect();

            for ty in all_types {
                if let TypeExpr::Primitive(sym) | TypeExpr::Named(sym) = ty {
                    if classify_type_for_c_abi(ty, interner, registry) == CAbiClass::ValueType {
                        if let Some(TypeDef::Struct { fields, .. }) = registry.get(*sym) {
                            let name = interner.resolve(*sym);
                            if emitted_structs.insert(name.to_string()) {
                                writeln!(out, "// ═══ Value Types ═══").unwrap();
                                writeln!(out, "typedef struct {{").unwrap();
                                for field in fields {
                                    let c_type = map_field_type_to_c(&field.ty, interner);
                                    writeln!(out, "    {} {};", c_type, interner.resolve(field.name)).unwrap();
                                }
                                writeln!(out, "}} {};\n", name).unwrap();
                            }
                        }
                    }
                }
            }
        }
    }

    // Exported function declarations
    writeln!(out, "// ═══ Exported Functions ═══").unwrap();
    for stmt in stmts {
        if let Stmt::FunctionDef { name, is_exported: true, export_target, params, return_type, .. } = stmt {
            let is_c = match export_target {
                None => true,
                Some(t) => interner.resolve(*t).eq_ignore_ascii_case("c"),
            };
            if !is_c { continue; }

            let func_name = format!("logos_{}", interner.resolve(*name));
            let has_ref_return = return_type.map_or(false, |ty| {
                classify_type_for_c_abi(ty, interner, registry) == CAbiClass::ReferenceType
            });
            let has_text_return = return_type.map_or(false, |ty| is_text_type(ty, interner));
            let has_result_return = return_type.map_or(false, |ty| is_result_type(ty, interner));
            let has_refinement_param = params.iter().any(|(_, ty)| matches!(ty, TypeExpr::Refinement { .. }));

            // Status-code pattern matches codegen: ref/text/result returns or refinement params
            let uses_status_code = has_ref_return || has_result_return || has_text_return || has_refinement_param;

            // Build C parameter list (ref-type params always become logos_handle_t)
            let mut c_params = Vec::new();
            for (pname, ptype) in params.iter() {
                let pn = interner.resolve(*pname);
                if classify_type_for_c_abi(ptype, interner, registry) == CAbiClass::ReferenceType {
                    c_params.push(format!("logos_handle_t {}", pn));
                } else {
                    c_params.push(format!("{} {}", map_type_to_c_header(ptype, interner, false), pn));
                }
            }

            if uses_status_code {
                // Out parameter for return value
                if let Some(ret_ty) = return_type {
                    if is_result_type(ret_ty, interner) {
                        if let TypeExpr::Generic { params: ref rparams, .. } = ret_ty {
                            if !rparams.is_empty() {
                                let ok_ty = &rparams[0];
                                if classify_type_for_c_abi(ok_ty, interner, registry) == CAbiClass::ReferenceType {
                                    c_params.push("logos_handle_t* out".to_string());
                                } else {
                                    c_params.push(format!("{}* out", map_type_to_c_header(ok_ty, interner, false)));
                                }
                            }
                        }
                    } else if classify_type_for_c_abi(ret_ty, interner, registry) == CAbiClass::ReferenceType {
                        c_params.push("logos_handle_t* out".to_string());
                    } else if has_text_return {
                        c_params.push("char** out".to_string());
                    }
                }
                writeln!(out, "logos_status_t {}({});", func_name, c_params.join(", ")).unwrap();
            } else {
                // Direct value return
                let ret = return_type
                    .map(|ty| map_type_to_c_header(ty, interner, true))
                    .unwrap_or_else(|| "void".to_string());
                writeln!(out, "{} {}({});", ret, func_name, c_params.join(", ")).unwrap();
            }
        }
    }
    writeln!(out).unwrap();

    // Accessor function declarations for reference types
    let ref_types = collect_c_export_reference_types(stmts, interner, registry);
    if !ref_types.is_empty() {
        for ref_ty in &ref_types {
            let mangled = mangle_type_for_c(ref_ty, interner);
            writeln!(out, "// ═══ {} Accessors ═══", mangled).unwrap();

            match ref_ty {
                TypeExpr::Generic { base, params } => {
                    let base_name = interner.resolve(*base);
                    match base_name {
                        "Seq" | "List" | "Vec" if !params.is_empty() => {
                            let is_inner_text = is_text_type(&params[0], interner);
                            writeln!(out, "size_t logos_{}_len(logos_handle_t handle);", mangled).unwrap();
                            if is_inner_text {
                                writeln!(out, "char* logos_{}_at(logos_handle_t handle, size_t index);", mangled).unwrap();
                            } else {
                                let inner_c = map_type_to_c_header(&params[0], interner, false);
                                writeln!(out, "logos_status_t logos_{}_at(logos_handle_t handle, size_t index, {}* out);", mangled, inner_c).unwrap();
                            }
                            writeln!(out, "logos_handle_t logos_{}_create(void);", mangled).unwrap();
                            if is_inner_text {
                                writeln!(out, "void logos_{}_push(logos_handle_t handle, const char* value);", mangled).unwrap();
                            } else {
                                let inner_c = map_type_to_c_header(&params[0], interner, false);
                                writeln!(out, "void logos_{}_push(logos_handle_t handle, {} value);", mangled, inner_c).unwrap();
                            }
                            if is_inner_text {
                                writeln!(out, "char* logos_{}_pop(logos_handle_t handle);", mangled).unwrap();
                            } else {
                                let inner_c = map_type_to_c_header(&params[0], interner, false);
                                writeln!(out, "logos_status_t logos_{}_pop(logos_handle_t handle, {}* out);", mangled, inner_c).unwrap();
                            }
                            writeln!(out, "char* logos_{}_to_json(logos_handle_t handle);", mangled).unwrap();
                            writeln!(out, "logos_status_t logos_{}_from_json(const char* json, logos_handle_t* out);", mangled).unwrap();
                            writeln!(out, "void logos_{}_free(logos_handle_t handle);", mangled).unwrap();
                        }
                        "Map" | "HashMap" if params.len() >= 2 => {
                            let is_key_text = is_text_type(&params[0], interner);
                            let is_val_text = is_text_type(&params[1], interner);
                            writeln!(out, "size_t logos_{}_len(logos_handle_t handle);", mangled).unwrap();
                            if is_key_text {
                                if is_val_text {
                                    writeln!(out, "char* logos_{}_get(logos_handle_t handle, const char* key);", mangled).unwrap();
                                } else {
                                    let val_c = map_type_to_c_header(&params[1], interner, false);
                                    writeln!(out, "logos_status_t logos_{}_get(logos_handle_t handle, const char* key, {}* out);", mangled, val_c).unwrap();
                                }
                            } else {
                                let key_c = map_type_to_c_header(&params[0], interner, false);
                                if is_val_text {
                                    writeln!(out, "char* logos_{}_get(logos_handle_t handle, {} key);", mangled, key_c).unwrap();
                                } else {
                                    let val_c = map_type_to_c_header(&params[1], interner, false);
                                    writeln!(out, "logos_status_t logos_{}_get(logos_handle_t handle, {} key, {}* out);", mangled, key_c, val_c).unwrap();
                                }
                            }
                            writeln!(out, "logos_handle_t logos_{}_keys(logos_handle_t handle);", mangled).unwrap();
                            writeln!(out, "logos_handle_t logos_{}_values(logos_handle_t handle);", mangled).unwrap();
                            writeln!(out, "logos_handle_t logos_{}_create(void);", mangled).unwrap();
                            if is_key_text {
                                let val_c = if is_val_text { "const char*".to_string() } else { map_type_to_c_header(&params[1], interner, false) };
                                writeln!(out, "void logos_{}_insert(logos_handle_t handle, const char* key, {} value);", mangled, val_c).unwrap();
                            } else {
                                let key_c = map_type_to_c_header(&params[0], interner, false);
                                let val_c = if is_val_text { "const char*".to_string() } else { map_type_to_c_header(&params[1], interner, false) };
                                writeln!(out, "void logos_{}_insert(logos_handle_t handle, {} key, {} value);", mangled, key_c, val_c).unwrap();
                            }
                            if is_key_text {
                                writeln!(out, "bool logos_{}_remove(logos_handle_t handle, const char* key);", mangled).unwrap();
                            } else {
                                let key_c = map_type_to_c_header(&params[0], interner, false);
                                writeln!(out, "bool logos_{}_remove(logos_handle_t handle, {} key);", mangled, key_c).unwrap();
                            }
                            writeln!(out, "char* logos_{}_to_json(logos_handle_t handle);", mangled).unwrap();
                            writeln!(out, "logos_status_t logos_{}_from_json(const char* json, logos_handle_t* out);", mangled).unwrap();
                            writeln!(out, "void logos_{}_free(logos_handle_t handle);", mangled).unwrap();
                        }
                        "Set" | "HashSet" if !params.is_empty() => {
                            let is_inner_text = is_text_type(&params[0], interner);
                            writeln!(out, "size_t logos_{}_len(logos_handle_t handle);", mangled).unwrap();
                            if is_inner_text {
                                writeln!(out, "bool logos_{}_contains(logos_handle_t handle, const char* value);", mangled).unwrap();
                            } else {
                                let inner_c = map_type_to_c_header(&params[0], interner, false);
                                writeln!(out, "bool logos_{}_contains(logos_handle_t handle, {} value);", mangled, inner_c).unwrap();
                            }
                            writeln!(out, "logos_handle_t logos_{}_create(void);", mangled).unwrap();
                            if is_inner_text {
                                writeln!(out, "void logos_{}_insert(logos_handle_t handle, const char* value);", mangled).unwrap();
                            } else {
                                let inner_c = map_type_to_c_header(&params[0], interner, false);
                                writeln!(out, "void logos_{}_insert(logos_handle_t handle, {} value);", mangled, inner_c).unwrap();
                            }
                            if is_inner_text {
                                writeln!(out, "bool logos_{}_remove(logos_handle_t handle, const char* value);", mangled).unwrap();
                            } else {
                                let inner_c = map_type_to_c_header(&params[0], interner, false);
                                writeln!(out, "bool logos_{}_remove(logos_handle_t handle, {} value);", mangled, inner_c).unwrap();
                            }
                            writeln!(out, "char* logos_{}_to_json(logos_handle_t handle);", mangled).unwrap();
                            writeln!(out, "void logos_{}_free(logos_handle_t handle);", mangled).unwrap();
                        }
                        "Option" | "Maybe" if !params.is_empty() => {
                            let is_inner_text = is_text_type(&params[0], interner);
                            writeln!(out, "bool logos_{}_is_some(logos_handle_t handle);", mangled).unwrap();
                            if is_inner_text {
                                writeln!(out, "char* logos_{}_unwrap(logos_handle_t handle);", mangled).unwrap();
                            } else {
                                let inner_c = map_type_to_c_header(&params[0], interner, false);
                                writeln!(out, "logos_status_t logos_{}_unwrap(logos_handle_t handle, {}* out);", mangled, inner_c).unwrap();
                            }
                            if is_inner_text {
                                writeln!(out, "logos_handle_t logos_{}_some(const char* value);", mangled).unwrap();
                            } else {
                                let inner_c = map_type_to_c_header(&params[0], interner, false);
                                writeln!(out, "logos_handle_t logos_{}_some({} value);", mangled, inner_c).unwrap();
                            }
                            writeln!(out, "logos_handle_t logos_{}_none(void);", mangled).unwrap();
                            writeln!(out, "void logos_{}_free(logos_handle_t handle);", mangled).unwrap();
                        }
                        _ => {}
                    }
                }
                TypeExpr::Primitive(sym) | TypeExpr::Named(sym) => {
                    let type_name = interner.resolve(*sym);
                    match registry.get(*sym) {
                        Some(TypeDef::Struct { fields, is_portable, .. }) => {
                            let struct_lower = type_name.to_lowercase();
                            for field in fields {
                                let field_name = interner.resolve(field.name);
                                let is_field_text = match &field.ty {
                                    FieldType::Primitive(s) | FieldType::Named(s) => {
                                        let n = interner.resolve(*s);
                                        n == "Text" || n == "String"
                                    }
                                    _ => false,
                                };
                                if is_field_text {
                                    writeln!(out, "char* logos_{}_{}(logos_handle_t handle);", struct_lower, field_name).unwrap();
                                } else {
                                    let c_type = map_field_type_to_c(&field.ty, interner);
                                    writeln!(out, "{} logos_{}_{}(logos_handle_t handle);", c_type, struct_lower, field_name).unwrap();
                                }
                            }
                            // to_json/from_json always available for C export reference-type structs
                            writeln!(out, "char* logos_{}_to_json(logos_handle_t handle);", struct_lower).unwrap();
                            writeln!(out, "logos_status_t logos_{}_from_json(const char* json, logos_handle_t* out);", struct_lower).unwrap();
                            writeln!(out, "void logos_{}_free(logos_handle_t handle);", struct_lower).unwrap();
                        }
                        Some(TypeDef::Enum { variants, .. }) => {
                            let enum_lower = type_name.to_lowercase();
                            // Tag enum constants
                            writeln!(out, "typedef enum {{").unwrap();
                            for (i, variant) in variants.iter().enumerate() {
                                let vname = interner.resolve(variant.name).to_uppercase();
                                writeln!(out, "    LOGOS_{}_{} = {},", type_name.to_uppercase(), vname, i).unwrap();
                            }
                            writeln!(out, "}} logos_{}_tag_t;", enum_lower).unwrap();
                            writeln!(out, "int32_t logos_{}_tag(logos_handle_t handle);", enum_lower).unwrap();
                            // Per-variant field accessors
                            for variant in variants {
                                let vname = interner.resolve(variant.name);
                                let vname_lower = vname.to_lowercase();
                                for field in &variant.fields {
                                    let fname = interner.resolve(field.name);
                                    let is_field_text = match &field.ty {
                                        FieldType::Primitive(s) | FieldType::Named(s) => {
                                            let n = interner.resolve(*s);
                                            n == "Text" || n == "String"
                                        }
                                        _ => false,
                                    };
                                    if is_field_text {
                                        writeln!(out, "char* logos_{}_{}_{fname}(logos_handle_t handle);", enum_lower, vname_lower, fname = fname).unwrap();
                                    } else {
                                        let c_type = map_field_type_to_c(&field.ty, interner);
                                        writeln!(out, "{} logos_{}_{}_{fname}(logos_handle_t handle);", c_type, enum_lower, vname_lower, fname = fname).unwrap();
                                    }
                                }
                            }
                            writeln!(out, "void logos_{}_free(logos_handle_t handle);", enum_lower).unwrap();
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
            writeln!(out).unwrap();
        }
    }

    writeln!(out, "#ifdef __cplusplus").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out, "#endif\n").unwrap();
    writeln!(out, "#endif // {}_H", guard).unwrap();

    out
}

/// Generate Python ctypes bindings for all C-exported functions.
pub fn generate_python_bindings(
    stmts: &[Stmt],
    module_name: &str,
    interner: &Interner,
    registry: &TypeRegistry,
) -> String {
    let mut out = String::new();

    writeln!(out, "\"\"\"Auto-generated Python bindings for {}.\"\"\"", module_name).unwrap();
    writeln!(out, "import ctypes").unwrap();
    writeln!(out, "from ctypes import c_int64, c_uint64, c_double, c_bool, c_char_p, c_void_p, c_size_t, POINTER").unwrap();
    writeln!(out, "import os").unwrap();
    writeln!(out, "import sys\n").unwrap();

    writeln!(out, "class LogosError(Exception):").unwrap();
    writeln!(out, "    pass\n").unwrap();

    writeln!(out, "class LogosRefinementError(LogosError):").unwrap();
    writeln!(out, "    pass\n").unwrap();

    writeln!(out, "def _lib_ext():").unwrap();
    writeln!(out, "    if sys.platform == \"darwin\":").unwrap();
    writeln!(out, "        return \".dylib\"").unwrap();
    writeln!(out, "    elif sys.platform == \"win32\":").unwrap();
    writeln!(out, "        return \".dll\"").unwrap();
    writeln!(out, "    else:").unwrap();
    writeln!(out, "        return \".so\"\n").unwrap();

    let class_name = module_name.chars().next().unwrap_or('M').to_uppercase().to_string()
        + &module_name[1..];

    writeln!(out, "class {}:", class_name).unwrap();
    writeln!(out, "    OK = 0").unwrap();
    writeln!(out, "    ERROR = 1").unwrap();
    writeln!(out, "    REFINEMENT_VIOLATION = 2").unwrap();
    writeln!(out, "    NULL_POINTER = 3").unwrap();
    writeln!(out, "    OUT_OF_BOUNDS = 4\n").unwrap();

    writeln!(out, "    def __init__(self, path=None):").unwrap();
    writeln!(out, "        if path is None:").unwrap();
    writeln!(out, "            path = os.path.join(os.path.dirname(__file__), \"lib{}\" + _lib_ext())", module_name).unwrap();
    writeln!(out, "        self._lib = ctypes.CDLL(path)").unwrap();
    writeln!(out, "        self._setup()\n").unwrap();

    writeln!(out, "    def _check(self, status):").unwrap();
    writeln!(out, "        if status != self.OK:").unwrap();
    writeln!(out, "            err = self._lib.logos_get_last_error()").unwrap();
    writeln!(out, "            msg = err.decode(\"utf-8\") if err else \"Unknown error\"").unwrap();
    writeln!(out, "            self._lib.logos_clear_error()").unwrap();
    writeln!(out, "            if status == self.REFINEMENT_VIOLATION:").unwrap();
    writeln!(out, "                raise LogosRefinementError(msg)").unwrap();
    writeln!(out, "            raise LogosError(msg)\n").unwrap();

    // _setup method
    writeln!(out, "    def _setup(self):").unwrap();
    writeln!(out, "        self._lib.logos_get_last_error.restype = c_char_p").unwrap();
    writeln!(out, "        self._lib.logos_clear_error.restype = None").unwrap();
    writeln!(out, "        self._lib.logos_free_string.argtypes = [c_char_p]").unwrap();
    writeln!(out, "        self._lib.logos_free_string.restype = None").unwrap();

    // Per-function setup
    for stmt in stmts {
        if let Stmt::FunctionDef { name, is_exported: true, export_target, params, return_type, .. } = stmt {
            let is_c = match export_target {
                None => true,
                Some(t) => interner.resolve(*t).eq_ignore_ascii_case("c"),
            };
            if !is_c { continue; }

            let func_name = format!("logos_{}", interner.resolve(*name));
            let mut argtypes = Vec::new();
            for (_, ptype) in params.iter() {
                argtypes.push(python_ctypes_type(ptype, interner, registry));
            }
            let restype = return_type
                .map(|ty| python_ctypes_type(ty, interner, registry))
                .unwrap_or_else(|| "None".to_string());

            writeln!(out, "        self._lib.{}.argtypes = [{}]", func_name, argtypes.join(", ")).unwrap();
            writeln!(out, "        self._lib.{}.restype = {}", func_name, restype).unwrap();
        }
    }
    writeln!(out).unwrap();

    // Per-function wrapper methods
    for stmt in stmts {
        if let Stmt::FunctionDef { name, is_exported: true, export_target, params, return_type, .. } = stmt {
            let is_c = match export_target {
                None => true,
                Some(t) => interner.resolve(*t).eq_ignore_ascii_case("c"),
            };
            if !is_c { continue; }

            let raw_name = interner.resolve(*name);
            let c_func_name = format!("logos_{}", raw_name);
            let param_names: Vec<String> = params.iter()
                .map(|(pname, _)| interner.resolve(*pname).to_string())
                .collect();
            let type_hints: Vec<String> = params.iter()
                .map(|(pname, ptype)| {
                    format!("{}: {}", interner.resolve(*pname), python_type_hint(ptype, interner))
                })
                .collect();
            let ret_hint = return_type
                .map(|ty| format!(" -> {}", python_type_hint(ty, interner)))
                .unwrap_or_default();

            // Python method uses the raw name for ergonomic API; delegates to prefixed C symbol
            writeln!(out, "    def {}(self, {}){}:", raw_name, type_hints.join(", "), ret_hint).unwrap();
            writeln!(out, "        return self._lib.{}({})", c_func_name, param_names.join(", ")).unwrap();
            writeln!(out).unwrap();
        }
    }

    out
}

fn python_ctypes_type(ty: &TypeExpr, interner: &Interner, registry: &TypeRegistry) -> String {
    match classify_type_for_c_abi(ty, interner, registry) {
        CAbiClass::ReferenceType => "c_void_p".to_string(),
        CAbiClass::ValueType => {
            match ty {
                TypeExpr::Primitive(sym) | TypeExpr::Named(sym) => {
                    let name = interner.resolve(*sym);
                    match name {
                        "Int" => "c_int64".to_string(),
                        "Nat" => "c_uint64".to_string(),
                        "Real" | "Float" => "c_double".to_string(),
                        "Bool" | "Boolean" => "c_bool".to_string(),
                        "Text" | "String" => "c_char_p".to_string(),
                        _ => "c_void_p".to_string(),
                    }
                }
                _ => "c_void_p".to_string(),
            }
        }
    }
}

fn python_type_hint(ty: &TypeExpr, interner: &Interner) -> String {
    match ty {
        TypeExpr::Primitive(sym) | TypeExpr::Named(sym) => {
            let name = interner.resolve(*sym);
            match name {
                "Int" | "Nat" => "int".to_string(),
                "Real" | "Float" => "float".to_string(),
                "Bool" | "Boolean" => "bool".to_string(),
                "Text" | "String" => "str".to_string(),
                other => other.to_string(),
            }
        }
        _ => "object".to_string(),
    }
}

/// Generate TypeScript type declarations (.d.ts) and FFI bindings (.js).
pub fn generate_typescript_bindings(
    stmts: &[Stmt],
    module_name: &str,
    interner: &Interner,
    registry: &TypeRegistry,
) -> (String, String) {
    let mut dts = String::new();
    let mut js = String::new();

    // .d.ts
    writeln!(dts, "// Auto-generated TypeScript definitions for {}", module_name).unwrap();
    let mut ffi_entries = Vec::new();

    for stmt in stmts {
        if let Stmt::FunctionDef { name, is_exported: true, export_target, params, return_type, .. } = stmt {
            let is_c = match export_target {
                None => true,
                Some(t) => interner.resolve(*t).eq_ignore_ascii_case("c"),
            };
            if !is_c { continue; }

            let raw_name = interner.resolve(*name);
            let c_symbol = format!("logos_{}", raw_name);
            let ts_params: Vec<String> = params.iter()
                .map(|(pname, ptype)| format!("{}: {}", interner.resolve(*pname), typescript_type(ptype, interner)))
                .collect();
            let ts_ret = return_type
                .map(|ty| typescript_type(ty, interner))
                .unwrap_or_else(|| "void".to_string());
            writeln!(dts, "export declare function {}({}): {};", raw_name, ts_params.join(", "), ts_ret).unwrap();

            // Collect FFI entries for .js (raw_name for JS API, c_symbol for C FFI)
            let ffi_params: Vec<String> = params.iter()
                .map(|(_, ptype)| ffi_napi_type(ptype, interner, registry))
                .collect();
            let ffi_ret = return_type
                .map(|ty| ffi_napi_type(ty, interner, registry))
                .unwrap_or_else(|| "'void'".to_string());
            ffi_entries.push((raw_name.to_string(), c_symbol, ffi_ret, ffi_params));
        }
    }

    // .js — uses koffi (pure JS, no native deps)
    writeln!(js, "const koffi = require('koffi');").unwrap();
    writeln!(js, "const path = require('path');\n").unwrap();
    writeln!(js, "const libPath = path.join(__dirname, 'lib{}');", module_name).unwrap();
    writeln!(js, "const lib = koffi.load(libPath);\n").unwrap();

    // Declare runtime functions
    writeln!(js, "const logos_get_last_error = lib.func('const char* logos_get_last_error()');").unwrap();
    writeln!(js, "const logos_clear_error = lib.func('void logos_clear_error()');").unwrap();
    writeln!(js, "const logos_free_string = lib.func('void logos_free_string(void* ptr)');\n").unwrap();

    // Declare user-exported functions (C symbols use logos_ prefix)
    for (raw_name, c_symbol, ffi_ret, ffi_params) in &ffi_entries {
        let koffi_ret = ffi_napi_to_koffi(ffi_ret);
        let koffi_params: Vec<String> = ffi_params.iter()
            .enumerate()
            .map(|(i, p)| format!("{} arg{}", ffi_napi_to_koffi(p), i))
            .collect();
        writeln!(js, "const _{} = lib.func('{} {}({})');\n", raw_name, koffi_ret, c_symbol, koffi_params.join(", ")).unwrap();
    }

    writeln!(js, "function checkStatus(status) {{").unwrap();
    writeln!(js, "  if (status !== 0) {{").unwrap();
    writeln!(js, "    const err = logos_get_last_error();").unwrap();
    writeln!(js, "    logos_clear_error();").unwrap();
    writeln!(js, "    throw new Error(err || 'Unknown LogicAffeine error');").unwrap();
    writeln!(js, "  }}").unwrap();
    writeln!(js, "}}\n").unwrap();

    for (raw_name, _, _, _) in &ffi_entries {
        let params_from_stmts = stmts.iter().find_map(|s| {
            if let Stmt::FunctionDef { name, is_exported: true, params, .. } = s {
                if interner.resolve(*name) == raw_name.as_str() {
                    Some(params)
                } else {
                    None
                }
            } else {
                None
            }
        });
        if let Some(params) = params_from_stmts {
            let param_names: Vec<String> = params.iter()
                .map(|(pname, _)| interner.resolve(*pname).to_string())
                .collect();
            writeln!(js, "module.exports.{} = ({}) => _{}({});", raw_name, param_names.join(", "), raw_name, param_names.join(", ")).unwrap();
        }
    }

    (js, dts)
}

fn typescript_type(ty: &TypeExpr, interner: &Interner) -> String {
    match ty {
        TypeExpr::Primitive(sym) | TypeExpr::Named(sym) => {
            let name = interner.resolve(*sym);
            match name {
                "Int" | "Nat" | "Real" | "Float" | "Byte" => "number".to_string(),
                "Bool" | "Boolean" => "boolean".to_string(),
                "Text" | "String" | "Char" => "string".to_string(),
                "Unit" => "void".to_string(),
                other => other.to_string(),
            }
        }
        TypeExpr::Generic { base, params } => {
            let base_name = interner.resolve(*base);
            match base_name {
                "Seq" | "List" | "Vec" if !params.is_empty() => {
                    format!("{}[]", typescript_type(&params[0], interner))
                }
                "Option" | "Maybe" if !params.is_empty() => {
                    format!("{} | null", typescript_type(&params[0], interner))
                }
                _ => "any".to_string(),
            }
        }
        _ => "any".to_string(),
    }
}

/// Convert ffi-napi type strings to koffi type strings for TypeScript bindings.
fn ffi_napi_to_koffi(ffi_type: &str) -> &str {
    match ffi_type {
        "'int64'" => "int64_t",
        "'uint64'" => "uint64_t",
        "'double'" => "double",
        "'bool'" => "bool",
        "'string'" => "const char*",
        "'pointer'" => "void*",
        "'void'" => "void",
        _ => "void*",
    }
}

fn ffi_napi_type(ty: &TypeExpr, interner: &Interner, registry: &TypeRegistry) -> String {
    match classify_type_for_c_abi(ty, interner, registry) {
        CAbiClass::ReferenceType => "'pointer'".to_string(),
        CAbiClass::ValueType => {
            match ty {
                TypeExpr::Primitive(sym) | TypeExpr::Named(sym) => {
                    let name = interner.resolve(*sym);
                    match name {
                        "Int" => "'int64'".to_string(),
                        "Nat" => "'uint64'".to_string(),
                        "Real" | "Float" => "'double'".to_string(),
                        "Bool" | "Boolean" => "'bool'".to_string(),
                        "Text" | "String" => "'string'".to_string(),
                        _ => "'pointer'".to_string(),
                    }
                }
                _ => "'pointer'".to_string(),
            }
        }
    }
}

/// Map a TypeExpr to its C header type representation.
fn map_type_to_c_header(ty: &TypeExpr, interner: &Interner, is_return: bool) -> String {
    match ty {
        TypeExpr::Primitive(sym) | TypeExpr::Named(sym) => {
            let name = interner.resolve(*sym);
            match name {
                "Int" => "int64_t".to_string(),
                "Nat" => "uint64_t".to_string(),
                "Real" | "Float" => "double".to_string(),
                "Bool" | "Boolean" => "bool".to_string(),
                "Byte" => "uint8_t".to_string(),
                "Char" => "uint32_t".to_string(), // UTF-32 char
                "Text" | "String" => {
                    if is_return { "char*".to_string() } else { "const char*".to_string() }
                }
                "Unit" => "void".to_string(),
                other => other.to_string(), // User struct name
            }
        }
        TypeExpr::Refinement { base, .. } => map_type_to_c_header(base, interner, is_return),
        TypeExpr::Generic { .. } => "logos_handle_t".to_string(),
        _ => "logos_handle_t".to_string(),
    }
}

/// Map a FieldType (from TypeRegistry) to a C header type string.
fn map_field_type_to_c(ft: &FieldType, interner: &Interner) -> String {
    match ft {
        FieldType::Primitive(sym) | FieldType::Named(sym) => {
            let name = interner.resolve(*sym);
            match name {
                "Int" => "int64_t".to_string(),
                "Nat" => "uint64_t".to_string(),
                "Real" | "Float" => "double".to_string(),
                "Bool" | "Boolean" => "bool".to_string(),
                "Byte" => "uint8_t".to_string(),
                "Char" => "uint32_t".to_string(),
                "Text" | "String" => "const char*".to_string(),
                other => other.to_string(),
            }
        }
        FieldType::Generic { .. } => "logos_handle_t".to_string(),
        FieldType::TypeParam(_) => "logos_handle_t".to_string(),
    }
}

/// Check if a TypeExpr is a Result type.
fn is_result_type(ty: &TypeExpr, interner: &Interner) -> bool {
    if let TypeExpr::Generic { base, .. } = ty {
        interner.resolve(*base) == "Result"
    } else {
        false
    }
}

/// Phase 51: Detect if any statements require async execution.
/// Returns true if the program needs #[tokio::main] async fn main().
fn requires_async(stmts: &[Stmt]) -> bool {
    stmts.iter().any(|s| requires_async_stmt(s))
}

fn requires_async_stmt(stmt: &Stmt) -> bool {
    match stmt {
        // Phase 9: Concurrent blocks use tokio::join!
        Stmt::Concurrent { tasks } => true,
        // Phase 51: Network operations and Sleep are async
        Stmt::Listen { .. } => true,
        Stmt::ConnectTo { .. } => true,
        Stmt::Sleep { .. } => true,
        // Phase 52: Sync is async (GossipSub subscription)
        Stmt::Sync { .. } => true,
        // Phase 53: Mount is async (VFS file operations)
        Stmt::Mount { .. } => true,
        // Phase 53: File I/O is async (VFS operations)
        Stmt::ReadFrom { source: ReadSource::File(_), .. } => true,
        Stmt::WriteFile { .. } => true,
        // Phase 54: Go-like concurrency is async
        Stmt::LaunchTask { .. } => true,
        Stmt::LaunchTaskWithHandle { .. } => true,
        Stmt::SendPipe { .. } => true,
        Stmt::ReceivePipe { .. } => true,
        Stmt::Select { .. } => true,
        // While and Repeat are now always async due to check_preemption()
        // (handled below in recursive check)
        // Recursively check nested blocks
        Stmt::If { then_block, else_block, .. } => {
            then_block.iter().any(|s| requires_async_stmt(s))
                || else_block.map_or(false, |b| b.iter().any(|s| requires_async_stmt(s)))
        }
        Stmt::While { body, .. } => body.iter().any(|s| requires_async_stmt(s)),
        Stmt::Repeat { body, .. } => body.iter().any(|s| requires_async_stmt(s)),
        Stmt::Zone { body, .. } => body.iter().any(|s| requires_async_stmt(s)),
        Stmt::Parallel { tasks } => tasks.iter().any(|s| requires_async_stmt(s)),
        Stmt::FunctionDef { body, .. } => body.iter().any(|s| requires_async_stmt(s)),
        // Check Inspect arms for async operations
        Stmt::Inspect { arms, .. } => {
            arms.iter().any(|arm| arm.body.iter().any(|s| requires_async_stmt(s)))
        }
        _ => false,
    }
}

/// Phase 53: Detect if any statements require VFS (Virtual File System).
/// Returns true if the program uses file operations or persistent storage.
fn requires_vfs(stmts: &[Stmt]) -> bool {
    stmts.iter().any(|s| requires_vfs_stmt(s))
}

fn requires_vfs_stmt(stmt: &Stmt) -> bool {
    match stmt {
        // Phase 53: Mount uses VFS for persistent storage
        Stmt::Mount { .. } => true,
        // Phase 53: File I/O uses VFS
        Stmt::ReadFrom { source: ReadSource::File(_), .. } => true,
        Stmt::WriteFile { .. } => true,
        // Recursively check nested blocks
        Stmt::If { then_block, else_block, .. } => {
            then_block.iter().any(|s| requires_vfs_stmt(s))
                || else_block.map_or(false, |b| b.iter().any(|s| requires_vfs_stmt(s)))
        }
        Stmt::While { body, .. } => body.iter().any(|s| requires_vfs_stmt(s)),
        Stmt::Repeat { body, .. } => body.iter().any(|s| requires_vfs_stmt(s)),
        Stmt::Zone { body, .. } => body.iter().any(|s| requires_vfs_stmt(s)),
        Stmt::Concurrent { tasks } => tasks.iter().any(|s| requires_vfs_stmt(s)),
        Stmt::Parallel { tasks } => tasks.iter().any(|s| requires_vfs_stmt(s)),
        Stmt::FunctionDef { body, .. } => body.iter().any(|s| requires_vfs_stmt(s)),
        _ => false,
    }
}

/// Phase 49b: Extract root identifier from expression for mutability analysis.
/// Works with both simple identifiers and field accesses.
fn get_root_identifier_for_mutability(expr: &Expr) -> Option<Symbol> {
    match expr {
        Expr::Identifier(sym) => Some(*sym),
        Expr::FieldAccess { object, .. } => get_root_identifier_for_mutability(object),
        _ => None,
    }
}

/// Grand Challenge: Collect all variables that need `let mut` in Rust.
/// This includes:
/// - Variables that are targets of `Set` statements (reassignment)
/// - Variables that are targets of `Push` statements (mutation via push)
/// - Variables that are targets of `Pop` statements (mutation via pop)
fn collect_mutable_vars(stmts: &[Stmt]) -> HashSet<Symbol> {
    let mut targets = HashSet::new();
    for stmt in stmts {
        collect_mutable_vars_stmt(stmt, &mut targets);
    }
    targets
}

fn collect_mutable_vars_stmt(stmt: &Stmt, targets: &mut HashSet<Symbol>) {
    match stmt {
        Stmt::Set { target, .. } => {
            targets.insert(*target);
        }
        Stmt::Push { collection, .. } => {
            // If collection is an identifier or field access, root needs to be mutable
            if let Some(sym) = get_root_identifier_for_mutability(collection) {
                targets.insert(sym);
            }
        }
        Stmt::Pop { collection, .. } => {
            // If collection is an identifier or field access, root needs to be mutable
            if let Some(sym) = get_root_identifier_for_mutability(collection) {
                targets.insert(sym);
            }
        }
        Stmt::Add { collection, .. } => {
            // If collection is an identifier (Set) or field access, root needs to be mutable
            if let Some(sym) = get_root_identifier_for_mutability(collection) {
                targets.insert(sym);
            }
        }
        Stmt::Remove { collection, .. } => {
            // If collection is an identifier (Set) or field access, root needs to be mutable
            if let Some(sym) = get_root_identifier_for_mutability(collection) {
                targets.insert(sym);
            }
        }
        Stmt::SetIndex { collection, .. } => {
            // If collection is an identifier or field access, root needs to be mutable
            if let Some(sym) = get_root_identifier_for_mutability(collection) {
                targets.insert(sym);
            }
        }
        Stmt::If { then_block, else_block, .. } => {
            for s in *then_block {
                collect_mutable_vars_stmt(s, targets);
            }
            if let Some(else_stmts) = else_block {
                for s in *else_stmts {
                    collect_mutable_vars_stmt(s, targets);
                }
            }
        }
        Stmt::While { body, .. } => {
            for s in *body {
                collect_mutable_vars_stmt(s, targets);
            }
        }
        Stmt::Repeat { body, .. } => {
            for s in *body {
                collect_mutable_vars_stmt(s, targets);
            }
        }
        Stmt::Zone { body, .. } => {
            for s in *body {
                collect_mutable_vars_stmt(s, targets);
            }
        }
        // Inspect (pattern match) arms may contain mutations
        Stmt::Inspect { arms, .. } => {
            for arm in arms.iter() {
                for s in arm.body.iter() {
                    collect_mutable_vars_stmt(s, targets);
                }
            }
        }
        // Phase 9: Structured Concurrency blocks
        Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => {
            for s in *tasks {
                collect_mutable_vars_stmt(s, targets);
            }
        }
        // Phase 49b: CRDT operations require mutable access
        Stmt::IncreaseCrdt { object, .. } | Stmt::DecreaseCrdt { object, .. } => {
            // Extract root variable from field access (e.g., g.score -> g)
            if let Some(sym) = get_root_identifier_for_mutability(object) {
                targets.insert(sym);
            }
        }
        Stmt::AppendToSequence { sequence, .. } => {
            if let Some(sym) = get_root_identifier_for_mutability(sequence) {
                targets.insert(sym);
            }
        }
        Stmt::ResolveConflict { object, .. } => {
            if let Some(sym) = get_root_identifier_for_mutability(object) {
                targets.insert(sym);
            }
        }
        // Phase 49b: SetField on MVRegister/LWWRegister uses .set() which requires &mut self
        Stmt::SetField { object, .. } => {
            if let Some(sym) = get_root_identifier_for_mutability(object) {
                targets.insert(sym);
            }
        }
        _ => {}
    }
}

// =============================================================================
// Phase 50: Policy Method Generation
// =============================================================================

/// Generate impl blocks with predicate and capability methods for security policies.
fn codegen_policy_impls(policies: &PolicyRegistry, interner: &Interner) -> String {
    let mut output = String::new();

    // Collect all types that have policies
    let mut type_predicates: HashMap<Symbol, Vec<&PredicateDef>> = HashMap::new();
    let mut type_capabilities: HashMap<Symbol, Vec<&CapabilityDef>> = HashMap::new();

    for (type_sym, predicates) in policies.iter_predicates() {
        type_predicates.entry(*type_sym).or_insert_with(Vec::new).extend(predicates.iter());
    }

    for (type_sym, capabilities) in policies.iter_capabilities() {
        type_capabilities.entry(*type_sym).or_insert_with(Vec::new).extend(capabilities.iter());
    }

    // Get all types that have any policies
    let mut all_types: HashSet<Symbol> = HashSet::new();
    all_types.extend(type_predicates.keys().copied());
    all_types.extend(type_capabilities.keys().copied());

    // Generate impl block for each type
    for type_sym in all_types {
        let type_name = interner.resolve(type_sym);

        writeln!(output, "impl {} {{", type_name).unwrap();

        // Generate predicate methods
        if let Some(predicates) = type_predicates.get(&type_sym) {
            for pred in predicates {
                let pred_name = interner.resolve(pred.predicate_name).to_lowercase();
                writeln!(output, "    pub fn is_{}(&self) -> bool {{", pred_name).unwrap();
                let condition_code = codegen_policy_condition(&pred.condition, interner);
                writeln!(output, "        {}", condition_code).unwrap();
                writeln!(output, "    }}\n").unwrap();
            }
        }

        // Generate capability methods
        if let Some(capabilities) = type_capabilities.get(&type_sym) {
            for cap in capabilities {
                let action_name = interner.resolve(cap.action).to_lowercase();
                let object_type = interner.resolve(cap.object_type);
                let object_param = object_type.to_lowercase();

                writeln!(output, "    pub fn can_{}(&self, {}: &{}) -> bool {{",
                         action_name, object_param, object_type).unwrap();
                let condition_code = codegen_policy_condition(&cap.condition, interner);
                writeln!(output, "        {}", condition_code).unwrap();
                writeln!(output, "    }}\n").unwrap();
            }
        }

        writeln!(output, "}}\n").unwrap();
    }

    output
}

/// Generate Rust code for a policy condition.
fn codegen_policy_condition(condition: &PolicyCondition, interner: &Interner) -> String {
    match condition {
        PolicyCondition::FieldEquals { field, value, is_string_literal } => {
            let field_name = interner.resolve(*field);
            let value_str = interner.resolve(*value);
            if *is_string_literal {
                format!("self.{} == \"{}\"", field_name, value_str)
            } else {
                format!("self.{} == {}", field_name, value_str)
            }
        }
        PolicyCondition::FieldBool { field, value } => {
            let field_name = interner.resolve(*field);
            format!("self.{} == {}", field_name, value)
        }
        PolicyCondition::Predicate { subject: _, predicate } => {
            let pred_name = interner.resolve(*predicate).to_lowercase();
            format!("self.is_{}()", pred_name)
        }
        PolicyCondition::ObjectFieldEquals { subject: _, object, field } => {
            let object_name = interner.resolve(*object).to_lowercase();
            let field_name = interner.resolve(*field);
            format!("self == &{}.{}", object_name, field_name)
        }
        PolicyCondition::Or(left, right) => {
            let left_code = codegen_policy_condition(left, interner);
            let right_code = codegen_policy_condition(right, interner);
            format!("{} || {}", left_code, right_code)
        }
        PolicyCondition::And(left, right) => {
            let left_code = codegen_policy_condition(left, interner);
            let right_code = codegen_policy_condition(right, interner);
            format!("{} && {}", left_code, right_code)
        }
    }
}

/// Collect CRDT register field paths for special handling in SetField codegen.
/// Returns two sets:
/// - LWW fields: (type_name, field_name) pairs where field is LastWriteWins (needs timestamp)
/// - MV fields: (type_name, field_name) pairs where field is Divergent/MVRegister (no timestamp)
fn collect_crdt_register_fields(registry: &TypeRegistry, interner: &Interner) -> (HashSet<(String, String)>, HashSet<(String, String)>) {
    let mut lww_fields = HashSet::new();
    let mut mv_fields = HashSet::new();
    for (type_sym, def) in registry.iter_types() {
        if let TypeDef::Struct { fields, .. } = def {
            let type_name = interner.resolve(*type_sym).to_string();
            for field in fields {
                if let FieldType::Generic { base, .. } = &field.ty {
                    let base_name = interner.resolve(*base);
                    let field_name = interner.resolve(field.name).to_string();
                    if base_name == "LastWriteWins" {
                        lww_fields.insert((type_name.clone(), field_name));
                    } else if base_name == "Divergent" || base_name == "MVRegister" {
                        mv_fields.insert((type_name.clone(), field_name));
                    }
                }
            }
        }
    }
    (lww_fields, mv_fields)
}

/// Phase 102: Collect enum fields that need Box<T> for recursion.
/// Returns a set of (EnumName, VariantName, FieldName) tuples.
fn collect_boxed_fields(registry: &TypeRegistry, interner: &Interner) -> HashSet<(String, String, String)> {
    let mut boxed_fields = HashSet::new();
    for (type_sym, def) in registry.iter_types() {
        if let TypeDef::Enum { variants, .. } = def {
            let enum_name = interner.resolve(*type_sym);
            for variant in variants {
                let variant_name = interner.resolve(variant.name);
                for field in &variant.fields {
                    if is_recursive_field(&field.ty, enum_name, interner) {
                        let field_name = interner.resolve(field.name).to_string();
                        boxed_fields.insert((
                            enum_name.to_string(),
                            variant_name.to_string(),
                            field_name,
                        ));
                    }
                }
            }
        }
    }
    boxed_fields
}

/// Phase 54: Collect function names that are async.
/// Used by LaunchTask codegen to determine if .await is needed.
///
/// Two-pass analysis:
/// 1. First pass: Collect directly async functions (have Sleep, LaunchTask, etc.)
/// 2. Second pass: Iterate until fixed point - if function calls an async function, mark it async
pub fn collect_async_functions(stmts: &[Stmt]) -> HashSet<Symbol> {
    // First, collect all function definitions
    let mut func_bodies: HashMap<Symbol, &[Stmt]> = HashMap::new();
    for stmt in stmts {
        if let Stmt::FunctionDef { name, body, .. } = stmt {
            func_bodies.insert(*name, *body);
        }
    }

    // Pass 1: Collect directly async functions
    let mut async_fns = HashSet::new();
    for stmt in stmts {
        if let Stmt::FunctionDef { name, body, .. } = stmt {
            if body.iter().any(|s| requires_async_stmt(s)) {
                async_fns.insert(*name);
            }
        }
    }

    // Pass 2: Propagate async-ness through call graph until fixed point
    loop {
        let mut changed = false;
        for (func_name, body) in &func_bodies {
            if async_fns.contains(func_name) {
                continue; // Already marked async
            }
            // Check if this function calls any async function
            if body.iter().any(|s| calls_async_function(s, &async_fns)) {
                async_fns.insert(*func_name);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    async_fns
}

/// Helper: Check if a statement calls any function in the async_fns set
fn calls_async_function(stmt: &Stmt, async_fns: &HashSet<Symbol>) -> bool {
    match stmt {
        Stmt::Call { function, args } => {
            // Check if the called function is async OR if any argument expression calls an async function
            async_fns.contains(function)
                || args.iter().any(|a| calls_async_function_in_expr(a, async_fns))
        }
        Stmt::If { cond, then_block, else_block } => {
            calls_async_function_in_expr(cond, async_fns)
                || then_block.iter().any(|s| calls_async_function(s, async_fns))
                || else_block.map_or(false, |b| b.iter().any(|s| calls_async_function(s, async_fns)))
        }
        Stmt::While { cond, body, .. } => {
            calls_async_function_in_expr(cond, async_fns)
                || body.iter().any(|s| calls_async_function(s, async_fns))
        }
        Stmt::Repeat { iterable, body, .. } => {
            calls_async_function_in_expr(iterable, async_fns)
                || body.iter().any(|s| calls_async_function(s, async_fns))
        }
        Stmt::Zone { body, .. } => {
            body.iter().any(|s| calls_async_function(s, async_fns))
        }
        Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => {
            tasks.iter().any(|s| calls_async_function(s, async_fns))
        }
        Stmt::FunctionDef { body, .. } => {
            body.iter().any(|s| calls_async_function(s, async_fns))
        }
        // Check Let statements for async function calls in the value expression
        Stmt::Let { value, .. } => calls_async_function_in_expr(value, async_fns),
        // Check Set statements for async function calls in the value expression
        Stmt::Set { value, .. } => calls_async_function_in_expr(value, async_fns),
        // Check Return statements for async function calls in the return value
        Stmt::Return { value } => {
            value.as_ref().map_or(false, |v| calls_async_function_in_expr(v, async_fns))
        }
        // Check RuntimeAssert condition for async calls
        Stmt::RuntimeAssert { condition } => calls_async_function_in_expr(condition, async_fns),
        // Check Show for async calls
        Stmt::Show { object, .. } => calls_async_function_in_expr(object, async_fns),
        // Check Push for async calls
        Stmt::Push { collection, value } => {
            calls_async_function_in_expr(collection, async_fns)
                || calls_async_function_in_expr(value, async_fns)
        }
        // Check SetIndex for async calls
        Stmt::SetIndex { collection, index, value } => {
            calls_async_function_in_expr(collection, async_fns)
                || calls_async_function_in_expr(index, async_fns)
                || calls_async_function_in_expr(value, async_fns)
        }
        // Check SendPipe for async calls
        Stmt::SendPipe { value, pipe } | Stmt::TrySendPipe { value, pipe, .. } => {
            calls_async_function_in_expr(value, async_fns)
                || calls_async_function_in_expr(pipe, async_fns)
        }
        // Check Inspect arms for async function calls
        Stmt::Inspect { target, arms, .. } => {
            calls_async_function_in_expr(target, async_fns)
                || arms.iter().any(|arm| arm.body.iter().any(|s| calls_async_function(s, async_fns)))
        }
        _ => false,
    }
}

/// Helper: Check if an expression calls any function in the async_fns set
fn calls_async_function_in_expr(expr: &Expr, async_fns: &HashSet<Symbol>) -> bool {
    match expr {
        Expr::Call { function, args } => {
            async_fns.contains(function)
                || args.iter().any(|a| calls_async_function_in_expr(a, async_fns))
        }
        Expr::BinaryOp { left, right, .. } => {
            calls_async_function_in_expr(left, async_fns)
                || calls_async_function_in_expr(right, async_fns)
        }
        Expr::Index { collection, index } => {
            calls_async_function_in_expr(collection, async_fns)
                || calls_async_function_in_expr(index, async_fns)
        }
        Expr::FieldAccess { object, .. } => calls_async_function_in_expr(object, async_fns),
        Expr::List(items) | Expr::Tuple(items) => {
            items.iter().any(|i| calls_async_function_in_expr(i, async_fns))
        }
        Expr::Closure { body, .. } => {
            match body {
                crate::ast::stmt::ClosureBody::Expression(expr) => calls_async_function_in_expr(expr, async_fns),
                crate::ast::stmt::ClosureBody::Block(_) => false,
            }
        }
        Expr::CallExpr { callee, args } => {
            calls_async_function_in_expr(callee, async_fns)
                || args.iter().any(|a| calls_async_function_in_expr(a, async_fns))
        }
        _ => false,
    }
}

// =============================================================================
// Purity Analysis
// =============================================================================

fn collect_pure_functions(stmts: &[Stmt]) -> HashSet<Symbol> {
    let mut func_bodies: HashMap<Symbol, &[Stmt]> = HashMap::new();
    for stmt in stmts {
        if let Stmt::FunctionDef { name, body, .. } = stmt {
            func_bodies.insert(*name, *body);
        }
    }

    // Pass 1: Mark functions as impure if they directly contain impure statements
    let mut impure_fns = HashSet::new();
    for (func_name, body) in &func_bodies {
        if body.iter().any(|s| is_directly_impure_stmt(s)) {
            impure_fns.insert(*func_name);
        }
    }

    // Pass 2: Propagate impurity through call graph until fixed point
    loop {
        let mut changed = false;
        for (func_name, body) in &func_bodies {
            if impure_fns.contains(func_name) {
                continue;
            }
            if body.iter().any(|s| calls_impure_function(s, &impure_fns)) {
                impure_fns.insert(*func_name);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    // Pure = all functions NOT in impure set
    let mut pure_fns = HashSet::new();
    for func_name in func_bodies.keys() {
        if !impure_fns.contains(func_name) {
            pure_fns.insert(*func_name);
        }
    }
    pure_fns
}

fn is_directly_impure_stmt(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Show { .. }
        | Stmt::Give { .. }
        | Stmt::WriteFile { .. }
        | Stmt::ReadFrom { .. }
        | Stmt::Listen { .. }
        | Stmt::ConnectTo { .. }
        | Stmt::SendMessage { .. }
        | Stmt::AwaitMessage { .. }
        | Stmt::Sleep { .. }
        | Stmt::Sync { .. }
        | Stmt::Mount { .. }
        | Stmt::MergeCrdt { .. }
        | Stmt::IncreaseCrdt { .. }
        | Stmt::DecreaseCrdt { .. }
        | Stmt::AppendToSequence { .. }
        | Stmt::ResolveConflict { .. }
        | Stmt::CreatePipe { .. }
        | Stmt::SendPipe { .. }
        | Stmt::ReceivePipe { .. }
        | Stmt::TrySendPipe { .. }
        | Stmt::TryReceivePipe { .. }
        | Stmt::LaunchTask { .. }
        | Stmt::LaunchTaskWithHandle { .. }
        | Stmt::StopTask { .. }
        | Stmt::Concurrent { .. }
        | Stmt::Parallel { .. } => true,
        Stmt::If { then_block, else_block, .. } => {
            then_block.iter().any(|s| is_directly_impure_stmt(s))
                || else_block.map_or(false, |b| b.iter().any(|s| is_directly_impure_stmt(s)))
        }
        Stmt::While { body, .. } | Stmt::Repeat { body, .. } => {
            body.iter().any(|s| is_directly_impure_stmt(s))
        }
        Stmt::Zone { body, .. } => {
            body.iter().any(|s| is_directly_impure_stmt(s))
        }
        Stmt::Inspect { arms, .. } => {
            arms.iter().any(|arm| arm.body.iter().any(|s| is_directly_impure_stmt(s)))
        }
        _ => false,
    }
}

fn calls_impure_function(stmt: &Stmt, impure_fns: &HashSet<Symbol>) -> bool {
    match stmt {
        Stmt::Call { function, args } => {
            impure_fns.contains(function)
                || args.iter().any(|a| expr_calls_impure(a, impure_fns))
        }
        Stmt::Let { value, .. } => expr_calls_impure(value, impure_fns),
        Stmt::Set { value, .. } => expr_calls_impure(value, impure_fns),
        Stmt::Return { value } => value.as_ref().map_or(false, |v| expr_calls_impure(v, impure_fns)),
        Stmt::If { cond, then_block, else_block } => {
            expr_calls_impure(cond, impure_fns)
                || then_block.iter().any(|s| calls_impure_function(s, impure_fns))
                || else_block.map_or(false, |b| b.iter().any(|s| calls_impure_function(s, impure_fns)))
        }
        Stmt::While { cond, body, .. } => {
            expr_calls_impure(cond, impure_fns)
                || body.iter().any(|s| calls_impure_function(s, impure_fns))
        }
        Stmt::Repeat { body, .. } => body.iter().any(|s| calls_impure_function(s, impure_fns)),
        Stmt::Zone { body, .. } => body.iter().any(|s| calls_impure_function(s, impure_fns)),
        Stmt::Inspect { arms, .. } => {
            arms.iter().any(|arm| arm.body.iter().any(|s| calls_impure_function(s, impure_fns)))
        }
        Stmt::Show { object, .. } => expr_calls_impure(object, impure_fns),
        Stmt::Push { value, collection } | Stmt::Add { value, collection } | Stmt::Remove { value, collection } => {
            expr_calls_impure(value, impure_fns) || expr_calls_impure(collection, impure_fns)
        }
        _ => false,
    }
}

fn expr_calls_impure(expr: &Expr, impure_fns: &HashSet<Symbol>) -> bool {
    match expr {
        Expr::Call { function, args } => {
            impure_fns.contains(function)
                || args.iter().any(|a| expr_calls_impure(a, impure_fns))
        }
        Expr::BinaryOp { left, right, .. } => {
            expr_calls_impure(left, impure_fns) || expr_calls_impure(right, impure_fns)
        }
        Expr::Index { collection, index } => {
            expr_calls_impure(collection, impure_fns) || expr_calls_impure(index, impure_fns)
        }
        Expr::FieldAccess { object, .. } => expr_calls_impure(object, impure_fns),
        Expr::List(items) | Expr::Tuple(items) => items.iter().any(|i| expr_calls_impure(i, impure_fns)),
        Expr::CallExpr { callee, args } => {
            expr_calls_impure(callee, impure_fns)
                || args.iter().any(|a| expr_calls_impure(a, impure_fns))
        }
        _ => false,
    }
}

// =============================================================================
// Memoization Detection
// =============================================================================

fn count_self_calls(func_name: Symbol, body: &[Stmt]) -> usize {
    let mut count = 0;
    for stmt in body {
        count += count_self_calls_in_stmt(func_name, stmt);
    }
    count
}

fn count_self_calls_in_stmt(func_name: Symbol, stmt: &Stmt) -> usize {
    match stmt {
        Stmt::Return { value: Some(expr) } => count_self_calls_in_expr(func_name, expr),
        Stmt::Let { value, .. } => count_self_calls_in_expr(func_name, value),
        Stmt::Set { value, .. } => count_self_calls_in_expr(func_name, value),
        Stmt::Call { function, args } => {
            let mut c = if *function == func_name { 1 } else { 0 };
            c += args.iter().map(|a| count_self_calls_in_expr(func_name, a)).sum::<usize>();
            c
        }
        Stmt::If { cond, then_block, else_block } => {
            let mut c = count_self_calls_in_expr(func_name, cond);
            c += count_self_calls(func_name, then_block);
            if let Some(else_stmts) = else_block {
                c += count_self_calls(func_name, else_stmts);
            }
            c
        }
        Stmt::While { cond, body, .. } => {
            count_self_calls_in_expr(func_name, cond) + count_self_calls(func_name, body)
        }
        Stmt::Repeat { body, .. } => count_self_calls(func_name, body),
        Stmt::Show { object, .. } => count_self_calls_in_expr(func_name, object),
        _ => 0,
    }
}

fn count_self_calls_in_expr(func_name: Symbol, expr: &Expr) -> usize {
    match expr {
        Expr::Call { function, args } => {
            let mut c = if *function == func_name { 1 } else { 0 };
            c += args.iter().map(|a| count_self_calls_in_expr(func_name, a)).sum::<usize>();
            c
        }
        Expr::BinaryOp { left, right, .. } => {
            count_self_calls_in_expr(func_name, left) + count_self_calls_in_expr(func_name, right)
        }
        Expr::Index { collection, index } => {
            count_self_calls_in_expr(func_name, collection) + count_self_calls_in_expr(func_name, index)
        }
        Expr::FieldAccess { object, .. } => count_self_calls_in_expr(func_name, object),
        Expr::List(items) | Expr::Tuple(items) => {
            items.iter().map(|i| count_self_calls_in_expr(func_name, i)).sum()
        }
        _ => 0,
    }
}

fn is_hashable_type(ty: &TypeExpr, interner: &Interner) -> bool {
    match ty {
        TypeExpr::Primitive(sym) | TypeExpr::Named(sym) => {
            let name = interner.resolve(*sym);
            matches!(name, "Int" | "Nat" | "Bool" | "Char" | "Byte" | "Text"
                | "i64" | "u64" | "bool" | "char" | "u8" | "String")
        }
        TypeExpr::Refinement { base, .. } => is_hashable_type(base, interner),
        _ => false,
    }
}

fn is_copy_type_expr(ty: &TypeExpr, interner: &Interner) -> bool {
    match ty {
        TypeExpr::Primitive(sym) | TypeExpr::Named(sym) => {
            let name = interner.resolve(*sym);
            matches!(name, "Int" | "Nat" | "Bool" | "Char" | "Byte"
                | "i64" | "u64" | "bool" | "char" | "u8")
        }
        TypeExpr::Refinement { base, .. } => is_copy_type_expr(base, interner),
        _ => false,
    }
}

fn should_memoize(
    name: Symbol,
    body: &[Stmt],
    params: &[(Symbol, &TypeExpr)],
    return_type: Option<&TypeExpr>,
    is_pure: bool,
    interner: &Interner,
) -> bool {
    if !is_pure {
        return false;
    }
    if !body_contains_self_call(name, body) {
        return false;
    }
    if count_self_calls(name, body) < 2 {
        return false;
    }
    if params.is_empty() {
        return false;
    }
    if !params.iter().all(|(_, ty)| is_hashable_type(ty, interner)) {
        return false;
    }
    if return_type.is_none() {
        return false;
    }
    true
}

// =============================================================================
// Tail Call Elimination (TCE) Detection
// =============================================================================

fn expr_is_self_call(func_name: Symbol, expr: &Expr) -> bool {
    matches!(expr, Expr::Call { function, .. } if *function == func_name)
}

fn has_tail_call_in_stmt(func_name: Symbol, stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Return { value: Some(expr) } => {
            if expr_is_self_call(func_name, expr) {
                return true;
            }
            // Check for nested self-call pattern: f(a, f(b, c))
            // The outer call is in tail position even if an arg is also a self-call
            if let Expr::Call { function, args } = expr {
                if *function == func_name {
                    return true;
                }
                // The outer is a self-call with a nested self-call arg — still tail position
                let _ = args;
            }
            false
        }
        Stmt::If { then_block, else_block, .. } => {
            let then_tail = then_block.last()
                .map_or(false, |s| has_tail_call_in_stmt(func_name, s));
            let else_tail = else_block
                .and_then(|block| block.last())
                .map_or(false, |s| has_tail_call_in_stmt(func_name, s));
            then_tail || else_tail
        }
        _ => false,
    }
}

fn is_tail_recursive(func_name: Symbol, body: &[Stmt]) -> bool {
    body.iter().any(|s| has_tail_call_in_stmt(func_name, s))
}

fn body_contains_self_call(func_name: Symbol, body: &[Stmt]) -> bool {
    body.iter().any(|s| stmt_contains_self_call(func_name, s))
}

fn stmt_contains_self_call(func_name: Symbol, stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Return { value: Some(expr) } => expr_contains_self_call(func_name, expr),
        Stmt::Return { value: None } => false,
        Stmt::Let { value, .. } => expr_contains_self_call(func_name, value),
        Stmt::Set { value, .. } => expr_contains_self_call(func_name, value),
        Stmt::Call { function, args } => {
            *function == func_name || args.iter().any(|a| expr_contains_self_call(func_name, a))
        }
        Stmt::If { cond, then_block, else_block } => {
            expr_contains_self_call(func_name, cond)
                || then_block.iter().any(|s| stmt_contains_self_call(func_name, s))
                || else_block.map_or(false, |b| b.iter().any(|s| stmt_contains_self_call(func_name, s)))
        }
        Stmt::While { cond, body, .. } => {
            expr_contains_self_call(func_name, cond)
                || body.iter().any(|s| stmt_contains_self_call(func_name, s))
        }
        Stmt::Repeat { body, .. } => {
            body.iter().any(|s| stmt_contains_self_call(func_name, s))
        }
        Stmt::Show { object, .. } => expr_contains_self_call(func_name, object),
        _ => false,
    }
}

fn expr_contains_self_call(func_name: Symbol, expr: &Expr) -> bool {
    match expr {
        Expr::Call { function, args } => {
            *function == func_name || args.iter().any(|a| expr_contains_self_call(func_name, a))
        }
        Expr::BinaryOp { left, right, .. } => {
            expr_contains_self_call(func_name, left) || expr_contains_self_call(func_name, right)
        }
        Expr::Index { collection, index } => {
            expr_contains_self_call(func_name, collection) || expr_contains_self_call(func_name, index)
        }
        Expr::FieldAccess { object, .. } => expr_contains_self_call(func_name, object),
        Expr::List(items) | Expr::Tuple(items) => {
            items.iter().any(|i| expr_contains_self_call(func_name, i))
        }
        _ => false,
    }
}

// =============================================================================
// Inline Annotation Detection
// =============================================================================

fn should_inline(name: Symbol, body: &[Stmt], is_native: bool, is_exported: bool, is_async: bool) -> bool {
    !is_native && !is_exported && !is_async
        && body.len() <= 5
        && !body_contains_self_call(name, body)
}

// =============================================================================
// Accumulator Introduction — Detection
// =============================================================================

#[derive(Debug)]
enum NonRecSide { Left, Right }

#[derive(Debug)]
struct AccumulatorInfo {
    op: BinaryOpKind,
    identity: &'static str,
    non_recursive_side: NonRecSide,
}

fn detect_accumulator_pattern(func_name: Symbol, body: &[Stmt]) -> Option<AccumulatorInfo> {
    if has_non_return_self_calls(func_name, body) {
        return None;
    }
    let (base_count, recursive_count) = count_recursive_returns(func_name, body);
    if recursive_count != 1 {
        return None;
    }
    if base_count == 0 {
        return None;
    }
    find_accumulator_return(func_name, body)
}

fn count_recursive_returns(func_name: Symbol, body: &[Stmt]) -> (usize, usize) {
    let mut base = 0;
    let mut recursive = 0;
    for stmt in body {
        match stmt {
            Stmt::Return { value: Some(expr) } => {
                if expr_contains_self_call(func_name, expr) {
                    recursive += 1;
                } else {
                    base += 1;
                }
            }
            Stmt::Return { value: None } => {
                base += 1;
            }
            Stmt::If { then_block, else_block, .. } => {
                let (tb, tr) = count_recursive_returns(func_name, then_block);
                base += tb;
                recursive += tr;
                if let Some(else_stmts) = else_block {
                    let (eb, er) = count_recursive_returns(func_name, else_stmts);
                    base += eb;
                    recursive += er;
                }
            }
            _ => {}
        }
    }
    (base, recursive)
}

fn has_non_return_self_calls(func_name: Symbol, body: &[Stmt]) -> bool {
    for stmt in body {
        match stmt {
            Stmt::Return { .. } => {}
            Stmt::If { cond, then_block, else_block } => {
                if expr_contains_self_call(func_name, cond) {
                    return true;
                }
                if has_non_return_self_calls(func_name, then_block) {
                    return true;
                }
                if let Some(else_stmts) = else_block {
                    if has_non_return_self_calls(func_name, else_stmts) {
                        return true;
                    }
                }
            }
            Stmt::Let { value, .. } => {
                if expr_contains_self_call(func_name, value) {
                    return true;
                }
            }
            Stmt::Set { value, .. } => {
                if expr_contains_self_call(func_name, value) {
                    return true;
                }
            }
            Stmt::Show { object, .. } => {
                if expr_contains_self_call(func_name, object) {
                    return true;
                }
            }
            Stmt::While { cond, body, .. } => {
                if expr_contains_self_call(func_name, cond) {
                    return true;
                }
                if has_non_return_self_calls(func_name, body) {
                    return true;
                }
            }
            Stmt::Repeat { body, .. } => {
                if has_non_return_self_calls(func_name, body) {
                    return true;
                }
            }
            Stmt::Call { function, args } => {
                if *function == func_name || args.iter().any(|a| expr_contains_self_call(func_name, a)) {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

fn find_accumulator_return(func_name: Symbol, body: &[Stmt]) -> Option<AccumulatorInfo> {
    for stmt in body {
        match stmt {
            Stmt::Return { value: Some(expr) } => {
                if let Expr::BinaryOp { op, left, right } = expr {
                    match op {
                        BinaryOpKind::Add | BinaryOpKind::Multiply => {
                            let left_has_call = expr_is_self_call(func_name, left);
                            let right_has_call = expr_is_self_call(func_name, right);
                            let left_contains_call = expr_contains_self_call(func_name, left);
                            let right_contains_call = expr_contains_self_call(func_name, right);
                            let identity = match op {
                                BinaryOpKind::Add => "0",
                                BinaryOpKind::Multiply => "1",
                                _ => unreachable!(),
                            };
                            if left_has_call && !right_contains_call {
                                return Some(AccumulatorInfo {
                                    op: *op,
                                    identity,
                                    non_recursive_side: NonRecSide::Right,
                                });
                            }
                            if right_has_call && !left_contains_call {
                                return Some(AccumulatorInfo {
                                    op: *op,
                                    identity,
                                    non_recursive_side: NonRecSide::Left,
                                });
                            }
                        }
                        _ => {}
                    }
                }
            }
            Stmt::If { then_block, else_block, .. } => {
                if let Some(info) = find_accumulator_return(func_name, then_block) {
                    return Some(info);
                }
                if let Some(else_stmts) = else_block {
                    if let Some(info) = find_accumulator_return(func_name, else_stmts) {
                        return Some(info);
                    }
                }
            }
            _ => {}
        }
    }
    None
}

// =============================================================================
// Accumulator Introduction — Statement Emitter
// =============================================================================

fn codegen_stmt_acc<'a>(
    stmt: &Stmt<'a>,
    func_name: Symbol,
    param_names: &[Symbol],
    acc_info: &AccumulatorInfo,
    interner: &Interner,
    indent: usize,
    mutable_vars: &HashSet<Symbol>,
    ctx: &mut RefinementContext<'a>,
    lww_fields: &HashSet<(String, String)>,
    mv_fields: &HashSet<(String, String)>,
    synced_vars: &mut HashSet<Symbol>,
    var_caps: &HashMap<Symbol, VariableCapabilities>,
    async_functions: &HashSet<Symbol>,
    pipe_vars: &HashSet<Symbol>,
    boxed_fields: &HashSet<(String, String, String)>,
    registry: &TypeRegistry,
) -> String {
    let indent_str = "    ".repeat(indent);
    let op_str = match acc_info.op {
        BinaryOpKind::Add => "+",
        BinaryOpKind::Multiply => "*",
        _ => unreachable!(),
    };

    match stmt {
        // Recursive return: BinaryOp(op, self_call, non_rec) or swapped
        Stmt::Return { value: Some(expr) } if expr_contains_self_call(func_name, expr) => {
            if let Expr::BinaryOp { left, right, .. } = expr {
                let (call_expr, non_rec_expr) = match acc_info.non_recursive_side {
                    NonRecSide::Left => (right, left),
                    NonRecSide::Right => (left, right),
                };
                // Extract args from the self-call
                if let Expr::Call { args, .. } = call_expr {
                    let mut output = String::new();
                    writeln!(output, "{}{{", indent_str).unwrap();
                    let non_rec_str = codegen_expr_with_async(non_rec_expr, interner, synced_vars, async_functions, ctx.get_variable_types());
                    writeln!(output, "{}    let __acc_expr = {};", indent_str, non_rec_str).unwrap();
                    writeln!(output, "{}    __acc = __acc {} __acc_expr;", indent_str, op_str).unwrap();
                    // Evaluate args into temporaries
                    for (i, arg) in args.iter().enumerate() {
                        let arg_str = codegen_expr_with_async(arg, interner, synced_vars, async_functions, ctx.get_variable_types());
                        writeln!(output, "{}    let __tce_{} = {};", indent_str, i, arg_str).unwrap();
                    }
                    // Assign temporaries to params
                    for (i, param_sym) in param_names.iter().enumerate() {
                        let param_name = interner.resolve(*param_sym);
                        writeln!(output, "{}    {} = __tce_{};", indent_str, param_name, i).unwrap();
                    }
                    writeln!(output, "{}    continue;", indent_str).unwrap();
                    writeln!(output, "{}}}", indent_str).unwrap();
                    return output;
                }
            }
            // Fallback
            codegen_stmt(stmt, interner, indent, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry)
        }

        // Base return: no self-call
        Stmt::Return { value: Some(expr) } => {
            let val_str = codegen_expr_with_async(expr, interner, synced_vars, async_functions, ctx.get_variable_types());
            format!("{}return __acc {} {};\n", indent_str, op_str, val_str)
        }

        Stmt::Return { value: None } => {
            format!("{}return __acc;\n", indent_str)
        }

        // If: recurse into branches
        Stmt::If { cond, then_block, else_block } => {
            let cond_str = codegen_expr_with_async(cond, interner, synced_vars, async_functions, ctx.get_variable_types());
            let mut output = String::new();
            writeln!(output, "{}if {} {{", indent_str, cond_str).unwrap();
            ctx.push_scope();
            for s in *then_block {
                output.push_str(&codegen_stmt_acc(s, func_name, param_names, acc_info, interner, indent + 1, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry));
            }
            ctx.pop_scope();
            if let Some(else_stmts) = else_block {
                writeln!(output, "{}}} else {{", indent_str).unwrap();
                ctx.push_scope();
                for s in *else_stmts {
                    output.push_str(&codegen_stmt_acc(s, func_name, param_names, acc_info, interner, indent + 1, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry));
                }
                ctx.pop_scope();
            }
            writeln!(output, "{}}}", indent_str).unwrap();
            output
        }

        // Everything else: delegate
        _ => codegen_stmt(stmt, interner, indent, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry),
    }
}

// =============================================================================
// Mutual Tail Call Optimization — Detection
// =============================================================================

fn find_tail_call_targets(func_name: Symbol, body: &[Stmt]) -> HashSet<Symbol> {
    let mut targets = HashSet::new();
    for stmt in body {
        collect_tail_targets(func_name, stmt, &mut targets);
    }
    targets
}

fn collect_tail_targets(func_name: Symbol, stmt: &Stmt, targets: &mut HashSet<Symbol>) {
    match stmt {
        Stmt::Return { value: Some(Expr::Call { function, .. }) } => {
            if *function != func_name {
                targets.insert(*function);
            }
        }
        Stmt::If { then_block, else_block, .. } => {
            if let Some(last) = then_block.last() {
                collect_tail_targets(func_name, last, targets);
            }
            if let Some(else_stmts) = else_block {
                if let Some(last) = else_stmts.last() {
                    collect_tail_targets(func_name, last, targets);
                }
            }
        }
        _ => {}
    }
}

fn detect_mutual_tce_pairs<'a>(stmts: &'a [Stmt<'a>], interner: &Interner) -> Vec<(Symbol, Symbol)> {
    // Collect function definitions
    let mut func_defs: HashMap<Symbol, (&[(Symbol, &TypeExpr)], &[Stmt], Option<&TypeExpr>, bool, bool, bool)> = HashMap::new();
    for stmt in stmts {
        if let Stmt::FunctionDef { name, params, body, return_type, is_native, is_exported, .. } = stmt {
            let is_async_fn = false; // Will be checked properly later
            func_defs.insert(*name, (params, body, return_type.as_ref().copied(), *is_native, *is_exported, is_async_fn));
        }
    }

    // Build tail-call graph
    let mut tail_targets: HashMap<Symbol, HashSet<Symbol>> = HashMap::new();
    for (name, (_, body, _, _, _, _)) in &func_defs {
        tail_targets.insert(*name, find_tail_call_targets(*name, body));
    }

    // Find mutually tail-calling pairs
    let mut pairs = Vec::new();
    let mut used = HashSet::new();
    let names: Vec<Symbol> = func_defs.keys().copied().collect();

    for i in 0..names.len() {
        for j in (i + 1)..names.len() {
            let a = names[i];
            let b = names[j];
            if used.contains(&a) || used.contains(&b) {
                continue;
            }

            let a_targets = tail_targets.get(&a).cloned().unwrap_or_default();
            let b_targets = tail_targets.get(&b).cloned().unwrap_or_default();

            // Both must tail-call each other
            if !a_targets.contains(&b) || !b_targets.contains(&a) {
                continue;
            }

            let (a_params, _, a_ret, a_native, a_exported, _) = func_defs[&a];
            let (b_params, _, b_ret, b_native, b_exported, _) = func_defs[&b];

            // Neither can be native or exported
            if a_native || b_native || a_exported || b_exported {
                continue;
            }

            // Same number of params
            if a_params.len() != b_params.len() {
                continue;
            }

            // Same param types
            let same_params = a_params.iter().zip(b_params.iter()).all(|((_, t1), (_, t2))| {
                codegen_type_expr(t1, interner) == codegen_type_expr(t2, interner)
            });
            if !same_params {
                continue;
            }

            // Same return type
            let a_ret_str = a_ret.map(|t| codegen_type_expr(t, interner));
            let b_ret_str = b_ret.map(|t| codegen_type_expr(t, interner));
            if a_ret_str != b_ret_str {
                continue;
            }

            // Verify that the mutual calls are actually in tail position
            // (the targets above only collect Return { Call } patterns, so they are)
            pairs.push((a, b));
            used.insert(a);
            used.insert(b);
        }
    }

    pairs
}

// =============================================================================
// Mutual Tail Call Optimization — Code Generation
// =============================================================================

fn codegen_mutual_tce_pair<'a>(
    func_a: Symbol,
    func_b: Symbol,
    stmts: &'a [Stmt<'a>],
    interner: &Interner,
    lww_fields: &HashSet<(String, String)>,
    mv_fields: &HashSet<(String, String)>,
    async_functions: &HashSet<Symbol>,
    boxed_fields: &HashSet<(String, String, String)>,
    registry: &TypeRegistry,
) -> String {
    // Extract function defs
    let mut a_def = None;
    let mut b_def = None;
    for stmt in stmts {
        if let Stmt::FunctionDef { name, params, body, return_type, .. } = stmt {
            if *name == func_a {
                a_def = Some((params.as_slice(), *body, return_type.as_ref().copied()));
            } else if *name == func_b {
                b_def = Some((params.as_slice(), *body, return_type.as_ref().copied()));
            }
        }
    }
    let (a_params, a_body, a_ret) = a_def.expect("mutual TCE: func_a not found");
    let (b_params, b_body, _b_ret) = b_def.expect("mutual TCE: func_b not found");

    let a_name = escape_rust_ident(interner.resolve(func_a));
    let b_name = escape_rust_ident(interner.resolve(func_b));
    let merged_name = format!("__mutual_{}_{}", a_name, b_name);

    // Build param list (using func_a's param names, since types match)
    let params_str: Vec<String> = a_params.iter()
        .map(|(p, t)| format!("mut {}: {}", interner.resolve(*p), codegen_type_expr(t, interner)))
        .collect();

    let ret_str = a_ret.map(|t| codegen_type_expr(t, interner));

    let mut output = String::new();

    // Merged function
    let sig = if let Some(ref r) = ret_str {
        if r != "()" {
            format!("fn {}(mut __tag: u8, {}) -> {}", merged_name, params_str.join(", "), r)
        } else {
            format!("fn {}(mut __tag: u8, {})", merged_name, params_str.join(", "))
        }
    } else {
        format!("fn {}(mut __tag: u8, {})", merged_name, params_str.join(", "))
    };

    writeln!(output, "{} {{", sig).unwrap();
    writeln!(output, "    loop {{").unwrap();
    writeln!(output, "        match __tag {{").unwrap();

    // Tag 0: func_a body
    writeln!(output, "            0 => {{").unwrap();
    let a_mutable = collect_mutable_vars(a_body);
    let mut a_ctx = RefinementContext::new();
    let mut a_synced = HashSet::new();
    let a_caps = HashMap::new();
    let a_pipes = HashSet::new();
    let a_param_syms: Vec<Symbol> = a_params.iter().map(|(s, _)| *s).collect();
    for s in a_body {
        output.push_str(&codegen_stmt_mutual_tce(s, func_a, func_b, &a_param_syms, 0, 1, interner, 4, &a_mutable, &mut a_ctx, lww_fields, mv_fields, &mut a_synced, &a_caps, async_functions, &a_pipes, boxed_fields, registry));
    }
    writeln!(output, "            }}").unwrap();

    // Tag 1: func_b body
    writeln!(output, "            1 => {{").unwrap();
    let b_mutable = collect_mutable_vars(b_body);
    let mut b_ctx = RefinementContext::new();
    let mut b_synced = HashSet::new();
    let b_caps = HashMap::new();
    let b_pipes = HashSet::new();
    let b_param_syms: Vec<Symbol> = b_params.iter().map(|(s, _)| *s).collect();
    // Map b's param names to a's param names for assignment
    for s in b_body {
        output.push_str(&codegen_stmt_mutual_tce(s, func_b, func_a, &b_param_syms, 1, 0, interner, 4, &b_mutable, &mut b_ctx, lww_fields, mv_fields, &mut b_synced, &b_caps, async_functions, &b_pipes, boxed_fields, registry));
    }
    writeln!(output, "            }}").unwrap();

    writeln!(output, "            _ => unreachable!()").unwrap();
    writeln!(output, "        }}").unwrap();
    writeln!(output, "    }}").unwrap();
    writeln!(output, "}}\n").unwrap();

    // Wrapper for func_a
    let wrapper_params_a: Vec<String> = a_params.iter()
        .map(|(p, t)| format!("{}: {}", interner.resolve(*p), codegen_type_expr(t, interner)))
        .collect();
    let wrapper_args_a: Vec<String> = a_params.iter()
        .map(|(p, _)| interner.resolve(*p).to_string())
        .collect();
    writeln!(output, "#[inline]").unwrap();
    if let Some(ref r) = ret_str {
        if r != "()" {
            writeln!(output, "fn {}({}) -> {} {{ {}(0, {}) }}\n", a_name, wrapper_params_a.join(", "), r, merged_name, wrapper_args_a.join(", ")).unwrap();
        } else {
            writeln!(output, "fn {}({}) {{ {}(0, {}) }}\n", a_name, wrapper_params_a.join(", "), merged_name, wrapper_args_a.join(", ")).unwrap();
        }
    } else {
        writeln!(output, "fn {}({}) {{ {}(0, {}) }}\n", a_name, wrapper_params_a.join(", "), merged_name, wrapper_args_a.join(", ")).unwrap();
    }

    // Wrapper for func_b
    let wrapper_params_b: Vec<String> = b_params.iter()
        .map(|(p, t)| format!("{}: {}", interner.resolve(*p), codegen_type_expr(t, interner)))
        .collect();
    let wrapper_args_b: Vec<String> = b_params.iter()
        .map(|(p, _)| interner.resolve(*p).to_string())
        .collect();
    writeln!(output, "#[inline]").unwrap();
    if let Some(ref r) = ret_str {
        if r != "()" {
            writeln!(output, "fn {}({}) -> {} {{ {}(1, {}) }}\n", b_name, wrapper_params_b.join(", "), r, merged_name, wrapper_args_b.join(", ")).unwrap();
        } else {
            writeln!(output, "fn {}({}) {{ {}(1, {}) }}\n", b_name, wrapper_params_b.join(", "), merged_name, wrapper_args_b.join(", ")).unwrap();
        }
    } else {
        writeln!(output, "fn {}({}) {{ {}(1, {}) }}\n", b_name, wrapper_params_b.join(", "), merged_name, wrapper_args_b.join(", ")).unwrap();
    }

    output
}

fn codegen_stmt_mutual_tce<'a>(
    stmt: &Stmt<'a>,
    self_name: Symbol,
    partner_name: Symbol,
    param_names: &[Symbol],
    self_tag: u8,
    partner_tag: u8,
    interner: &Interner,
    indent: usize,
    mutable_vars: &HashSet<Symbol>,
    ctx: &mut RefinementContext<'a>,
    lww_fields: &HashSet<(String, String)>,
    mv_fields: &HashSet<(String, String)>,
    synced_vars: &mut HashSet<Symbol>,
    var_caps: &HashMap<Symbol, VariableCapabilities>,
    async_functions: &HashSet<Symbol>,
    pipe_vars: &HashSet<Symbol>,
    boxed_fields: &HashSet<(String, String, String)>,
    registry: &TypeRegistry,
) -> String {
    let indent_str = "    ".repeat(indent);

    match stmt {
        // Return with a call to partner → switch tag + continue
        Stmt::Return { value: Some(expr) } if expr_is_call_to(partner_name, expr) => {
            if let Expr::Call { args, .. } = expr {
                let mut output = String::new();
                writeln!(output, "{}{{", indent_str).unwrap();
                for (i, arg) in args.iter().enumerate() {
                    let arg_str = codegen_expr_with_async(arg, interner, synced_vars, async_functions, ctx.get_variable_types());
                    writeln!(output, "{}    let __tce_{} = {};", indent_str, i, arg_str).unwrap();
                }
                for (i, param_sym) in param_names.iter().enumerate() {
                    let param_name = interner.resolve(*param_sym);
                    writeln!(output, "{}    {} = __tce_{};", indent_str, param_name, i).unwrap();
                }
                writeln!(output, "{}    __tag = {};", indent_str, partner_tag).unwrap();
                writeln!(output, "{}    continue;", indent_str).unwrap();
                writeln!(output, "{}}}", indent_str).unwrap();
                return output;
            }
            codegen_stmt(stmt, interner, indent, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry)
        }

        // Return with a call to self → standard self-TCE
        Stmt::Return { value: Some(expr) } if expr_is_call_to(self_name, expr) => {
            if let Expr::Call { args, .. } = expr {
                let mut output = String::new();
                writeln!(output, "{}{{", indent_str).unwrap();
                for (i, arg) in args.iter().enumerate() {
                    let arg_str = codegen_expr_with_async(arg, interner, synced_vars, async_functions, ctx.get_variable_types());
                    writeln!(output, "{}    let __tce_{} = {};", indent_str, i, arg_str).unwrap();
                }
                for (i, param_sym) in param_names.iter().enumerate() {
                    let param_name = interner.resolve(*param_sym);
                    writeln!(output, "{}    {} = __tce_{};", indent_str, param_name, i).unwrap();
                }
                writeln!(output, "{}    continue;", indent_str).unwrap();
                writeln!(output, "{}}}", indent_str).unwrap();
                return output;
            }
            codegen_stmt(stmt, interner, indent, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry)
        }

        // If: recurse into branches
        Stmt::If { cond, then_block, else_block } => {
            let cond_str = codegen_expr_with_async(cond, interner, synced_vars, async_functions, ctx.get_variable_types());
            let mut output = String::new();
            writeln!(output, "{}if {} {{", indent_str, cond_str).unwrap();
            ctx.push_scope();
            for s in *then_block {
                output.push_str(&codegen_stmt_mutual_tce(s, self_name, partner_name, param_names, self_tag, partner_tag, interner, indent + 1, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry));
            }
            ctx.pop_scope();
            if let Some(else_stmts) = else_block {
                writeln!(output, "{}}} else {{", indent_str).unwrap();
                ctx.push_scope();
                for s in *else_stmts {
                    output.push_str(&codegen_stmt_mutual_tce(s, self_name, partner_name, param_names, self_tag, partner_tag, interner, indent + 1, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry));
                }
                ctx.pop_scope();
            }
            writeln!(output, "{}}}", indent_str).unwrap();
            output
        }

        // Everything else: delegate
        _ => codegen_stmt(stmt, interner, indent, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry),
    }
}

fn expr_is_call_to(target: Symbol, expr: &Expr) -> bool {
    matches!(expr, Expr::Call { function, .. } if *function == target)
}

// =============================================================================
// Tail Call Elimination (TCE) Statement Emitter
// =============================================================================

fn codegen_stmt_tce<'a>(
    stmt: &Stmt<'a>,
    func_name: Symbol,
    param_names: &[Symbol],
    interner: &Interner,
    indent: usize,
    mutable_vars: &HashSet<Symbol>,
    ctx: &mut RefinementContext<'a>,
    lww_fields: &HashSet<(String, String)>,
    mv_fields: &HashSet<(String, String)>,
    synced_vars: &mut HashSet<Symbol>,
    var_caps: &HashMap<Symbol, VariableCapabilities>,
    async_functions: &HashSet<Symbol>,
    pipe_vars: &HashSet<Symbol>,
    boxed_fields: &HashSet<(String, String, String)>,
    registry: &TypeRegistry,
) -> String {
    let indent_str = "    ".repeat(indent);

    match stmt {
        // Case 1 & 2: Return with a self-call in tail position
        Stmt::Return { value: Some(expr) } if expr_is_self_call(func_name, expr) => {
            if let Expr::Call { args, .. } = expr {
                let mut output = String::new();
                writeln!(output, "{}{{", indent_str).unwrap();
                // Evaluate all args into temporaries first (prevents ordering bugs)
                for (i, arg) in args.iter().enumerate() {
                    let arg_str = codegen_expr_with_async(arg, interner, synced_vars, async_functions, ctx.get_variable_types());
                    writeln!(output, "{}    let __tce_{} = {};", indent_str, i, arg_str).unwrap();
                }
                // Assign temporaries to params
                for (i, param_sym) in param_names.iter().enumerate() {
                    let param_name = interner.resolve(*param_sym);
                    writeln!(output, "{}    {} = __tce_{};", indent_str, param_name, i).unwrap();
                }
                writeln!(output, "{}    continue;", indent_str).unwrap();
                writeln!(output, "{}}}", indent_str).unwrap();
                return output;
            }
            // Shouldn't reach here, but fall through to default
            codegen_stmt(stmt, interner, indent, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry)
        }

        // Case 2: Return with outer self-call that has a nested self-call arg (Ackermann pattern)
        Stmt::Return { value: Some(expr) } => {
            if let Expr::Call { function, args } = expr {
                if *function == func_name {
                    let mut output = String::new();
                    writeln!(output, "{}{{", indent_str).unwrap();
                    // Evaluate args — nested self-calls remain as normal recursion,
                    // but the outer call becomes a loop iteration
                    for (i, arg) in args.iter().enumerate() {
                        if expr_is_self_call(func_name, arg) {
                            // Inner self-call: evaluate as normal recursive call
                            let arg_str = codegen_expr_with_async(arg, interner, synced_vars, async_functions, ctx.get_variable_types());
                            writeln!(output, "{}    let __tce_{} = {};", indent_str, i, arg_str).unwrap();
                        } else {
                            let arg_str = codegen_expr_with_async(arg, interner, synced_vars, async_functions, ctx.get_variable_types());
                            writeln!(output, "{}    let __tce_{} = {};", indent_str, i, arg_str).unwrap();
                        }
                    }
                    // Assign temporaries to params
                    for (i, param_sym) in param_names.iter().enumerate() {
                        let param_name = interner.resolve(*param_sym);
                        writeln!(output, "{}    {} = __tce_{};", indent_str, param_name, i).unwrap();
                    }
                    writeln!(output, "{}    continue;", indent_str).unwrap();
                    writeln!(output, "{}}}", indent_str).unwrap();
                    return output;
                }
            }
            // Not a self-call — delegate to normal codegen
            codegen_stmt(stmt, interner, indent, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry)
        }

        // Case 3: If statement — recurse into branches
        Stmt::If { cond, then_block, else_block } => {
            let cond_str = codegen_expr_with_async(cond, interner, synced_vars, async_functions, ctx.get_variable_types());
            let mut output = String::new();
            writeln!(output, "{}if {} {{", indent_str, cond_str).unwrap();
            ctx.push_scope();
            for s in *then_block {
                output.push_str(&codegen_stmt_tce(s, func_name, param_names, interner, indent + 1, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry));
            }
            ctx.pop_scope();
            if let Some(else_stmts) = else_block {
                writeln!(output, "{}}} else {{", indent_str).unwrap();
                ctx.push_scope();
                for s in *else_stmts {
                    output.push_str(&codegen_stmt_tce(s, func_name, param_names, interner, indent + 1, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry));
                }
                ctx.pop_scope();
            }
            writeln!(output, "{}}}", indent_str).unwrap();
            output
        }

        // Case 4: Everything else — delegate
        _ => codegen_stmt(stmt, interner, indent, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry),
    }
}

/// Phase 54: Collect parameters that are used as pipe senders in function body.
/// If a param appears in `SendPipe { pipe: Expr::Identifier(param) }`, it's a sender.
pub fn collect_pipe_sender_params(body: &[Stmt]) -> HashSet<Symbol> {
    let mut senders = HashSet::new();
    for stmt in body {
        collect_pipe_sender_params_stmt(stmt, &mut senders);
    }
    senders
}

fn collect_pipe_sender_params_stmt(stmt: &Stmt, senders: &mut HashSet<Symbol>) {
    match stmt {
        Stmt::SendPipe { pipe, .. } | Stmt::TrySendPipe { pipe, .. } => {
            if let Expr::Identifier(sym) = pipe {
                senders.insert(*sym);
            }
        }
        Stmt::If { then_block, else_block, .. } => {
            for s in *then_block {
                collect_pipe_sender_params_stmt(s, senders);
            }
            if let Some(else_stmts) = else_block {
                for s in *else_stmts {
                    collect_pipe_sender_params_stmt(s, senders);
                }
            }
        }
        Stmt::While { body, .. } | Stmt::Repeat { body, .. } | Stmt::Zone { body, .. } => {
            for s in *body {
                collect_pipe_sender_params_stmt(s, senders);
            }
        }
        Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => {
            for s in *tasks {
                collect_pipe_sender_params_stmt(s, senders);
            }
        }
        _ => {}
    }
}

/// Phase 54: Collect variables that are pipe declarations (created with CreatePipe).
/// These have _tx/_rx suffixes, while pipe parameters don't.
pub fn collect_pipe_vars(stmts: &[Stmt]) -> HashSet<Symbol> {
    let mut pipe_vars = HashSet::new();
    for stmt in stmts {
        collect_pipe_vars_stmt(stmt, &mut pipe_vars);
    }
    pipe_vars
}

fn collect_pipe_vars_stmt(stmt: &Stmt, pipe_vars: &mut HashSet<Symbol>) {
    match stmt {
        Stmt::CreatePipe { var, .. } => {
            pipe_vars.insert(*var);
        }
        Stmt::If { then_block, else_block, .. } => {
            for s in *then_block {
                collect_pipe_vars_stmt(s, pipe_vars);
            }
            if let Some(else_stmts) = else_block {
                for s in *else_stmts {
                    collect_pipe_vars_stmt(s, pipe_vars);
                }
            }
        }
        Stmt::While { body, .. } | Stmt::Repeat { body, .. } | Stmt::Zone { body, .. } => {
            for s in *body {
                collect_pipe_vars_stmt(s, pipe_vars);
            }
        }
        Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => {
            for s in *tasks {
                collect_pipe_vars_stmt(s, pipe_vars);
            }
        }
        _ => {}
    }
}

/// Collect all identifier symbols from an expression recursively.
/// Used by Concurrent/Parallel codegen to find variables that need cloning.
fn collect_expr_identifiers(expr: &Expr, identifiers: &mut HashSet<Symbol>) {
    match expr {
        Expr::Identifier(sym) => {
            identifiers.insert(*sym);
        }
        Expr::BinaryOp { left, right, .. } => {
            collect_expr_identifiers(left, identifiers);
            collect_expr_identifiers(right, identifiers);
        }
        Expr::Call { args, .. } => {
            for arg in args {
                collect_expr_identifiers(arg, identifiers);
            }
        }
        Expr::Index { collection, index } => {
            collect_expr_identifiers(collection, identifiers);
            collect_expr_identifiers(index, identifiers);
        }
        Expr::Slice { collection, start, end } => {
            collect_expr_identifiers(collection, identifiers);
            collect_expr_identifiers(start, identifiers);
            collect_expr_identifiers(end, identifiers);
        }
        Expr::Copy { expr: inner } | Expr::Give { value: inner } | Expr::Length { collection: inner } => {
            collect_expr_identifiers(inner, identifiers);
        }
        Expr::Contains { collection, value } | Expr::Union { left: collection, right: value } | Expr::Intersection { left: collection, right: value } => {
            collect_expr_identifiers(collection, identifiers);
            collect_expr_identifiers(value, identifiers);
        }
        Expr::ManifestOf { zone } | Expr::ChunkAt { zone, .. } => {
            collect_expr_identifiers(zone, identifiers);
        }
        Expr::List(items) | Expr::Tuple(items) => {
            for item in items {
                collect_expr_identifiers(item, identifiers);
            }
        }
        Expr::Range { start, end } => {
            collect_expr_identifiers(start, identifiers);
            collect_expr_identifiers(end, identifiers);
        }
        Expr::FieldAccess { object, .. } => {
            collect_expr_identifiers(object, identifiers);
        }
        Expr::New { init_fields, .. } => {
            for (_, value) in init_fields {
                collect_expr_identifiers(value, identifiers);
            }
        }
        Expr::NewVariant { fields, .. } => {
            for (_, value) in fields {
                collect_expr_identifiers(value, identifiers);
            }
        }
        Expr::OptionSome { value } => {
            collect_expr_identifiers(value, identifiers);
        }
        Expr::WithCapacity { value, capacity } => {
            collect_expr_identifiers(value, identifiers);
            collect_expr_identifiers(capacity, identifiers);
        }
        Expr::Closure { body, .. } => {
            match body {
                crate::ast::stmt::ClosureBody::Expression(expr) => collect_expr_identifiers(expr, identifiers),
                crate::ast::stmt::ClosureBody::Block(_) => {}
            }
        }
        Expr::CallExpr { callee, args } => {
            collect_expr_identifiers(callee, identifiers);
            for arg in args {
                collect_expr_identifiers(arg, identifiers);
            }
        }
        Expr::OptionNone => {}
        Expr::Escape { .. } => {}
        Expr::Literal(_) => {}
    }
}

/// Collect identifiers from a statement's expressions (for Concurrent/Parallel variable capture).
fn collect_stmt_identifiers(stmt: &Stmt, identifiers: &mut HashSet<Symbol>) {
    match stmt {
        Stmt::Let { value, .. } => {
            collect_expr_identifiers(value, identifiers);
        }
        Stmt::Call { args, .. } => {
            for arg in args {
                collect_expr_identifiers(arg, identifiers);
            }
        }
        _ => {}
    }
}

/// Generate a complete Rust program from LOGOS statements.
///
/// This is the main entry point for code generation. It produces a complete,
/// compilable Rust program including:
///
/// 1. Prelude imports (`logicaffeine_data`, `logicaffeine_system`)
/// 2. User-defined types in `mod user_types`
/// 3. Policy impl blocks with predicate/capability methods
/// 4. Function definitions
/// 5. `fn main()` with the program logic
///
/// # Arguments
///
/// * `stmts` - The parsed LOGOS statements
/// * `registry` - Type registry containing struct/enum definitions
/// * `policies` - Policy registry containing security predicates and capabilities
/// * `interner` - Symbol interner for name resolution
///
/// # Returns
///
/// A complete Rust source code string ready to compile.
///
/// # Generated Features
///
/// - Wraps user types in `mod user_types` for visibility control
/// - Emits function definitions before main
/// - Handles CRDT field mutations with proper `.set()` calls
/// - Generates policy predicate and capability methods
/// - Adds `#[tokio::main]` async when needed
/// - Injects VFS when file operations detected
/// - Uses `Distributed<T>` when both Mount and Sync detected
/// - Boxes recursive enum fields
/// Generates a complete Rust program from LOGOS statements.
///
/// This is the main entry point for code generation. It produces a full Rust
/// program including:
/// - Prelude imports (`use logicaffeine_data::*;`)
/// - Type definitions (structs, enums, inductive types)
/// - Policy structs with capability methods
/// - Main function with async runtime if needed
/// - VFS initialization for file operations
///
/// # Arguments
///
/// * `stmts` - The parsed LOGOS statements to compile
/// * `registry` - Type definitions discovered during parsing
/// * `policies` - Policy definitions for access control
/// * `interner` - Symbol interner for resolving names
///
/// # Returns
///
/// A complete Rust source code string ready for compilation.
pub fn codegen_program(stmts: &[Stmt], registry: &TypeRegistry, policies: &PolicyRegistry, interner: &Interner) -> String {
    let mut output = String::new();

    // Prelude
    // Use extracted crates instead of logos_core
    writeln!(output, "#[allow(unused_imports)]").unwrap();
    writeln!(output, "use std::fmt::Write as _;").unwrap();
    writeln!(output, "use logicaffeine_data::*;").unwrap();
    writeln!(output, "use logicaffeine_system::*;\n").unwrap();

    // FFI: Emit wasm_bindgen preamble if any function is exported for WASM
    if has_wasm_exports(stmts, interner) {
        writeln!(output, "use wasm_bindgen::prelude::*;\n").unwrap();
    }

    // FFI: Emit CStr/CString imports if any C export uses Text types
    if has_c_exports_with_text(stmts, interner) {
        writeln!(output, "use std::ffi::{{CStr, CString}};\n").unwrap();
    }

    // Universal ABI: Emit LogosStatus runtime preamble if any C exports exist
    let c_exports_exist = has_c_exports(stmts, interner);
    if c_exports_exist {
        output.push_str(&codegen_logos_runtime_preamble());
    }

    // Phase 49: Collect CRDT register fields for special SetField handling
    // LWW fields need timestamp, MV fields don't
    let (lww_fields, mv_fields) = collect_crdt_register_fields(registry, interner);

    // Phase 54: Collect async functions for Launch codegen
    let async_functions = collect_async_functions(stmts);

    // Purity analysis for memoization
    let pure_functions = collect_pure_functions(stmts);

    // Phase 54: Collect pipe declarations (variables with _tx/_rx suffixes)
    let main_pipe_vars = collect_pipe_vars(stmts);

    // Phase 102: Collect boxed fields for recursive enum handling
    let boxed_fields = collect_boxed_fields(registry, interner);

    // Collect value-type struct names used in C exports (need #[repr(C)])
    let c_abi_value_structs: HashSet<Symbol> = if c_exports_exist {
        collect_c_export_value_type_structs(stmts, interner, registry)
    } else {
        HashSet::new()
    };

    // Collect reference-type struct names used in C exports (need serde derives for from_json/to_json)
    let c_abi_ref_structs: HashSet<Symbol> = if c_exports_exist {
        collect_c_export_ref_structs(stmts, interner, registry)
    } else {
        HashSet::new()
    };

    // Collect user-defined structs from registry (Phase 34: generics, Phase 47: is_portable, Phase 49: is_shared)
    let structs: Vec<_> = registry.iter_types()
        .filter_map(|(name, def)| {
            if let TypeDef::Struct { fields, generics, is_portable, is_shared } = def {
                if !fields.is_empty() || !generics.is_empty() {
                    Some((*name, fields.clone(), generics.clone(), *is_portable, *is_shared))
                } else {
                    None
                }
            } else {
                None
            }
        })
        .collect();

    // Phase 33/34: Collect user-defined enums from registry (generics, Phase 47: is_portable, Phase 49: is_shared)
    let enums: Vec<_> = registry.iter_types()
        .filter_map(|(name, def)| {
            if let TypeDef::Enum { variants, generics, is_portable, is_shared } = def {
                if !variants.is_empty() || !generics.is_empty() {
                    Some((*name, variants.clone(), generics.clone(), *is_portable, *is_shared))
                } else {
                    None
                }
            } else {
                None
            }
        })
        .collect();

    // Emit struct and enum definitions in user_types module if any exist
    if !structs.is_empty() || !enums.is_empty() {
        writeln!(output, "pub mod user_types {{").unwrap();
        writeln!(output, "    use super::*;\n").unwrap();

        for (name, fields, generics, is_portable, is_shared) in &structs {
            output.push_str(&codegen_struct_def(*name, fields, generics, *is_portable, *is_shared, interner, 4, &c_abi_value_structs, &c_abi_ref_structs));
        }

        for (name, variants, generics, is_portable, is_shared) in &enums {
            output.push_str(&codegen_enum_def(*name, variants, generics, *is_portable, *is_shared, interner, 4));
        }

        writeln!(output, "}}\n").unwrap();
        writeln!(output, "use user_types::*;\n").unwrap();
    }

    // Phase 50: Generate policy impl blocks with predicate and capability methods
    output.push_str(&codegen_policy_impls(policies, interner));

    // Mutual TCO: Detect pairs of mutually tail-calling functions
    let mutual_tce_pairs = detect_mutual_tce_pairs(stmts, interner);
    let mut mutual_tce_members: HashSet<Symbol> = HashSet::new();
    for (a, b) in &mutual_tce_pairs {
        mutual_tce_members.insert(*a);
        mutual_tce_members.insert(*b);
    }
    let mut mutual_tce_emitted: HashSet<Symbol> = HashSet::new();

    // Phase 32/38: Emit function definitions before main
    for stmt in stmts {
        if let Stmt::FunctionDef { name, params, body, return_type, is_native, native_path, is_exported, export_target } = stmt {
            if mutual_tce_members.contains(name) {
                // Part of a mutual pair — emit merged function when we see the first member
                if !mutual_tce_emitted.contains(name) {
                    // Find the pair this function belongs to
                    if let Some((a, b)) = mutual_tce_pairs.iter().find(|(a, b)| *a == *name || *b == *name) {
                        output.push_str(&codegen_mutual_tce_pair(*a, *b, stmts, interner, &lww_fields, &mv_fields, &async_functions, &boxed_fields, registry));
                        mutual_tce_emitted.insert(*a);
                        mutual_tce_emitted.insert(*b);
                    }
                }
                // Skip individual emission — already emitted as part of merged pair
            } else {
                output.push_str(&codegen_function_def(*name, params, body, return_type.as_ref().copied(), *is_native, *native_path, *is_exported, *export_target, interner, &lww_fields, &mv_fields, &async_functions, &boxed_fields, registry, &pure_functions));
            }
        }
    }

    // Universal ABI: Emit accessor/free functions for reference types in C exports
    if c_exports_exist {
        let ref_types = collect_c_export_reference_types(stmts, interner, registry);
        for ref_ty in &ref_types {
            output.push_str(&codegen_c_accessors(ref_ty, interner, registry));
        }
    }

    // Grand Challenge: Collect variables that need to be mutable
    let main_stmts: Vec<&Stmt> = stmts.iter()
        .filter(|s| !matches!(s, Stmt::FunctionDef { .. }))
        .collect();
    let mut main_mutable_vars = HashSet::new();
    for stmt in &main_stmts {
        collect_mutable_vars_stmt(stmt, &mut main_mutable_vars);
    }

    // Main function
    // Phase 51: Use async main when async operations are present
    if requires_async(stmts) {
        writeln!(output, "#[tokio::main]").unwrap();
        writeln!(output, "async fn main() {{").unwrap();
    } else {
        writeln!(output, "fn main() {{").unwrap();
    }
    // Phase 53: Inject VFS when file operations or persistence is used
    if requires_vfs(stmts) {
        writeln!(output, "    let vfs = logicaffeine_system::fs::NativeVfs::new(\".\");").unwrap();
    }
    let mut main_ctx = RefinementContext::new();
    let mut main_synced_vars = HashSet::new();  // Phase 52: Track synced variables in main
    // Phase 56: Pre-scan for Mount+Sync combinations
    let main_var_caps = analyze_variable_capabilities(stmts, interner);
    {
        let stmt_refs: Vec<&Stmt> = stmts.iter().collect();
        let mut i = 0;
        while i < stmt_refs.len() {
            // Skip function definitions - they're already emitted above
            if matches!(stmt_refs[i], Stmt::FunctionDef { .. }) {
                i += 1;
                continue;
            }
            // Peephole: Vec fill pattern optimization (most specific — check first)
            if let Some((code, skip)) = try_emit_vec_fill_pattern(&stmt_refs, i, interner, 1) {
                output.push_str(&code);
                i += 1 + skip;
                continue;
            }
            // Peephole: For-range loop optimization
            if let Some((code, skip)) = try_emit_for_range_pattern(&stmt_refs, i, interner, 1, &main_mutable_vars, &mut main_ctx, &lww_fields, &mv_fields, &mut main_synced_vars, &main_var_caps, &async_functions, &main_pipe_vars, &boxed_fields, registry) {
                output.push_str(&code);
                i += 1 + skip;
                continue;
            }
            // Peephole: swap pattern optimization
            if let Some((code, skip)) = try_emit_swap_pattern(&stmt_refs, i, interner, 1, main_ctx.get_variable_types()) {
                output.push_str(&code);
                i += 1 + skip;
                continue;
            }
            output.push_str(&codegen_stmt(stmt_refs[i], interner, 1, &main_mutable_vars, &mut main_ctx, &lww_fields, &mv_fields, &mut main_synced_vars, &main_var_caps, &async_functions, &main_pipe_vars, &boxed_fields, registry));
            i += 1;
        }
    }
    writeln!(output, "}}").unwrap();
    output
}

/// Phase 32/38: Generate a function definition.
/// Phase 38: Updated for native functions and TypeExpr types.
/// Phase 49: Accepts lww_fields for LWWRegister SetField handling.
/// Phase 103: Accepts registry for polymorphic enum type inference.
fn codegen_function_def(
    name: Symbol,
    params: &[(Symbol, &TypeExpr)],
    body: &[Stmt],
    return_type: Option<&TypeExpr>,
    is_native: bool,
    native_path: Option<Symbol>,
    is_exported: bool,
    export_target: Option<Symbol>,
    interner: &Interner,
    lww_fields: &HashSet<(String, String)>,
    mv_fields: &HashSet<(String, String)>,  // Phase 49b: MVRegister fields
    async_functions: &HashSet<Symbol>,  // Phase 54
    boxed_fields: &HashSet<(String, String, String)>,  // Phase 102
    registry: &TypeRegistry,  // Phase 103
    pure_functions: &HashSet<Symbol>,
) -> String {
    let mut output = String::new();
    let raw_name = interner.resolve(name);
    let func_name = escape_rust_ident(raw_name);
    let export_target_lower = export_target.map(|s| interner.resolve(s).to_lowercase());

    // Phase 54: Detect which parameters are used as pipe senders
    let pipe_sender_params = collect_pipe_sender_params(body);

    // FFI: Exported functions need special signatures
    let is_c_export_early = is_exported && matches!(export_target_lower.as_deref(), None | Some("c"));

    // TCE: Detect tail recursion eligibility
    let is_tce = !is_native && !is_c_export_early && is_tail_recursive(name, body);
    let param_syms: Vec<Symbol> = params.iter().map(|(s, _)| *s).collect();

    // Accumulator Introduction: Detect non-tail single-call + / * patterns
    let acc_info = if !is_tce && !is_native && !is_c_export_early {
        detect_accumulator_pattern(name, body)
    } else {
        None
    };
    let is_acc = acc_info.is_some();

    // Memoization: Detect pure multi-call recursive functions with hashable params
    let is_memo = !is_tce && !is_acc && !is_native && !is_c_export_early
        && should_memoize(name, body, params, return_type, pure_functions.contains(&name), interner);

    let needs_mut_params = is_tce || is_acc;

    // Build parameter list using TypeExpr
    let params_str: Vec<String> = params.iter()
        .map(|(param_name, param_type)| {
            let pname = interner.resolve(*param_name);
            let ty = codegen_type_expr(param_type, interner);
            // Phase 54: If param is used as a pipe sender, wrap type in Sender<T>
            if pipe_sender_params.contains(param_name) {
                format!("{}: tokio::sync::mpsc::Sender<{}>", pname, ty)
            } else if needs_mut_params {
                format!("mut {}: {}", pname, ty)
            } else {
                format!("{}: {}", pname, ty)
            }
        })
        .collect();

    // Get return type string from TypeExpr or infer from body
    let return_type_str = return_type
        .map(|t| codegen_type_expr(t, interner))
        .or_else(|| infer_return_type_from_body(body, interner));

    // Phase 51/54: Check if function is async (includes transitive async detection)
    let is_async = async_functions.contains(&name);
    let fn_keyword = if is_async { "async fn" } else { "fn" };

    // FFI: Exported functions need special signatures
    let is_c_export = is_c_export_early;

    // FFI: Check if C export needs type marshaling
    // Triggers for: Text params/return, reference types, Result return, refinement params
    let needs_c_marshaling = is_c_export && {
        let has_text_param = params.iter().any(|(_, ty)| is_text_type(ty, interner));
        let has_text_return = return_type.map_or(false, |ty| is_text_type(ty, interner));
        let has_ref_param = params.iter().any(|(_, ty)| {
            classify_type_for_c_abi(ty, interner, registry) == CAbiClass::ReferenceType
        });
        let has_ref_return = return_type.map_or(false, |ty| {
            classify_type_for_c_abi(ty, interner, registry) == CAbiClass::ReferenceType
        });
        let has_result_return = return_type.map_or(false, |ty| is_result_type(ty, interner));
        let has_refinement_param = params.iter().any(|(_, ty)| {
            matches!(ty, TypeExpr::Refinement { .. })
        });
        has_text_param || has_text_return || has_ref_param || has_ref_return
            || has_result_return || has_refinement_param
    };

    if needs_c_marshaling {
        // Generate two-function pattern: inner function + C ABI wrapper
        return codegen_c_export_with_marshaling(
            name, params, body, return_type, interner,
            lww_fields, mv_fields, async_functions, boxed_fields, registry,
        );
    }

    // Build function signature
    let (vis_prefix, abi_prefix) = if is_exported {
        match export_target_lower.as_deref() {
            None | Some("c") => ("pub ", "extern \"C\" "),
            Some("wasm") => ("pub ", ""),
            _ => ("pub ", ""),
        }
    } else {
        ("", "")
    };

    let signature = if let Some(ref ret_ty) = return_type_str {
        if ret_ty != "()" {
            format!("{}{}{} {}({}) -> {}", vis_prefix, abi_prefix, fn_keyword, func_name, params_str.join(", "), ret_ty)
        } else {
            format!("{}{}{} {}({})", vis_prefix, abi_prefix, fn_keyword, func_name, params_str.join(", "))
        }
    } else {
        format!("{}{}{} {}({})", vis_prefix, abi_prefix, fn_keyword, func_name, params_str.join(", "))
    };

    // Emit #[inline] for small non-recursive, non-exported functions
    if !is_tce && !is_acc && should_inline(name, body, is_native, is_exported, is_async) {
        writeln!(output, "#[inline]").unwrap();
    }

    // FFI: Emit export attributes before the function
    if is_exported {
        match export_target_lower.as_deref() {
            None | Some("c") => {
                writeln!(output, "#[export_name = \"logos_{}\"]", raw_name).unwrap();
            }
            Some("wasm") => {
                writeln!(output, "#[wasm_bindgen]").unwrap();
            }
            _ => {}
        }
    }

    // Phase 38: Handle native functions
    if is_native {
        let arg_names: Vec<&str> = params.iter()
            .map(|(n, _)| interner.resolve(*n))
            .collect();

        if let Some(path_sym) = native_path {
            // User-defined native path: call the Rust path directly
            let path = interner.resolve(path_sym);
            // Validate path looks like a valid Rust path (identifiers separated by ::)
            let is_valid_path = !path.is_empty() && path.split("::").all(|seg| {
                !seg.is_empty() && seg.chars().all(|c| c.is_alphanumeric() || c == '_')
            });
            if is_valid_path {
                writeln!(output, "{} {{", signature).unwrap();
                writeln!(output, "    {}({})", path, arg_names.join(", ")).unwrap();
                writeln!(output, "}}\n").unwrap();
            } else {
                writeln!(output, "{} {{", signature).unwrap();
                writeln!(output, "    compile_error!(\"Invalid native function path: '{}'. Path must be a valid Rust path like \\\"crate::module::function\\\".\")", path).unwrap();
                writeln!(output, "}}\n").unwrap();
            }
        } else {
            // Legacy system functions: use map_native_function()
            if let Some((module, core_fn)) = map_native_function(raw_name) {
                writeln!(output, "{} {{", signature).unwrap();
                writeln!(output, "    logicaffeine_system::{}::{}({})", module, core_fn, arg_names.join(", ")).unwrap();
                writeln!(output, "}}\n").unwrap();
            } else {
                writeln!(output, "{} {{", signature).unwrap();
                writeln!(output, "    compile_error!(\"Unknown system native function: '{}'. Use `is native \\\"crate::path\\\"` syntax for user-defined native functions.\")", raw_name).unwrap();
                writeln!(output, "}}\n").unwrap();
            }
        }
    } else {
        // Non-native: emit body (also used for exported functions which have bodies)
        // Grand Challenge: Collect mutable vars for this function
        let func_mutable_vars = collect_mutable_vars(body);
        writeln!(output, "{} {{", signature).unwrap();

        // Wrap exported C functions in catch_unwind for panic safety
        let wrap_catch_unwind = is_c_export;
        if wrap_catch_unwind {
            writeln!(output, "    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {{").unwrap();
        }

        let mut func_ctx = RefinementContext::new();
        let mut func_synced_vars = HashSet::new();  // Phase 52: Track synced variables in function
        // Phase 56: Pre-scan for Mount+Sync combinations in function body
        let func_var_caps = analyze_variable_capabilities(body, interner);

        // Phase 50: Register parameter types for capability Check resolution
        for (param_name, param_type) in params {
            let type_name = codegen_type_expr(param_type, interner);
            func_ctx.register_variable_type(*param_name, type_name);
        }

        // Phase 54: Functions receive pipe senders as parameters, no local pipe declarations
        let func_pipe_vars = HashSet::new();

        if is_tce {
            // TCE: Wrap body in loop, use TCE-aware statement emitter
            writeln!(output, "    loop {{").unwrap();
            let stmt_refs: Vec<&Stmt> = body.iter().collect();
            let mut si = 0;
            while si < stmt_refs.len() {
                if let Some((code, skip)) = try_emit_vec_fill_pattern(&stmt_refs, si, interner, 2) {
                    output.push_str(&code);
                    si += 1 + skip;
                    continue;
                }
                if let Some((code, skip)) = try_emit_for_range_pattern(&stmt_refs, si, interner, 2, &func_mutable_vars, &mut func_ctx, lww_fields, mv_fields, &mut func_synced_vars, &func_var_caps, async_functions, &func_pipe_vars, boxed_fields, registry) {
                    output.push_str(&code);
                    si += 1 + skip;
                    continue;
                }
                if let Some((code, skip)) = try_emit_swap_pattern(&stmt_refs, si, interner, 2, func_ctx.get_variable_types()) {
                    output.push_str(&code);
                    si += 1 + skip;
                    continue;
                }
                output.push_str(&codegen_stmt_tce(stmt_refs[si], name, &param_syms, interner, 2, &func_mutable_vars, &mut func_ctx, lww_fields, mv_fields, &mut func_synced_vars, &func_var_caps, async_functions, &func_pipe_vars, boxed_fields, registry));
                si += 1;
            }
            writeln!(output, "    }}").unwrap();
        } else if let Some(ref acc) = acc_info {
            // Accumulator Introduction: Wrap body in loop with accumulator variable
            writeln!(output, "    let mut __acc: i64 = {};", acc.identity).unwrap();
            writeln!(output, "    loop {{").unwrap();
            let stmt_refs: Vec<&Stmt> = body.iter().collect();
            let mut si = 0;
            while si < stmt_refs.len() {
                if let Some((code, skip)) = try_emit_vec_fill_pattern(&stmt_refs, si, interner, 2) {
                    output.push_str(&code);
                    si += 1 + skip;
                    continue;
                }
                if let Some((code, skip)) = try_emit_for_range_pattern(&stmt_refs, si, interner, 2, &func_mutable_vars, &mut func_ctx, lww_fields, mv_fields, &mut func_synced_vars, &func_var_caps, async_functions, &func_pipe_vars, boxed_fields, registry) {
                    output.push_str(&code);
                    si += 1 + skip;
                    continue;
                }
                if let Some((code, skip)) = try_emit_swap_pattern(&stmt_refs, si, interner, 2, func_ctx.get_variable_types()) {
                    output.push_str(&code);
                    si += 1 + skip;
                    continue;
                }
                output.push_str(&codegen_stmt_acc(stmt_refs[si], name, &param_syms, acc, interner, 2, &func_mutable_vars, &mut func_ctx, lww_fields, mv_fields, &mut func_synced_vars, &func_var_caps, async_functions, &func_pipe_vars, boxed_fields, registry));
                si += 1;
            }
            writeln!(output, "    }}").unwrap();
        } else if is_memo {
            // Memoization: Wrap body in closure with thread-local cache
            let ret_ty = return_type_str.as_deref().unwrap_or("i64");
            let memo_name = format!("__MEMO_{}", func_name.to_uppercase());

            // Build key type and key expression
            let (key_type, key_expr, copy_method) = if params.len() == 1 {
                let ty = codegen_type_expr(params[0].1, interner);
                let pname = interner.resolve(params[0].0).to_string();
                let copy = if is_copy_type_expr(params[0].1, interner) { "copied" } else { "cloned" };
                (ty, pname, copy)
            } else {
                let types: Vec<String> = params.iter().map(|(_, t)| codegen_type_expr(t, interner)).collect();
                let names: Vec<String> = params.iter().map(|(n, _)| interner.resolve(*n).to_string()).collect();
                let copy = if params.iter().all(|(_, t)| is_copy_type_expr(t, interner)) { "copied" } else { "cloned" };
                (format!("({})", types.join(", ")), format!("({})", names.join(", ")), copy)
            };

            writeln!(output, "    use std::cell::RefCell;").unwrap();
            writeln!(output, "    use std::collections::HashMap;").unwrap();
            writeln!(output, "    thread_local! {{").unwrap();
            writeln!(output, "        static {}: RefCell<HashMap<{}, {}>> = RefCell::new(HashMap::new());", memo_name, key_type, ret_ty).unwrap();
            writeln!(output, "    }}").unwrap();
            writeln!(output, "    if let Some(__v) = {}.with(|c| c.borrow().get(&{}).{}()) {{", memo_name, key_expr, copy_method).unwrap();
            writeln!(output, "        return __v;").unwrap();
            writeln!(output, "    }}").unwrap();
            writeln!(output, "    let __memo_result = (|| -> {} {{", ret_ty).unwrap();
            let stmt_refs: Vec<&Stmt> = body.iter().collect();
            let mut si = 0;
            while si < stmt_refs.len() {
                if let Some((code, skip)) = try_emit_vec_fill_pattern(&stmt_refs, si, interner, 2) {
                    output.push_str(&code);
                    si += 1 + skip;
                    continue;
                }
                if let Some((code, skip)) = try_emit_for_range_pattern(&stmt_refs, si, interner, 2, &func_mutable_vars, &mut func_ctx, lww_fields, mv_fields, &mut func_synced_vars, &func_var_caps, async_functions, &func_pipe_vars, boxed_fields, registry) {
                    output.push_str(&code);
                    si += 1 + skip;
                    continue;
                }
                if let Some((code, skip)) = try_emit_swap_pattern(&stmt_refs, si, interner, 2, func_ctx.get_variable_types()) {
                    output.push_str(&code);
                    si += 1 + skip;
                    continue;
                }
                output.push_str(&codegen_stmt(stmt_refs[si], interner, 2, &func_mutable_vars, &mut func_ctx, lww_fields, mv_fields, &mut func_synced_vars, &func_var_caps, async_functions, &func_pipe_vars, boxed_fields, registry));
                si += 1;
            }
            writeln!(output, "    }})();").unwrap();
            writeln!(output, "    {}.with(|c| c.borrow_mut().insert({}, __memo_result));", memo_name, key_expr).unwrap();
            writeln!(output, "    __memo_result").unwrap();
        } else {
            let stmt_refs: Vec<&Stmt> = body.iter().collect();
            let mut si = 0;
            while si < stmt_refs.len() {
                if let Some((code, skip)) = try_emit_vec_fill_pattern(&stmt_refs, si, interner, 1) {
                    output.push_str(&code);
                    si += 1 + skip;
                    continue;
                }
                if let Some((code, skip)) = try_emit_for_range_pattern(&stmt_refs, si, interner, 1, &func_mutable_vars, &mut func_ctx, lww_fields, mv_fields, &mut func_synced_vars, &func_var_caps, async_functions, &func_pipe_vars, boxed_fields, registry) {
                    output.push_str(&code);
                    si += 1 + skip;
                    continue;
                }
                if let Some((code, skip)) = try_emit_swap_pattern(&stmt_refs, si, interner, 1, func_ctx.get_variable_types()) {
                    output.push_str(&code);
                    si += 1 + skip;
                    continue;
                }
                output.push_str(&codegen_stmt(stmt_refs[si], interner, 1, &func_mutable_vars, &mut func_ctx, lww_fields, mv_fields, &mut func_synced_vars, &func_var_caps, async_functions, &func_pipe_vars, boxed_fields, registry));
                si += 1;
            }
        }

        if wrap_catch_unwind {
            writeln!(output, "    }})) {{").unwrap();
            writeln!(output, "        Ok(__v) => __v,").unwrap();
            writeln!(output, "        Err(__panic) => {{").unwrap();
            writeln!(output, "            let __msg = if let Some(s) = __panic.downcast_ref::<String>() {{ s.clone() }} else if let Some(s) = __panic.downcast_ref::<&str>() {{ s.to_string() }} else {{ \"Unknown panic\".to_string() }};").unwrap();
            writeln!(output, "            logos_set_last_error(__msg);").unwrap();
            // Determine default for panic case based on return type
            if let Some(ref ret_str) = return_type_str {
                if ret_str != "()" {
                    writeln!(output, "            Default::default()").unwrap();
                }
            }
            writeln!(output, "        }}").unwrap();
            writeln!(output, "    }}").unwrap();
        }

        writeln!(output, "}}\n").unwrap();
    }

    output
}

/// Phase 38: Map native function names to logicaffeine_system module paths.
/// For system functions only — user-defined native paths bypass this entirely.
/// Returns None for unknown functions (caller emits compile_error!).
fn map_native_function(name: &str) -> Option<(&'static str, &'static str)> {
    match name {
        "read" => Some(("file", "read")),
        "write" => Some(("file", "write")),
        "now" => Some(("time", "now")),
        "sleep" => Some(("time", "sleep")),
        "randomInt" => Some(("random", "randomInt")),
        "randomFloat" => Some(("random", "randomFloat")),
        "get" => Some(("env", "get")),
        "args" => Some(("env", "args")),
        "parseInt" => Some(("text", "parseInt")),
        "parseFloat" => Some(("text", "parseFloat")),
        "format" => Some(("fmt", "format")),
        _ => None,
    }
}

/// FFI: Check if a TypeExpr resolves to Text/String.
fn is_text_type(ty: &TypeExpr, interner: &Interner) -> bool {
    match ty {
        TypeExpr::Primitive(sym) | TypeExpr::Named(sym) => {
            matches!(interner.resolve(*sym), "Text" | "String")
        }
        TypeExpr::Refinement { base, .. } => is_text_type(base, interner),
        _ => false,
    }
}

/// FFI: Map a TypeExpr to its C ABI representation.
/// Primitives pass through; Text becomes raw pointer.
fn map_type_to_c_abi(ty: &TypeExpr, interner: &Interner, is_return: bool) -> String {
    if is_text_type(ty, interner) {
        if is_return {
            "*mut std::os::raw::c_char".to_string()
        } else {
            "*const std::os::raw::c_char".to_string()
        }
    } else {
        codegen_type_expr(ty, interner)
    }
}

/// FFI: Generate a C-exported function with Universal ABI marshaling.
///
/// Produces: 1) an inner function with normal Rust types, 2) a #[no_mangle] extern "C" wrapper.
///
/// The wrapper handles:
/// - Text param/return marshaling (*const c_char ↔ String)
/// - Reference type params/returns via opaque LogosHandle
/// - Result<T, E> returns via status code + out-parameter
/// - Refinement type boundary guards
fn codegen_c_export_with_marshaling(
    name: Symbol,
    params: &[(Symbol, &TypeExpr)],
    body: &[Stmt],
    return_type: Option<&TypeExpr>,
    interner: &Interner,
    lww_fields: &HashSet<(String, String)>,
    mv_fields: &HashSet<(String, String)>,
    async_functions: &HashSet<Symbol>,
    boxed_fields: &HashSet<(String, String, String)>,
    registry: &crate::analysis::registry::TypeRegistry,
) -> String {
    let mut output = String::new();
    let raw_name = interner.resolve(name);
    // All exported C ABI symbols use the `logos_` prefix to avoid keyword
    // collisions in target languages (C, Python, JS, etc.) and to provide
    // a consistent namespace for the generated library.
    let func_name = format!("logos_{}", raw_name);
    let inner_name = escape_rust_ident(raw_name);

    // Classify return type
    let has_ref_return = return_type.map_or(false, |ty| {
        classify_type_for_c_abi(ty, interner, registry) == CAbiClass::ReferenceType
    });
    let has_result_return = return_type.map_or(false, |ty| is_result_type(ty, interner));
    let has_text_return = return_type.map_or(false, |t| is_text_type(t, interner));

    // Determine if we need status-code return pattern
    // Status code is needed when the return value requires an out-parameter (ref/text/result)
    // or when refinement parameters need validation error paths.
    // Ref-type parameters do NOT force status code — catch_unwind handles invalid handle panics.
    let uses_status_code = has_ref_return || has_result_return || has_text_return
        || params.iter().any(|(_, ty)| matches!(ty, TypeExpr::Refinement { .. }));

    // 1) Emit the inner function with normal Rust types
    let inner_params: Vec<String> = params.iter()
        .map(|(pname, ptype)| {
            format!("{}: {}", interner.resolve(*pname), codegen_type_expr(ptype, interner))
        })
        .collect();
    let inner_ret = return_type.map(|t| codegen_type_expr(t, interner));

    let inner_sig = if let Some(ref ret) = inner_ret {
        if ret != "()" {
            format!("fn {}({}) -> {}", inner_name, inner_params.join(", "), ret)
        } else {
            format!("fn {}({})", inner_name, inner_params.join(", "))
        }
    } else {
        format!("fn {}({})", inner_name, inner_params.join(", "))
    };

    writeln!(output, "{} {{", inner_sig).unwrap();
    let func_mutable_vars = collect_mutable_vars(body);
    let mut func_ctx = RefinementContext::new();
    let mut func_synced_vars = HashSet::new();
    let func_var_caps = analyze_variable_capabilities(body, interner);
    for (param_name, param_type) in params {
        let type_name = codegen_type_expr(param_type, interner);
        func_ctx.register_variable_type(*param_name, type_name);
    }
    let func_pipe_vars = HashSet::new();
    {
        let stmt_refs: Vec<&Stmt> = body.iter().collect();
        let mut si = 0;
        while si < stmt_refs.len() {
            if let Some((code, skip)) = try_emit_vec_fill_pattern(&stmt_refs, si, interner, 1) {
                output.push_str(&code);
                si += 1 + skip;
                continue;
            }
            if let Some((code, skip)) = try_emit_for_range_pattern(&stmt_refs, si, interner, 1, &func_mutable_vars, &mut func_ctx, lww_fields, mv_fields, &mut func_synced_vars, &func_var_caps, async_functions, &func_pipe_vars, boxed_fields, registry) {
                output.push_str(&code);
                si += 1 + skip;
                continue;
            }
            if let Some((code, skip)) = try_emit_swap_pattern(&stmt_refs, si, interner, 1, func_ctx.get_variable_types()) {
                output.push_str(&code);
                si += 1 + skip;
                continue;
            }
            output.push_str(&codegen_stmt(stmt_refs[si], interner, 1, &func_mutable_vars, &mut func_ctx, lww_fields, mv_fields, &mut func_synced_vars, &func_var_caps, async_functions, &func_pipe_vars, boxed_fields, registry));
            si += 1;
        }
    }
    writeln!(output, "}}\n").unwrap();

    // 2) Build the C ABI wrapper parameters
    let mut c_params: Vec<String> = Vec::new();

    for (pname, ptype) in params.iter() {
        let pn = interner.resolve(*pname);
        if classify_type_for_c_abi(ptype, interner, registry) == CAbiClass::ReferenceType {
            c_params.push(format!("{}: LogosHandle", pn));
        } else if is_text_type(ptype, interner) {
            c_params.push(format!("{}: *const std::os::raw::c_char", pn));
        } else {
            c_params.push(format!("{}: {}", pn, codegen_type_expr(ptype, interner)));
        }
    }

    // Add out-parameter if using status-code pattern with return value
    if uses_status_code {
        if let Some(ret_ty) = return_type {
            if has_result_return {
                // Result<T, E>: out param for the Ok(T) type
                if let TypeExpr::Generic { params: ref rparams, .. } = ret_ty {
                    if !rparams.is_empty() {
                        let ok_ty = &rparams[0];
                        if classify_type_for_c_abi(ok_ty, interner, registry) == CAbiClass::ReferenceType {
                            c_params.push("out: *mut LogosHandle".to_string());
                        } else if is_text_type(ok_ty, interner) {
                            c_params.push("out: *mut *mut std::os::raw::c_char".to_string());
                        } else {
                            let ty_str = codegen_type_expr(ok_ty, interner);
                            c_params.push(format!("out: *mut {}", ty_str));
                        }
                    }
                }
            } else if has_ref_return {
                c_params.push("out: *mut LogosHandle".to_string());
            } else if has_text_return {
                c_params.push("out: *mut *mut std::os::raw::c_char".to_string());
            }
        }
    }

    // Build the wrapper signature
    let c_sig = if uses_status_code {
        format!("pub extern \"C\" fn {}({}) -> LogosStatus", func_name, c_params.join(", "))
    } else if has_text_return {
        format!("pub extern \"C\" fn {}({}) -> *mut std::os::raw::c_char", func_name, c_params.join(", "))
    } else if let Some(ret_ty) = return_type {
        let ret_str = codegen_type_expr(ret_ty, interner);
        if ret_str != "()" {
            format!("pub extern \"C\" fn {}({}) -> {}", func_name, c_params.join(", "), ret_str)
        } else {
            format!("pub extern \"C\" fn {}({})", func_name, c_params.join(", "))
        }
    } else {
        format!("pub extern \"C\" fn {}({})", func_name, c_params.join(", "))
    };

    writeln!(output, "#[no_mangle]").unwrap();
    writeln!(output, "{} {{", c_sig).unwrap();

    // 3) Marshal parameters
    let call_args: Vec<String> = params.iter()
        .map(|(pname, ptype)| {
            let pname_str = interner.resolve(*pname);
            if classify_type_for_c_abi(ptype, interner, registry) == CAbiClass::ReferenceType {
                // Look up handle in registry, dereference, and clone for inner
                let rust_ty = codegen_type_expr(ptype, interner);
                writeln!(output, "    let {pn} = {{", pn = pname_str).unwrap();
                writeln!(output, "        let __id = {pn} as u64;", pn = pname_str).unwrap();
                writeln!(output, "        let __reg = logos_handle_registry().lock().unwrap_or_else(|e| e.into_inner());").unwrap();
                writeln!(output, "        let __ptr = __reg.deref(__id).expect(\"InvalidHandle: handle not found in registry\");").unwrap();
                writeln!(output, "        drop(__reg);").unwrap();
                writeln!(output, "        unsafe {{ &*(__ptr as *const {ty}) }}.clone()", ty = rust_ty).unwrap();
                writeln!(output, "    }};").unwrap();
            } else if is_text_type(ptype, interner) {
                // Null-safety: check for NULL *const c_char before CStr::from_ptr
                if uses_status_code {
                    writeln!(output, "    if {pn}.is_null() {{ logos_set_last_error(\"NullPointer: text parameter '{pn}' is null\".to_string()); return LogosStatus::NullPointer; }}",
                        pn = pname_str).unwrap();
                    writeln!(output, "    let {pn} = unsafe {{ std::ffi::CStr::from_ptr({pn}).to_string_lossy().into_owned() }};",
                        pn = pname_str).unwrap();
                } else {
                    // Non-status-code function: substitute empty string for NULL
                    writeln!(output, "    let {pn} = if {pn}.is_null() {{ String::new() }} else {{ unsafe {{ std::ffi::CStr::from_ptr({pn}).to_string_lossy().into_owned() }} }};",
                        pn = pname_str).unwrap();
                }
            }
            pname_str.to_string()
        })
        .collect();

    // 4) Emit refinement guards for parameters
    for (pname, ptype) in params.iter() {
        if let TypeExpr::Refinement { base: _, var, predicate } = ptype {
            let pname_str = interner.resolve(*pname);
            let bound = interner.resolve(*var);
            let assertion = codegen_assertion(predicate, interner);
            let check = if bound == pname_str {
                assertion
            } else {
                replace_word(&assertion, bound, pname_str)
            };
            writeln!(output, "    if !({}) {{", check).unwrap();
            writeln!(output, "        logos_set_last_error(format!(\"Refinement violation: expected {check}, got {pn} = {{}}\", {pn}));",
                check = check, pn = pname_str).unwrap();
            writeln!(output, "        return LogosStatus::RefinementViolation;").unwrap();
            writeln!(output, "    }}").unwrap();
        }
    }

    // 4b) Null out-parameter check (before catch_unwind to avoid calling inner fn)
    if uses_status_code && (has_ref_return || has_text_return || has_result_return) {
        writeln!(output, "    if out.is_null() {{ logos_set_last_error(\"NullPointer: output parameter is null\".to_string()); return LogosStatus::NullPointer; }}").unwrap();
    }

    // 5) Determine panic default for catch_unwind error arm
    let panic_default = if uses_status_code {
        "LogosStatus::ThreadPanic"
    } else if has_text_return {
        "std::ptr::null_mut()"
    } else if return_type.map_or(false, |t| codegen_type_expr(t, interner) != "()") {
        "Default::default()"
    } else {
        "" // void function
    };

    // 6) Open catch_unwind panic boundary
    writeln!(output, "    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {{").unwrap();

    // 7) Call inner and marshal return (inside catch_unwind closure)
    if uses_status_code {
        if has_result_return {
            // Result<T, E>: match on Ok/Err
            writeln!(output, "    match {}({}) {{", inner_name, call_args.join(", ")).unwrap();
            writeln!(output, "        Ok(val) => {{").unwrap();

            if let Some(TypeExpr::Generic { params: ref rparams, .. }) = return_type {
                if !rparams.is_empty() {
                    let ok_ty = &rparams[0];
                    if classify_type_for_c_abi(ok_ty, interner, registry) == CAbiClass::ReferenceType {
                        writeln!(output, "            let __ptr = Box::into_raw(Box::new(val)) as usize;").unwrap();
                        writeln!(output, "            let mut __reg = logos_handle_registry().lock().unwrap_or_else(|e| e.into_inner());").unwrap();
                        writeln!(output, "            let (__id, _) = __reg.register(__ptr);").unwrap();
                        writeln!(output, "            unsafe {{ *out = __id as LogosHandle; }}").unwrap();
                    } else if is_text_type(ok_ty, interner) {
                        writeln!(output, "            match std::ffi::CString::new(val) {{").unwrap();
                        writeln!(output, "                Ok(cstr) => unsafe {{ *out = cstr.into_raw(); }},").unwrap();
                        writeln!(output, "                Err(_) => {{").unwrap();
                        writeln!(output, "                    logos_set_last_error(\"Return value contains null byte\".to_string());").unwrap();
                        writeln!(output, "                    return LogosStatus::ContainsNullByte;").unwrap();
                        writeln!(output, "                }}").unwrap();
                        writeln!(output, "            }}").unwrap();
                    } else {
                        writeln!(output, "            unsafe {{ *out = val; }}").unwrap();
                    }
                }
            }

            writeln!(output, "            LogosStatus::Ok").unwrap();
            writeln!(output, "        }}").unwrap();
            writeln!(output, "        Err(e) => {{").unwrap();
            writeln!(output, "            logos_set_last_error(format!(\"{{}}\", e));").unwrap();
            writeln!(output, "            LogosStatus::Error").unwrap();
            writeln!(output, "        }}").unwrap();
            writeln!(output, "    }}").unwrap();
        } else if has_ref_return {
            // Reference type return → box, register in handle registry, and write to out-parameter
            writeln!(output, "    let result = {}({});", inner_name, call_args.join(", ")).unwrap();
            writeln!(output, "    let __ptr = Box::into_raw(Box::new(result)) as usize;").unwrap();
            writeln!(output, "    let mut __reg = logos_handle_registry().lock().unwrap_or_else(|e| e.into_inner());").unwrap();
            writeln!(output, "    let (__id, _) = __reg.register(__ptr);").unwrap();
            writeln!(output, "    unsafe {{ *out = __id as LogosHandle; }}").unwrap();
            writeln!(output, "    LogosStatus::Ok").unwrap();
        } else if has_text_return {
            // Text return with status code → write to out-parameter
            writeln!(output, "    let result = {}({});", inner_name, call_args.join(", ")).unwrap();
            writeln!(output, "    match std::ffi::CString::new(result) {{").unwrap();
            writeln!(output, "        Ok(cstr) => {{").unwrap();
            writeln!(output, "            unsafe {{ *out = cstr.into_raw(); }}").unwrap();
            writeln!(output, "            LogosStatus::Ok").unwrap();
            writeln!(output, "        }}").unwrap();
            writeln!(output, "        Err(_) => {{").unwrap();
            writeln!(output, "            logos_set_last_error(\"Return value contains null byte\".to_string());").unwrap();
            writeln!(output, "            LogosStatus::ContainsNullByte").unwrap();
            writeln!(output, "        }}").unwrap();
            writeln!(output, "    }}").unwrap();
        } else {
            // No return value but status code (e.g., refinement-only)
            writeln!(output, "    {}({});", inner_name, call_args.join(", ")).unwrap();
            writeln!(output, "    LogosStatus::Ok").unwrap();
        }
    } else if has_text_return {
        // Text-only marshaling (legacy path, no status code)
        writeln!(output, "    let result = {}({});", inner_name, call_args.join(", ")).unwrap();
        writeln!(output, "    match std::ffi::CString::new(result) {{").unwrap();
        writeln!(output, "        Ok(cstr) => cstr.into_raw(),").unwrap();
        writeln!(output, "        Err(_) => {{ logos_set_last_error(\"Return value contains null byte\".to_string()); std::ptr::null_mut() }}").unwrap();
        writeln!(output, "    }}").unwrap();
    } else if return_type.is_some() {
        writeln!(output, "    {}({})", inner_name, call_args.join(", ")).unwrap();
    } else {
        writeln!(output, "    {}({})", inner_name, call_args.join(", ")).unwrap();
    }

    // 8) Close catch_unwind with panic handler
    writeln!(output, "    }})) {{").unwrap();
    writeln!(output, "        Ok(__v) => __v,").unwrap();
    writeln!(output, "        Err(__panic) => {{").unwrap();
    writeln!(output, "            let __msg = if let Some(s) = __panic.downcast_ref::<String>() {{ s.clone() }} else if let Some(s) = __panic.downcast_ref::<&str>() {{ s.to_string() }} else {{ \"Unknown panic\".to_string() }};").unwrap();
    writeln!(output, "            logos_set_last_error(__msg);").unwrap();
    if !panic_default.is_empty() {
        writeln!(output, "            {}", panic_default).unwrap();
    }
    writeln!(output, "        }}").unwrap();
    writeln!(output, "    }}").unwrap();

    writeln!(output, "}}\n").unwrap();

    output
}

/// Phase 38: Convert TypeExpr to Rust type string.
fn codegen_type_expr(ty: &TypeExpr, interner: &Interner) -> String {
    match ty {
        TypeExpr::Primitive(sym) => {
            map_type_to_rust(interner.resolve(*sym))
        }
        TypeExpr::Named(sym) => {
            let name = interner.resolve(*sym);
            // Check for common mappings
            map_type_to_rust(name)
        }
        TypeExpr::Generic { base, params } => {
            let base_name = interner.resolve(*base);
            let params_str: Vec<String> = params.iter()
                .map(|p| codegen_type_expr(p, interner))
                .collect();

            match base_name {
                "Result" => {
                    if params_str.len() == 2 {
                        format!("Result<{}, {}>", params_str[0], params_str[1])
                    } else if params_str.len() == 1 {
                        format!("Result<{}, String>", params_str[0])
                    } else {
                        "Result<(), String>".to_string()
                    }
                }
                "Option" | "Maybe" => {
                    if !params_str.is_empty() {
                        format!("Option<{}>", params_str[0])
                    } else {
                        "Option<()>".to_string()
                    }
                }
                "Seq" | "List" | "Vec" => {
                    if !params_str.is_empty() {
                        format!("Vec<{}>", params_str[0])
                    } else {
                        "Vec<()>".to_string()
                    }
                }
                "Map" | "HashMap" => {
                    if params_str.len() >= 2 {
                        format!("std::collections::HashMap<{}, {}>", params_str[0], params_str[1])
                    } else {
                        "std::collections::HashMap<String, String>".to_string()
                    }
                }
                "Set" | "HashSet" => {
                    if !params_str.is_empty() {
                        format!("std::collections::HashSet<{}>", params_str[0])
                    } else {
                        "std::collections::HashSet<()>".to_string()
                    }
                }
                other => {
                    if params_str.is_empty() {
                        other.to_string()
                    } else {
                        format!("{}<{}>", other, params_str.join(", "))
                    }
                }
            }
        }
        TypeExpr::Function { inputs, output } => {
            let inputs_str: Vec<String> = inputs.iter()
                .map(|i| codegen_type_expr(i, interner))
                .collect();
            let output_str = codegen_type_expr(output, interner);
            format!("impl Fn({}) -> {}", inputs_str.join(", "), output_str)
        }
        // Phase 43C: Refinement types use the base type for Rust type annotation
        // The constraint predicate is handled separately via debug_assert!
        TypeExpr::Refinement { base, .. } => {
            codegen_type_expr(base, interner)
        }
        // Phase 53: Persistent storage wrapper
        TypeExpr::Persistent { inner } => {
            let inner_type = codegen_type_expr(inner, interner);
            format!("logicaffeine_system::storage::Persistent<{}>", inner_type)
        }
    }
}

/// Infer return type from function body by looking at Return statements.
fn infer_return_type_from_body(body: &[Stmt], _interner: &Interner) -> Option<String> {
    for stmt in body {
        if let Stmt::Return { value: Some(_) } = stmt {
            // For now, assume i64 for any expression return
            // TODO: Implement proper type inference
            return Some("i64".to_string());
        }
    }
    None
}

/// Map LOGOS type names to Rust types.
fn map_type_to_rust(ty: &str) -> String {
    match ty {
        "Int" => "i64".to_string(),
        "Nat" => "u64".to_string(),
        "Text" => "String".to_string(),
        "Bool" | "Boolean" => "bool".to_string(),
        "Real" | "Float" => "f64".to_string(),
        "Char" => "char".to_string(),
        "Byte" => "u8".to_string(),
        "Unit" | "()" => "()".to_string(),
        "Duration" => "std::time::Duration".to_string(),
        other => other.to_string(),
    }
}

/// Generate a single struct definition with derives and visibility.
/// Phase 34: Now supports generic type parameters.
/// Phase 47: Now supports is_portable for Serialize/Deserialize derives.
/// Phase 49: Now supports is_shared for CRDT Merge impl.
fn codegen_struct_def(name: Symbol, fields: &[FieldDef], generics: &[Symbol], is_portable: bool, is_shared: bool, interner: &Interner, indent: usize, c_abi_value_structs: &HashSet<Symbol>, c_abi_ref_structs: &HashSet<Symbol>) -> String {
    let ind = " ".repeat(indent);
    let mut output = String::new();

    // Build generic parameter string: <T, U> or empty
    let generic_str = if generics.is_empty() {
        String::new()
    } else {
        let params: Vec<&str> = generics.iter()
            .map(|g| interner.resolve(*g))
            .collect();
        format!("<{}>", params.join(", "))
    };

    // Value-type structs used in C ABI exports need #[repr(C)] for stable field layout
    if c_abi_value_structs.contains(&name) {
        writeln!(output, "{}#[repr(C)]", ind).unwrap();
    }

    // Phase 47: Add Serialize, Deserialize derives if portable
    // Phase 50: Add PartialEq for policy equality comparisons
    // Phase 52: Shared types also need Serialize/Deserialize for Synced<T>
    // C ABI reference-type structs also need serde for from_json/to_json support
    if is_portable || is_shared || c_abi_ref_structs.contains(&name) {
        writeln!(output, "{}#[derive(Default, Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]", ind).unwrap();
    } else {
        writeln!(output, "{}#[derive(Default, Debug, Clone, PartialEq)]", ind).unwrap();
    }
    writeln!(output, "{}pub struct {}{} {{", ind, interner.resolve(name), generic_str).unwrap();

    for field in fields {
        let vis = if field.is_public { "pub " } else { "" };
        let rust_type = codegen_field_type(&field.ty, interner);
        writeln!(output, "{}    {}{}: {},", ind, vis, interner.resolve(field.name), rust_type).unwrap();
    }

    writeln!(output, "{}}}\n", ind).unwrap();

    // Phase 49: Generate Merge impl for Shared structs
    if is_shared {
        output.push_str(&codegen_merge_impl(name, fields, generics, interner, indent));
    }

    output
}

/// Phase 49: Generate impl Merge for a Shared struct.
fn codegen_merge_impl(name: Symbol, fields: &[FieldDef], generics: &[Symbol], interner: &Interner, indent: usize) -> String {
    let ind = " ".repeat(indent);
    let name_str = interner.resolve(name);
    let mut output = String::new();

    // Build generic parameter string: <T, U> or empty
    let generic_str = if generics.is_empty() {
        String::new()
    } else {
        let params: Vec<&str> = generics.iter()
            .map(|g| interner.resolve(*g))
            .collect();
        format!("<{}>", params.join(", "))
    };

    writeln!(output, "{}impl{} logicaffeine_data::crdt::Merge for {}{} {{", ind, generic_str, name_str, generic_str).unwrap();
    writeln!(output, "{}    fn merge(&mut self, other: &Self) {{", ind).unwrap();

    for field in fields {
        let field_name = interner.resolve(field.name);
        // Only merge fields that implement Merge (CRDT types)
        if is_crdt_field_type(&field.ty, interner) {
            writeln!(output, "{}        self.{}.merge(&other.{});", ind, field_name, field_name).unwrap();
        }
    }

    writeln!(output, "{}    }}", ind).unwrap();
    writeln!(output, "{}}}\n", ind).unwrap();

    output
}

/// Phase 49: Check if a field type is a CRDT type that implements Merge.
fn is_crdt_field_type(ty: &FieldType, interner: &Interner) -> bool {
    match ty {
        FieldType::Named(sym) => {
            let name = interner.resolve(*sym);
            matches!(name,
                "ConvergentCount" | "GCounter" |
                "Tally" | "PNCounter"
            )
        }
        FieldType::Generic { base, .. } => {
            let name = interner.resolve(*base);
            matches!(name,
                "LastWriteWins" | "LWWRegister" |
                "SharedSet" | "ORSet" | "SharedSet_AddWins" | "SharedSet_RemoveWins" |
                "SharedSequence" | "RGA" | "SharedSequence_YATA" | "CollaborativeSequence" |
                "SharedMap" | "ORMap" |
                "Divergent" | "MVRegister"
            )
        }
        _ => false,
    }
}

/// Phase 33/34: Generate enum definition with optional generic parameters.
/// Phase 47: Now supports is_portable for Serialize/Deserialize derives.
/// Phase 49: Now accepts is_shared parameter (enums don't generate Merge impl yet).
fn codegen_enum_def(name: Symbol, variants: &[VariantDef], generics: &[Symbol], is_portable: bool, _is_shared: bool, interner: &Interner, indent: usize) -> String {
    let ind = " ".repeat(indent);
    let mut output = String::new();

    // Build generic parameter string: <T, U> or empty
    let generic_str = if generics.is_empty() {
        String::new()
    } else {
        let params: Vec<&str> = generics.iter()
            .map(|g| interner.resolve(*g))
            .collect();
        format!("<{}>", params.join(", "))
    };

    // Phase 47: Add Serialize, Deserialize derives if portable
    if is_portable {
        writeln!(output, "{}#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]", ind).unwrap();
    } else {
        writeln!(output, "{}#[derive(Debug, Clone, PartialEq)]", ind).unwrap();
    }
    writeln!(output, "{}pub enum {}{} {{", ind, interner.resolve(name), generic_str).unwrap();

    for variant in variants {
        let variant_name = interner.resolve(variant.name);
        if variant.fields.is_empty() {
            // Unit variant
            writeln!(output, "{}    {},", ind, variant_name).unwrap();
        } else {
            // Struct variant with named fields
            // Phase 102: Detect and box recursive fields
            let enum_name_str = interner.resolve(name);
            let fields_str: Vec<String> = variant.fields.iter()
                .map(|f| {
                    let rust_type = codegen_field_type(&f.ty, interner);
                    let field_name = interner.resolve(f.name);
                    // Check if this field references the enum itself (recursive type)
                    if is_recursive_field(&f.ty, enum_name_str, interner) {
                        format!("{}: Box<{}>", field_name, rust_type)
                    } else {
                        format!("{}: {}", field_name, rust_type)
                    }
                })
                .collect();
            writeln!(output, "{}    {} {{ {} }},", ind, variant_name, fields_str.join(", ")).unwrap();
        }
    }

    writeln!(output, "{}}}\n", ind).unwrap();

    // Generate Default impl for enum (defaults to first variant)
    // This is needed when the enum is used as a struct field and the struct derives Default
    // Only for non-generic enums — generic enums can't assume their type params implement Default
    if generics.is_empty() {
    if let Some(first_variant) = variants.first() {
        let enum_name_str = interner.resolve(name);
        let first_variant_name = interner.resolve(first_variant.name);
        writeln!(output, "{}impl{} Default for {}{} {{", ind, generic_str, enum_name_str, generic_str).unwrap();
        writeln!(output, "{}    fn default() -> Self {{", ind).unwrap();
        if first_variant.fields.is_empty() {
            writeln!(output, "{}        {}::{}", ind, enum_name_str, first_variant_name).unwrap();
        } else {
            // Default with default field values
            let default_fields: Vec<String> = first_variant.fields.iter()
                .map(|f| {
                    let field_name = interner.resolve(f.name);
                    let enum_name_check = interner.resolve(name);
                    if is_recursive_field(&f.ty, enum_name_check, interner) {
                        format!("{}: Box::new(Default::default())", field_name)
                    } else {
                        format!("{}: Default::default()", field_name)
                    }
                })
                .collect();
            writeln!(output, "{}        {}::{} {{ {} }}", ind, enum_name_str, first_variant_name, default_fields.join(", ")).unwrap();
        }
        writeln!(output, "{}    }}", ind).unwrap();
        writeln!(output, "{}}}\n", ind).unwrap();
    }
    }

    output
}

/// Convert FieldType to Rust type string.
fn codegen_field_type(ty: &FieldType, interner: &Interner) -> String {
    match ty {
        FieldType::Primitive(sym) => {
            match interner.resolve(*sym) {
                "Int" => "i64".to_string(),
                "Nat" => "u64".to_string(),
                "Text" => "String".to_string(),
                "Bool" | "Boolean" => "bool".to_string(),
                "Real" | "Float" => "f64".to_string(),
                "Char" => "char".to_string(),
                "Byte" => "u8".to_string(),
                "Unit" => "()".to_string(),
                "Duration" => "std::time::Duration".to_string(),
                other => other.to_string(),
            }
        }
        FieldType::Named(sym) => {
            let name = interner.resolve(*sym);
            match name {
                // Phase 49: CRDT type mapping
                "ConvergentCount" => "logicaffeine_data::crdt::GCounter".to_string(),
                // Phase 49b: New CRDT types (Wave 5)
                "Tally" => "logicaffeine_data::crdt::PNCounter".to_string(),
                _ => name.to_string(),
            }
        }
        FieldType::Generic { base, params } => {
            let base_name = interner.resolve(*base);
            let param_strs: Vec<String> = params.iter()
                .map(|p| codegen_field_type(p, interner))
                .collect();

            // Phase 49c: Handle CRDT types with bias/algorithm modifiers
            match base_name {
                // SharedSet with explicit bias
                "SharedSet_RemoveWins" => {
                    return format!("logicaffeine_data::crdt::ORSet<{}, logicaffeine_data::crdt::RemoveWins>", param_strs.join(", "));
                }
                "SharedSet_AddWins" => {
                    return format!("logicaffeine_data::crdt::ORSet<{}, logicaffeine_data::crdt::AddWins>", param_strs.join(", "));
                }
                // SharedSequence with YATA algorithm
                "SharedSequence_YATA" | "CollaborativeSequence" => {
                    return format!("logicaffeine_data::crdt::YATA<{}>", param_strs.join(", "));
                }
                _ => {}
            }

            let base_str = match base_name {
                "List" | "Seq" => "Vec",
                "Set" => "std::collections::HashSet",
                "Map" => "std::collections::HashMap",
                "Option" | "Maybe" => "Option",
                "Result" => "Result",
                // Phase 49: CRDT generic type
                "LastWriteWins" => "logicaffeine_data::crdt::LWWRegister",
                // Phase 49b: New CRDT generic types (Wave 5) - default to AddWins for ORSet
                "SharedSet" | "ORSet" => "logicaffeine_data::crdt::ORSet",
                "SharedSequence" | "RGA" => "logicaffeine_data::crdt::RGA",
                "SharedMap" | "ORMap" => "logicaffeine_data::crdt::ORMap",
                "Divergent" | "MVRegister" => "logicaffeine_data::crdt::MVRegister",
                other => other,
            };
            format!("{}<{}>", base_str, param_strs.join(", "))
        }
        // Phase 34: Type parameter reference (T, U, etc.)
        FieldType::TypeParam(sym) => interner.resolve(*sym).to_string(),
    }
}

/// Phase 102: Check if a field type references the containing enum (recursive type).
/// Recursive types need to be wrapped in Box<T> for Rust to know the size.
fn is_recursive_field(ty: &FieldType, enum_name: &str, interner: &Interner) -> bool {
    match ty {
        FieldType::Primitive(sym) => interner.resolve(*sym) == enum_name,
        FieldType::Named(sym) => interner.resolve(*sym) == enum_name,
        FieldType::TypeParam(_) => false,
        FieldType::Generic { base, params } => {
            // Check if base matches or any type parameter contains the enum
            interner.resolve(*base) == enum_name ||
            params.iter().any(|p| is_recursive_field(p, enum_name, interner))
        }
    }
}

/// Phase 103: Infer type annotation for multi-param generic enum variants.
/// Returns Some(type_annotation) if the enum has multiple type params, None otherwise.
fn infer_variant_type_annotation(
    expr: &Expr,
    registry: &TypeRegistry,
    interner: &Interner,
) -> Option<String> {
    // Only handle NewVariant expressions
    let (enum_name, variant_name, field_values) = match expr {
        Expr::NewVariant { enum_name, variant, fields } => (*enum_name, *variant, fields),
        _ => return None,
    };

    // Look up the enum in the registry
    let enum_def = registry.get(enum_name)?;
    let (generics, variants) = match enum_def {
        TypeDef::Enum { generics, variants, .. } => (generics, variants),
        _ => return None,
    };

    // Only generate type annotations for multi-param generics
    if generics.len() < 2 {
        return None;
    }

    // Find the variant definition
    let variant_def = variants.iter().find(|v| v.name == variant_name)?;

    // Collect which type params are bound by which field types
    let mut type_param_types: HashMap<Symbol, String> = HashMap::new();
    for (field_name, field_value) in field_values {
        // Find the field in the variant definition
        if let Some(field_def) = variant_def.fields.iter().find(|f| f.name == *field_name) {
            // If the field type is a type parameter, infer its type from the value
            if let FieldType::TypeParam(type_param) = &field_def.ty {
                let inferred = infer_rust_type_from_expr(field_value, interner);
                type_param_types.insert(*type_param, inferred);
            }
        }
    }

    // Build the type annotation: EnumName<T1, T2, ...>
    // For bound params, use the inferred type; for unbound, use ()
    let enum_str = interner.resolve(enum_name);
    let param_strs: Vec<String> = generics.iter()
        .map(|g| {
            type_param_types.get(g)
                .cloned()
                .unwrap_or_else(|| "()".to_string())
        })
        .collect();

    Some(format!("{}<{}>", enum_str, param_strs.join(", ")))
}

/// Phase 103: Infer Rust type from a LOGOS expression.
fn infer_rust_type_from_expr(expr: &Expr, interner: &Interner) -> String {
    match expr {
        Expr::Literal(lit) => match lit {
            Literal::Number(_) => "i64".to_string(),
            Literal::Float(_) => "f64".to_string(),
            Literal::Text(_) => "String".to_string(),
            Literal::Boolean(_) => "bool".to_string(),
            Literal::Char(_) => "char".to_string(),
            Literal::Nothing => "()".to_string(),
            Literal::Duration(_) => "std::time::Duration".to_string(),
            Literal::Date(_) => "LogosDate".to_string(),
            Literal::Moment(_) => "LogosMoment".to_string(),
            Literal::Span { .. } => "LogosSpan".to_string(),
            Literal::Time(_) => "LogosTime".to_string(),
        },
        // For identifiers and complex expressions, let Rust infer
        _ => "_".to_string(),
    }
}

/// Peephole optimization: detect `Let counter = start. While counter <= limit: body; Set counter to counter + 1`
/// and emit `for counter in start..=limit { body } let mut counter = limit + 1;` instead.
/// The for-range form enables LLVM trip count analysis, unrolling, and vectorization.
/// Returns (generated_code, number_of_extra_statements_consumed) or None if pattern doesn't match.
fn try_emit_for_range_pattern<'a>(
    stmts: &[&Stmt<'a>],
    idx: usize,
    interner: &Interner,
    indent: usize,
    mutable_vars: &HashSet<Symbol>,
    ctx: &mut RefinementContext<'a>,
    lww_fields: &HashSet<(String, String)>,
    mv_fields: &HashSet<(String, String)>,
    synced_vars: &mut HashSet<Symbol>,
    var_caps: &HashMap<Symbol, VariableCapabilities>,
    async_functions: &HashSet<Symbol>,
    pipe_vars: &HashSet<Symbol>,
    boxed_fields: &HashSet<(String, String, String)>,
    registry: &TypeRegistry,
) -> Option<(String, usize)> {
    if idx + 1 >= stmts.len() {
        return None;
    }

    // Statement 1: Let counter = start_literal (integer)
    // Note: mutable flag may be false in AST even when counter is mutated via Set.
    // The counter's mutability is proven by the while body's increment statement.
    let (counter_sym, counter_start) = match stmts[idx] {
        Stmt::Let { var, value: Expr::Literal(Literal::Number(n)), .. } => {
            (*var, *n)
        }
        _ => return None,
    };

    // Statement 2: While (counter <= limit) or (counter < limit)
    let (body, limit_expr, is_exclusive) = match stmts[idx + 1] {
        Stmt::While { cond, body, .. } => {
            match cond {
                Expr::BinaryOp { op: BinaryOpKind::LtEq, left, right } => {
                    if let Expr::Identifier(sym) = left {
                        if *sym == counter_sym {
                            (body, *right, false)
                        } else {
                            return None;
                        }
                    } else {
                        return None;
                    }
                }
                Expr::BinaryOp { op: BinaryOpKind::Lt, left, right } => {
                    if let Expr::Identifier(sym) = left {
                        if *sym == counter_sym {
                            (body, *right, true)
                        } else {
                            return None;
                        }
                    } else {
                        return None;
                    }
                }
                _ => return None,
            }
        }
        _ => return None,
    };

    // Body must have at least 1 statement (the counter increment)
    if body.is_empty() {
        return None;
    }

    // Last body statement must be: Set counter to counter + 1
    let last = &body[body.len() - 1];
    match last {
        Stmt::Set { target, value, .. } => {
            if *target != counter_sym {
                return None;
            }
            match value {
                Expr::BinaryOp { op: BinaryOpKind::Add, left, right } => {
                    let is_counter_plus_1 = match (left, right) {
                        (Expr::Identifier(s), Expr::Literal(Literal::Number(1))) if *s == counter_sym => true,
                        (Expr::Literal(Literal::Number(1)), Expr::Identifier(s)) if *s == counter_sym => true,
                        _ => false,
                    };
                    if !is_counter_plus_1 {
                        return None;
                    }
                }
                _ => return None,
            }
        }
        _ => return None,
    }

    // Validity: counter must NOT be modified anywhere in the body EXCEPT the last statement.
    // Walk all body statements (excluding the last) and check for Set { target: counter_sym }.
    let body_without_increment = &body[..body.len() - 1];
    if body_modifies_var(body_without_increment, counter_sym) {
        return None;
    }

    // Pattern matched! Emit for-range loop.
    let indent_str = "    ".repeat(indent);
    let counter_name = interner.resolve(counter_sym);
    let limit_str = codegen_expr_simple(limit_expr, interner);

    let range_str = if is_exclusive {
        format!("{}..{}", counter_start, limit_str)
    } else {
        format!("{}..={}", counter_start, limit_str)
    };

    let mut output = String::new();
    writeln!(output, "{}for {} in {} {{", indent_str, counter_name, range_str).unwrap();

    // Emit body statements (excluding the final counter increment)
    ctx.push_scope();
    let body_refs: Vec<&Stmt> = body_without_increment.iter().collect();
    let mut bi = 0;
    while bi < body_refs.len() {
        if let Some((code, skip)) = try_emit_swap_pattern(&body_refs, bi, interner, indent + 1, ctx.get_variable_types()) {
            output.push_str(&code);
            bi += 1 + skip;
            continue;
        }
        output.push_str(&codegen_stmt(body_refs[bi], interner, indent + 1, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry));
        bi += 1;
    }
    ctx.pop_scope();
    writeln!(output, "{}}}", indent_str).unwrap();

    // Emit post-loop counter value so subsequent code sees the correct value.
    // After `while (i <= limit) { ...; i++ }`, i == limit + 1.
    // After `while (i < limit) { ...; i++ }`, i == limit.
    let post_value = if is_exclusive {
        limit_str
    } else {
        format!("({} + 1)", limit_str)
    };
    writeln!(output, "{}let mut {} = {};", indent_str, counter_name, post_value).unwrap();

    Some((output, 1)) // consumed 1 extra statement (the While)
}

/// Check if a slice of statements modifies a specific variable (used for for-range validity).
/// Recursively walks into nested If/While/Repeat blocks.
fn body_modifies_var(stmts: &[Stmt], sym: Symbol) -> bool {
    for stmt in stmts {
        match stmt {
            Stmt::Set { target, .. } if *target == sym => return true,
            Stmt::If { then_block, else_block, .. } => {
                if body_modifies_var(then_block, sym) {
                    return true;
                }
                if let Some(else_stmts) = else_block {
                    if body_modifies_var(else_stmts, sym) {
                        return true;
                    }
                }
            }
            Stmt::While { body, .. } => {
                if body_modifies_var(body, sym) {
                    return true;
                }
            }
            Stmt::Repeat { body, .. } => {
                if body_modifies_var(body, sym) {
                    return true;
                }
            }
            Stmt::Zone { body, .. } => {
                if body_modifies_var(body, sym) {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

/// Check if a loop body mutates a specific collection (used for iterator optimization).
/// Scans for Push, Pop, SetIndex, Remove, Set, and Add targeting the collection.
/// Recursively walks into nested If/While/Repeat/Zone blocks.
fn body_mutates_collection(stmts: &[Stmt], coll_sym: Symbol) -> bool {
    for stmt in stmts {
        match stmt {
            Stmt::Push { collection, .. } | Stmt::Pop { collection, .. }
            | Stmt::Add { collection, .. } | Stmt::Remove { collection, .. } => {
                if let Expr::Identifier(sym) = collection {
                    if *sym == coll_sym {
                        return true;
                    }
                }
            }
            Stmt::SetIndex { collection, .. } => {
                if let Expr::Identifier(sym) = collection {
                    if *sym == coll_sym {
                        return true;
                    }
                }
            }
            Stmt::Set { target, .. } if *target == coll_sym => return true,
            Stmt::If { then_block, else_block, .. } => {
                if body_mutates_collection(then_block, coll_sym) {
                    return true;
                }
                if let Some(else_stmts) = else_block {
                    if body_mutates_collection(else_stmts, coll_sym) {
                        return true;
                    }
                }
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } => {
                if body_mutates_collection(body, coll_sym) {
                    return true;
                }
            }
            Stmt::Zone { body, .. } => {
                if body_mutates_collection(body, coll_sym) {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

/// Peephole optimization: detect `Let vec = new Seq. Let i = 0. While i <= limit: push const to vec, i = i+1`
/// and emit `let mut vec: Vec<T> = vec![const; (limit + 1) as usize]` instead.
/// Returns (generated_code, number_of_extra_statements_consumed) or None if pattern doesn't match.
fn try_emit_vec_fill_pattern<'a>(
    stmts: &[&Stmt<'a>],
    idx: usize,
    interner: &Interner,
    indent: usize,
) -> Option<(String, usize)> {
    if idx + 2 >= stmts.len() {
        return None;
    }

    // Statement 1: Let mutable vec_var be a new Seq of T.
    let (vec_sym, elem_type) = match stmts[idx] {
        Stmt::Let { var, value, mutable: true, ty, .. } => {
            // Check for explicit type annotation like `: Seq of Bool`
            let type_from_annotation = if let Some(TypeExpr::Generic { base, params }) = ty {
                let base_name = interner.resolve(*base);
                if matches!(base_name, "Seq" | "List" | "Vec") && !params.is_empty() {
                    Some(codegen_type_expr(&params[0], interner))
                } else {
                    None
                }
            } else {
                None
            };

            // Check for `a new Seq of T`
            let type_from_new = if let Expr::New { type_name, type_args, init_fields } = value {
                let tn = interner.resolve(*type_name);
                if matches!(tn, "Seq" | "List" | "Vec") && init_fields.is_empty() {
                    if !type_args.is_empty() {
                        Some(codegen_type_expr(&type_args[0], interner))
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            };

            match type_from_annotation.or(type_from_new) {
                Some(t) => (*var, t),
                None => return None,
            }
        }
        _ => return None,
    };

    // Statement 2: Let mutable counter = 0 (or 1).
    let (counter_sym, counter_start) = match stmts[idx + 1] {
        Stmt::Let { var, value: Expr::Literal(Literal::Number(n)), mutable: true, .. } => {
            (*var, *n)
        }
        _ => return None,
    };

    // Statement 3: While counter <= limit (or counter < limit): Push const_val to vec_var. Set counter to counter + 1.
    match stmts[idx + 2] {
        Stmt::While { cond, body, .. } => {
            // Check condition: counter <= limit OR counter < limit
            let (limit_expr, is_exclusive) = match cond {
                Expr::BinaryOp { op: BinaryOpKind::LtEq, left, right } => {
                    if let Expr::Identifier(sym) = left {
                        if *sym == counter_sym {
                            (Some(*right), false)
                        } else {
                            (None, false)
                        }
                    } else {
                        (None, false)
                    }
                }
                Expr::BinaryOp { op: BinaryOpKind::Lt, left, right } => {
                    if let Expr::Identifier(sym) = left {
                        if *sym == counter_sym {
                            (Some(*right), true)
                        } else {
                            (None, false)
                        }
                    } else {
                        (None, false)
                    }
                }
                _ => (None, false),
            };
            let limit_expr = limit_expr?;

            // Body must have exactly 2 statements: Push and Set
            if body.len() != 2 {
                return None;
            }

            // First body stmt: Push const_val to vec_var
            let push_val = match &body[0] {
                Stmt::Push { value, collection } => {
                    if let Expr::Identifier(sym) = collection {
                        if *sym == vec_sym {
                            Some(*value)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
                _ => None,
            }?;

            // Push value must be a constant literal
            let val_str = match push_val {
                Expr::Literal(Literal::Number(n)) => format!("{}", n),
                Expr::Literal(Literal::Float(f)) => format!("{:.1}", f),
                Expr::Literal(Literal::Boolean(b)) => format!("{}", b),
                Expr::Literal(Literal::Char(c)) => format!("'{}'", c),
                Expr::Literal(Literal::Text(s)) => format!("{}.to_string()", interner.resolve(*s)),
                _ => return None,
            };

            // Second body stmt: Set counter to counter + 1
            match &body[1] {
                Stmt::Set { target, value, .. } => {
                    if *target != counter_sym {
                        return None;
                    }
                    // Value must be counter + 1
                    match value {
                        Expr::BinaryOp { op: BinaryOpKind::Add, left, right } => {
                            let is_counter_plus_1 = match (left, right) {
                                (Expr::Identifier(s), Expr::Literal(Literal::Number(1))) if *s == counter_sym => true,
                                (Expr::Literal(Literal::Number(1)), Expr::Identifier(s)) if *s == counter_sym => true,
                                _ => false,
                            };
                            if !is_counter_plus_1 {
                                return None;
                            }
                        }
                        _ => return None,
                    }
                }
                _ => return None,
            }

            // Pattern matched! Emit optimized code.
            let indent_str = "    ".repeat(indent);
            let vec_name = interner.resolve(vec_sym);
            let limit_str = codegen_expr_simple(limit_expr, interner);

            // Calculate count based on bound type (exclusive vs inclusive) and start value
            // Inclusive (<=): count = limit - start + 1
            // Exclusive (<): count = limit - start
            let count_expr = if is_exclusive {
                // Exclusive bound: counter < limit
                if counter_start == 0 {
                    format!("{} as usize", limit_str)
                } else {
                    format!("({} - {}) as usize", limit_str, counter_start)
                }
            } else {
                // Inclusive bound: counter <= limit
                if counter_start == 0 {
                    format!("({} + 1) as usize", limit_str)
                } else if counter_start == 1 {
                    format!("{} as usize", limit_str)
                } else {
                    format!("({} - {} + 1) as usize", limit_str, counter_start)
                }
            };

            let mut output = String::new();
            writeln!(output, "{}let mut {}: Vec<{}> = vec![{}; {}];",
                indent_str, vec_name, elem_type, val_str, count_expr).unwrap();
            // Re-emit counter variable declaration (it may be reused after the fill loop)
            let counter_name = interner.resolve(counter_sym);
            writeln!(output, "{}let mut {} = {};",
                indent_str, counter_name, counter_start).unwrap();

            Some((output, 2)) // consumed 2 extra statements (counter init + while loop)
        }
        _ => None,
    }
}

/// Simple expression codegen for peephole patterns (no async/context needed).
fn codegen_expr_simple(expr: &Expr, interner: &Interner) -> String {
    match expr {
        Expr::Literal(Literal::Number(n)) => format!("{}", n),
        Expr::Literal(Literal::Float(f)) => format!("{:.1}", f),
        Expr::Literal(Literal::Boolean(b)) => format!("{}", b),
        Expr::Identifier(sym) => interner.resolve(*sym).to_string(),
        Expr::BinaryOp { op, left, right } => {
            let l = codegen_expr_simple(left, interner);
            let r = codegen_expr_simple(right, interner);
            let op_str = match op {
                BinaryOpKind::Add => "+",
                BinaryOpKind::Subtract => "-",
                BinaryOpKind::Multiply => "*",
                BinaryOpKind::Divide => "/",
                BinaryOpKind::Modulo => "%",
                _ => return format!("({})", l),
            };
            format!("({} {} {})", l, op_str, r)
        }
        _ => "_".to_string(),
    }
}

/// Check if two expressions are structurally equal (for swap pattern detection).
fn exprs_equal(a: &Expr, b: &Expr) -> bool {
    match (a, b) {
        (Expr::Identifier(s1), Expr::Identifier(s2)) => s1 == s2,
        (Expr::Literal(Literal::Number(n1)), Expr::Literal(Literal::Number(n2))) => n1 == n2,
        (Expr::BinaryOp { op: op1, left: l1, right: r1 }, Expr::BinaryOp { op: op2, left: l2, right: r2 }) => {
            op1 == op2 && exprs_equal(l1, l2) && exprs_equal(r1, r2)
        }
        _ => false,
    }
}

/// Peephole optimization: detect swap pattern:
///   Let a be item j of arr. Let b be item (j+1) of arr.
///   If a > b then: Set item j of arr to b. Set item (j+1) of arr to a.
/// and emit `arr.swap((j-1) as usize, ((j+1)-1) as usize)` instead.
/// Returns (generated_code, number_of_extra_statements_consumed) or None.
fn try_emit_swap_pattern<'a>(
    stmts: &[&Stmt<'a>],
    idx: usize,
    interner: &Interner,
    indent: usize,
    variable_types: &HashMap<Symbol, String>,
) -> Option<(String, usize)> {
    if idx + 2 >= stmts.len() {
        return None;
    }

    // Statement 1: Let a be item j of arr (index expression)
    let (a_sym, arr_sym_1, idx_expr_1) = match stmts[idx] {
        Stmt::Let { var, value: Expr::Index { collection, index }, mutable: false, .. } => {
            if let Expr::Identifier(coll_sym) = collection {
                (*var, *coll_sym, *index)
            } else {
                return None;
            }
        }
        _ => return None,
    };

    // Only optimize for known Vec types (direct indexing)
    if let Some(t) = variable_types.get(&arr_sym_1) {
        if !t.starts_with("Vec") {
            return None;
        }
    } else {
        return None;
    }

    // Statement 2: Let b be item (j+1) of arr (adjacent index)
    let (b_sym, arr_sym_2, idx_expr_2) = match stmts[idx + 1] {
        Stmt::Let { var, value: Expr::Index { collection, index }, mutable: false, .. } => {
            if let Expr::Identifier(coll_sym) = collection {
                (*var, *coll_sym, *index)
            } else {
                return None;
            }
        }
        _ => return None,
    };

    // Must be the same array
    if arr_sym_1 != arr_sym_2 {
        return None;
    }

    // idx_expr_2 must be idx_expr_1 + 1
    let is_adjacent = match idx_expr_2 {
        Expr::BinaryOp { op: BinaryOpKind::Add, left, right } => {
            (exprs_equal(left, idx_expr_1) && matches!(right, Expr::Literal(Literal::Number(1))))
            || (matches!(left, Expr::Literal(Literal::Number(1))) && exprs_equal(right, idx_expr_1))
        }
        _ => false,
    };
    if !is_adjacent {
        return None;
    }

    // Statement 3: If a > b (or a < b, etc.) then: SetIndex arr j b, SetIndex arr j+1 a
    match stmts[idx + 2] {
        Stmt::If { cond, then_block, else_block } => {
            // Condition must compare a and b
            let compares_a_b = match cond {
                Expr::BinaryOp { op, left, right } => {
                    matches!(op, BinaryOpKind::Gt | BinaryOpKind::Lt | BinaryOpKind::GtEq | BinaryOpKind::LtEq | BinaryOpKind::Eq | BinaryOpKind::NotEq) &&
                    ((matches!(left, Expr::Identifier(s) if *s == a_sym) && matches!(right, Expr::Identifier(s) if *s == b_sym)) ||
                     (matches!(left, Expr::Identifier(s) if *s == b_sym) && matches!(right, Expr::Identifier(s) if *s == a_sym)))
                }
                _ => false,
            };
            if !compares_a_b {
                return None;
            }

            // Must have no else block
            if else_block.is_some() {
                return None;
            }

            // Then block must have exactly 2 SetIndex statements forming a cross-swap
            if then_block.len() != 2 {
                return None;
            }

            // Check: SetIndex arr idx1 b, SetIndex arr idx2 a (cross pattern)
            let swap_ok = match (&then_block[0], &then_block[1]) {
                (
                    Stmt::SetIndex { collection: c1, index: i1, value: v1 },
                    Stmt::SetIndex { collection: c2, index: i2, value: v2 },
                ) => {
                    // c1 and c2 must be the same array
                    let same_arr = matches!((c1, c2), (Expr::Identifier(s1), Expr::Identifier(s2)) if *s1 == arr_sym_1 && *s2 == arr_sym_1);
                    // Cross pattern: set idx1 to b, set idx2 to a
                    let cross = exprs_equal(i1, idx_expr_1) && exprs_equal(i2, idx_expr_2) &&
                        matches!(v1, Expr::Identifier(s) if *s == b_sym) &&
                        matches!(v2, Expr::Identifier(s) if *s == a_sym);
                    // Also check reverse: set idx1 to b via idx2/a pattern
                    let cross_rev = exprs_equal(i1, idx_expr_2) && exprs_equal(i2, idx_expr_1) &&
                        matches!(v1, Expr::Identifier(s) if *s == a_sym) &&
                        matches!(v2, Expr::Identifier(s) if *s == b_sym);
                    same_arr && (cross || cross_rev)
                }
                _ => false,
            };

            if !swap_ok {
                return None;
            }

            // Pattern matched! Emit optimized swap
            let indent_str = "    ".repeat(indent);
            let arr_name = interner.resolve(arr_sym_1);
            let idx1_str = codegen_expr_simple(idx_expr_1, interner);
            let idx2_str = codegen_expr_simple(idx_expr_2, interner);

            let op_str = match cond {
                Expr::BinaryOp { op, .. } => match op {
                    BinaryOpKind::Gt => ">", BinaryOpKind::Lt => "<",
                    BinaryOpKind::GtEq => ">=", BinaryOpKind::LtEq => "<=",
                    BinaryOpKind::Eq => "==", BinaryOpKind::NotEq => "!=",
                    _ => unreachable!(),
                },
                _ => unreachable!(),
            };

            let mut output = String::new();
            writeln!(output, "{}if {}[({} - 1) as usize] {} {}[({} - 1) as usize] {{",
                indent_str, arr_name, idx1_str, op_str, arr_name, idx2_str,
            ).unwrap();
            writeln!(output, "{}    {}.swap(({} - 1) as usize, ({} - 1) as usize);",
                indent_str, arr_name, idx1_str, idx2_str).unwrap();
            writeln!(output, "{}}}", indent_str).unwrap();

            Some((output, 2)) // consumed 2 extra statements
        }
        _ => None,
    }
}

pub fn codegen_stmt<'a>(
    stmt: &Stmt<'a>,
    interner: &Interner,
    indent: usize,
    mutable_vars: &HashSet<Symbol>,
    ctx: &mut RefinementContext<'a>,
    lww_fields: &HashSet<(String, String)>,
    mv_fields: &HashSet<(String, String)>,  // Phase 49b: MVRegister fields (no timestamp)
    synced_vars: &mut HashSet<Symbol>,  // Phase 52: Track synced variables
    var_caps: &HashMap<Symbol, VariableCapabilities>,  // Phase 56: Mount+Sync detection
    async_functions: &HashSet<Symbol>,  // Phase 54: Functions that are async
    pipe_vars: &HashSet<Symbol>,  // Phase 54: Pipe declarations (have _tx/_rx suffixes)
    boxed_fields: &HashSet<(String, String, String)>,  // Phase 102: Recursive enum fields
    registry: &TypeRegistry,  // Phase 103: For type annotations on polymorphic enums
) -> String {
    let indent_str = "    ".repeat(indent);
    let mut output = String::new();

    match stmt {
        Stmt::Let { var, ty, value, mutable } => {
            let var_name = interner.resolve(*var);

            // Register collection type for direct indexing optimization.
            // Check explicit type annotation first, then infer from Expr::New.
            if let Some(TypeExpr::Generic { base, params }) = ty {
                let base_name = interner.resolve(*base);
                match base_name {
                    "Seq" | "List" | "Vec" => {
                        let rust_type = if !params.is_empty() {
                            format!("Vec<{}>", codegen_type_expr(&params[0], interner))
                        } else {
                            "Vec<()>".to_string()
                        };
                        ctx.register_variable_type(*var, rust_type);
                    }
                    "Map" | "HashMap" => {
                        let rust_type = if params.len() >= 2 {
                            format!("std::collections::HashMap<{}, {}>", codegen_type_expr(&params[0], interner), codegen_type_expr(&params[1], interner))
                        } else {
                            "std::collections::HashMap<String, String>".to_string()
                        };
                        ctx.register_variable_type(*var, rust_type);
                    }
                    _ => {}
                }
            } else if let Expr::New { type_name, type_args, .. } = value {
                let type_str = interner.resolve(*type_name);
                match type_str {
                    "Seq" | "List" | "Vec" => {
                        let rust_type = if !type_args.is_empty() {
                            format!("Vec<{}>", codegen_type_expr(&type_args[0], interner))
                        } else {
                            "Vec<()>".to_string()
                        };
                        ctx.register_variable_type(*var, rust_type);
                    }
                    "Map" | "HashMap" => {
                        let rust_type = if type_args.len() >= 2 {
                            format!("std::collections::HashMap<{}, {}>", codegen_type_expr(&type_args[0], interner), codegen_type_expr(&type_args[1], interner))
                        } else {
                            "std::collections::HashMap<String, String>".to_string()
                        };
                        ctx.register_variable_type(*var, rust_type);
                    }
                    _ => {}
                }
            } else if let Expr::List(items) = value {
                // Infer element type from first literal in the list for Copy elimination
                let elem_type = items.first()
                    .map(|e| infer_rust_type_from_expr(e, interner))
                    .unwrap_or_else(|| "_".to_string());
                ctx.register_variable_type(*var, format!("Vec<{}>", elem_type));
            }

            // Phase 54+: Use codegen_expr_boxed with string+type tracking for proper codegen
            let value_str = codegen_expr_boxed_with_types(
                value, interner, synced_vars, boxed_fields, registry, async_functions,
                ctx.get_string_vars(), ctx.get_variable_types()
            );

            // Phase 103: Get explicit type annotation or infer for multi-param generic enums
            let type_annotation = ty.map(|t| codegen_type_expr(t, interner))
                .or_else(|| infer_variant_type_annotation(value, registry, interner));

            // Grand Challenge: Variable is mutable if explicitly marked OR if it's a Set target
            let is_mutable = *mutable || mutable_vars.contains(var);

            match (is_mutable, type_annotation) {
                (true, Some(t)) => writeln!(output, "{}let mut {}: {} = {};", indent_str, var_name, t, value_str).unwrap(),
                (true, None) => writeln!(output, "{}let mut {} = {};", indent_str, var_name, value_str).unwrap(),
                (false, Some(t)) => writeln!(output, "{}let {}: {} = {};", indent_str, var_name, t, value_str).unwrap(),
                (false, None) => writeln!(output, "{}let {} = {};", indent_str, var_name, value_str).unwrap(),
            }

            // Track string variables for proper concatenation in subsequent expressions
            if is_definitely_string_expr_with_vars(value, ctx.get_string_vars()) {
                ctx.register_string_var(*var);
            }

            // Phase 43C: Handle refinement type
            if let Some(TypeExpr::Refinement { base: _, var: bound_var, predicate }) = ty {
                emit_refinement_check(var_name, *bound_var, predicate, interner, &indent_str, &mut output);
                ctx.register(*var, *bound_var, predicate);
            }
        }

        Stmt::Set { target, value } => {
            let target_name = interner.resolve(*target);
            let string_vars = ctx.get_string_vars();
            let var_types = ctx.get_variable_types();

            // Optimization: detect self-append pattern (result = result + x + y)
            // and emit write!(result, "{}{}", x, y) instead of result = format!(...).
            // This is O(n) amortized (in-place append) vs O(n²) (full copy each iteration).
            let used_write = if ctx.is_string_var(*target)
                && is_definitely_string_expr_with_vars(value, string_vars)
            {
                let mut operands = Vec::new();
                collect_string_concat_operands(value, string_vars, &mut operands);

                // Need at least 2 operands, leftmost must be the target variable
                if operands.len() >= 2 && matches!(operands[0], Expr::Identifier(sym) if *sym == *target) {
                    // Check no other operand references target (would cause borrow conflict)
                    let tail = &operands[1..];
                    let mut tail_ids = HashSet::new();
                    for op in tail {
                        collect_expr_identifiers(op, &mut tail_ids);
                    }

                    if !tail_ids.contains(target) {
                        // Safe to emit write!() — target not referenced in tail operands
                        let placeholders: String = tail.iter().map(|_| "{}").collect::<Vec<_>>().join("");
                        let values: Vec<String> = tail.iter().map(|e| {
                            // String literals can be &str inside write!() — no heap allocation needed
                            if let Expr::Literal(Literal::Text(sym)) = e {
                                format!("\"{}\"", interner.resolve(*sym))
                            } else {
                                codegen_expr_boxed_with_types(
                                    e, interner, synced_vars, boxed_fields, registry, async_functions,
                                    string_vars, var_types
                                )
                            }
                        }).collect();
                        writeln!(output, "{}write!({}, \"{}\", {}).unwrap();",
                            indent_str, target_name, placeholders, values.join(", ")).unwrap();
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            } else {
                false
            };

            if !used_write {
                // Fallback: standard assignment with format!
                let value_str = codegen_expr_boxed_with_types(
                    value, interner, synced_vars, boxed_fields, registry, async_functions,
                    string_vars, var_types
                );
                writeln!(output, "{}{} = {};", indent_str, target_name, value_str).unwrap();
            }

            // Phase 43C: Check if this variable has a refinement constraint
            if let Some((bound_var, predicate)) = ctx.get_constraint(*target) {
                emit_refinement_check(target_name, bound_var, predicate, interner, &indent_str, &mut output);
            }
        }

        Stmt::Call { function, args } => {
            let func_name = escape_rust_ident(interner.resolve(*function));
            let args_str: Vec<String> = args.iter().map(|a| codegen_expr_with_async(a, interner, synced_vars, async_functions, ctx.get_variable_types())).collect();
            // Add .await if calling an async function
            let await_suffix = if async_functions.contains(function) { ".await" } else { "" };
            writeln!(output, "{}{}({}){};", indent_str, func_name, args_str.join(", "), await_suffix).unwrap();
        }

        Stmt::If { cond, then_block, else_block } => {
            let cond_str = codegen_expr_with_async(cond, interner, synced_vars, async_functions, ctx.get_variable_types());
            writeln!(output, "{}if {} {{", indent_str, cond_str).unwrap();
            ctx.push_scope();
            for stmt in *then_block {
                output.push_str(&codegen_stmt(stmt, interner, indent + 1, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry));
            }
            ctx.pop_scope();
            if let Some(else_stmts) = else_block {
                writeln!(output, "{}}} else {{", indent_str).unwrap();
                ctx.push_scope();
                for stmt in *else_stmts {
                    output.push_str(&codegen_stmt(stmt, interner, indent + 1, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry));
                }
                ctx.pop_scope();
            }
            writeln!(output, "{}}}", indent_str).unwrap();
        }

        Stmt::While { cond, body, decreasing: _ } => {
            // decreasing is compile-time only, ignored at runtime
            let cond_str = codegen_expr_with_async(cond, interner, synced_vars, async_functions, ctx.get_variable_types());
            writeln!(output, "{}while {} {{", indent_str, cond_str).unwrap();
            ctx.push_scope();
            // Peephole: process body statements with peephole optimizations
            let body_refs: Vec<&Stmt> = body.iter().collect();
            let mut bi = 0;
            while bi < body_refs.len() {
                if let Some((code, skip)) = try_emit_vec_fill_pattern(&body_refs, bi, interner, indent + 1) {
                    output.push_str(&code);
                    bi += 1 + skip;
                    continue;
                }
                if let Some((code, skip)) = try_emit_for_range_pattern(&body_refs, bi, interner, indent + 1, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry) {
                    output.push_str(&code);
                    bi += 1 + skip;
                    continue;
                }
                if let Some((code, skip)) = try_emit_swap_pattern(&body_refs, bi, interner, indent + 1, ctx.get_variable_types()) {
                    output.push_str(&code);
                    bi += 1 + skip;
                    continue;
                }
                output.push_str(&codegen_stmt(body_refs[bi], interner, indent + 1, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry));
                bi += 1;
            }
            ctx.pop_scope();
            writeln!(output, "{}}}", indent_str).unwrap();
        }

        Stmt::Repeat { pattern, iterable, body } => {
            use crate::ast::stmt::Pattern;

            // Generate pattern string for Rust code
            let pattern_str = match pattern {
                Pattern::Identifier(sym) => interner.resolve(*sym).to_string(),
                Pattern::Tuple(syms) => {
                    let names = syms.iter()
                        .map(|s| interner.resolve(*s))
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("({})", names)
                }
            };

            let iter_str = codegen_expr_with_async(iterable, interner, synced_vars, async_functions, ctx.get_variable_types());

            // Check if body contains async operations - if so, use while-let pattern
            // because standard for loops cannot contain .await
            let body_has_async = body.iter().any(|s| {
                requires_async_stmt(s) || calls_async_function(s, async_functions)
            });

            if body_has_async {
                // Use while-let with explicit iterator for async compatibility
                writeln!(output, "{}let mut __iter = ({}).into_iter();", indent_str, iter_str).unwrap();
                writeln!(output, "{}while let Some({}) = __iter.next() {{", indent_str, pattern_str).unwrap();
            } else {
                // Optimization: for known Vec<T> with Copy element type and non-mutating body,
                // use .iter().copied() instead of .clone() to avoid copying the entire collection.
                let use_iter_copied = if let Expr::Identifier(coll_sym) = iterable {
                    if let Some(coll_type) = ctx.get_variable_types().get(coll_sym) {
                        coll_type.starts_with("Vec") && has_copy_element_type(coll_type)
                            && !body_mutates_collection(body, *coll_sym)
                    } else {
                        false
                    }
                } else {
                    false
                };

                if use_iter_copied {
                    writeln!(output, "{}for {} in {}.iter().copied() {{", indent_str, pattern_str, iter_str).unwrap();
                } else {
                    // Clone the collection before iterating to avoid moving it.
                    // This allows the collection to be reused after the loop.
                    writeln!(output, "{}for {} in {}.clone() {{", indent_str, pattern_str, iter_str).unwrap();
                }
            }
            ctx.push_scope();
            // Peephole: process body statements with swap pattern detection
            {
                let body_refs: Vec<&Stmt> = body.iter().collect();
                let mut bi = 0;
                while bi < body_refs.len() {
                    if let Some((code, skip)) = try_emit_swap_pattern(&body_refs, bi, interner, indent + 1, ctx.get_variable_types()) {
                        output.push_str(&code);
                        bi += 1 + skip;
                        continue;
                    }
                    output.push_str(&codegen_stmt(body_refs[bi], interner, indent + 1, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry));
                    bi += 1;
                }
            }
            ctx.pop_scope();
            writeln!(output, "{}}}", indent_str).unwrap();
        }

        Stmt::Return { value } => {
            if let Some(v) = value {
                let value_str = codegen_expr_with_async(v, interner, synced_vars, async_functions, ctx.get_variable_types());
                writeln!(output, "{}return {};", indent_str, value_str).unwrap();
            } else {
                writeln!(output, "{}return;", indent_str).unwrap();
            }
        }

        Stmt::Assert { proposition } => {
            let condition = codegen_assertion(proposition, interner);
            writeln!(output, "{}debug_assert!({});", indent_str, condition).unwrap();
        }

        // Phase 35: Trust with documented justification
        Stmt::Trust { proposition, justification } => {
            let reason = interner.resolve(*justification);
            // Strip quotes if present (string literals include their quotes)
            let reason_clean = reason.trim_matches('"');
            writeln!(output, "{}// TRUST: {}", indent_str, reason_clean).unwrap();
            let condition = codegen_assertion(proposition, interner);
            writeln!(output, "{}debug_assert!({});", indent_str, condition).unwrap();
        }

        Stmt::RuntimeAssert { condition } => {
            let cond_str = codegen_expr_with_async(condition, interner, synced_vars, async_functions, ctx.get_variable_types());
            writeln!(output, "{}debug_assert!({});", indent_str, cond_str).unwrap();
        }

        // Phase 50: Security Check - mandatory runtime guard (NEVER optimized out)
        Stmt::Check { subject, predicate, is_capability, object, source_text, span } => {
            let subj_name = interner.resolve(*subject);
            let pred_name = interner.resolve(*predicate).to_lowercase();

            let call = if *is_capability {
                let obj_sym = object.expect("capability must have object");
                let obj_word = interner.resolve(obj_sym);

                // Phase 50: Type-based resolution
                // "Check that user can publish the document" -> find variable of type Document
                // First try to find a variable whose type matches the object word
                let obj_name = ctx.find_variable_by_type(obj_word, interner)
                    .unwrap_or_else(|| obj_word.to_string());

                format!("{}.can_{}(&{})", subj_name, pred_name, obj_name)
            } else {
                format!("{}.is_{}()", subj_name, pred_name)
            };

            writeln!(output, "{}if !({}) {{", indent_str, call).unwrap();
            writeln!(output, "{}    logicaffeine_system::panic_with(\"Security Check Failed at line {}: {}\");",
                     indent_str, span.start, source_text).unwrap();
            writeln!(output, "{}}}", indent_str).unwrap();
        }

        // Phase 51: P2P Networking - Listen on network address
        Stmt::Listen { address } => {
            let addr_str = codegen_expr_with_async(address, interner, synced_vars, async_functions, ctx.get_variable_types());
            // Pass &str instead of String
            writeln!(output, "{}logicaffeine_system::network::listen(&{}).await.expect(\"Failed to listen\");",
                     indent_str, addr_str).unwrap();
        }

        // Phase 51: P2P Networking - Connect to remote peer
        Stmt::ConnectTo { address } => {
            let addr_str = codegen_expr_with_async(address, interner, synced_vars, async_functions, ctx.get_variable_types());
            // Pass &str instead of String
            writeln!(output, "{}logicaffeine_system::network::connect(&{}).await.expect(\"Failed to connect\");",
                     indent_str, addr_str).unwrap();
        }

        // Phase 51: P2P Networking - Create PeerAgent remote handle
        Stmt::LetPeerAgent { var, address } => {
            let var_name = interner.resolve(*var);
            let addr_str = codegen_expr_with_async(address, interner, synced_vars, async_functions, ctx.get_variable_types());
            // Pass &str instead of String
            writeln!(output, "{}let {} = logicaffeine_system::network::PeerAgent::new(&{}).expect(\"Invalid address\");",
                     indent_str, var_name, addr_str).unwrap();
        }

        // Phase 51: Sleep - supports Duration literals or milliseconds
        Stmt::Sleep { milliseconds } => {
            let expr_str = codegen_expr_with_async(milliseconds, interner, synced_vars, async_functions, ctx.get_variable_types());
            let inferred_type = infer_rust_type_from_expr(milliseconds, interner);

            if inferred_type == "std::time::Duration" {
                // Duration type: use directly (already a std::time::Duration)
                writeln!(output, "{}tokio::time::sleep({}).await;",
                         indent_str, expr_str).unwrap();
            } else {
                // Assume milliseconds (integer) - legacy behavior
                writeln!(output, "{}tokio::time::sleep(std::time::Duration::from_millis({} as u64)).await;",
                         indent_str, expr_str).unwrap();
            }
        }

        // Phase 52/56: Sync CRDT variable on topic
        Stmt::Sync { var, topic } => {
            let var_name = interner.resolve(*var);
            let topic_str = codegen_expr_with_async(topic, interner, synced_vars, async_functions, ctx.get_variable_types());

            // Phase 56: Check if this variable is also mounted
            if let Some(caps) = var_caps.get(var) {
                if caps.mounted {
                    // Both Mount and Sync: use Distributed<T>
                    // Mount statement will handle the Distributed::mount call
                    // Here we just track it as synced
                    synced_vars.insert(*var);
                    return output;  // Skip - Mount will emit Distributed<T>
                }
            }

            // Sync-only: use Synced<T>
            writeln!(
                output,
                "{}let {} = logicaffeine_system::crdt::Synced::new({}, &{}).await;",
                indent_str, var_name, var_name, topic_str
            ).unwrap();
            synced_vars.insert(*var);
        }

        // Phase 53/56: Mount persistent CRDT from journal
        Stmt::Mount { var, path } => {
            let var_name = interner.resolve(*var);
            let path_str = codegen_expr_with_async(path, interner, synced_vars, async_functions, ctx.get_variable_types());

            // Phase 56: Check if this variable is also synced
            if let Some(caps) = var_caps.get(var) {
                if caps.synced {
                    // Both Mount and Sync: use Distributed<T>
                    let topic_str = caps.sync_topic.as_ref()
                        .map(|s| s.as_str())
                        .unwrap_or("\"default\"");
                    writeln!(
                        output,
                        "{}let {} = logicaffeine_system::distributed::Distributed::mount(std::sync::Arc::new(vfs.clone()), &{}, Some({}.to_string())).await.expect(\"Failed to mount\");",
                        indent_str, var_name, path_str, topic_str
                    ).unwrap();
                    synced_vars.insert(*var);
                    return output;
                }
            }

            // Mount-only: use Persistent<T>
            writeln!(
                output,
                "{}let {} = logicaffeine_system::storage::Persistent::mount(&vfs, &{}).await.expect(\"Failed to mount\");",
                indent_str, var_name, path_str
            ).unwrap();
            synced_vars.insert(*var);
        }

        // =====================================================================
        // Phase 54: Go-like Concurrency Codegen
        // =====================================================================

        Stmt::LaunchTask { function, args } => {
            let fn_name = escape_rust_ident(interner.resolve(*function));
            // Phase 54: When passing a pipe variable, pass the sender (_tx)
            let args_str: Vec<String> = args.iter()
                .map(|a| {
                    if let Expr::Identifier(sym) = a {
                        if pipe_vars.contains(sym) {
                            return format!("{}_tx.clone()", interner.resolve(*sym));
                        }
                    }
                    codegen_expr_with_async(a, interner, synced_vars, async_functions, ctx.get_variable_types())
                })
                .collect();
            // Phase 54: Add .await only if the function is async
            let await_suffix = if async_functions.contains(function) { ".await" } else { "" };
            writeln!(
                output,
                "{}tokio::spawn(async move {{ {}({}){await_suffix}; }});",
                indent_str, fn_name, args_str.join(", ")
            ).unwrap();
        }

        Stmt::LaunchTaskWithHandle { handle, function, args } => {
            let handle_name = interner.resolve(*handle);
            let fn_name = escape_rust_ident(interner.resolve(*function));
            // Phase 54: When passing a pipe variable, pass the sender (_tx)
            let args_str: Vec<String> = args.iter()
                .map(|a| {
                    if let Expr::Identifier(sym) = a {
                        if pipe_vars.contains(sym) {
                            return format!("{}_tx.clone()", interner.resolve(*sym));
                        }
                    }
                    codegen_expr_with_async(a, interner, synced_vars, async_functions, ctx.get_variable_types())
                })
                .collect();
            // Phase 54: Add .await only if the function is async
            let await_suffix = if async_functions.contains(function) { ".await" } else { "" };
            writeln!(
                output,
                "{}let {} = tokio::spawn(async move {{ {}({}){await_suffix} }});",
                indent_str, handle_name, fn_name, args_str.join(", ")
            ).unwrap();
        }

        Stmt::CreatePipe { var, element_type, capacity } => {
            let var_name = interner.resolve(*var);
            let type_name = interner.resolve(*element_type);
            let cap = capacity.unwrap_or(32);
            // Map LOGOS types to Rust types
            let rust_type = match type_name {
                "Int" => "i64",
                "Nat" => "u64",
                "Text" => "String",
                "Bool" => "bool",
                _ => type_name,
            };
            writeln!(
                output,
                "{}let ({}_tx, mut {}_rx) = tokio::sync::mpsc::channel::<{}>({});",
                indent_str, var_name, var_name, rust_type, cap
            ).unwrap();
        }

        Stmt::SendPipe { value, pipe } => {
            let val_str = codegen_expr_boxed_with_types(value, interner, synced_vars, boxed_fields, registry, async_functions, ctx.get_string_vars(), ctx.get_variable_types());
            let pipe_str = codegen_expr_with_async(pipe, interner, synced_vars, async_functions, ctx.get_variable_types());
            // Phase 54: Check if pipe is a local declaration (has _tx suffix) or parameter (no suffix)
            let is_local_pipe = if let Expr::Identifier(sym) = pipe {
                pipe_vars.contains(sym)
            } else {
                false
            };
            if is_local_pipe {
                writeln!(
                    output,
                    "{}{}_tx.send({}).await.expect(\"pipe send failed\");",
                    indent_str, pipe_str, val_str
                ).unwrap();
            } else {
                writeln!(
                    output,
                    "{}{}.send({}).await.expect(\"pipe send failed\");",
                    indent_str, pipe_str, val_str
                ).unwrap();
            }
        }

        Stmt::ReceivePipe { var, pipe } => {
            let var_name = interner.resolve(*var);
            let pipe_str = codegen_expr_with_async(pipe, interner, synced_vars, async_functions, ctx.get_variable_types());
            // Phase 54: Check if pipe is a local declaration (has _rx suffix) or parameter (no suffix)
            let is_local_pipe = if let Expr::Identifier(sym) = pipe {
                pipe_vars.contains(sym)
            } else {
                false
            };
            if is_local_pipe {
                writeln!(
                    output,
                    "{}let {} = {}_rx.recv().await.expect(\"pipe closed\");",
                    indent_str, var_name, pipe_str
                ).unwrap();
            } else {
                writeln!(
                    output,
                    "{}let {} = {}.recv().await.expect(\"pipe closed\");",
                    indent_str, var_name, pipe_str
                ).unwrap();
            }
        }

        Stmt::TrySendPipe { value, pipe, result } => {
            let val_str = codegen_expr_boxed_with_types(value, interner, synced_vars, boxed_fields, registry, async_functions, ctx.get_string_vars(), ctx.get_variable_types());
            let pipe_str = codegen_expr_with_async(pipe, interner, synced_vars, async_functions, ctx.get_variable_types());
            // Phase 54: Check if pipe is a local declaration
            let is_local_pipe = if let Expr::Identifier(sym) = pipe {
                pipe_vars.contains(sym)
            } else {
                false
            };
            let suffix = if is_local_pipe { "_tx" } else { "" };
            if let Some(res) = result {
                let res_name = interner.resolve(*res);
                writeln!(
                    output,
                    "{}let {} = {}{}.try_send({}).is_ok();",
                    indent_str, res_name, pipe_str, suffix, val_str
                ).unwrap();
            } else {
                writeln!(
                    output,
                    "{}let _ = {}{}.try_send({});",
                    indent_str, pipe_str, suffix, val_str
                ).unwrap();
            }
        }

        Stmt::TryReceivePipe { var, pipe } => {
            let var_name = interner.resolve(*var);
            let pipe_str = codegen_expr_with_async(pipe, interner, synced_vars, async_functions, ctx.get_variable_types());
            // Phase 54: Check if pipe is a local declaration
            let is_local_pipe = if let Expr::Identifier(sym) = pipe {
                pipe_vars.contains(sym)
            } else {
                false
            };
            let suffix = if is_local_pipe { "_rx" } else { "" };
            writeln!(
                output,
                "{}let {} = {}{}.try_recv().ok();",
                indent_str, var_name, pipe_str, suffix
            ).unwrap();
        }

        Stmt::StopTask { handle } => {
            let handle_str = codegen_expr_with_async(handle, interner, synced_vars, async_functions, ctx.get_variable_types());
            writeln!(output, "{}{}.abort();", indent_str, handle_str).unwrap();
        }

        Stmt::Select { branches } => {
            use crate::ast::stmt::SelectBranch;

            writeln!(output, "{}tokio::select! {{", indent_str).unwrap();
            for branch in branches {
                match branch {
                    SelectBranch::Receive { var, pipe, body } => {
                        let var_name = interner.resolve(*var);
                        let pipe_str = codegen_expr_with_async(pipe, interner, synced_vars, async_functions, ctx.get_variable_types());
                        // Check if pipe is a local declaration (has _rx suffix) or a parameter (no suffix)
                        let is_local_pipe = if let Expr::Identifier(sym) = pipe {
                            pipe_vars.contains(sym)
                        } else {
                            false
                        };
                        let suffix = if is_local_pipe { "_rx" } else { "" };
                        writeln!(
                            output,
                            "{}    {} = {}{}.recv() => {{",
                            indent_str, var_name, pipe_str, suffix
                        ).unwrap();
                        writeln!(
                            output,
                            "{}        if let Some({}) = {} {{",
                            indent_str, var_name, var_name
                        ).unwrap();
                        for stmt in *body {
                            let stmt_code = codegen_stmt(stmt, interner, indent + 3, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry);
                            write!(output, "{}", stmt_code).unwrap();
                        }
                        writeln!(output, "{}        }}", indent_str).unwrap();
                        writeln!(output, "{}    }}", indent_str).unwrap();
                    }
                    SelectBranch::Timeout { milliseconds, body } => {
                        let ms_str = codegen_expr_with_async(milliseconds, interner, synced_vars, async_functions, ctx.get_variable_types());
                        // Convert seconds to milliseconds if the value looks like seconds
                        writeln!(
                            output,
                            "{}    _ = tokio::time::sleep(std::time::Duration::from_secs({} as u64)) => {{",
                            indent_str, ms_str
                        ).unwrap();
                        for stmt in *body {
                            let stmt_code = codegen_stmt(stmt, interner, indent + 2, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry);
                            write!(output, "{}", stmt_code).unwrap();
                        }
                        writeln!(output, "{}    }}", indent_str).unwrap();
                    }
                }
            }
            writeln!(output, "{}}}", indent_str).unwrap();
        }

        Stmt::Give { object, recipient } => {
            // Move semantics: pass ownership without borrowing
            let obj_str = codegen_expr_with_async(object, interner, synced_vars, async_functions, ctx.get_variable_types());
            let recv_str = codegen_expr_with_async(recipient, interner, synced_vars, async_functions, ctx.get_variable_types());
            writeln!(output, "{}{}({});", indent_str, recv_str, obj_str).unwrap();
        }

        Stmt::Show { object, recipient } => {
            // Borrow semantics: pass immutable reference
            // Use string_vars for proper concatenation of string variables
            let obj_str = codegen_expr_with_async_and_strings(object, interner, synced_vars, async_functions, ctx.get_string_vars(), ctx.get_variable_types());
            let recv_str = codegen_expr_with_async(recipient, interner, synced_vars, async_functions, ctx.get_variable_types());
            writeln!(output, "{}{}(&{});", indent_str, recv_str, obj_str).unwrap();
        }

        Stmt::SetField { object, field, value } => {
            let obj_str = codegen_expr_with_async(object, interner, synced_vars, async_functions, ctx.get_variable_types());
            let field_name = interner.resolve(*field);
            let value_str = codegen_expr_boxed_with_types(value, interner, synced_vars, boxed_fields, registry, async_functions, ctx.get_string_vars(), ctx.get_variable_types());

            // Phase 49: Check if this field is an LWWRegister or MVRegister
            // LWW needs .set(value, timestamp), MV needs .set(value)
            let is_lww = lww_fields.iter().any(|(_, f)| f == field_name);
            let is_mv = mv_fields.iter().any(|(_, f)| f == field_name);
            if is_lww {
                // LWWRegister needs a timestamp - use current system time in microseconds
                writeln!(output, "{}{}.{}.set({}, std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_micros() as u64);", indent_str, obj_str, field_name, value_str).unwrap();
            } else if is_mv {
                // MVRegister just needs the value
                writeln!(output, "{}{}.{}.set({});", indent_str, obj_str, field_name, value_str).unwrap();
            } else {
                writeln!(output, "{}{}.{} = {};", indent_str, obj_str, field_name, value_str).unwrap();
            }
        }

        Stmt::StructDef { .. } => {
            // Struct definitions are handled in codegen_program, not here
        }

        Stmt::FunctionDef { .. } => {
            // Function definitions are handled in codegen_program, not here
        }

        Stmt::Inspect { target, arms, .. } => {
            let target_str = codegen_expr_with_async(target, interner, synced_vars, async_functions, ctx.get_variable_types());

            // Phase 102: Track which bindings come from boxed fields for inner Inspects
            // Use NAMES (strings) not symbols, because parser may create different symbols
            // for the same identifier in different syntactic positions.
            let mut inner_boxed_binding_names: HashSet<String> = HashSet::new();

            writeln!(output, "{}match {} {{", indent_str, target_str).unwrap();

            for arm in arms {
                if let Some(variant) = arm.variant {
                    let variant_name = interner.resolve(variant);
                    // Get the enum name from the arm, or fallback to just variant name
                    let enum_name_str = arm.enum_name.map(|e| interner.resolve(e));
                    let enum_prefix = enum_name_str
                        .map(|e| format!("{}::", e))
                        .unwrap_or_default();

                    if arm.bindings.is_empty() {
                        // Unit variant pattern
                        writeln!(output, "{}    {}{} => {{", indent_str, enum_prefix, variant_name).unwrap();
                    } else {
                        // Pattern with bindings
                        // Phase 102: Check which bindings are from boxed fields
                        let bindings_str: Vec<String> = arm.bindings.iter()
                            .map(|(field, binding)| {
                                let field_name = interner.resolve(*field);
                                let binding_name = interner.resolve(*binding);

                                // Check if this field is boxed
                                if let Some(enum_name) = enum_name_str {
                                    let key = (enum_name.to_string(), variant_name.to_string(), field_name.to_string());
                                    if boxed_fields.contains(&key) {
                                        inner_boxed_binding_names.insert(binding_name.to_string());
                                    }
                                }

                                if field_name == binding_name {
                                    field_name.to_string()
                                } else {
                                    format!("{}: {}", field_name, binding_name)
                                }
                            })
                            .collect();
                        writeln!(output, "{}    {}{} {{ {} }} => {{", indent_str, enum_prefix, variant_name, bindings_str.join(", ")).unwrap();
                    }
                } else {
                    // Otherwise (wildcard) pattern
                    writeln!(output, "{}    _ => {{", indent_str).unwrap();
                }

                ctx.push_scope();

                // Generate explicit dereferences for boxed bindings at the start of the arm
                // This makes them usable as regular values in the rest of the body
                for binding_name in &inner_boxed_binding_names {
                    writeln!(output, "{}        let {} = (*{}).clone();", indent_str, binding_name, binding_name).unwrap();
                }

                for stmt in arm.body {
                    // Phase 102: Handle inner Inspect statements with boxed bindings
                    // Note: Since we now dereference boxed bindings at the start of the arm,
                    // inner matches don't need the `*` dereference operator.
                    let inner_stmt_code = if let Stmt::Inspect { target: inner_target, .. } = stmt {
                        // Check if the inner target is a boxed binding (already dereferenced above)
                        // Use name comparison since symbols may differ between binding and reference
                        if let Expr::Identifier(sym) = inner_target {
                            let target_name = interner.resolve(*sym);
                            if inner_boxed_binding_names.contains(target_name) {
                                // Generate match (binding was already dereferenced at arm start)
                                let mut inner_output = String::new();
                                writeln!(inner_output, "{}match {} {{", "    ".repeat(indent + 2), target_name).unwrap();

                                if let Stmt::Inspect { arms: inner_arms, .. } = stmt {
                                    for inner_arm in inner_arms.iter() {
                                        if let Some(v) = inner_arm.variant {
                                            let v_name = interner.resolve(v);
                                            let inner_enum_prefix = inner_arm.enum_name
                                                .map(|e| format!("{}::", interner.resolve(e)))
                                                .unwrap_or_default();

                                            if inner_arm.bindings.is_empty() {
                                                writeln!(inner_output, "{}    {}{} => {{", "    ".repeat(indent + 2), inner_enum_prefix, v_name).unwrap();
                                            } else {
                                                let bindings: Vec<String> = inner_arm.bindings.iter()
                                                    .map(|(f, b)| {
                                                        let fn_name = interner.resolve(*f);
                                                        let bn_name = interner.resolve(*b);
                                                        if fn_name == bn_name { fn_name.to_string() }
                                                        else { format!("{}: {}", fn_name, bn_name) }
                                                    })
                                                    .collect();
                                                writeln!(inner_output, "{}    {}{} {{ {} }} => {{", "    ".repeat(indent + 2), inner_enum_prefix, v_name, bindings.join(", ")).unwrap();
                                            }
                                        } else {
                                            writeln!(inner_output, "{}    _ => {{", "    ".repeat(indent + 2)).unwrap();
                                        }

                                        ctx.push_scope();
                                        for inner_stmt in inner_arm.body {
                                            inner_output.push_str(&codegen_stmt(inner_stmt, interner, indent + 4, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry));
                                        }
                                        ctx.pop_scope();
                                        writeln!(inner_output, "{}    }}", "    ".repeat(indent + 2)).unwrap();
                                    }
                                }
                                writeln!(inner_output, "{}}}", "    ".repeat(indent + 2)).unwrap();
                                inner_output
                            } else {
                                codegen_stmt(stmt, interner, indent + 2, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry)
                            }
                        } else {
                            codegen_stmt(stmt, interner, indent + 2, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry)
                        }
                    } else {
                        codegen_stmt(stmt, interner, indent + 2, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry)
                    };
                    output.push_str(&inner_stmt_code);
                }
                ctx.pop_scope();
                writeln!(output, "{}    }}", indent_str).unwrap();
            }

            writeln!(output, "{}}}", indent_str).unwrap();
        }

        Stmt::Push { value, collection } => {
            let val_str = codegen_expr_boxed_with_types(value, interner, synced_vars, boxed_fields, registry, async_functions, ctx.get_string_vars(), ctx.get_variable_types());
            let coll_str = codegen_expr_with_async(collection, interner, synced_vars, async_functions, ctx.get_variable_types());
            writeln!(output, "{}{}.push({});", indent_str, coll_str, val_str).unwrap();
        }

        Stmt::Pop { collection, into } => {
            let coll_str = codegen_expr_with_async(collection, interner, synced_vars, async_functions, ctx.get_variable_types());
            match into {
                Some(var) => {
                    let var_name = interner.resolve(*var);
                    // Unwrap the Option returned by pop() - panics if empty
                    writeln!(output, "{}let {} = {}.pop().expect(\"Pop from empty collection\");", indent_str, var_name, coll_str).unwrap();
                }
                None => {
                    writeln!(output, "{}{}.pop();", indent_str, coll_str).unwrap();
                }
            }
        }

        Stmt::Add { value, collection } => {
            let val_str = codegen_expr_boxed_with_types(value, interner, synced_vars, boxed_fields, registry, async_functions, ctx.get_string_vars(), ctx.get_variable_types());
            let coll_str = codegen_expr_with_async(collection, interner, synced_vars, async_functions, ctx.get_variable_types());
            writeln!(output, "{}{}.insert({});", indent_str, coll_str, val_str).unwrap();
        }

        Stmt::Remove { value, collection } => {
            let val_str = codegen_expr_boxed_with_types(value, interner, synced_vars, boxed_fields, registry, async_functions, ctx.get_string_vars(), ctx.get_variable_types());
            let coll_str = codegen_expr_with_async(collection, interner, synced_vars, async_functions, ctx.get_variable_types());
            writeln!(output, "{}{}.remove(&{});", indent_str, coll_str, val_str).unwrap();
        }

        Stmt::SetIndex { collection, index, value } => {
            let coll_str = codegen_expr_with_async(collection, interner, synced_vars, async_functions, ctx.get_variable_types());
            let index_str = codegen_expr_with_async(index, interner, synced_vars, async_functions, ctx.get_variable_types());
            let value_str = codegen_expr_boxed_with_types(value, interner, synced_vars, boxed_fields, registry, async_functions, ctx.get_string_vars(), ctx.get_variable_types());

            // Direct indexing for known collection types (avoids trait dispatch)
            let known_type = if let Expr::Identifier(sym) = collection {
                ctx.get_variable_types().get(sym).map(|s| s.as_str())
            } else {
                None
            };

            match known_type {
                Some(t) if t.starts_with("Vec") => {
                    // Evaluate value first if it references the same collection (borrow safety)
                    if value_str.contains(&coll_str) {
                        writeln!(output, "{}let __set_tmp = {};", indent_str, value_str).unwrap();
                        writeln!(output, "{}{}[({} - 1) as usize] = __set_tmp;", indent_str, coll_str, index_str).unwrap();
                    } else {
                        writeln!(output, "{}{}[({} - 1) as usize] = {};", indent_str, coll_str, index_str, value_str).unwrap();
                    }
                }
                Some(t) if t.starts_with("std::collections::HashMap") || t.starts_with("HashMap") => {
                    writeln!(output, "{}{}.insert({}, {});", indent_str, coll_str, index_str, value_str).unwrap();
                }
                _ => {
                    // Fallback: polymorphic indexing via trait
                    if value_str.contains("logos_get") && value_str.contains(&coll_str) {
                        writeln!(output, "{}let __set_tmp = {};", indent_str, value_str).unwrap();
                        writeln!(output, "{}LogosIndexMut::logos_set(&mut {}, {}, __set_tmp);", indent_str, coll_str, index_str).unwrap();
                    } else {
                        writeln!(output, "{}LogosIndexMut::logos_set(&mut {}, {}, {});", indent_str, coll_str, index_str, value_str).unwrap();
                    }
                }
            }
        }

        // Phase 8.5: Zone (memory arena) block
        Stmt::Zone { name, capacity, source_file, body } => {
            let zone_name = interner.resolve(*name);

            // Generate zone creation based on type
            if let Some(path_sym) = source_file {
                // Memory-mapped file zone
                let path = interner.resolve(*path_sym);
                writeln!(
                    output,
                    "{}let {} = logicaffeine_system::memory::Zone::new_mapped(\"{}\").expect(\"Failed to map file\");",
                    indent_str, zone_name, path
                ).unwrap();
            } else {
                // Heap arena zone
                let cap = capacity.unwrap_or(4096); // Default 4KB
                writeln!(
                    output,
                    "{}let {} = logicaffeine_system::memory::Zone::new_heap({});",
                    indent_str, zone_name, cap
                ).unwrap();
            }

            // Open block scope
            writeln!(output, "{}{{", indent_str).unwrap();
            ctx.push_scope();

            // Generate body statements
            for stmt in *body {
                output.push_str(&codegen_stmt(stmt, interner, indent + 1, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry));
            }

            ctx.pop_scope();
            writeln!(output, "{}}}", indent_str).unwrap();
        }

        // Phase 9: Concurrent execution block (async, I/O-bound)
        // Generates tokio::join! for concurrent task execution
        // Phase 51: Variables used across multiple tasks are cloned to avoid move issues
        Stmt::Concurrent { tasks } => {
            // Collect Let statements to generate tuple destructuring
            let let_bindings: Vec<_> = tasks.iter().filter_map(|s| {
                if let Stmt::Let { var, .. } = s {
                    Some(interner.resolve(*var).to_string())
                } else {
                    None
                }
            }).collect();

            // Collect variables DEFINED in this block (to exclude from cloning)
            let defined_vars: HashSet<Symbol> = tasks.iter().filter_map(|s| {
                if let Stmt::Let { var, .. } = s {
                    Some(*var)
                } else {
                    None
                }
            }).collect();

            // Check if there are intra-block dependencies (a later task uses a var from earlier task)
            // If so, fall back to sequential execution
            let mut has_intra_dependency = false;
            let mut seen_defs: HashSet<Symbol> = HashSet::new();
            for s in *tasks {
                // Check if this task uses any variable defined by previous tasks in this block
                let mut used_in_task: HashSet<Symbol> = HashSet::new();
                collect_stmt_identifiers(s, &mut used_in_task);
                for used_var in &used_in_task {
                    if seen_defs.contains(used_var) {
                        has_intra_dependency = true;
                        break;
                    }
                }
                // Track variables defined by this task
                if let Stmt::Let { var, .. } = s {
                    seen_defs.insert(*var);
                }
                if has_intra_dependency {
                    break;
                }
            }

            // Collect ALL variables used in task expressions (not just Call args)
            // Exclude variables defined within this block
            let mut used_syms: HashSet<Symbol> = HashSet::new();
            for s in *tasks {
                collect_stmt_identifiers(s, &mut used_syms);
            }
            // Remove variables that are defined in this block
            for def_var in &defined_vars {
                used_syms.remove(def_var);
            }
            let used_vars: HashSet<String> = used_syms.iter()
                .map(|sym| interner.resolve(*sym).to_string())
                .collect();

            // If there are intra-block dependencies, execute sequentially
            if has_intra_dependency {
                // Generate sequential Let bindings
                for stmt in *tasks {
                    output.push_str(&codegen_stmt(stmt, interner, indent, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry));
                }
            } else {
                // Generate concurrent execution with tokio::join!
                if !let_bindings.is_empty() {
                    // Generate tuple destructuring for concurrent Let bindings
                    writeln!(output, "{}let ({}) = tokio::join!(", indent_str, let_bindings.join(", ")).unwrap();
                } else {
                    writeln!(output, "{}tokio::join!(", indent_str).unwrap();
                }

                for (i, stmt) in tasks.iter().enumerate() {
                    // For Let statements, generate only the VALUE so the async block returns it
                    // For Call statements, generate the call with .await
                    let inner_code = match stmt {
                        Stmt::Let { value, .. } => {
                            // Return the value expression directly (not "let x = value;")
                            // Phase 54+: Use codegen_expr_with_async to handle all nested async calls
                            codegen_expr_with_async(value, interner, synced_vars, async_functions, ctx.get_variable_types())
                        }
                        Stmt::Call { function, args } => {
                            let func_name = interner.resolve(*function);
                            let args_str: Vec<String> = args.iter()
                                .map(|a| codegen_expr_with_async(a, interner, synced_vars, async_functions, ctx.get_variable_types()))
                                .collect();
                            // Only add .await for async functions
                            let await_suffix = if async_functions.contains(function) { ".await" } else { "" };
                            format!("{}({}){}", func_name, args_str.join(", "), await_suffix)
                        }
                        _ => {
                            // Fallback for other statement types
                            let inner = codegen_stmt(stmt, interner, indent + 1, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry);
                            inner.trim().to_string()
                        }
                    };

                    // For tasks that use shared variables, wrap in a block that clones them
                    if !used_vars.is_empty() && i < tasks.len() - 1 {
                        // Clone variables for all tasks except the last one
                        let clones: Vec<String> = used_vars.iter()
                            .map(|v| format!("let {} = {}.clone();", v, v))
                            .collect();
                        write!(output, "{}    {{ {} async move {{ {} }} }}",
                               indent_str, clones.join(" "), inner_code).unwrap();
                    } else {
                        // Last task can use original variables
                        write!(output, "{}    async {{ {} }}", indent_str, inner_code).unwrap();
                    }

                    if i < tasks.len() - 1 {
                        writeln!(output, ",").unwrap();
                    } else {
                        writeln!(output).unwrap();
                    }
                }

                writeln!(output, "{});", indent_str).unwrap();
            }
        }

        // Phase 9: Parallel execution block (CPU-bound)
        // Generates rayon::join for two tasks, or thread::spawn for 3+ tasks
        Stmt::Parallel { tasks } => {
            // Collect Let statements to generate tuple destructuring
            let let_bindings: Vec<_> = tasks.iter().filter_map(|s| {
                if let Stmt::Let { var, .. } = s {
                    Some(interner.resolve(*var).to_string())
                } else {
                    None
                }
            }).collect();

            if tasks.len() == 2 {
                // Use rayon::join for exactly 2 tasks
                if !let_bindings.is_empty() {
                    writeln!(output, "{}let ({}) = rayon::join(", indent_str, let_bindings.join(", ")).unwrap();
                } else {
                    writeln!(output, "{}rayon::join(", indent_str).unwrap();
                }

                for (i, stmt) in tasks.iter().enumerate() {
                    // For Let statements, generate only the VALUE so the closure returns it
                    let inner_code = match stmt {
                        Stmt::Let { value, .. } => {
                            // Return the value expression directly (not "let x = value;")
                            codegen_expr(value, interner, synced_vars)
                        }
                        Stmt::Call { function, args } => {
                            let func_name = interner.resolve(*function);
                            let args_str: Vec<String> = args.iter()
                                .map(|a| codegen_expr_with_async(a, interner, synced_vars, async_functions, ctx.get_variable_types()))
                                .collect();
                            format!("{}({})", func_name, args_str.join(", "))
                        }
                        _ => {
                            // Fallback for other statement types
                            let inner = codegen_stmt(stmt, interner, indent + 1, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry);
                            inner.trim().to_string()
                        }
                    };
                    write!(output, "{}    || {{ {} }}", indent_str, inner_code).unwrap();
                    if i == 0 {
                        writeln!(output, ",").unwrap();
                    } else {
                        writeln!(output).unwrap();
                    }
                }
                writeln!(output, "{});", indent_str).unwrap();
            } else {
                // For 3+ tasks, use thread::spawn pattern
                writeln!(output, "{}{{", indent_str).unwrap();
                writeln!(output, "{}    let handles: Vec<_> = vec![", indent_str).unwrap();
                for stmt in *tasks {
                    // For Let statements, generate only the VALUE so the closure returns it
                    let inner_code = match stmt {
                        Stmt::Let { value, .. } => {
                            codegen_expr(value, interner, synced_vars)
                        }
                        Stmt::Call { function, args } => {
                            let func_name = interner.resolve(*function);
                            let args_str: Vec<String> = args.iter()
                                .map(|a| codegen_expr_with_async(a, interner, synced_vars, async_functions, ctx.get_variable_types()))
                                .collect();
                            format!("{}({})", func_name, args_str.join(", "))
                        }
                        _ => {
                            let inner = codegen_stmt(stmt, interner, indent + 2, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry);
                            inner.trim().to_string()
                        }
                    };
                    writeln!(output, "{}        std::thread::spawn(move || {{ {} }}),",
                             indent_str, inner_code).unwrap();
                }
                writeln!(output, "{}    ];", indent_str).unwrap();
                writeln!(output, "{}    for h in handles {{ h.join().unwrap(); }}", indent_str).unwrap();
                writeln!(output, "{}}}", indent_str).unwrap();
            }
        }

        // Phase 10: Read from console or file
        // Phase 53: File reads now use async VFS
        Stmt::ReadFrom { var, source } => {
            let var_name = interner.resolve(*var);
            match source {
                ReadSource::Console => {
                    writeln!(output, "{}let {} = logicaffeine_system::io::read_line();", indent_str, var_name).unwrap();
                }
                ReadSource::File(path_expr) => {
                    let path_str = codegen_expr_with_async(path_expr, interner, synced_vars, async_functions, ctx.get_variable_types());
                    // Phase 53: Use VFS with async
                    writeln!(
                        output,
                        "{}let {} = vfs.read_to_string(&{}).await.expect(\"Failed to read file\");",
                        indent_str, var_name, path_str
                    ).unwrap();
                }
            }
        }

        // Phase 10: Write to file
        // Phase 53: File writes now use async VFS
        Stmt::WriteFile { content, path } => {
            let content_str = codegen_expr_with_async(content, interner, synced_vars, async_functions, ctx.get_variable_types());
            let path_str = codegen_expr_with_async(path, interner, synced_vars, async_functions, ctx.get_variable_types());
            // Phase 53: Use VFS with async
            writeln!(
                output,
                "{}vfs.write(&{}, {}.as_bytes()).await.expect(\"Failed to write file\");",
                indent_str, path_str, content_str
            ).unwrap();
        }

        // Phase 46: Spawn an agent
        Stmt::Spawn { agent_type, name } => {
            let type_name = interner.resolve(*agent_type);
            let agent_name = interner.resolve(*name);
            // Generate agent spawn with tokio channel
            writeln!(
                output,
                "{}let {} = tokio::spawn(async move {{ /* {} agent loop */ }});",
                indent_str, agent_name, type_name
            ).unwrap();
        }

        // Phase 46: Send message to agent
        Stmt::SendMessage { message, destination } => {
            let msg_str = codegen_expr_with_async(message, interner, synced_vars, async_functions, ctx.get_variable_types());
            let dest_str = codegen_expr_with_async(destination, interner, synced_vars, async_functions, ctx.get_variable_types());
            writeln!(
                output,
                "{}{}.send({}).await.expect(\"Failed to send message\");",
                indent_str, dest_str, msg_str
            ).unwrap();
        }

        // Phase 46: Await response from agent
        Stmt::AwaitMessage { source, into } => {
            let src_str = codegen_expr_with_async(source, interner, synced_vars, async_functions, ctx.get_variable_types());
            let var_name = interner.resolve(*into);
            writeln!(
                output,
                "{}let {} = {}.recv().await.expect(\"Failed to receive message\");",
                indent_str, var_name, src_str
            ).unwrap();
        }

        // Phase 49: Merge CRDT state
        Stmt::MergeCrdt { source, target } => {
            let src_str = codegen_expr_with_async(source, interner, synced_vars, async_functions, ctx.get_variable_types());
            let tgt_str = codegen_expr_with_async(target, interner, synced_vars, async_functions, ctx.get_variable_types());
            writeln!(
                output,
                "{}{}.merge(&{});",
                indent_str, tgt_str, src_str
            ).unwrap();
        }

        // Phase 49: Increment GCounter
        // Phase 52: If object is synced, wrap in .mutate() for auto-publish
        Stmt::IncreaseCrdt { object, field, amount } => {
            let field_name = interner.resolve(*field);
            let amount_str = codegen_expr_with_async(amount, interner, synced_vars, async_functions, ctx.get_variable_types());

            // Check if the root object is synced
            let root_sym = get_root_identifier(object);
            if let Some(sym) = root_sym {
                if synced_vars.contains(&sym) {
                    // Synced: use .mutate() for auto-publish
                    let obj_name = interner.resolve(sym);
                    writeln!(
                        output,
                        "{}{}.mutate(|inner| inner.{}.increment({} as u64)).await;",
                        indent_str, obj_name, field_name, amount_str
                    ).unwrap();
                    return output;
                }
            }

            // Not synced: direct access
            let obj_str = codegen_expr_with_async(object, interner, synced_vars, async_functions, ctx.get_variable_types());
            writeln!(
                output,
                "{}{}.{}.increment({} as u64);",
                indent_str, obj_str, field_name, amount_str
            ).unwrap();
        }

        // Phase 49b: Decrement PNCounter
        Stmt::DecreaseCrdt { object, field, amount } => {
            let field_name = interner.resolve(*field);
            let amount_str = codegen_expr_with_async(amount, interner, synced_vars, async_functions, ctx.get_variable_types());

            // Check if the root object is synced
            let root_sym = get_root_identifier(object);
            if let Some(sym) = root_sym {
                if synced_vars.contains(&sym) {
                    // Synced: use .mutate() for auto-publish
                    let obj_name = interner.resolve(sym);
                    writeln!(
                        output,
                        "{}{}.mutate(|inner| inner.{}.decrement({} as u64)).await;",
                        indent_str, obj_name, field_name, amount_str
                    ).unwrap();
                    return output;
                }
            }

            // Not synced: direct access
            let obj_str = codegen_expr_with_async(object, interner, synced_vars, async_functions, ctx.get_variable_types());
            writeln!(
                output,
                "{}{}.{}.decrement({} as u64);",
                indent_str, obj_str, field_name, amount_str
            ).unwrap();
        }

        // Phase 49b: Append to SharedSequence (RGA)
        Stmt::AppendToSequence { sequence, value } => {
            let seq_str = codegen_expr_with_async(sequence, interner, synced_vars, async_functions, ctx.get_variable_types());
            let val_str = codegen_expr_boxed_with_types(value, interner, synced_vars, boxed_fields, registry, async_functions, ctx.get_string_vars(), ctx.get_variable_types());
            writeln!(
                output,
                "{}{}.append({});",
                indent_str, seq_str, val_str
            ).unwrap();
        }

        // Phase 49b: Resolve MVRegister conflicts
        Stmt::ResolveConflict { object, field, value } => {
            let field_name = interner.resolve(*field);
            let val_str = codegen_expr_boxed_with_types(value, interner, synced_vars, boxed_fields, registry, async_functions, ctx.get_string_vars(), ctx.get_variable_types());
            let obj_str = codegen_expr_with_async(object, interner, synced_vars, async_functions, ctx.get_variable_types());
            writeln!(
                output,
                "{}{}.{}.resolve({});",
                indent_str, obj_str, field_name, val_str
            ).unwrap();
        }

        // Escape hatch: emit raw foreign code wrapped in braces for scope isolation
        Stmt::Escape { code, .. } => {
            let raw_code = interner.resolve(*code);
            write!(output, "{}{{\n", indent_str).unwrap();
            for line in raw_code.lines() {
                write!(output, "{}    {}\n", indent_str, line).unwrap();
            }
            write!(output, "{}}}\n", indent_str).unwrap();
        }

        // Dependencies are metadata; no Rust code emitted.
        Stmt::Require { .. } => {}

        // Phase 63: Theorems are verified at compile-time, no runtime code generated
        Stmt::Theorem(_) => {
            // Theorems don't generate runtime code - they're processed separately
            // by compile_theorem() at the meta-level
        }
    }

    output
}

/// Phase 52: Extract the root identifier from an expression.
/// For `x.field.subfield`, returns `x`.
fn get_root_identifier(expr: &Expr) -> Option<Symbol> {
    match expr {
        Expr::Identifier(sym) => Some(*sym),
        Expr::FieldAccess { object, .. } => get_root_identifier(object),
        _ => None,
    }
}

/// Check if a type string represents a Copy type (no .clone() needed on indexing).
fn is_copy_type(ty: &str) -> bool {
    matches!(ty, "i64" | "u64" | "f64" | "i32" | "u32" | "f32" | "bool" | "char" | "u8" | "i8" | "()")
}

/// Check if a Vec<T> type has a Copy element type.
fn has_copy_element_type(vec_type: &str) -> bool {
    if let Some(inner) = vec_type.strip_prefix("Vec<").and_then(|s| s.strip_suffix('>')) {
        is_copy_type(inner)
    } else {
        false
    }
}

/// Check if a HashMap<K, V> type has a Copy value type.
fn has_copy_value_type(map_type: &str) -> bool {
    let inner = map_type.strip_prefix("std::collections::HashMap<")
        .or_else(|| map_type.strip_prefix("HashMap<"));
    if let Some(inner) = inner.and_then(|s| s.strip_suffix('>')) {
        // Split on ", " to get key and value types
        if let Some((_key, value)) = inner.split_once(", ") {
            return is_copy_type(value);
        }
    }
    false
}

pub fn codegen_expr(expr: &Expr, interner: &Interner, synced_vars: &HashSet<Symbol>) -> String {
    // Use empty registry, boxed_fields, and async_functions for simple expression codegen
    let empty_registry = TypeRegistry::new();
    let empty_async = HashSet::new();
    codegen_expr_boxed(expr, interner, synced_vars, &HashSet::new(), &empty_registry, &empty_async)
}

/// Phase 54+: Codegen expression with async function tracking.
/// Adds .await to async function calls at the expression level, handling nested calls.
pub fn codegen_expr_with_async(
    expr: &Expr,
    interner: &Interner,
    synced_vars: &HashSet<Symbol>,
    async_functions: &HashSet<Symbol>,
    variable_types: &HashMap<Symbol, String>,
) -> String {
    let empty_registry = TypeRegistry::new();
    let empty_strings = HashSet::new();
    codegen_expr_boxed_internal(expr, interner, synced_vars, &HashSet::new(), &empty_registry, async_functions, &HashSet::new(), &empty_strings, variable_types)
}

/// Codegen expression with async support and string variable tracking.
fn codegen_expr_with_async_and_strings(
    expr: &Expr,
    interner: &Interner,
    synced_vars: &HashSet<Symbol>,
    async_functions: &HashSet<Symbol>,
    string_vars: &HashSet<Symbol>,
    variable_types: &HashMap<Symbol, String>,
) -> String {
    let empty_registry = TypeRegistry::new();
    codegen_expr_boxed_internal(expr, interner, synced_vars, &HashSet::new(), &empty_registry, async_functions, &HashSet::new(), string_vars, variable_types)
}

/// Check if an expression is definitely numeric (safe to use + operator).
/// This is conservative for Add operations - treats it as string concat only
/// when clearly dealing with strings (string literals).
fn is_definitely_numeric_expr(expr: &Expr) -> bool {
    match expr {
        Expr::Literal(Literal::Number(_)) => true,
        Expr::Literal(Literal::Float(_)) => true,
        Expr::Literal(Literal::Duration(_)) => true,
        // Identifiers might be strings, but without a string literal nearby,
        // assume numeric (Rust will catch type errors)
        Expr::Identifier(_) => true,
        // Arithmetic operations are numeric
        Expr::BinaryOp { op: BinaryOpKind::Subtract, .. } => true,
        Expr::BinaryOp { op: BinaryOpKind::Multiply, .. } => true,
        Expr::BinaryOp { op: BinaryOpKind::Divide, .. } => true,
        Expr::BinaryOp { op: BinaryOpKind::Modulo, .. } => true,
        // Length always returns a number
        Expr::Length { .. } => true,
        // Add is numeric if both operands seem numeric
        Expr::BinaryOp { op: BinaryOpKind::Add, left, right } => {
            is_definitely_numeric_expr(left) && is_definitely_numeric_expr(right)
        }
        // Function calls - assume numeric (Rust type checker will validate)
        Expr::Call { .. } => true,
        // Index expressions - assume numeric
        Expr::Index { .. } => true,
        _ => true,
    }
}

/// Check if an expression is definitely a string (needs format! for concatenation).
/// Takes a set of known string variable symbols for identifier lookup.
fn is_definitely_string_expr_with_vars(expr: &Expr, string_vars: &HashSet<Symbol>) -> bool {
    match expr {
        // String literals are definitely strings
        Expr::Literal(Literal::Text(_)) => true,
        // Variables known to be strings
        Expr::Identifier(sym) => string_vars.contains(sym),
        // Concat always produces strings
        Expr::BinaryOp { op: BinaryOpKind::Concat, .. } => true,
        // Add with a string operand produces a string
        Expr::BinaryOp { op: BinaryOpKind::Add, left, right } => {
            is_definitely_string_expr_with_vars(left, string_vars)
                || is_definitely_string_expr_with_vars(right, string_vars)
        }
        // WithCapacity wrapping a string value is a string
        Expr::WithCapacity { value, .. } => is_definitely_string_expr_with_vars(value, string_vars),
        _ => false,
    }
}

/// Check if an expression is definitely a string (without variable tracking).
/// This is a fallback for contexts where string_vars isn't available.
fn is_definitely_string_expr(expr: &Expr) -> bool {
    let empty = HashSet::new();
    is_definitely_string_expr_with_vars(expr, &empty)
}

/// Collect leaf operands from a chain of string Add/Concat operations.
///
/// Walks left-leaning trees of `+` (on strings) and `Concat` operations,
/// collecting all leaf expressions into a flat Vec. This enables emitting
/// a single `format!("{}{}{}", a, b, c)` instead of nested
/// `format!("{}{}", format!("{}{}", a, b), c)`, avoiding O(n^2) allocation.
fn collect_string_concat_operands<'a, 'b>(
    expr: &'b Expr<'a>,
    string_vars: &HashSet<Symbol>,
    operands: &mut Vec<&'b Expr<'a>>,
) {
    match expr {
        Expr::BinaryOp { op: BinaryOpKind::Concat, left, right } => {
            collect_string_concat_operands(left, string_vars, operands);
            collect_string_concat_operands(right, string_vars, operands);
        }
        Expr::BinaryOp { op: BinaryOpKind::Add, left, right } => {
            let has_string = is_definitely_string_expr_with_vars(left, string_vars)
                || is_definitely_string_expr_with_vars(right, string_vars);
            if has_string {
                collect_string_concat_operands(left, string_vars, operands);
                collect_string_concat_operands(right, string_vars, operands);
            } else {
                operands.push(expr);
            }
        }
        _ => {
            operands.push(expr);
        }
    }
}

/// Phase 102: Codegen with boxed field support for recursive enums.
/// Phase 103: Added registry for polymorphic enum type inference.
/// Phase 54+: Added async_functions for proper .await on nested async calls.
fn codegen_expr_boxed(
    expr: &Expr,
    interner: &Interner,
    synced_vars: &HashSet<Symbol>,
    boxed_fields: &HashSet<(String, String, String)>,  // (EnumName, VariantName, FieldName)
    registry: &TypeRegistry,  // Phase 103: For type annotations on polymorphic enums
    async_functions: &HashSet<Symbol>,  // Phase 54+: Functions that are async
) -> String {
    // Delegate to codegen_expr_full with empty context for boxed bindings and string vars
    let empty_boxed = HashSet::new();
    let empty_strings = HashSet::new();
    let empty_types = HashMap::new();
    codegen_expr_boxed_internal(expr, interner, synced_vars, boxed_fields, registry, async_functions, &empty_boxed, &empty_strings, &empty_types)
}

/// Codegen with string variable tracking for proper string concatenation.
fn codegen_expr_boxed_with_strings(
    expr: &Expr,
    interner: &Interner,
    synced_vars: &HashSet<Symbol>,
    boxed_fields: &HashSet<(String, String, String)>,
    registry: &TypeRegistry,
    async_functions: &HashSet<Symbol>,
    string_vars: &HashSet<Symbol>,
) -> String {
    let empty_boxed = HashSet::new();
    let empty_types = HashMap::new();
    codegen_expr_boxed_internal(expr, interner, synced_vars, boxed_fields, registry, async_functions, &empty_boxed, string_vars, &empty_types)
}

/// Codegen with variable type tracking for direct collection indexing optimization.
fn codegen_expr_boxed_with_types(
    expr: &Expr,
    interner: &Interner,
    synced_vars: &HashSet<Symbol>,
    boxed_fields: &HashSet<(String, String, String)>,
    registry: &TypeRegistry,
    async_functions: &HashSet<Symbol>,
    string_vars: &HashSet<Symbol>,
    variable_types: &HashMap<Symbol, String>,
) -> String {
    let empty_boxed = HashSet::new();
    codegen_expr_boxed_internal(expr, interner, synced_vars, boxed_fields, registry, async_functions, &empty_boxed, string_vars, variable_types)
}

/// Internal implementation of codegen_expr_boxed that can handle extra context.
fn codegen_expr_boxed_internal(
    expr: &Expr,
    interner: &Interner,
    synced_vars: &HashSet<Symbol>,
    boxed_fields: &HashSet<(String, String, String)>,
    registry: &TypeRegistry,
    async_functions: &HashSet<Symbol>,
    boxed_bindings: &HashSet<Symbol>,
    string_vars: &HashSet<Symbol>,
    variable_types: &HashMap<Symbol, String>,
) -> String {
    // Helper macro for recursive calls with all context
    macro_rules! recurse {
        ($e:expr) => {
            codegen_expr_boxed_internal($e, interner, synced_vars, boxed_fields, registry, async_functions, boxed_bindings, string_vars, variable_types)
        };
    }

    match expr {
        Expr::Literal(lit) => codegen_literal(lit, interner),

        Expr::Identifier(sym) => {
            let name = interner.resolve(*sym).to_string();
            // Dereference boxed bindings from enum destructuring
            if boxed_bindings.contains(sym) {
                format!("(*{})", name)
            } else {
                name
            }
        }

        Expr::BinaryOp { op, left, right } => {
            // Flatten chained string concat/add into a single format! call.
            // Turns O(n^2) nested format! into O(n) single-allocation.
            let is_string_concat = matches!(op, BinaryOpKind::Concat)
                || (matches!(op, BinaryOpKind::Add)
                    && (is_definitely_string_expr_with_vars(left, string_vars)
                        || is_definitely_string_expr_with_vars(right, string_vars)));

            if is_string_concat {
                let mut operands = Vec::new();
                collect_string_concat_operands(expr, string_vars, &mut operands);
                let placeholders: String = operands.iter().map(|_| "{}").collect::<Vec<_>>().join("");
                let values: Vec<String> = operands.iter().map(|e| {
                    // String literals can be &str inside format!() — no heap allocation needed
                    if let Expr::Literal(Literal::Text(sym)) = e {
                        format!("\"{}\"", interner.resolve(*sym))
                    } else {
                        recurse!(e)
                    }
                }).collect();
                return format!("format!(\"{}\", {})", placeholders, values.join(", "));
            }

            // Optimize HashMap .get() for equality comparisons to avoid cloning
            if matches!(op, BinaryOpKind::Eq | BinaryOpKind::NotEq) {
                let neg = matches!(op, BinaryOpKind::NotEq);
                // Check if left side is a HashMap index
                if let Expr::Index { collection, index } = left {
                    if let Expr::Identifier(sym) = collection {
                        if let Some(t) = variable_types.get(sym) {
                            if t.starts_with("std::collections::HashMap") || t.starts_with("HashMap") {
                                let coll_str = recurse!(collection);
                                let key_str = recurse!(index);
                                let val_str = recurse!(right);
                                let cmp = if neg { "!=" } else { "==" };
                                if has_copy_value_type(t) {
                                    return format!("({}.get(&({})).copied() {} Some({}))", coll_str, key_str, cmp, val_str);
                                } else {
                                    return format!("({}.get(&({})) {} Some(&({})))", coll_str, key_str, cmp, val_str);
                                }
                            }
                        }
                    }
                }
                // Check if right side is a HashMap index
                if let Expr::Index { collection, index } = right {
                    if let Expr::Identifier(sym) = collection {
                        if let Some(t) = variable_types.get(sym) {
                            if t.starts_with("std::collections::HashMap") || t.starts_with("HashMap") {
                                let coll_str = recurse!(collection);
                                let key_str = recurse!(index);
                                let val_str = recurse!(left);
                                let cmp = if neg { "!=" } else { "==" };
                                if has_copy_value_type(t) {
                                    return format!("(Some({}) {} {}.get(&({})).copied())", val_str, cmp, coll_str, key_str);
                                } else {
                                    return format!("(Some(&({})) {} {}.get(&({})))", val_str, cmp, coll_str, key_str);
                                }
                            }
                        }
                    }
                }
            }

            let left_str = recurse!(left);
            let right_str = recurse!(right);
            let op_str = match op {
                BinaryOpKind::Add => "+",
                BinaryOpKind::Subtract => "-",
                BinaryOpKind::Multiply => "*",
                BinaryOpKind::Divide => "/",
                BinaryOpKind::Modulo => "%",
                BinaryOpKind::Eq => "==",
                BinaryOpKind::NotEq => "!=",
                BinaryOpKind::Lt => "<",
                BinaryOpKind::Gt => ">",
                BinaryOpKind::LtEq => "<=",
                BinaryOpKind::GtEq => ">=",
                BinaryOpKind::And => "&&",
                BinaryOpKind::Or => "||",
                BinaryOpKind::Concat => unreachable!(), // Handled above
            };
            format!("({} {} {})", left_str, op_str, right_str)
        }

        Expr::Call { function, args } => {
            let func_name = escape_rust_ident(interner.resolve(*function));
            // Recursively codegen args with full context
            let args_str: Vec<String> = args.iter()
                .map(|a| recurse!(a))
                .collect();
            // Add .await if this function is async
            if async_functions.contains(function) {
                format!("{}({}).await", func_name, args_str.join(", "))
            } else {
                format!("{}({})", func_name, args_str.join(", "))
            }
        }

        Expr::Index { collection, index } => {
            let coll_str = recurse!(collection);
            let index_str = recurse!(index);
            // Direct indexing for known collection types (avoids trait dispatch)
            let known_type = if let Expr::Identifier(sym) = collection {
                variable_types.get(sym).map(|s| s.as_str())
            } else {
                None
            };
            match known_type {
                Some(t) if t.starts_with("Vec") => {
                    let suffix = if has_copy_element_type(t) { "" } else { ".clone()" };
                    format!("{}[({} - 1) as usize]{}", coll_str, index_str, suffix)
                }
                Some(t) if t.starts_with("std::collections::HashMap") || t.starts_with("HashMap") => {
                    let suffix = if has_copy_value_type(t) { "" } else { ".clone()" };
                    format!("{}[&({})]{}", coll_str, index_str, suffix)
                }
                _ => {
                    // Fallback: polymorphic indexing via trait
                    format!("LogosIndex::logos_get(&{}, {})", coll_str, index_str)
                }
            }
        }

        Expr::Slice { collection, start, end } => {
            let coll_str = recurse!(collection);
            let start_str = recurse!(start);
            let end_str = recurse!(end);
            // Phase 43D: 1-indexed inclusive to 0-indexed exclusive
            // "items 1 through 3" → &items[0..3] (elements at indices 0, 1, 2)
            format!("&{}[({} - 1) as usize..{} as usize]", coll_str, start_str, end_str)
        }

        Expr::Copy { expr: inner } => {
            let expr_str = recurse!(inner);
            // Phase 43D: Explicit owned copy — .to_owned() is universal:
            // - &[T] (slices) → Vec<T> via [T]: ToOwned<Owned=Vec<T>>
            // - Vec<T>, HashMap<K,V>, HashSet<T> → Self via Clone blanket impl
            format!("{}.to_owned()", expr_str)
        }

        Expr::Give { value } => {
            // Ownership transfer: emit value without .clone()
            // The move semantics are implicit in Rust - no special syntax needed
            recurse!(value)
        }

        Expr::Length { collection } => {
            let coll_str = recurse!(collection);
            // Phase 43D: Collection length - cast to i64 for LOGOS integer semantics
            format!("({}.len() as i64)", coll_str)
        }

        Expr::Contains { collection, value } => {
            let coll_str = recurse!(collection);
            let val_str = recurse!(value);
            // Use LogosContains trait for unified contains across List, Set, Map, Text
            format!("{}.logos_contains(&{})", coll_str, val_str)
        }

        Expr::Union { left, right } => {
            let left_str = recurse!(left);
            let right_str = recurse!(right);
            format!("{}.union(&{}).cloned().collect::<std::collections::HashSet<_>>()", left_str, right_str)
        }

        Expr::Intersection { left, right } => {
            let left_str = recurse!(left);
            let right_str = recurse!(right);
            format!("{}.intersection(&{}).cloned().collect::<std::collections::HashSet<_>>()", left_str, right_str)
        }

        // Phase 48: Sipping Protocol expressions
        Expr::ManifestOf { zone } => {
            let zone_str = recurse!(zone);
            format!("logicaffeine_system::network::FileSipper::from_zone(&{}).manifest()", zone_str)
        }

        Expr::ChunkAt { index, zone } => {
            let zone_str = recurse!(zone);
            let index_str = recurse!(index);
            // LOGOS uses 1-indexed, Rust uses 0-indexed
            format!("logicaffeine_system::network::FileSipper::from_zone(&{}).get_chunk(({} - 1) as usize)", zone_str, index_str)
        }

        Expr::List(ref items) => {
            let item_strs: Vec<String> = items.iter()
                .map(|i| recurse!(i))
                .collect();
            format!("vec![{}]", item_strs.join(", "))
        }

        Expr::Tuple(ref items) => {
            let item_strs: Vec<String> = items.iter()
                .map(|i| format!("Value::from({})", recurse!(i)))
                .collect();
            // Tuples as Vec<Value> for heterogeneous support
            format!("vec![{}]", item_strs.join(", "))
        }

        Expr::Range { start, end } => {
            let start_str = recurse!(start);
            let end_str = recurse!(end);
            format!("({}..={})", start_str, end_str)
        }

        Expr::FieldAccess { object, field } => {
            let field_name = interner.resolve(*field);

            // Phase 52: Check if root object is synced - use .get().await
            let root_sym = get_root_identifier(object);
            if let Some(sym) = root_sym {
                if synced_vars.contains(&sym) {
                    let obj_name = interner.resolve(sym);
                    return format!("{}.get().await.{}", obj_name, field_name);
                }
            }

            let obj_str = recurse!(object);
            format!("{}.{}", obj_str, field_name)
        }

        Expr::New { type_name, type_args, init_fields } => {
            let type_str = interner.resolve(*type_name);
            if !init_fields.is_empty() {
                // Struct initialization with fields: Point { x: 10, y: 20, ..Default::default() }
                // Always add ..Default::default() to handle partial initialization (e.g., CRDT fields)
                let fields_str = init_fields.iter()
                    .map(|(name, value)| {
                        let field_name = interner.resolve(*name);
                        let value_str = recurse!(value);
                        format!("{}: {}", field_name, value_str)
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{} {{ {}, ..Default::default() }}", type_str, fields_str)
            } else if type_args.is_empty() {
                format!("{}::default()", type_str)
            } else {
                // Phase 34: Turbofish syntax for generic instantiation
                // Bug fix: Use codegen_type_expr to support nested types like Seq of (Seq of Int)
                let args_str = type_args.iter()
                    .map(|t| codegen_type_expr(t, interner))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{}::<{}>::default()", type_str, args_str)
            }
        }

        Expr::NewVariant { enum_name, variant, fields } => {
            let enum_str = interner.resolve(*enum_name);
            let variant_str = interner.resolve(*variant);
            if fields.is_empty() {
                // Unit variant: Shape::Point
                format!("{}::{}", enum_str, variant_str)
            } else {
                // Phase 103: Count identifier usage to handle cloning for reused values
                // We need to clone on all uses except the last one
                let mut identifier_counts: HashMap<Symbol, usize> = HashMap::new();
                for (_, value) in fields.iter() {
                    if let Expr::Identifier(sym) = value {
                        *identifier_counts.entry(*sym).or_insert(0) += 1;
                    }
                }

                // Track remaining uses for each identifier
                let mut remaining_uses: HashMap<Symbol, usize> = identifier_counts.clone();

                // Struct variant: Shape::Circle { radius: 10 }
                // Phase 102: Check if any field is recursive and needs Box::new()
                let fields_str: Vec<String> = fields.iter()
                    .map(|(field_name, value)| {
                        let name = interner.resolve(*field_name);

                        // Phase 103: Clone identifiers that are used multiple times
                        // Clone on all uses except the last one (to allow move on final use)
                        let val = if let Expr::Identifier(sym) = value {
                            let total = identifier_counts.get(sym).copied().unwrap_or(0);
                            let remaining = remaining_uses.get_mut(sym);
                            let base_name = if boxed_bindings.contains(sym) {
                                format!("(*{})", interner.resolve(*sym))
                            } else {
                                interner.resolve(*sym).to_string()
                            };
                            if total > 1 {
                                if let Some(r) = remaining {
                                    *r -= 1;
                                    if *r > 0 {
                                        // Not the last use, need to clone
                                        format!("{}.clone()", base_name)
                                    } else {
                                        // Last use, can move
                                        base_name
                                    }
                                } else {
                                    base_name
                                }
                            } else {
                                base_name
                            }
                        } else {
                            recurse!(value)
                        };

                        // Check if this field needs to be boxed (recursive type)
                        let key = (enum_str.to_string(), variant_str.to_string(), name.to_string());
                        if boxed_fields.contains(&key) {
                            format!("{}: Box::new({})", name, val)
                        } else {
                            format!("{}: {}", name, val)
                        }
                    })
                    .collect();
                format!("{}::{} {{ {} }}", enum_str, variant_str, fields_str.join(", "))
            }
        }

        Expr::OptionSome { value } => {
            format!("Some({})", recurse!(value))
        }

        Expr::OptionNone => {
            "None".to_string()
        }

        Expr::Escape { code, .. } => {
            let raw_code = interner.resolve(*code);
            let mut block = String::from("{\n");
            for line in raw_code.lines() {
                block.push_str("    ");
                block.push_str(line);
                block.push('\n');
            }
            block.push('}');
            block
        }

        Expr::WithCapacity { value, capacity } => {
            let cap_str = recurse!(capacity);
            match value {
                // Empty string → String::with_capacity(cap)
                Expr::Literal(Literal::Text(sym)) if interner.resolve(*sym).is_empty() => {
                    format!("String::with_capacity(({}) as usize)", cap_str)
                }
                // Non-empty string → { let mut __s = String::with_capacity(cap); __s.push_str("..."); __s }
                Expr::Literal(Literal::Text(sym)) => {
                    let text = interner.resolve(*sym);
                    format!("{{ let mut __s = String::with_capacity(({}) as usize); __s.push_str(\"{}\"); __s }}", cap_str, text)
                }
                // Collection Expr::New → Type::with_capacity(cap)
                Expr::New { type_name, type_args, .. } => {
                    let type_str = interner.resolve(*type_name);
                    match type_str {
                        "Seq" | "List" | "Vec" => {
                            let elem = if !type_args.is_empty() {
                                codegen_type_expr(&type_args[0], interner)
                            } else { "()".to_string() };
                            format!("{{ let __v: Vec<{}> = Vec::with_capacity(({}) as usize); __v }}", elem, cap_str)
                        }
                        "Map" | "HashMap" => {
                            let (k, v) = if type_args.len() >= 2 {
                                (codegen_type_expr(&type_args[0], interner),
                                 codegen_type_expr(&type_args[1], interner))
                            } else { ("String".to_string(), "String".to_string()) };
                            format!("{{ let __m: std::collections::HashMap<{}, {}> = std::collections::HashMap::with_capacity(({}) as usize); __m }}", k, v, cap_str)
                        }
                        "Set" | "HashSet" => {
                            let elem = if !type_args.is_empty() {
                                codegen_type_expr(&type_args[0], interner)
                            } else { "()".to_string() };
                            format!("{{ let __s: std::collections::HashSet<{}> = std::collections::HashSet::with_capacity(({}) as usize); __s }}", elem, cap_str)
                        }
                        _ => recurse!(value) // Unknown type — ignore capacity
                    }
                }
                // Other expressions — ignore capacity hint
                _ => recurse!(value)
            }
        }

        Expr::Closure { params, body, .. } => {
            use crate::ast::stmt::ClosureBody;
            let params_str: Vec<String> = params.iter()
                .map(|(name, ty)| {
                    let param_name = escape_rust_ident(interner.resolve(*name));
                    let param_type = codegen_type_expr(ty, interner);
                    format!("{}: {}", param_name, param_type)
                })
                .collect();

            match body {
                ClosureBody::Expression(expr) => {
                    let body_str = recurse!(expr);
                    format!("move |{}| {{ {} }}", params_str.join(", "), body_str)
                }
                ClosureBody::Block(stmts) => {
                    let mut body_str = String::new();
                    let mut ctx = RefinementContext::new();
                    let empty_mutable = collect_mutable_vars(stmts);
                    let empty_lww = HashSet::new();
                    let empty_mv = HashSet::new();
                    let mut empty_synced = HashSet::new();
                    let empty_caps = HashMap::new();
                    let empty_pipes = HashSet::new();
                    let empty_boxed = HashSet::new();
                    let empty_registry = TypeRegistry::new();
                    for stmt in stmts.iter() {
                        body_str.push_str(&codegen_stmt(
                            stmt, interner, 2, &empty_mutable, &mut ctx,
                            &empty_lww, &empty_mv, &mut empty_synced, &empty_caps,
                            async_functions, &empty_pipes, &empty_boxed, &empty_registry,
                        ));
                    }
                    format!("move |{}| {{\n{}{}}}", params_str.join(", "), body_str, "    ")
                }
            }
        }

        Expr::CallExpr { callee, args } => {
            let callee_str = recurse!(callee);
            let args_str: Vec<String> = args.iter().map(|a| recurse!(a)).collect();
            format!("({})({})", callee_str, args_str.join(", "))
        }
    }
}

fn codegen_literal(lit: &Literal, interner: &Interner) -> String {
    match lit {
        Literal::Number(n) => n.to_string(),
        Literal::Float(f) => format!("{}f64", f),
        // String literals are converted to String for consistent Text type handling
        Literal::Text(sym) => format!("String::from(\"{}\")", interner.resolve(*sym)),
        Literal::Boolean(b) => b.to_string(),
        Literal::Nothing => "()".to_string(),
        // Character literals
        Literal::Char(c) => {
            // Handle escape sequences for special characters
            match c {
                '\n' => "'\\n'".to_string(),
                '\t' => "'\\t'".to_string(),
                '\r' => "'\\r'".to_string(),
                '\\' => "'\\\\'".to_string(),
                '\'' => "'\\''".to_string(),
                '\0' => "'\\0'".to_string(),
                c => format!("'{}'", c),
            }
        }
        // Temporal literals: Duration stored as nanoseconds (i64)
        Literal::Duration(nanos) => format!("std::time::Duration::from_nanos({}u64)", nanos),
        // Date stored as days since Unix epoch (i32)
        Literal::Date(days) => format!("LogosDate({})", days),
        // Moment stored as nanoseconds since Unix epoch (i64)
        Literal::Moment(nanos) => format!("LogosMoment({})", nanos),
        // Span stored as (months, days) - separate because they're incommensurable
        Literal::Span { months, days } => format!("LogosSpan::new({}, {})", months, days),
        // Time-of-day stored as nanoseconds from midnight
        Literal::Time(nanos) => format!("LogosTime({})", nanos),
    }
}

/// Converts a LogicExpr to a Rust boolean expression for debug_assert!().
/// Uses RustFormatter to unify all logic-to-Rust translation.
pub fn codegen_assertion(expr: &LogicExpr, interner: &Interner) -> String {
    let mut registry = SymbolRegistry::new();
    let formatter = RustFormatter;
    let mut buf = String::new();

    match expr.write_logic(&mut buf, &mut registry, interner, &formatter) {
        Ok(_) => buf,
        Err(_) => "/* error generating assertion */ false".to_string(),
    }
}

pub fn codegen_term(term: &Term, interner: &Interner) -> String {
    match term {
        Term::Constant(sym) => interner.resolve(*sym).to_string(),
        Term::Variable(sym) => interner.resolve(*sym).to_string(),
        Term::Value { kind, .. } => match kind {
            NumberKind::Integer(n) => n.to_string(),
            NumberKind::Real(f) => f.to_string(),
            NumberKind::Symbolic(sym) => interner.resolve(*sym).to_string(),
        },
        Term::Function(name, args) => {
            let args_str: Vec<String> = args.iter()
                .map(|a| codegen_term(a, interner))
                .collect();
            format!("{}({})", interner.resolve(*name), args_str.join(", "))
        }
        Term::Possessed { possessor, possessed } => {
            let poss_str = codegen_term(possessor, interner);
            format!("{}.{}", poss_str, interner.resolve(*possessed))
        }
        Term::Group(members) => {
            let members_str: Vec<String> = members.iter()
                .map(|m| codegen_term(m, interner))
                .collect();
            format!("({})", members_str.join(", "))
        }
        _ => "/* unsupported Term */".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_literal_number() {
        let interner = Interner::new();
        let synced_vars = HashSet::new();
        let expr = Expr::Literal(Literal::Number(42));
        assert_eq!(codegen_expr(&expr, &interner, &synced_vars), "42");
    }

    #[test]
    fn test_literal_boolean() {
        let interner = Interner::new();
        let synced_vars = HashSet::new();
        assert_eq!(codegen_expr(&Expr::Literal(Literal::Boolean(true)), &interner, &synced_vars), "true");
        assert_eq!(codegen_expr(&Expr::Literal(Literal::Boolean(false)), &interner, &synced_vars), "false");
    }

    #[test]
    fn test_literal_nothing() {
        let interner = Interner::new();
        let synced_vars = HashSet::new();
        assert_eq!(codegen_expr(&Expr::Literal(Literal::Nothing), &interner, &synced_vars), "()");
    }
}
