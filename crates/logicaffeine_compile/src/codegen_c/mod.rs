use std::collections::HashMap;
use std::fmt::Write;

use crate::analysis::TypeRegistry;
use crate::ast::stmt::*;
use crate::intern::{Interner, Symbol};

pub(crate) mod runtime;
pub(crate) mod types;
pub(crate) mod emit;

use runtime::C_RUNTIME;
use types::{CType, CContext, c_type_str, c_type_str_resolved, resolve_type_expr,
            resolve_type_expr_with_registry, field_type_to_ctype, escape_c_ident, infer_expr_type};
use emit::{codegen_expr, codegen_stmt, codegen_literal};

fn codegen_function(stmt: &Stmt, ctx: &mut CContext, output: &mut String) {
    if let Stmt::FunctionDef { name, params, body, return_type, is_native, .. } = stmt {
        if *is_native {
            return;
        }

        let func_name = ctx.resolve(*name).to_string();

        let ret_type = if let Some(rt) = return_type {
            resolve_type_expr_with_registry(rt, ctx.interner, Some(ctx.registry))
        } else {
            CType::Void
        };

        ctx.funcs.insert(*name, ret_type.clone());

        let mut param_strs = Vec::new();
        let mut param_types = Vec::new();
        for (param_name, param_type) in params {
            let p_type = resolve_type_expr_with_registry(param_type, ctx.interner, Some(ctx.registry));
            param_strs.push(format!("{} {}", c_type_str_resolved(&p_type, ctx.interner), ctx.resolve(*param_name)));
            param_types.push((*param_name, p_type));
        }

        write!(output, "{} {}({})", c_type_str_resolved(&ret_type, ctx.interner), func_name, param_strs.join(", ")).unwrap();
        writeln!(output, " {{").unwrap();

        let saved_vars = ctx.vars.clone();
        for (pname, ptype) in &param_types {
            ctx.vars.insert(*pname, ptype.clone());
        }

        for s in *body {
            codegen_stmt(s, ctx, output, 1);
        }

        ctx.vars = saved_vars;
        writeln!(output, "}}\n").unwrap();
    }
}

// =============================================================================
// Entry Point
// =============================================================================

fn codegen_c_struct_defs(registry: &TypeRegistry, interner: &Interner, output: &mut String) {
    use std::fmt::Write;
    use std::collections::HashSet;

    // Collect all struct symbols
    let struct_syms: Vec<Symbol> = registry.iter_types()
        .filter_map(|(sym, td)| {
            if matches!(td, crate::analysis::TypeDef::Struct { .. }) { Some(*sym) } else { None }
        })
        .collect();

    // Topological sort: emit structs whose field types are already emitted first
    let mut emitted: HashSet<Symbol> = HashSet::new();
    let mut ordered: Vec<Symbol> = Vec::new();

    fn field_deps(fields: &[crate::analysis::FieldDef], registry: &TypeRegistry) -> Vec<Symbol> {
        fields.iter().filter_map(|f| {
            if let crate::analysis::FieldType::Named(sym) = &f.ty {
                if matches!(registry.get(*sym), Some(crate::analysis::TypeDef::Struct { .. })) {
                    return Some(*sym);
                }
            }
            None
        }).collect()
    }

    // Simple iterative topological sort (O(n^2) but n is small)
    let mut remaining = struct_syms;
    while !remaining.is_empty() {
        let prev_len = remaining.len();
        remaining.retain(|sym| {
            if let Some(crate::analysis::TypeDef::Struct { fields, .. }) = registry.get(*sym) {
                let deps = field_deps(fields, registry);
                if deps.iter().all(|d| emitted.contains(d)) {
                    emitted.insert(*sym);
                    ordered.push(*sym);
                    return false; // remove from remaining
                }
            }
            true
        });
        if remaining.len() == prev_len {
            // Circular dependency or missing type — emit remaining as-is
            for sym in &remaining {
                ordered.push(*sym);
            }
            break;
        }
    }

    for sym in &ordered {
        if let Some(crate::analysis::TypeDef::Struct { fields, .. }) = registry.get(*sym) {
            let name = escape_c_ident(interner.resolve(*sym));
            writeln!(output, "typedef struct {{").unwrap();
            for field in fields {
                let field_name = escape_c_ident(interner.resolve(field.name));
                let ctype = field_type_to_ctype(&field.ty, interner, registry);
                let type_str = c_type_str_resolved(&ctype, interner);
                writeln!(output, "    {} {};", type_str, field_name).unwrap();
            }
            writeln!(output, "}} {};\n", name).unwrap();
        }
    }
}

fn codegen_c_enum_defs(registry: &TypeRegistry, interner: &Interner, output: &mut String) {
    use std::fmt::Write;
    for (sym, typedef) in registry.iter_types() {
        if let crate::analysis::TypeDef::Enum { variants, .. } = typedef {
            let name = escape_c_ident(interner.resolve(*sym));

            // Tag enum
            write!(output, "typedef enum {{ ").unwrap();
            for (i, v) in variants.iter().enumerate() {
                let vname = escape_c_ident(interner.resolve(v.name));
                if i > 0 { write!(output, ", ").unwrap(); }
                write!(output, "{}_{}", name, vname).unwrap();
            }
            writeln!(output, " }} {}_tag;\n", name).unwrap();

            // Check if any variant has fields
            let has_data = variants.iter().any(|v| !v.fields.is_empty());

            // Check if any variant is recursive (contains pointer to self)
            let is_recursive = variants.iter().any(|v| {
                v.fields.iter().any(|f| {
                    if let crate::analysis::FieldType::Named(fsym) = &f.ty {
                        *fsym == *sym
                    } else {
                        false
                    }
                })
            });

            if is_recursive {
                writeln!(output, "typedef struct {} {};", name, name).unwrap();
            }

            if is_recursive {
                writeln!(output, "struct {} {{", name).unwrap();
            } else {
                writeln!(output, "typedef struct {{").unwrap();
            }
            writeln!(output, "    {}_tag tag;", name).unwrap();
            if has_data {
                writeln!(output, "    union {{").unwrap();
                for v in variants {
                    if v.fields.is_empty() { continue; }
                    let vname = escape_c_ident(interner.resolve(v.name));
                    writeln!(output, "        struct {{").unwrap();
                    for f in &v.fields {
                        let fname = escape_c_ident(interner.resolve(f.name));
                        let is_self_ref = if let crate::analysis::FieldType::Named(fsym) = &f.ty {
                            *fsym == *sym
                        } else {
                            false
                        };
                        if is_self_ref {
                            writeln!(output, "            {} *{};", name, fname).unwrap();
                        } else {
                            let ctype = field_type_to_ctype(&f.ty, interner, registry);
                            let type_str = c_type_str_resolved(&ctype, interner);
                            writeln!(output, "            {} {};", type_str, fname).unwrap();
                        }
                    }
                    writeln!(output, "        }} {};", vname).unwrap();
                }
                writeln!(output, "    }} data;").unwrap();
            }
            if is_recursive {
                writeln!(output, "}};\n").unwrap();
            } else {
                writeln!(output, "}} {};\n", name).unwrap();
            }
        }
    }
}

pub fn codegen_program_c(stmts: &[Stmt], _registry: &TypeRegistry, interner: &Interner) -> String {
    let mut output = String::with_capacity(4096);
    let mut ctx = CContext::new(interner, _registry);

    output.push_str(C_RUNTIME);

    // Emit struct and enum type definitions
    codegen_c_struct_defs(_registry, interner, &mut output);
    codegen_c_enum_defs(_registry, interner, &mut output);

    // First pass: register all function return types (for forward references)
    for stmt in stmts {
        if let Stmt::FunctionDef { name, return_type, is_native, .. } = stmt {
            if *is_native {
                let fname = interner.resolve(*name);
                let ret_type = match fname {
                    "args" => CType::SeqStr,
                    "parseInt" => CType::Int64,
                    "parseFloat" => CType::Float64,
                    _ => {
                        if let Some(rt) = return_type {
                            resolve_type_expr_with_registry(rt, interner, Some(_registry))
                        } else {
                            CType::Void
                        }
                    }
                };
                ctx.funcs.insert(*name, ret_type);
            } else {
                let ret_type = if let Some(rt) = return_type {
                    resolve_type_expr_with_registry(rt, interner, Some(_registry))
                } else {
                    CType::Void
                };
                ctx.funcs.insert(*name, ret_type);
            }
        }
    }

    // Forward declarations
    for stmt in stmts {
        if let Stmt::FunctionDef { name, params, return_type, is_native, .. } = stmt {
            if *is_native {
                continue;
            }
            let func_name = ctx.resolve(*name).to_string();
            let ret_type = if let Some(rt) = return_type {
                resolve_type_expr_with_registry(rt, interner, Some(_registry))
            } else {
                CType::Void
            };
            let param_strs: Vec<String> = params.iter().map(|(pname, ptype)| {
                let p_type = resolve_type_expr_with_registry(ptype, interner, Some(_registry));
                format!("{} {}", c_type_str_resolved(&p_type, interner), ctx.resolve(*pname))
            }).collect();
            writeln!(output, "{} {}({});", c_type_str_resolved(&ret_type, interner), func_name, param_strs.join(", ")).unwrap();
        }
    }
    output.push('\n');

    // Function definitions
    for stmt in stmts {
        if let Stmt::FunctionDef { is_native: false, .. } = stmt {
            codegen_function(stmt, &mut ctx, &mut output);
        }
    }

    // Main function
    writeln!(output, "int main(int argc, char **argv) {{").unwrap();
    writeln!(output, "    _logos_argc = argc;").unwrap();
    writeln!(output, "    _logos_argv = argv;").unwrap();

    for stmt in stmts {
        match stmt {
            Stmt::FunctionDef { .. } => continue,
            _ => codegen_stmt(stmt, &mut ctx, &mut output, 1),
        }
    }

    writeln!(output, "    return 0;").unwrap();
    writeln!(output, "}}").unwrap();

    output
}
