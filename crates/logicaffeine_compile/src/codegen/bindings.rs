use std::fmt::Write;

use crate::analysis::registry::TypeRegistry;
use crate::ast::stmt::{Stmt, TypeExpr};
use crate::intern::Interner;
use super::ffi::{CAbiClass, classify_type_for_c_abi};

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
