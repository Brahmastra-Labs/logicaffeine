use std::collections::HashSet;
use std::fmt::Write;

use crate::analysis::registry::TypeRegistry;
use crate::analysis::types::RustNames;
use crate::ast::stmt::{Stmt, TypeExpr};
use crate::intern::{Interner, Symbol};

use super::context::{RefinementContext, analyze_variable_capabilities, replace_word};
use super::detection::{is_result_type, collect_mutable_vars};
use super::types::codegen_type_expr;
use super::{
    CAbiClass, classify_type_for_c_abi,
    codegen_assertion, codegen_stmt,
    try_emit_vec_fill_pattern, try_emit_for_range_pattern, try_emit_swap_pattern,
};

pub(super) fn is_text_type(ty: &TypeExpr, interner: &Interner) -> bool {
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
pub(super) fn map_type_to_c_abi(ty: &TypeExpr, interner: &Interner, is_return: bool) -> String {
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
/// - Text param/return marshaling (*const c_char <-> String)
/// - Reference type params/returns via opaque LogosHandle
/// - Result<T, E> returns via status code + out-parameter
/// - Refinement type boundary guards
pub(super) fn codegen_c_export_with_marshaling(
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
    type_env: &crate::analysis::types::TypeEnv,
) -> String {
    let mut output = String::new();
    let names = RustNames::new(interner);
    let raw_name = names.raw(name);
    // All exported C ABI symbols use the `logos_` prefix to avoid keyword
    // collisions in target languages (C, Python, JS, etc.) and to provide
    // a consistent namespace for the generated library.
    let func_name = format!("logos_{}", raw_name);
    let inner_name = names.ident(name);

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
            if let Some((code, skip)) = try_emit_vec_fill_pattern(&stmt_refs, si, interner, 1, &mut func_ctx) {
                output.push_str(&code);
                si += 1 + skip;
                continue;
            }
            if let Some((code, skip)) = try_emit_for_range_pattern(&stmt_refs, si, interner, 1, &func_mutable_vars, &mut func_ctx, lww_fields, mv_fields, &mut func_synced_vars, &func_var_caps, async_functions, &func_pipe_vars, boxed_fields, registry, type_env) {
                output.push_str(&code);
                si += 1 + skip;
                continue;
            }
            if let Some((code, skip)) = try_emit_swap_pattern(&stmt_refs, si, interner, 1, func_ctx.get_variable_types()) {
                output.push_str(&code);
                si += 1 + skip;
                continue;
            }
            output.push_str(&codegen_stmt(stmt_refs[si], interner, 1, &func_mutable_vars, &mut func_ctx, lww_fields, mv_fields, &mut func_synced_vars, &func_var_caps, async_functions, &func_pipe_vars, boxed_fields, registry, type_env));
            si += 1;
        }
    }
    writeln!(output, "}}\n").unwrap();

    // 2) Build the C ABI wrapper parameters
    let mut c_params: Vec<String> = Vec::new();

    for (pname, ptype) in params.iter() {
        let pn = names.ident(*pname);
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
            let pname_str = names.ident(*pname);
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
            // Reference type return -> box, register in handle registry, and write to out-parameter
            writeln!(output, "    let result = {}({});", inner_name, call_args.join(", ")).unwrap();
            writeln!(output, "    let __ptr = Box::into_raw(Box::new(result)) as usize;").unwrap();
            writeln!(output, "    let mut __reg = logos_handle_registry().lock().unwrap_or_else(|e| e.into_inner());").unwrap();
            writeln!(output, "    let (__id, _) = __reg.register(__ptr);").unwrap();
            writeln!(output, "    unsafe {{ *out = __id as LogosHandle; }}").unwrap();
            writeln!(output, "    LogosStatus::Ok").unwrap();
        } else if has_text_return {
            // Text return with status code -> write to out-parameter
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
