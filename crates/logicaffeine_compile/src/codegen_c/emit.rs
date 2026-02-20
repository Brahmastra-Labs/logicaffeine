use std::fmt::Write;

use crate::ast::stmt::*;
use crate::intern::Symbol;

use super::types::{CType, CContext, c_type_str, c_type_str_resolved, resolve_type_expr, infer_expr_type, escape_c_ident, field_type_to_ctype};

pub(super) fn codegen_expr(expr: &Expr, ctx: &CContext) -> String {
    match expr {
        Expr::Literal(lit) => codegen_literal(lit, ctx),
        Expr::Identifier(sym) => ctx.resolve(*sym).to_string(),
        Expr::BinaryOp { op, left, right } => {
            let lt = infer_expr_type(left, ctx);
            let rt = infer_expr_type(right, ctx);

            // String concatenation via + operator
            if *op == BinaryOpKind::Add && (lt == CType::String || rt == CType::String) {
                let ls = if lt == CType::String {
                    codegen_expr(left, ctx)
                } else {
                    format!("i64_to_str({})", codegen_expr(left, ctx))
                };
                let rs = if rt == CType::String {
                    codegen_expr(right, ctx)
                } else {
                    format!("i64_to_str({})", codegen_expr(right, ctx))
                };
                return format!("str_concat({}, {})", ls, rs);
            }

            if *op == BinaryOpKind::Concat {
                let ls = if lt == CType::String {
                    codegen_expr(left, ctx)
                } else {
                    format!("i64_to_str({})", codegen_expr(left, ctx))
                };
                let rs = if rt == CType::String {
                    codegen_expr(right, ctx)
                } else {
                    format!("i64_to_str({})", codegen_expr(right, ctx))
                };
                return format!("str_concat({}, {})", ls, rs);
            }

            // String comparison
            if (*op == BinaryOpKind::Eq || *op == BinaryOpKind::NotEq) && (lt == CType::String || rt == CType::String) {
                let l = codegen_expr(left, ctx);
                let r = codegen_expr(right, ctx);
                if *op == BinaryOpKind::Eq {
                    return format!("str_equals({}, {})", l, r);
                } else {
                    return format!("(!str_equals({}, {}))", l, r);
                }
            }

            let l = codegen_expr(left, ctx);
            let r = codegen_expr(right, ctx);
            let op_str = match op {
                BinaryOpKind::Add => "+",
                BinaryOpKind::Subtract => "-",
                BinaryOpKind::Multiply => "*",
                BinaryOpKind::Divide => "/",
                BinaryOpKind::Modulo => "%",
                BinaryOpKind::Eq => "==",
                BinaryOpKind::NotEq => "!=",
                BinaryOpKind::Lt => "<",
                BinaryOpKind::LtEq => "<=",
                BinaryOpKind::Gt => ">",
                BinaryOpKind::GtEq => ">=",
                BinaryOpKind::And => "&&",
                BinaryOpKind::Or => "||",
                BinaryOpKind::Concat => "+",
                BinaryOpKind::BitXor => "^",
                BinaryOpKind::Shl => "<<",
                BinaryOpKind::Shr => ">>",
            };
            format!("({} {} {})", l, op_str, r)
        }
        Expr::Call { function, args } => {
            let raw_name = ctx.interner.resolve(*function);
            match raw_name {
                "args" => "logos_args()".to_string(),
                "parseInt" => {
                    let arg = if let Some(a) = args.first() {
                        codegen_expr(a, ctx)
                    } else {
                        "\"0\"".to_string()
                    };
                    format!("logos_parseInt({})", arg)
                }
                "parseFloat" => {
                    let arg = if let Some(a) = args.first() {
                        codegen_expr(a, ctx)
                    } else {
                        "\"0\"".to_string()
                    };
                    format!("atof({})", arg)
                }
                "sqrt" => {
                    let arg = if let Some(a) = args.first() {
                        codegen_expr(a, ctx)
                    } else {
                        "0.0".to_string()
                    };
                    format!("sqrt({})", arg)
                }
                "abs" => {
                    let arg = if let Some(a) = args.first() {
                        codegen_expr(a, ctx)
                    } else {
                        "0".to_string()
                    };
                    format!("fabs({})", arg)
                }
                "floor" => {
                    let arg = if let Some(a) = args.first() {
                        codegen_expr(a, ctx)
                    } else {
                        "0.0".to_string()
                    };
                    format!("(int64_t)floor({})", arg)
                }
                "ceil" => {
                    let arg = if let Some(a) = args.first() {
                        codegen_expr(a, ctx)
                    } else {
                        "0.0".to_string()
                    };
                    format!("(int64_t)ceil({})", arg)
                }
                "round" => {
                    let arg = if let Some(a) = args.first() {
                        codegen_expr(a, ctx)
                    } else {
                        "0.0".to_string()
                    };
                    format!("(int64_t)round({})", arg)
                }
                "pow" => {
                    let base = if let Some(a) = args.first() {
                        codegen_expr(a, ctx)
                    } else {
                        "0.0".to_string()
                    };
                    let exp = if let Some(a) = args.get(1) {
                        codegen_expr(a, ctx)
                    } else {
                        "1.0".to_string()
                    };
                    format!("pow((double)({}), (double)({}))", base, exp)
                }
                "min" => {
                    let a_str = if let Some(a) = args.first() {
                        codegen_expr(a, ctx)
                    } else {
                        "0".to_string()
                    };
                    let b_str = if let Some(b) = args.get(1) {
                        codegen_expr(b, ctx)
                    } else {
                        "0".to_string()
                    };
                    format!("(({a}) < ({b}) ? ({a}) : ({b}))", a = a_str, b = b_str)
                }
                "max" => {
                    let a_str = if let Some(a) = args.first() {
                        codegen_expr(a, ctx)
                    } else {
                        "0".to_string()
                    };
                    let b_str = if let Some(b) = args.get(1) {
                        codegen_expr(b, ctx)
                    } else {
                        "0".to_string()
                    };
                    format!("(({a}) > ({b}) ? ({a}) : ({b}))", a = a_str, b = b_str)
                }
                _ => {
                    let fname = escape_c_ident(raw_name);
                    let args_str: Vec<String> = args.iter().map(|a| codegen_expr(a, ctx)).collect();
                    format!("{}({})", fname, args_str.join(", "))
                }
            }
        }
        Expr::Index { collection, index } => {
            let idx = codegen_expr(index, ctx);
            if let Expr::Identifier(sym) = collection {
                let coll = ctx.resolve(*sym).to_string();
                match ctx.vars.get(sym) {
                    Some(CType::SeqI64) => format!("seq_i64_get(&{}, {})", coll, idx),
                    Some(CType::SeqBool) => format!("seq_bool_get(&{}, {})", coll, idx),
                    Some(CType::SeqStr) => format!("seq_str_get(&{}, {})", coll, idx),
                    Some(CType::SeqF64) => format!("seq_f64_get(&{}, {})", coll, idx),
                    Some(CType::MapI64I64) => format!("map_i64_i64_get(&{}, {})", coll, idx),
                    Some(CType::MapStrI64) => format!("map_str_i64_get(&{}, {})", coll, idx),
                    Some(CType::MapStrStr) => format!("map_str_str_get(&{}, {})", coll, idx),
                    Some(CType::MapI64Str) => format!("map_i64_str_get(&{}, {})", coll, idx),
                    _ => format!("seq_i64_get(&{}, {})", coll, idx),
                }
            } else {
                let coll = codegen_expr(collection, ctx);
                format!("seq_i64_get(&{}, {})", coll, idx)
            }
        }
        Expr::Length { collection } => {
            if let Expr::Identifier(sym) = collection {
                let coll = ctx.resolve(*sym).to_string();
                match ctx.vars.get(sym) {
                    Some(CType::SeqI64) => format!("seq_i64_len(&{})", coll),
                    Some(CType::SeqBool) => format!("seq_bool_len(&{})", coll),
                    Some(CType::SeqStr) => format!("seq_str_len(&{})", coll),
                    Some(CType::SeqF64) => format!("seq_f64_len(&{})", coll),
                    Some(CType::String) => format!("str_len({})", coll),
                    Some(CType::SetI64) => format!("set_i64_len(&{})", coll),
                    Some(CType::SetStr) => format!("set_str_len(&{})", coll),
                    _ => format!("seq_i64_len(&{})", coll),
                }
            } else {
                let coll = codegen_expr(collection, ctx);
                format!("seq_i64_len(&{})", coll)
            }
        }
        Expr::Contains { collection, value } => {
            let val_str = codegen_expr(value, ctx);
            if let Expr::Identifier(sym) = collection {
                let coll = ctx.resolve(*sym).to_string();
                match ctx.vars.get(sym) {
                    Some(CType::SeqI64) => format!("seq_i64_contains(&{}, {})", coll, val_str),
                    Some(CType::SeqBool) => format!("seq_bool_contains(&{}, {})", coll, val_str),
                    Some(CType::SeqStr) => format!("seq_str_contains(&{}, {})", coll, val_str),
                    Some(CType::SeqF64) => format!("seq_f64_contains(&{}, {})", coll, val_str),
                    Some(CType::MapI64I64) => format!("map_i64_i64_contains(&{}, {})", coll, val_str),
                    Some(CType::MapStrI64) => format!("map_str_i64_contains(&{}, {})", coll, val_str),
                    Some(CType::MapStrStr) => format!("map_str_str_contains(&{}, {})", coll, val_str),
                    Some(CType::MapI64Str) => format!("map_i64_str_contains(&{}, {})", coll, val_str),
                    Some(CType::SetI64) => format!("set_i64_contains(&{}, {})", coll, val_str),
                    Some(CType::SetStr) => format!("set_str_contains(&{}, {})", coll, val_str),
                    _ => format!("seq_i64_contains(&{}, {})", coll, val_str),
                }
            } else {
                let coll = codegen_expr(collection, ctx);
                format!("seq_i64_contains(&{}, {})", coll, val_str)
            }
        }
        Expr::New { type_name, type_args, init_fields, .. } => {
            let name = ctx.interner.resolve(*type_name);
            match name {
                "Seq" | "List" => {
                    if let Some(arg) = type_args.first() {
                        match resolve_type_expr(arg, ctx.interner) {
                            CType::Bool => "seq_bool_new()".to_string(),
                            CType::String => "seq_str_new()".to_string(),
                            CType::Float64 => "seq_f64_new()".to_string(),
                            _ => "seq_i64_new()".to_string(),
                        }
                    } else {
                        "seq_i64_new()".to_string()
                    }
                }
                "Map" => {
                    let key_type = type_args.first().map(|p| resolve_type_expr(p, ctx.interner)).unwrap_or(CType::Int64);
                    let val_type = type_args.get(1).map(|p| resolve_type_expr(p, ctx.interner)).unwrap_or(CType::Int64);
                    match (&key_type, &val_type) {
                        (CType::String, CType::Int64) => "map_str_i64_new()".to_string(),
                        (CType::String, CType::String) => "map_str_str_new()".to_string(),
                        (CType::Int64, CType::String) => "map_i64_str_new()".to_string(),
                        _ => "map_i64_i64_new()".to_string(),
                    }
                }
                "Set" => {
                    if let Some(arg) = type_args.first() {
                        match resolve_type_expr(arg, ctx.interner) {
                            CType::String => "set_str_new()".to_string(),
                            _ => "set_i64_new()".to_string(),
                        }
                    } else {
                        "set_i64_new()".to_string()
                    }
                }
                _ => {
                    // Check if this is a user-defined struct
                    if let Some(crate::analysis::TypeDef::Struct { .. }) = ctx.registry.get(*type_name) {
                        let escaped = escape_c_ident(name);
                        if init_fields.is_empty() {
                            format!("({}){{0}}", escaped)
                        } else {
                            let fields_str: Vec<String> = init_fields.iter().map(|(fname, fexpr)| {
                                let fn_str = escape_c_ident(ctx.interner.resolve(*fname));
                                let val = codegen_expr(fexpr, ctx);
                                format!(".{} = {}", fn_str, val)
                            }).collect();
                            format!("({}){{{}}}", escaped, fields_str.join(", "))
                        }
                    } else {
                        format!("/* unsupported new {} */0", name)
                    }
                }
            }
        }
        Expr::WithCapacity { value, capacity } => {
            let cap = codegen_expr(capacity, ctx);
            let ty = infer_expr_type(value, ctx);
            match ty {
                CType::MapI64I64 => format!("map_i64_i64_with_capacity({})", cap),
                CType::SeqI64 => format!("seq_i64_with_capacity({})", cap),
                CType::SeqBool => format!("seq_bool_with_capacity({})", cap),
                CType::SeqStr => format!("seq_str_with_capacity({})", cap),
                CType::SeqF64 => format!("seq_f64_with_capacity({})", cap),
                CType::String => "strdup(\"\")".to_string(),
                _ => codegen_expr(value, ctx),
            }
        }
        Expr::List(elems) => {
            if elems.is_empty() {
                return "seq_i64_new()".to_string();
            }
            let elem_type = infer_expr_type(elems.first().unwrap(), ctx);
            let (new_fn, _push_fn) = match elem_type {
                CType::Bool => ("seq_bool_new", "seq_bool_push"),
                CType::String => ("seq_str_new", "seq_str_push"),
                CType::Float64 => ("seq_f64_new", "seq_f64_push"),
                _ => ("seq_i64_new", "seq_i64_push"),
            };
            let mut parts = Vec::new();
            for e in elems {
                parts.push(codegen_expr(e, ctx));
            }
            format!("{}() /* list literal: {} */", new_fn, parts.join(", "))
        }
        Expr::Copy { expr: inner } => {
            let ty = infer_expr_type(inner, ctx);
            let inner_str = codegen_expr(inner, ctx);
            match ty {
                CType::SeqI64 => format!("seq_i64_copy(&{})", inner_str),
                CType::SeqBool => format!("seq_bool_copy(&{})", inner_str),
                CType::SeqStr => format!("seq_str_copy(&{})", inner_str),
                CType::SeqF64 => format!("seq_f64_copy(&{})", inner_str),
                _ => inner_str,
            }
        }
        Expr::Give { value } => codegen_expr(value, ctx),
        Expr::FieldAccess { object, field } => {
            let obj = codegen_expr(object, ctx);
            let fname = escape_c_ident(ctx.interner.resolve(*field));
            format!("{}.{}", obj, fname)
        }
        Expr::NewVariant { enum_name, variant, fields } => {
            let ename = escape_c_ident(ctx.interner.resolve(*enum_name));
            let vname = escape_c_ident(ctx.interner.resolve(*variant));
            if fields.is_empty() {
                format!("({}){{{}.tag = {}_{}}}", ename, "", ename, vname)
            } else {
                // Check which fields are recursive (self-referencing pointer)
                let variant_def_fields = ctx.registry.get(*enum_name)
                    .and_then(|td| {
                        if let crate::analysis::TypeDef::Enum { variants, .. } = td {
                            variants.iter().find(|v| v.name == *variant).map(|v| &v.fields)
                        } else {
                            None
                        }
                    });

                let field_inits: Vec<String> = fields.iter().map(|(fname, fexpr)| {
                    let fn_str = escape_c_ident(ctx.interner.resolve(*fname));
                    let val = codegen_expr(fexpr, ctx);

                    // Check if this field is a self-reference (pointer type)
                    let is_recursive = variant_def_fields
                        .and_then(|vfields| vfields.iter().find(|f| f.name == *fname))
                        .map(|f| {
                            if let crate::analysis::FieldType::Named(fsym) = &f.ty {
                                *fsym == *enum_name
                            } else {
                                false
                            }
                        })
                        .unwrap_or(false);

                    if is_recursive {
                        // Heap-allocate recursive field: ({Type *__p = malloc(sizeof(Type)); *__p = val; __p;})
                        format!(".{} = ({{{}* __p = ({}*)malloc(sizeof({})); *__p = {}; __p;}})", fn_str, ename, ename, ename, val)
                    } else {
                        format!(".{} = {}", fn_str, val)
                    }
                }).collect();
                format!("({}){{{}.tag = {}_{}, .data.{} = {{{}}}}}", ename, "", ename, vname, vname, field_inits.join(", "))
            }
        }
        Expr::Slice { collection, start, end } => {
            let start_str = codegen_expr(start, ctx);
            let end_str = codegen_expr(end, ctx);
            if let Expr::Identifier(sym) = collection {
                let coll = ctx.resolve(*sym).to_string();
                match ctx.vars.get(sym) {
                    Some(CType::SeqBool) => format!("seq_bool_slice(&{}, {}, {})", coll, start_str, end_str),
                    Some(CType::SeqStr) => format!("seq_str_slice(&{}, {}, {})", coll, start_str, end_str),
                    Some(CType::SeqF64) => format!("seq_f64_slice(&{}, {}, {})", coll, start_str, end_str),
                    _ => format!("seq_i64_slice(&{}, {}, {})", coll, start_str, end_str),
                }
            } else {
                let coll = codegen_expr(collection, ctx);
                format!("seq_i64_slice(&{}, {}, {})", coll, start_str, end_str)
            }
        }
        Expr::InterpolatedString(parts) => {
            codegen_interpolated_string_c(parts, ctx)
        }
        Expr::Escape { code, .. } => {
            format!("/* Escape: {} */0", ctx.interner.resolve(*code).replace("*/", "* /"))
        }
        _ => "0".to_string(),
    }
}

pub(super) fn codegen_literal(lit: &Literal, ctx: &CContext) -> String {
    match lit {
        Literal::Number(n) => format!("{}LL", n),
        Literal::Float(f) => format!("{}", f),
        Literal::Boolean(b) => if *b { "true".to_string() } else { "false".to_string() },
        Literal::Text(sym) => {
            let raw = ctx.interner.resolve(*sym);
            let escaped: String = raw.chars().map(|c| match c {
                '\n' => "\\n".to_string(),
                '\r' => "\\r".to_string(),
                '\t' => "\\t".to_string(),
                '\\' => "\\\\".to_string(),
                '"' => "\\\"".to_string(),
                other => other.to_string(),
            }).collect();
            format!("\"{}\"", escaped)
        }
        Literal::Nothing => "0".to_string(),
        _ => "0".to_string(),
    }
}

pub(super) fn c_format_spec_for_hole(expr: &Expr, spec: Option<&str>, ctx: &CContext) -> (String, String) {
    let ty = infer_expr_type(expr, ctx);
    let val_str = codegen_expr(expr, ctx);

    if let Some(spec) = spec {
        if spec == "$" {
            return ("$%.2f".to_string(), format!("(double)({})", val_str));
        }
        if spec.starts_with('.') {
            if let Ok(prec) = spec[1..].parse::<usize>() {
                return (format!("%.{}f", prec), format!("(double)({})", val_str));
            }
        }
        if spec.starts_with('>') || spec.starts_with('<') || spec.starts_with('^') {
            let align = &spec[..1];
            if let Ok(width) = spec[1..].parse::<usize>() {
                if align == "^" {
                    match ty {
                        CType::Int64 => return ("%s".to_string(), format!("logos_center_i64({}, {})", val_str, width)),
                        _ => return ("%s".to_string(), format!("logos_center_str({}, {})", val_str, width)),
                    }
                }
                let c_flag = match align {
                    ">" => "",
                    "<" => "-",
                    _ => unreachable!(),
                };
                match ty {
                    CType::String => return (format!("%{}{}s", c_flag, width), val_str),
                    CType::Int64 => return (format!("%{}{}\" PRId64 \"", c_flag, width), val_str),
                    _ => return (format!("%{}{}s", c_flag, width), val_str),
                }
            }
        }
    }

    match ty {
        CType::Int64 => ("%\" PRId64 \"".to_string(), val_str),
        CType::Float64 => ("%g".to_string(), val_str),
        CType::Bool => ("%s".to_string(), format!("{} ? \"true\" : \"false\"", val_str)),
        CType::String => ("%s".to_string(), val_str),
        _ => ("%\" PRId64 \"".to_string(), val_str),
    }
}

pub(super) fn codegen_interpolated_string_c(parts: &[crate::ast::stmt::StringPart], ctx: &CContext) -> String {
    let mut fmt = String::new();
    let mut args = Vec::new();

    for part in parts {
        match part {
            crate::ast::stmt::StringPart::Literal(sym) => {
                let text = ctx.interner.resolve(*sym);
                for c in text.chars() {
                    match c {
                        '\n' => fmt.push_str("\\n"),
                        '\r' => fmt.push_str("\\r"),
                        '\t' => fmt.push_str("\\t"),
                        '\\' => fmt.push_str("\\\\"),
                        '"' => fmt.push_str("\\\""),
                        '%' => fmt.push_str("%%"),
                        other => fmt.push(other),
                    }
                }
            }
            crate::ast::stmt::StringPart::Expr { value, format_spec, debug } => {
                if *debug {
                    let name = match value {
                        Expr::Identifier(sym) => ctx.interner.resolve(*sym).to_string(),
                        _ => "?".to_string(),
                    };
                    fmt.push_str(&name);
                    fmt.push('=');
                }
                let spec_str = format_spec.map(|s| ctx.interner.resolve(s).to_string());
                let (hole_fmt, hole_arg) = c_format_spec_for_hole(value, spec_str.as_deref(), ctx);
                fmt.push_str(&hole_fmt);
                args.push(hole_arg);
            }
        }
    }

    if args.is_empty() {
        format!("strdup(\"{}\")", fmt)
    } else {
        format!("logos_dyn_sprintf(\"{}\", {})", fmt, args.join(", "))
    }
}

pub(super) fn codegen_interpolated_show_c(parts: &[crate::ast::stmt::StringPart], ctx: &CContext, pad: &str, output: &mut String) {
    let mut fmt = String::new();
    let mut args = Vec::new();

    for part in parts {
        match part {
            crate::ast::stmt::StringPart::Literal(sym) => {
                let text = ctx.interner.resolve(*sym);
                for c in text.chars() {
                    match c {
                        '\n' => fmt.push_str("\\n"),
                        '\r' => fmt.push_str("\\r"),
                        '\t' => fmt.push_str("\\t"),
                        '\\' => fmt.push_str("\\\\"),
                        '"' => fmt.push_str("\\\""),
                        '%' => fmt.push_str("%%"),
                        other => fmt.push(other),
                    }
                }
            }
            crate::ast::stmt::StringPart::Expr { value, format_spec, debug } => {
                if *debug {
                    let name = match value {
                        Expr::Identifier(sym) => ctx.interner.resolve(*sym).to_string(),
                        _ => "?".to_string(),
                    };
                    fmt.push_str(&name);
                    fmt.push('=');
                }
                let spec_str = format_spec.map(|s| ctx.interner.resolve(s).to_string());
                let (hole_fmt, hole_arg) = c_format_spec_for_hole(value, spec_str.as_deref(), ctx);
                fmt.push_str(&hole_fmt);
                args.push(hole_arg);
            }
        }
    }

    if args.is_empty() {
        writeln!(output, "{}puts(\"{}\");", pad, fmt).unwrap();
    } else {
        writeln!(output, "{}printf(\"{}\\n\", {});", pad, fmt, args.join(", ")).unwrap();
    }
}

// =============================================================================
// Statement Codegen
// =============================================================================

pub(super) fn codegen_stmt(stmt: &Stmt, ctx: &mut CContext, output: &mut String, indent: usize) {
    let pad = "    ".repeat(indent);
    match stmt {
        Stmt::Let { var, value, ty, .. } => {
            let var_name = ctx.resolve(*var).to_string();

            // Determine C type
            let c_type = if let Some(ty_expr) = ty {
                resolve_type_expr(ty_expr, ctx.interner)
            } else {
                infer_expr_type(value, ctx)
            };

            // Handle list literals specially — need to build inline
            if let Expr::List(elems) = value {
                if !elems.is_empty() {
                    let (new_fn, push_fn) = match &c_type {
                        CType::SeqBool => ("seq_bool_new", "seq_bool_push"),
                        CType::SeqStr => ("seq_str_new", "seq_str_push"),
                        CType::SeqF64 => ("seq_f64_new", "seq_f64_push"),
                        _ => ("seq_i64_new", "seq_i64_push"),
                    };
                    writeln!(output, "{}{} {} = {}();", pad, c_type_str(&c_type), var_name, new_fn).unwrap();
                    for e in elems {
                        let v = codegen_expr(e, ctx);
                        writeln!(output, "{}{}(&{}, {});", pad, push_fn, var_name, v).unwrap();
                    }
                    ctx.vars.insert(*var, c_type);
                    return;
                }
            }

            let val_str = codegen_expr(value, ctx);
            let type_str = c_type_str_resolved(&c_type, ctx.interner);
            writeln!(output, "{}{} {} = {};", pad, type_str, var_name, val_str).unwrap();
            ctx.vars.insert(*var, c_type);
        }
        Stmt::Set { target, value } => {
            let val_str = codegen_expr(value, ctx);
            let var_name = ctx.resolve(*target);
            writeln!(output, "{}{} = {};", pad, var_name, val_str).unwrap();
        }
        Stmt::Show { object, .. } => {
            if let Expr::InterpolatedString(parts) = object {
                codegen_interpolated_show_c(parts, ctx, &pad, output);
                return;
            }
            let ty = infer_expr_type(object, ctx);
            let val_str = codegen_expr(object, ctx);
            match ty {
                CType::Int64 => writeln!(output, "{}show_i64({});", pad, val_str).unwrap(),
                CType::Float64 => writeln!(output, "{}show_f64({});", pad, val_str).unwrap(),
                CType::Bool => writeln!(output, "{}show_bool({});", pad, val_str).unwrap(),
                CType::String => writeln!(output, "{}show_str({});", pad, val_str).unwrap(),
                CType::SeqI64 => {
                    if let Expr::Identifier(_) = object {
                        writeln!(output, "{}show_seq_i64(&{});", pad, val_str).unwrap();
                    } else {
                        writeln!(output, "{}{{ Seq_i64 __tmp = {}; show_seq_i64(&__tmp); }}", pad, val_str).unwrap();
                    }
                }
                CType::SeqBool => {
                    if let Expr::Identifier(_) = object {
                        writeln!(output, "{}show_seq_bool(&{});", pad, val_str).unwrap();
                    } else {
                        writeln!(output, "{}{{ Seq_bool __tmp = {}; show_seq_bool(&__tmp); }}", pad, val_str).unwrap();
                    }
                }
                CType::SeqStr => {
                    if let Expr::Identifier(_) = object {
                        writeln!(output, "{}show_seq_str(&{});", pad, val_str).unwrap();
                    } else {
                        writeln!(output, "{}{{ Seq_str __tmp = {}; show_seq_str(&__tmp); }}", pad, val_str).unwrap();
                    }
                }
                CType::SeqF64 => {
                    if let Expr::Identifier(_) = object {
                        writeln!(output, "{}show_seq_f64(&{});", pad, val_str).unwrap();
                    } else {
                        writeln!(output, "{}{{ Seq_f64 __tmp = {}; show_seq_f64(&__tmp); }}", pad, val_str).unwrap();
                    }
                }
                CType::Struct(sym) => {
                    if let Some(crate::analysis::TypeDef::Struct { fields, .. }) = ctx.registry.get(sym) {
                        let struct_name = escape_c_ident(ctx.interner.resolve(sym));
                        // Print struct as: StructName(field1: val1, field2: val2)
                        let mut fmt_parts = Vec::new();
                        let mut arg_parts = Vec::new();
                        fmt_parts.push(format!("{}(", struct_name));
                        for (i, f) in fields.iter().enumerate() {
                            let fname = escape_c_ident(ctx.interner.resolve(f.name));
                            let ctype = field_type_to_ctype(&f.ty, ctx.interner, ctx.registry);
                            if i > 0 { fmt_parts.push(", ".to_string()); }
                            let fmt_spec = match ctype {
                                CType::Int64 => "%\" PRId64 \"",
                                CType::Float64 => "%g",
                                CType::Bool => "%s",
                                CType::String => "%s",
                                _ => "%\" PRId64 \"",
                            };
                            fmt_parts.push(format!("{}: {}", fname, fmt_spec));
                            match ctype {
                                CType::Bool => arg_parts.push(format!("{}.{} ? \"true\" : \"false\"", val_str, fname)),
                                _ => arg_parts.push(format!("{}.{}", val_str, fname)),
                            }
                        }
                        fmt_parts.push(")".to_string());
                        let fmt_string = fmt_parts.join("");
                        if arg_parts.is_empty() {
                            writeln!(output, "{}printf(\"{}\\n\");", pad, fmt_string).unwrap();
                        } else {
                            writeln!(output, "{}printf(\"{}\\n\", {});", pad, fmt_string, arg_parts.join(", ")).unwrap();
                        }
                    } else {
                        writeln!(output, "{}show_i64({});", pad, val_str).unwrap();
                    }
                }
                _ => writeln!(output, "{}show_i64({});", pad, val_str).unwrap(),
            }
        }
        Stmt::Return { value } => {
            if let Some(val) = value {
                let val_str = codegen_expr(val, ctx);
                writeln!(output, "{}return {};", pad, val_str).unwrap();
            } else {
                writeln!(output, "{}return;", pad).unwrap();
            }
        }
        Stmt::If { cond, then_block, else_block } => {
            let cond_str = codegen_expr(cond, ctx);
            writeln!(output, "{}if ({}) {{", pad, cond_str).unwrap();
            for s in *then_block {
                codegen_stmt(s, ctx, output, indent + 1);
            }
            if let Some(eb) = else_block {
                writeln!(output, "{}}} else {{", pad).unwrap();
                for s in *eb {
                    codegen_stmt(s, ctx, output, indent + 1);
                }
            }
            writeln!(output, "{}}}", pad).unwrap();
        }
        Stmt::While { cond, body, .. } => {
            let cond_str = codegen_expr(cond, ctx);
            writeln!(output, "{}while ({}) {{", pad, cond_str).unwrap();
            for s in *body {
                codegen_stmt(s, ctx, output, indent + 1);
            }
            writeln!(output, "{}}}", pad).unwrap();
        }
        Stmt::Push { collection, value } => {
            let val_str = codegen_expr(value, ctx);
            if let Expr::Identifier(sym) = collection {
                let coll_name = ctx.resolve(*sym).to_string();
                match ctx.vars.get(sym) {
                    Some(CType::SeqI64) => writeln!(output, "{}seq_i64_push(&{}, {});", pad, coll_name, val_str).unwrap(),
                    Some(CType::SeqBool) => writeln!(output, "{}seq_bool_push(&{}, {});", pad, coll_name, val_str).unwrap(),
                    Some(CType::SeqStr) => writeln!(output, "{}seq_str_push(&{}, {});", pad, coll_name, val_str).unwrap(),
                    Some(CType::SeqF64) => writeln!(output, "{}seq_f64_push(&{}, {});", pad, coll_name, val_str).unwrap(),
                    _ => writeln!(output, "{}seq_i64_push(&{}, {});", pad, coll_name, val_str).unwrap(),
                }
            }
        }
        Stmt::Pop { collection, into } => {
            if let Expr::Identifier(sym) = collection {
                let coll_name = ctx.resolve(*sym).to_string();
                let (pop_fn, elem_type) = match ctx.vars.get(sym) {
                    Some(CType::SeqBool) => ("seq_bool_pop", CType::Bool),
                    Some(CType::SeqStr) => ("seq_str_pop", CType::String),
                    Some(CType::SeqF64) => ("seq_f64_pop", CType::Float64),
                    _ => ("seq_i64_pop", CType::Int64),
                };
                if let Some(var) = into {
                    let var_name = ctx.resolve(*var).to_string();
                    writeln!(output, "{}{} {} = {}(&{});", pad, c_type_str(&elem_type), var_name, pop_fn, coll_name).unwrap();
                    ctx.vars.insert(*var, elem_type);
                } else {
                    writeln!(output, "{}{}(&{});", pad, pop_fn, coll_name).unwrap();
                }
            }
        }
        Stmt::Call { function, args } => {
            let raw_name = ctx.interner.resolve(*function);
            let fname = escape_c_ident(raw_name);
            let args_str: Vec<String> = args.iter().map(|a| codegen_expr(a, ctx)).collect();
            writeln!(output, "{}{}({});", pad, fname, args_str.join(", ")).unwrap();
        }
        Stmt::SetIndex { collection, index, value } => {
            let idx_str = codegen_expr(index, ctx);
            let val_str = codegen_expr(value, ctx);
            if let Expr::Identifier(sym) = collection {
                let coll_name = ctx.resolve(*sym).to_string();
                match ctx.vars.get(sym) {
                    Some(CType::SeqI64) => writeln!(output, "{}seq_i64_set(&{}, {}, {});", pad, coll_name, idx_str, val_str).unwrap(),
                    Some(CType::SeqBool) => writeln!(output, "{}seq_bool_set(&{}, {}, {});", pad, coll_name, idx_str, val_str).unwrap(),
                    Some(CType::SeqF64) => writeln!(output, "{}seq_f64_set(&{}, {}, {});", pad, coll_name, idx_str, val_str).unwrap(),
                    Some(CType::MapI64I64) => writeln!(output, "{}map_i64_i64_set(&{}, {}, {});", pad, coll_name, idx_str, val_str).unwrap(),
                    Some(CType::MapStrI64) => writeln!(output, "{}map_str_i64_set(&{}, {}, {});", pad, coll_name, idx_str, val_str).unwrap(),
                    Some(CType::MapStrStr) => writeln!(output, "{}map_str_str_set(&{}, {}, {});", pad, coll_name, idx_str, val_str).unwrap(),
                    Some(CType::MapI64Str) => writeln!(output, "{}map_i64_str_set(&{}, {}, {});", pad, coll_name, idx_str, val_str).unwrap(),
                    _ => writeln!(output, "{}seq_i64_set(&{}, {}, {});", pad, coll_name, idx_str, val_str).unwrap(),
                }
            }
        }
        Stmt::Repeat { pattern, iterable, body } => {
            // Check for Range-based iteration (optimized)
            if let Expr::Range { start, end } = iterable {
                let var_sym = match pattern {
                    Pattern::Identifier(sym) => *sym,
                    Pattern::Tuple(syms) => if let Some(s) = syms.first() { *s } else { return },
                };
                let var_name = ctx.resolve(var_sym).to_string();
                let start_str = codegen_expr(start, ctx);
                let end_str = codegen_expr(end, ctx);
                writeln!(output, "{}for (int64_t {} = {}; {} <= {}; {}++) {{", pad, var_name, start_str, var_name, end_str, var_name).unwrap();
                ctx.vars.insert(var_sym, CType::Int64);
                for s in *body {
                    codegen_stmt(s, ctx, output, indent + 1);
                }
                writeln!(output, "{}}}", pad).unwrap();
                return;
            }

            let coll_type = infer_expr_type(iterable, ctx);

            // Map iteration — direct bucket scan
            let map_types = match &coll_type {
                CType::MapI64I64 => Some((CType::Int64, CType::Int64)),
                CType::MapStrI64 => Some((CType::String, CType::Int64)),
                CType::MapStrStr => Some((CType::String, CType::String)),
                CType::MapI64Str => Some((CType::Int64, CType::String)),
                _ => None,
            };

            if let Some((key_type, val_type)) = map_types {
                let iter_str = codegen_expr(iterable, ctx);
                writeln!(output, "{}for (size_t __mi = 0; __mi < {}.cap; __mi++) {{", pad, iter_str).unwrap();
                writeln!(output, "{}    if (!{}.state[__mi]) continue;", pad, iter_str).unwrap();

                match pattern {
                    Pattern::Tuple(syms) if syms.len() >= 2 => {
                        let k_sym = syms[0];
                        let v_sym = syms[1];
                        let k_name = ctx.resolve(k_sym).to_string();
                        let v_name = ctx.resolve(v_sym).to_string();
                        writeln!(output, "{}    {} {} = {}.keys[__mi];", pad, c_type_str(&key_type), k_name, iter_str).unwrap();
                        writeln!(output, "{}    {} {} = {}.vals[__mi];", pad, c_type_str(&val_type), v_name, iter_str).unwrap();
                        ctx.vars.insert(k_sym, key_type);
                        ctx.vars.insert(v_sym, val_type);
                    }
                    _ => {
                        let var_sym = match pattern {
                            Pattern::Identifier(sym) => *sym,
                            Pattern::Tuple(syms) => if let Some(s) = syms.first() { *s } else { return },
                        };
                        let var_name = ctx.resolve(var_sym).to_string();
                        writeln!(output, "{}    {} {} = {}.keys[__mi];", pad, c_type_str(&key_type), var_name, iter_str).unwrap();
                        ctx.vars.insert(var_sym, key_type);
                    }
                }

                for s in *body {
                    codegen_stmt(s, ctx, output, indent + 1);
                }
                writeln!(output, "{}}}", pad).unwrap();
            } else {
                // Seq iteration
                let var_sym = match pattern {
                    Pattern::Identifier(sym) => *sym,
                    Pattern::Tuple(syms) => if let Some(s) = syms.first() { *s } else { return },
                };
                let var_name = ctx.resolve(var_sym).to_string();
                let iter_str = codegen_expr(iterable, ctx);
                let (len_fn, get_fn, elem_type) = match coll_type {
                    CType::SeqBool => ("seq_bool_len", "seq_bool_get", CType::Bool),
                    CType::SeqStr => ("seq_str_len", "seq_str_get", CType::String),
                    CType::SeqF64 => ("seq_f64_len", "seq_f64_get", CType::Float64),
                    _ => ("seq_i64_len", "seq_i64_get", CType::Int64),
                };
                writeln!(output, "{}for (int64_t __idx = 1; __idx <= {}(&{}); __idx++) {{", pad, len_fn, iter_str).unwrap();
                writeln!(output, "{}    {} {} = {}(&{}, __idx);", pad, c_type_str(&elem_type), var_name, get_fn, iter_str).unwrap();
                ctx.vars.insert(var_sym, elem_type);
                for s in *body {
                    codegen_stmt(s, ctx, output, indent + 1);
                }
                writeln!(output, "{}}}", pad).unwrap();
            }
        }
        Stmt::Add { value, collection } => {
            let val_str = codegen_expr(value, ctx);
            if let Expr::Identifier(sym) = collection {
                let coll_name = ctx.resolve(*sym).to_string();
                match ctx.vars.get(sym) {
                    Some(CType::SetI64) => writeln!(output, "{}set_i64_add(&{}, {});", pad, coll_name, val_str).unwrap(),
                    Some(CType::SetStr) => writeln!(output, "{}set_str_add(&{}, {});", pad, coll_name, val_str).unwrap(),
                    _ => writeln!(output, "{}set_i64_add(&{}, {});", pad, coll_name, val_str).unwrap(),
                }
            }
        }
        Stmt::Remove { value, collection } => {
            let val_str = codegen_expr(value, ctx);
            if let Expr::Identifier(sym) = collection {
                let coll_name = ctx.resolve(*sym).to_string();
                match ctx.vars.get(sym) {
                    Some(CType::SetI64) => writeln!(output, "{}set_i64_remove(&{}, {});", pad, coll_name, val_str).unwrap(),
                    Some(CType::SetStr) => writeln!(output, "{}set_str_remove(&{}, {});", pad, coll_name, val_str).unwrap(),
                    _ => writeln!(output, "{}set_i64_remove(&{}, {});", pad, coll_name, val_str).unwrap(),
                }
            }
        }
        Stmt::SetField { object, field, value } => {
            let obj = codegen_expr(object, ctx);
            let fname = escape_c_ident(ctx.interner.resolve(*field));
            let val = codegen_expr(value, ctx);
            writeln!(output, "{}{}.{} = {};", pad, obj, fname, val).unwrap();
        }
        Stmt::Inspect { target, arms, .. } => {
            let target_str = codegen_expr(target, ctx);
            let target_type = infer_expr_type(target, ctx);
            let enum_sym = if let CType::Enum(sym) = target_type { Some(sym) } else { None };

            let mut first = true;
            for arm in arms {
                if let Some(variant_sym) = arm.variant {
                    let ename = arm.enum_name
                        .map(|s| escape_c_ident(ctx.interner.resolve(s)))
                        .or_else(|| enum_sym.map(|s| escape_c_ident(ctx.interner.resolve(s))))
                        .unwrap_or_else(|| "Unknown".to_string());
                    let vname = escape_c_ident(ctx.interner.resolve(variant_sym));

                    if first {
                        writeln!(output, "{}if ({}.tag == {}_{}) {{", pad, target_str, ename, vname).unwrap();
                        first = false;
                    } else {
                        writeln!(output, "{}}} else if ({}.tag == {}_{}) {{", pad, target_str, ename, vname).unwrap();
                    }

                    // Extract bindings from variant fields
                    for (field_name, binding_name) in &arm.bindings {
                        let fname = escape_c_ident(ctx.interner.resolve(*field_name));
                        let bname = escape_c_ident(ctx.interner.resolve(*binding_name));
                        // Infer the field type from the registry, detecting recursive fields
                        let (field_ctype, is_recursive_field) = if let Some(esym) = enum_sym {
                            if let Some(crate::analysis::TypeDef::Enum { variants, .. }) = ctx.registry.get(esym) {
                                variants.iter()
                                    .find(|v| v.name == variant_sym)
                                    .and_then(|v| v.fields.iter().find(|f| f.name == *field_name))
                                    .map(|f| {
                                        let is_self = matches!(&f.ty, crate::analysis::FieldType::Named(fsym) if *fsym == esym);
                                        (field_type_to_ctype(&f.ty, ctx.interner, ctx.registry), is_self)
                                    })
                                    .unwrap_or((CType::Int64, false))
                            } else {
                                (CType::Int64, false)
                            }
                        } else {
                            (CType::Int64, false)
                        };
                        let type_str = c_type_str_resolved(&field_ctype, ctx.interner);
                        if is_recursive_field {
                            // Recursive field is a pointer — dereference to get value copy
                            writeln!(output, "{}    {} {} = *{}.data.{}.{};", pad, type_str, bname, target_str, vname, fname).unwrap();
                        } else {
                            writeln!(output, "{}    {} {} = {}.data.{}.{};", pad, type_str, bname, target_str, vname, fname).unwrap();
                        }
                        ctx.vars.insert(*binding_name, field_ctype);
                    }
                } else {
                    // Otherwise arm
                    if first {
                        writeln!(output, "{}{{", pad).unwrap();
                        first = false;
                    } else {
                        writeln!(output, "{}}} else {{", pad).unwrap();
                    }
                }

                for s in arm.body {
                    codegen_stmt(s, ctx, output, indent + 1);
                }
            }
            if !first {
                writeln!(output, "{}}}", pad).unwrap();
            }
        }
        Stmt::FunctionDef { .. } => {}
        _ => {
            writeln!(output, "{}/* unsupported stmt */", pad).unwrap();
        }
    }
}

