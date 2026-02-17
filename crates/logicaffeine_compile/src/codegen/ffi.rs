use std::collections::{HashMap, HashSet};
use std::fmt::Write;

use crate::analysis::registry::{FieldDef, FieldType, TypeDef, TypeRegistry, VariantDef};
use crate::ast::stmt::{Expr, Stmt, TypeExpr};
use crate::intern::{Interner, Symbol};

use super::detection::is_result_type;
use super::marshal::is_text_type;
use super::types::codegen_type_expr;

/// FFI: Detect if any function is exported for WASM.
/// Used to emit `use wasm_bindgen::prelude::*;` preamble.
pub(crate) fn has_wasm_exports(stmts: &[Stmt], interner: &Interner) -> bool {
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
pub(crate) fn has_c_exports(stmts: &[Stmt], interner: &Interner) -> bool {
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
pub(crate) fn has_c_exports_with_text(stmts: &[Stmt], interner: &Interner) -> bool {
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
pub(crate) fn classify_type_for_c_abi(ty: &TypeExpr, interner: &Interner, registry: &TypeRegistry) -> CAbiClass {
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

pub(crate) fn mangle_type_for_c(ty: &TypeExpr, interner: &Interner) -> String {
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
pub(crate) fn codegen_logos_runtime_preamble() -> String {
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
pub(crate) fn codegen_c_accessors(ty: &TypeExpr, interner: &Interner, registry: &TypeRegistry) -> String {
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
pub(crate) fn collect_c_export_reference_types<'a>(
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
pub(crate) fn collect_c_export_value_type_structs(
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
pub(crate) fn collect_c_export_ref_structs(
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
pub(crate) fn map_type_to_c_header(ty: &TypeExpr, interner: &Interner, is_return: bool) -> String {
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
pub(crate) fn map_field_type_to_c(ft: &FieldType, interner: &Interner) -> String {
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
