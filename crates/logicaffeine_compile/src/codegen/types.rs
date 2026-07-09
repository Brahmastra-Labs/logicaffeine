use std::collections::{HashMap, HashSet};
use std::fmt::Write;

use crate::analysis::registry::{FieldDef, FieldType, TypeDef, TypeRegistry, VariantDef};
use crate::ast::stmt::{Expr, Stmt, TypeExpr};
use crate::intern::{Interner, Symbol};

thread_local! {
    /// When on, every emitted `enum` also gets `WireEncode`/`WireDecode` impls (the shared
    /// `logicaffeine_data::wire` codec). Off by default so ordinary programs' generated Rust is
    /// unchanged; enabled only for the compile-once native partial evaluator, whose types must
    /// receive a program as data over the same fast codec the interpreter uses.
    static EMIT_WIRE_IMPLS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Run `f` with `WireEncode`/`WireDecode` codegen forced on/off, restoring the prior value.
pub fn with_wire_impls<T>(on: bool, f: impl FnOnce() -> T) -> T {
    let prev = EMIT_WIRE_IMPLS.with(|c| c.replace(on));
    let out = f();
    EMIT_WIRE_IMPLS.with(|c| c.set(prev));
    out
}

#[inline]
fn wire_impls_enabled() -> bool {
    EMIT_WIRE_IMPLS.with(|c| c.get())
}

/// The Rust type string of a variant field, matching how [`codegen_enum_def`] declares it —
/// a self-referential field is `Box<Enum>`. Used to emit the field's wire (de)serialization.
fn wire_field_type(f: &FieldDef, enum_name: &str, interner: &Interner) -> String {
    let rust_type = codegen_field_type(&f.ty, interner);
    if is_recursive_field(&f.ty, enum_name, interner) {
        format!("Box<{}>", rust_type)
    } else {
        rust_type
    }
}

/// Whether a field type has a `logicaffeine_data::wire` impl: the primitives we implement
/// (`i64`/`String`/`bool`/`f64`), any `Named` user type (assumed a fellow wire-ok enum — a wrong
/// guess is a loud compile error in the generated crate, never silent corruption), or a `Seq`/`List`
/// of such. `Map`, CRDTs, temporal/quantity primitives, and type params are NOT serializable, so an
/// enum carrying one (e.g. the PE's runtime `PEState`/`CVal`) is skipped.
fn field_wire_ok(ty: &FieldType, interner: &Interner) -> bool {
    match ty {
        FieldType::Primitive(sym) => matches!(
            interner.resolve(*sym),
            "Int" | "Text" | "Bool" | "Boolean" | "Real" | "Float"
        ),
        FieldType::Named(_) => true,
        FieldType::Generic { base, params } => {
            matches!(interner.resolve(*base), "Seq" | "List")
                && params.iter().all(|p| field_wire_ok(p, interner))
        }
        FieldType::TypeParam(_) => false,
    }
}

/// An enum is wire-serializable iff every field of every variant is [`field_wire_ok`].
fn enum_is_wire_ok(variants: &[VariantDef], interner: &Interner) -> bool {
    variants.iter().all(|v| v.fields.iter().all(|f| field_wire_ok(&f.ty, interner)))
}

/// Emit `WireEncode`/`WireDecode` impls for a non-generic enum, byte-compatible with the peer
/// codec (proven by `concurrency::marshal::tests::peer_and_wire_core_produce_identical_bytes`).
/// Each value is an `Inductive`: header (`type_name`, `constructor`, arg count) then each field.
fn emit_enum_wire_impls(output: &mut String, enum_name: &str, variants: &[VariantDef], interner: &Interner) {
    let w = "logicaffeine_data::wire";
    // ── WireEncode ──
    writeln!(output, "impl {w}::WireEncode for {enum_name} {{").unwrap();
    writeln!(output, "    fn wire_encode(&self, __out: &mut Vec<u8>) {{").unwrap();
    writeln!(output, "        match self {{").unwrap();
    for variant in variants {
        let vname = interner.resolve(variant.name);
        if variant.fields.is_empty() {
            writeln!(output, "            {enum_name}::{vname} => {w}::write_inductive_header(__out, \"{enum_name}\", \"{vname}\", 0),").unwrap();
        } else {
            let binds: Vec<String> = variant.fields.iter().map(|f| interner.resolve(f.name).to_string()).collect();
            writeln!(output, "            {enum_name}::{vname} {{ {} }} => {{", binds.join(", ")).unwrap();
            writeln!(output, "                {w}::write_inductive_header(__out, \"{enum_name}\", \"{vname}\", {}u64);", variant.fields.len()).unwrap();
            for b in &binds {
                writeln!(output, "                {w}::WireEncode::wire_encode({b}, __out);").unwrap();
            }
            writeln!(output, "            }}").unwrap();
        }
    }
    writeln!(output, "        }}").unwrap();
    writeln!(output, "    }}").unwrap();
    writeln!(output, "}}").unwrap();
    // ── WireDecode ──
    writeln!(output, "impl {w}::WireDecode for {enum_name} {{").unwrap();
    writeln!(output, "    fn wire_decode(__buf: &[u8], __pos: &mut usize) -> Option<Self> {{").unwrap();
    writeln!(output, "        let (_ty, __ctor, _n) = {w}::read_inductive_header(__buf, __pos)?;").unwrap();
    writeln!(output, "        Some(match __ctor.as_str() {{").unwrap();
    for variant in variants {
        let vname = interner.resolve(variant.name);
        if variant.fields.is_empty() {
            writeln!(output, "            \"{vname}\" => {enum_name}::{vname},").unwrap();
        } else {
            let fields: Vec<String> = variant.fields.iter().map(|f| {
                let fname = interner.resolve(f.name);
                let fty = wire_field_type(f, enum_name, interner);
                format!("{fname}: <{fty} as {w}::WireDecode>::wire_decode(__buf, __pos)?")
            }).collect();
            writeln!(output, "            \"{vname}\" => {enum_name}::{vname} {{ {} }},", fields.join(", ")).unwrap();
        }
    }
    writeln!(output, "            _ => return None,").unwrap();
    writeln!(output, "        }})").unwrap();
    writeln!(output, "    }}").unwrap();
    writeln!(output, "}}\n").unwrap();
}

pub(super) fn codegen_type_expr(ty: &TypeExpr, interner: &Interner) -> String {
    match ty {
        // A `mutable` parameter codegens as its underlying type.
        TypeExpr::Mutable { inner } => codegen_type_expr(inner, interner),
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
                // A dimensioned quantity erases its dimension at runtime: `Quantity of Length`
                // lowers to a plain `LogosQuantity` (the dimension was a compile-time refinement).
                "Quantity" => "LogosQuantity".to_string(),
                "Money" => "LogosMoney".to_string(),
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
                        format!("LogosSeq<{}>", params_str[0])
                    } else {
                        "LogosSeq<()>".to_string()
                    }
                }
                "Map" | "HashMap" => {
                    if params_str.len() >= 2 {
                        format!("LogosMap<{}, {}>", params_str[0], params_str[1])
                    } else {
                        "LogosMap<String, String>".to_string()
                    }
                }
                "Set" | "HashSet" => {
                    if !params_str.is_empty() {
                        format!("Set<{}>", params_str[0])
                    } else {
                        "Set<()>".to_string()
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

/// Infer a function's return type from its body — REAL inference via the
/// analysis layer, seeded with the typed params and the body's `Let`
/// bindings in order, then unified over every reachable `Return` (nested
/// blocks included). Falls back to `i64` only when inference can't name the
/// type — never a silent wrong signature for Text/Bool/Float returns.
pub(super) fn infer_return_type_from_body(
    body: &[Stmt],
    params: &[(Symbol, &TypeExpr)],
    interner: &Interner,
) -> Option<String> {
    use crate::analysis::types::{LogosType, TypeEnv};

    let mut env = TypeEnv::new();
    for (sym, ty) in params {
        env.register(*sym, LogosType::from_type_expr(ty, interner));
    }

    fn walk(
        stmts: &[Stmt],
        env: &mut TypeEnv,
        interner: &Interner,
        found: &mut Option<LogosType>,
    ) {
        for stmt in stmts {
            match stmt {
                Stmt::Let { var, ty, value, .. } => {
                    let t = ty
                        .map(|t| LogosType::from_type_expr(t, interner))
                        .unwrap_or_else(|| env.infer_expr(value, interner));
                    env.register(*var, t);
                }
                Stmt::Return { value: Some(v) } => {
                    let t = env.infer_expr(v, interner);
                    match found {
                        None => *found = Some(t),
                        // Disagreeing returns keep the FIRST inference; a
                        // genuinely polymorphic body needs an annotation.
                        Some(_) => {}
                    }
                }
                Stmt::If { then_block, else_block, .. } => {
                    walk(then_block, env, interner, found);
                    if let Some(b) = else_block {
                        walk(b, env, interner, found);
                    }
                }
                Stmt::While { body, .. } | Stmt::Repeat { body, .. } | Stmt::Zone { body, .. } => {
                    walk(body, env, interner, found)
                }
                Stmt::Inspect { arms, .. } => {
                    for arm in arms {
                        walk(arm.body, env, interner, found);
                    }
                }
                _ => {}
            }
        }
    }

    let mut found = None;
    walk(body, &mut env, interner, &mut found);
    let has_return = found.is_some()
        || body.iter().any(|s| matches!(s, Stmt::Return { value: Some(_) }));
    match found {
        Some(t) => Some(t.to_rust_type()),
        None if has_return => Some("i64".to_string()),
        None => None,
    }
}

/// Map LOGOS type names to Rust types.
pub(crate) fn map_type_to_rust(ty: &str) -> String {
    match ty {
        "Int" => "i64".to_string(),
        "Nat" => "u64".to_string(),
        "Text" => "String".to_string(),
        "Bool" | "Boolean" => "bool".to_string(),
        "Real" | "Float" => "f64".to_string(),
        "Char" => "char".to_string(),
        "Byte" => "u8".to_string(),
        // The unit value `Nothing` lowers to Rust's unit; its runtime `type_name()` is "Nothing".
        "Nothing" | "Unit" | "()" => "()".to_string(),
        // Fixed-width machine words — the ℤ/2ⁿ wrapping ring. Crypto computes here. These are the
        // `#[repr(transparent)]` newtypes from logicaffeine_base (zero-cost over u8/u16/u32/u64),
        // whose operator impls wrap, so emitted `a + b` / `a ^ b` is ring-correct as written.
        "Word8" => "Word8".to_string(),
        "Word16" => "Word16".to_string(),
        "Word32" => "Word32".to_string(),
        "Word64" => "Word64".to_string(),
        // A SIMD lane vector — 8 lanes of Word32 in one `__m256i`. The `logicaffeine_base` newtype
        // carries `[u32; 8]`; its lane-op trait impls lower `v ^ w` to the AVX2 intrinsic.
        "Lanes8Word32" => "Lanes8Word32".to_string(),
        // 4 lanes of Word32 (one `__m128i`) — the SHA-1 SHA-NI config; the four SHA ops
        // lower `sha1rnds4`/`sha1msg1/2`/`sha1nexte` to the hardware instructions.
        "Lanes4Word32" => "Lanes4Word32".to_string(),
        "Lanes16Word8" => "Lanes16Word8".to_string(),
        // 4 lanes of Word64 (one `__m256i`) — the Poly1305 accumulator config.
        "Lanes4Word64" => "Lanes4Word64".to_string(),
        // 16 lanes of Word16 (one `__m256i`) — the NTT coefficient config.
        "Lanes16Word16" => "Lanes16Word16".to_string(),
        "Rational" => "LogosRational".to_string(),
        "Decimal" => "LogosDecimal".to_string(),
        "Complex" => "LogosComplex".to_string(),
        "Modular" => "LogosModular".to_string(),
        "Duration" => "std::time::Duration".to_string(),
        // Temporal value types — first-class as function params, fields, and locals.
        "Moment" => "LogosMoment".to_string(),
        "Date" => "LogosDate".to_string(),
        "Time" => "LogosTime".to_string(),
        "Span" => "LogosSpan".to_string(),
        // Dimensioned physical quantity (the dimension is runtime-tagged on the value).
        "Quantity" => "LogosQuantity".to_string(),
        // Money (the currency is runtime-tagged on the value).
        "Money" => "LogosMoney".to_string(),
        // A 128-bit UUID — a Copy newtype over `[u8; 16]`, alloc-free in compiled code.
        "Uuid" | "UUID" => "LogosUuid".to_string(),
        other => other.to_string(),
    }
}

/// Generate a single struct definition with derives and visibility.
/// Phase 34: Now supports generic type parameters.
/// Phase 47: Now supports is_portable for Serialize/Deserialize derives.
/// Phase 49: Now supports is_shared for CRDT Merge impl.
pub(super) fn codegen_struct_def(name: Symbol, fields: &[FieldDef], generics: &[Symbol], is_portable: bool, is_shared: bool, interner: &Interner, indent: usize, c_abi_value_structs: &HashSet<Symbol>, c_abi_ref_structs: &HashSet<Symbol>) -> String {
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
pub(super) fn codegen_merge_impl(name: Symbol, fields: &[FieldDef], generics: &[Symbol], interner: &Interner, indent: usize) -> String {
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
pub(super) fn is_crdt_field_type(ty: &FieldType, interner: &Interner) -> bool {
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
pub(super) fn codegen_enum_def(name: Symbol, variants: &[VariantDef], generics: &[Symbol], is_portable: bool, _is_shared: bool, interner: &Interner, indent: usize) -> String {
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

    let enum_name_str = interner.resolve(name);

    // Phase 47: Add Serialize, Deserialize derives if portable
    if is_portable {
        writeln!(output, "{}#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]", ind).unwrap();
    } else {
        writeln!(output, "{}#[derive(Debug, Clone, PartialEq)]", ind).unwrap();
    }
    writeln!(output, "{}pub enum {}{} {{", ind, enum_name_str, generic_str).unwrap();

    for variant in variants {
        let variant_name = interner.resolve(variant.name);
        if variant.fields.is_empty() {
            // Unit variant
            writeln!(output, "{}    {},", ind, variant_name).unwrap();
        } else {
            // Struct variant with named fields
            // Phase 102: Detect and box recursive fields
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

    // Opt-in shared-wire codec impls (native partial evaluator only). Non-generic, fully
    // wire-serializable enums only — a generic enum would need per-parameter bounds, and a
    // Map/CRDT-carrying enum (the PE's runtime PEState/CVal) has no wire form.
    if wire_impls_enabled() && generics.is_empty() && enum_is_wire_ok(variants, interner) {
        emit_enum_wire_impls(&mut output, enum_name_str, variants, interner);
    }

    output
}

/// Convert FieldType to Rust type string.
pub(super) fn codegen_field_type(ty: &FieldType, interner: &Interner) -> String {
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
                // Temporal value types as struct fields.
                "Moment" => "LogosMoment".to_string(),
                "Date" => "LogosDate".to_string(),
                "Time" => "LogosTime".to_string(),
                "Span" => "LogosSpan".to_string(),
                "Quantity" => "LogosQuantity".to_string(),
                "Money" => "LogosMoney".to_string(),
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

            // A dimensioned quantity erases its dimension at runtime — `Quantity of Length` is just
            // a `LogosQuantity` (the dimension was a compile-time-only refinement).
            if base_name == "Quantity" {
                return "LogosQuantity".to_string();
            }

            let base_str = match base_name {
                "List" | "Seq" => "LogosSeq",
                "Set" => "Set",
                "Map" => "LogosMap",
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

/// Check if a field type is a collection type (LogosSeq or LogosMap) that needs deep_clone
/// instead of clone for proper value semantics in struct/enum copies.
fn is_collection_field(ty: &FieldType, interner: &Interner) -> bool {
    match ty {
        FieldType::Generic { base, .. } => {
            let name = interner.resolve(*base);
            matches!(name, "Seq" | "List" | "Map" | "HashMap" | "Vec")
        }
        _ => false,
    }
}

/// Phase 102: Check if a field type references the containing enum (recursive type).
/// Recursive types need to be wrapped in Box<T> for Rust to know the size.
pub(crate) fn is_recursive_field(ty: &FieldType, enum_name: &str, interner: &Interner) -> bool {
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
pub(super) fn infer_variant_type_annotation(
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

/// Infer Rust type string from a LOGOS expression.
/// Delegates to `LogosType::from_literal()` for literals.
pub(super) fn infer_rust_type_from_expr(expr: &Expr, _interner: &Interner) -> String {
    match expr {
        Expr::Literal(lit) => {
            let ty = crate::analysis::types::LogosType::from_literal(lit);
            ty.to_rust_type()
        }
        _ => "_".to_string(),
    }
}

/// Infer the numeric type of an expression for mixed Float*Int arithmetic coercion.
///
/// Follows the standard numeric promotion rule (Z embeds into R):
/// if either operand of an arithmetic operation is f64, the result is f64.
/// Returns "i64", "f64", or "unknown".
///
/// Delegates to a temporary TypeEnv built from variable_types for inference.
pub(super) fn infer_numeric_type(
    expr: &Expr,
    interner: &Interner,
    variable_types: &HashMap<Symbol, String>,
) -> &'static str {
    // Build a temporary TypeEnv from the string-based variable types
    let mut env = crate::analysis::types::TypeEnv::new();
    for (sym, ty_str) in variable_types {
        let ty = crate::analysis::types::LogosType::from_rust_type_str(ty_str);
        env.register(*sym, ty);
    }
    let inferred = env.infer_expr(expr, interner);
    match inferred {
        crate::analysis::types::LogosType::Int => "i64",
        crate::analysis::types::LogosType::Float => "f64",
        _ => "unknown",
    }
}

/// Full [`LogosType`] inference for an expression (same temporary-TypeEnv
/// delegation as [`infer_numeric_type`], without the string flattening) —
/// the truthiness lowering needs Bool vs everything-else, not just numerics.
pub(super) fn infer_logos_type(
    expr: &Expr,
    interner: &Interner,
    variable_types: &HashMap<Symbol, String>,
) -> crate::analysis::types::LogosType {
    let mut env = crate::analysis::types::TypeEnv::new();
    for (sym, ty_str) in variable_types {
        let ty = crate::analysis::types::LogosType::from_rust_type_str(ty_str);
        env.register(*sym, ty);
    }
    env.infer_expr(expr, interner)
}
