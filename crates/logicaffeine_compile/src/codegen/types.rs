use std::collections::{HashMap, HashSet};
use std::fmt::Write;

use crate::analysis::registry::{FieldDef, FieldType, TypeDef, TypeRegistry, VariantDef};
use crate::ast::stmt::{Expr, Stmt, TypeExpr};
use crate::intern::{Interner, Symbol};

/// Opens the trailing marker comment a NAMED-record Rust type carries its field order + scalar field
/// types in — a record's type is `Vec<Value> /* logos_record: quotient=i64,remainder=i64 */` (see the
/// `TypeExpr::Record` arm of [`codegen_type_expr`]). The comment is valid Rust (the compiler ignores
/// it), and it is the only per-variable channel from the record type to a `r's field` access, which
/// reads it via [`logos_record_field_index`] (position) + [`logos_record_field_type`] (unbox target).
pub(crate) const RECORD_FIELD_MARKER: &str = "/* logos_record: ";

/// Opens the analogous marker a heterogeneous TUPLE Rust type carries its per-position scalar element
/// types in — `Vec<Value> /* logos_tuple: i64,String */`. Read by [`logos_tuple_elem_type`] so a
/// constant `item N of t` UNBOXES its element to the concrete scalar the use site demands.
pub(crate) const TUPLE_ELEM_MARKER: &str = "/* logos_tuple: ";

/// How a boxed `Value` element comes back as the Rust type `rust_ty`: the method call that unboxes it
/// (`logos_i64()`, `logos_seq_string()`, `logos_boxed::<Point>()`, `logos_seq(…)`), or `None` if a
/// `Value` cannot hold that type — extraction then stays a `Value` (correct for `Show`). Scalars ride
/// `Value`'s scalar variants, a homogeneous `Seq` rides `Value::Seq` unboxed element by element, and a
/// record or variant the program declares rides `Value::Boxed`. Only these types ever appear in a
/// marker (see [`unboxable_marker_types`]).
pub(crate) fn logos_value_unbox_call(rust_ty: &str) -> Option<String> {
    let ty = rust_ty.trim();
    if let Some(method) = named_unbox_method(ty) {
        return Some(format!("{method}()"));
    }
    if let Some(inner) = ty
        .strip_prefix("LogosSeq<")
        .and_then(|t| t.strip_suffix('>'))
    {
        return Some(format!("logos_seq({})", logos_value_unbox_fn(inner)?));
    }
    is_program_boxable(ty).then(|| format!("logos_boxed::<{ty}>()"))
}

/// [`logos_value_unbox_call`] as a function from `Value` — the element unboxer `Value::logos_seq` takes.
fn logos_value_unbox_fn(rust_ty: &str) -> Option<String> {
    let ty = rust_ty.trim();
    if let Some(method) = named_unbox_method(ty) {
        return Some(format!("logicaffeine_data::Value::{method}"));
    }
    if let Some(inner) = ty
        .strip_prefix("LogosSeq<")
        .and_then(|t| t.strip_suffix('>'))
    {
        return Some(format!(
            "|v: logicaffeine_data::Value| v.logos_seq({})",
            logos_value_unbox_fn(inner)?
        ));
    }
    is_program_boxable(ty).then(|| format!("logicaffeine_data::Value::logos_boxed::<{ty}>"))
}

/// The `Value` methods that unbox a scalar, or a `Seq` of scalars one or two levels deep, by name.
fn named_unbox_method(ty: &str) -> Option<&'static str> {
    match ty {
        "i64" => Some("logos_i64"),
        "f64" => Some("logos_f64"),
        "bool" => Some("logos_bool"),
        "String" => Some("logos_string"),
        "char" => Some("logos_char"),
        "LogosSeq<i64>" => Some("logos_seq_i64"),
        "LogosSeq<f64>" => Some("logos_seq_f64"),
        "LogosSeq<bool>" => Some("logos_seq_bool"),
        "LogosSeq<String>" => Some("logos_seq_string"),
        "LogosSeq<LogosSeq<i64>>" => Some("logos_seq_seq_i64"),
        "LogosSeq<LogosSeq<f64>>" => Some("logos_seq_seq_f64"),
        "LogosSeq<LogosSeq<bool>>" => Some("logos_seq_seq_bool"),
        "LogosSeq<LogosSeq<String>>" => Some("logos_seq_seq_string"),
        _ => None,
    }
}

/// Whether `rust_ty` names a record or variant the program declares that implements `Boxable` (see
/// `ProgramTypes::showable`), possibly applied to type arguments (`Pair<i64>`). A type whose
/// spelling holds a `,` is refused: marker bodies are comma-separated.
fn is_program_boxable(rust_ty: &str) -> bool {
    let base = rust_ty.split('<').next().unwrap_or(rust_ty);
    !rust_ty.contains(',') && PROGRAM.with(|p| p.borrow().showable_idents.contains(base))
}

/// The comma-joined type list inside a marker (`RECORD_FIELD_MARKER` / `TUPLE_ELEM_MARKER`), or `None`
/// if `ty` carries no such marker. The marker must be the TOP-LEVEL type (`Vec<Value> /* … */`), NOT
/// nested in a container (`LogosSeq<Vec<Value> /* … */>` is a SEQ of tuples — a seq, not a tuple), so a
/// `split_once(marker)` alone would wrongly fire; we require the exact `Vec<Value> ` prefix first. A
/// trailing hoisting suffix (`…*/|__hl:…`) is fine — `split_once(" */")` stops at the marker's close.
/// No marker type contains `,` (see [`is_program_boxable`]), so the inner `split(',')` at the call
/// sites is unambiguous.
fn marker_body<'a>(ty: &'a str, marker: &str) -> Option<&'a str> {
    // A tuple's Rust type is `LogosTuple /* logos_tuple: … */`; a record's is `Vec<Value> /* logos_record:
    // … */`. Both markers are TOP-LEVEL (a nested `LogosSeq<LogosTuple /* … */>` starts with `LogosSeq<`,
    // so neither prefix matches → correctly not treated as a bare tuple/record).
    let after_prefix = ty
        .strip_prefix("Vec<Value> ")
        .or_else(|| ty.strip_prefix("LogosTuple "))?
        .strip_prefix(marker)?;
    after_prefix.split_once(" */").map(|(body, _)| body)
}

/// Given a record variable's marked Rust type and a field name, the field's 1-based position in the
/// positional `Vec<Value>` (what `LogosIndex::logos_get` wants). `None` if not a marked record / no field.
pub(crate) fn logos_record_field_index(ty: &str, field: &str) -> Option<i64> {
    let body = marker_body(ty, RECORD_FIELD_MARKER)?;
    body.split(',')
        .position(|f| f.split('=').next().unwrap_or(f).trim() == field)
        .map(|p| p as i64 + 1)
}

/// Given a record variable's marked Rust type and a field name, the field's scalar Rust type (for the
/// unbox), or `None` if not a marked record / the field's type is non-scalar (unmarked).
pub(crate) fn logos_record_field_type<'a>(ty: &'a str, field: &str) -> Option<&'a str> {
    let body = marker_body(ty, RECORD_FIELD_MARKER)?;
    body.split(',').find_map(|f| {
        let (n, t) = f.split_once('=')?;
        (n.trim() == field).then(|| t.trim())
    })
}

/// Given a tuple variable's marked Rust type and a 1-based position, that element's scalar Rust type
/// (for the unbox), or `None` if not a marked tuple / out of range.
pub(crate) fn logos_tuple_elem_type(ty: &str, pos_1based: i64) -> Option<&str> {
    if pos_1based < 1 {
        return None;
    }
    marker_body(ty, TUPLE_ELEM_MARKER)?
        .split(',')
        .nth((pos_1based - 1) as usize)
        .map(|t| t.trim())
}

/// Whether every one of these Rust type strings can ride a `Value` and come back out (see
/// [`logos_value_unbox_call`]), so a marker carrying them is both parseable and unboxable. A
/// tuple/record with any other element gets no typed marker (records still carry names-only for
/// position resolution).
fn unboxable_marker_types(types: &[String]) -> bool {
    types.iter().all(|t| logos_value_unbox_call(t).is_some())
}

/// The Rust type of a tuple whose elements have the Rust types `elems`: a `LogosTuple` carrying them
/// in its marker when every one can come back out of a `Value`, a bare `LogosTuple` otherwise. The
/// one spelling a written tuple type, a tuple field and a tuple literal's binding share.
pub(crate) fn tuple_rust_type(elems: &[String]) -> String {
    if unboxable_marker_types(elems) {
        format!("LogosTuple {}{} */", TUPLE_ELEM_MARKER, elems.join(","))
    } else {
        "LogosTuple".to_string()
    }
}

/// What the program's types are, for the whole emission: installed once per program at codegen entry
/// ([`set_program_types`]), so a type's Rust spelling, its field layout and its impls are known
/// wherever code is emitted, whatever context its caller built.
#[derive(Default)]
struct ProgramTypes {
    /// The type names the program declares itself. A written type naming one the language also
    /// builds in reads as the program's: a program's own `Maybe` is its enum, never `Option`.
    declared: HashSet<String>,
    /// The records and enums the program's code defines (`callgraph::program_type_defs`).
    defined: HashSet<Symbol>,
    /// Those that show (see [`showable_program_types`]) and so also ride a tuple as `Boxable`: by
    /// symbol, and by Rust identifier.
    showable: HashSet<Symbol>,
    showable_idents: HashSet<String>,
    /// Every record, by its Rust identifier: each field's name and Rust type.
    records: HashMap<String, Vec<(Symbol, String)>>,
}

thread_local! {
    static PROGRAM: std::cell::RefCell<ProgramTypes> = std::cell::RefCell::new(ProgramTypes::default());
}

/// Whether the program's type `name` gets a `Showable` (and `Boxable`) impl.
fn program_type_shows(name: Symbol) -> bool {
    PROGRAM.with(|p| p.borrow().showable.contains(&name))
}

/// The Rust type of field `field` of the program's record `record` (its Rust identifier), if the
/// program declares one.
pub(crate) fn program_record_field_type(record: &str, field: Symbol) -> Option<String> {
    PROGRAM.with(|p| {
        p.borrow()
            .records
            .get(record)?
            .iter()
            .find(|(name, _)| *name == field)
            .map(|(_, ty)| ty.clone())
    })
}

/// Install the program's [`ProgramTypes`] — each part in turn, since the showable set reads which
/// types are defined and a field's Rust type reads what is declared and what shows.
pub(crate) fn set_program_types(
    stmts: &[Stmt],
    registry: &crate::analysis::registry::TypeRegistry,
    interner: &Interner,
) {
    let declared = registry
        .iter_types()
        .filter(|(_, def)| matches!(def, TypeDef::Struct { .. } | TypeDef::Enum { .. }))
        .map(|(sym, _)| interner.resolve(*sym).to_string())
        .collect();
    let defined = crate::analysis::callgraph::program_type_defs(stmts, registry);
    PROGRAM.with(|p| {
        *p.borrow_mut() = ProgramTypes {
            declared,
            defined,
            ..ProgramTypes::default()
        }
    });
    let showable = showable_program_types(registry, interner);
    let showable_idents = showable
        .iter()
        .map(|sym| rust_type_ident(interner.resolve(*sym)))
        .collect();
    PROGRAM.with(|p| {
        let mut p = p.borrow_mut();
        p.showable = showable;
        p.showable_idents = showable_idents;
    });
    let records = registry
        .iter_types()
        .filter_map(|(sym, def)| match def {
            TypeDef::Struct { fields, .. } => Some((
                rust_type_ident(interner.resolve(*sym)),
                fields
                    .iter()
                    .map(|f| (f.name, super::absent::field(*sym, f.name, codegen_field_type(&f.ty, interner))))
                    .collect(),
            )),
            _ => None,
        })
        .collect();
    PROGRAM.with(|p| p.borrow_mut().records = records);
}

/// `Maybe` is the built-in `Option` only when the program does not declare a `Maybe` of its own.
fn is_builtin_maybe() -> bool {
    !PROGRAM.with(|p| p.borrow().declared.contains("Maybe"))
}

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
    if field_is_boxed(&f.ty, enum_name, interner) {
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
        FieldType::TypeParam(_) | FieldType::Tuple(_) | FieldType::Record(_) => false,
    }
}

/// An enum is wire-serializable iff every field of every variant is [`field_wire_ok`].
fn enum_is_wire_ok(variants: &[VariantDef], interner: &Interner) -> bool {
    variants
        .iter()
        .all(|v| v.fields.iter().all(|f| field_wire_ok(&f.ty, interner)))
}

/// Emit `WireEncode`/`WireDecode` impls for a non-generic enum, byte-compatible with the peer
/// codec (proven by `concurrency::marshal::tests::peer_and_wire_core_produce_identical_bytes`).
/// Each value is an `Inductive`: header (`type_name`, `constructor`, arg count) then each field.
fn emit_enum_wire_impls(
    output: &mut String,
    enum_name: &str,
    variants: &[VariantDef],
    interner: &Interner,
) {
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
            let binds: Vec<String> = variant
                .fields
                .iter()
                .map(|f| super::escape_rust_ident(interner.resolve(f.name)))
                .collect();
            let shape = VariantShape::of(variant, enum_name, interner);
            let named: Vec<(String, String)> =
                binds.iter().map(|b| (b.clone(), b.clone())).collect();
            let (pattern, prelude) = shape.pattern(&format!("{enum_name}::{vname}"), &named, false);
            writeln!(output, "            {pattern} => {{ {prelude}").unwrap();
            writeln!(output, "                {w}::write_inductive_header(__out, \"{enum_name}\", \"{vname}\", {}u64);", variant.fields.len()).unwrap();
            for b in &binds {
                writeln!(
                    output,
                    "                {w}::WireEncode::wire_encode({b}, __out);"
                )
                .unwrap();
            }
            writeln!(output, "            }}").unwrap();
        }
    }
    writeln!(output, "        }}").unwrap();
    writeln!(output, "    }}").unwrap();
    writeln!(output, "}}").unwrap();
    // ── WireDecode ──
    writeln!(output, "impl {w}::WireDecode for {enum_name} {{").unwrap();
    writeln!(
        output,
        "    fn wire_decode(__buf: &[u8], __pos: &mut usize) -> Option<Self> {{"
    )
    .unwrap();
    writeln!(
        output,
        "        let (_ty, __ctor, _n) = {w}::read_inductive_header(__buf, __pos)?;"
    )
    .unwrap();
    writeln!(output, "        Some(match __ctor.as_str() {{").unwrap();
    for variant in variants {
        let vname = interner.resolve(variant.name);
        if variant.fields.is_empty() {
            writeln!(output, "            \"{vname}\" => {enum_name}::{vname},").unwrap();
        } else {
            let fields: Vec<(String, String)> = variant
                .fields
                .iter()
                .map(|f| {
                    let fname = super::escape_rust_ident(interner.resolve(f.name));
                    let fty = wire_field_type(f, enum_name, interner);
                    (
                        fname,
                        format!("<{fty} as {w}::WireDecode>::wire_decode(__buf, __pos)?"),
                    )
                })
                .collect();
            let built = VariantShape::of(variant, enum_name, interner)
                .build(&format!("{enum_name}::{vname}"), &fields);
            writeln!(output, "            \"{vname}\" => {built},").unwrap();
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
        TypeExpr::Primitive(sym) => map_type_to_rust(interner.resolve(*sym)),
        TypeExpr::Named(sym) => {
            let name = interner.resolve(*sym);
            // Check for common mappings
            map_type_to_rust(name)
        }
        TypeExpr::Generic { base, params } => {
            let base_name = interner.resolve(*base);
            let params_str: Vec<String> = params
                .iter()
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
                "Option" => {
                    if !params_str.is_empty() {
                        format!("Option<{}>", params_str[0])
                    } else {
                        "Option<()>".to_string()
                    }
                }
                "Maybe" if is_builtin_maybe() => {
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
                    let base = rust_type_ident(other);
                    if params_str.is_empty() {
                        base
                    } else {
                        format!("{}<{}>", base, params_str.join(", "))
                    }
                }
            }
        }
        // A tuple `(Int, Text)` is a positional, heterogeneous `Vec<Value>` — matching the VALUE emission
        // (`Expr::Tuple` → `vec![Value::from(..)]`) so a `-> (Int, Int)` return type-checks, and `item N of
        // t` resolves via `LogosIndex` over the `Vec` (`Vec<T>: LogosIndex<i64>`). A scalar-element tuple
        // also carries its per-position element TYPES in a marker comment (valid Rust, ignored by the
        // compiler) so a constant `item N of t` can UNBOX the boxed `Value` to the scalar the use site
        // wants (`upper(item 2 of t)` needs a `String`, not a `Value`). Non-scalar elements can't ride a
        // `Value` at all, so those get a bare `Vec<Value>` (extraction stays a `Value`, fine for `Show`).
        TypeExpr::Tuple(params) => {
            let elems: Vec<String> = params
                .iter()
                .map(|p| codegen_type_expr(p, interner))
                .collect();
            tuple_rust_type(&elems)
        }
        // A named record `(quotient: Int, remainder: Int)` is the SAME positional `Vec<Value>`, but its
        // FIELD ORDER (+ scalar field TYPES) ride along in a marker comment — the only per-variable channel
        // from the record type to `FieldAccess`, where `r's quotient` reads the marker to resolve the field
        // NAME to its 1-based `Vec` position (`logos_record_field_index`) and UNBOX it to the field's scalar
        // type (`logos_record_field_type`). A non-scalar-fielded record keeps names-only (position resolves;
        // unbox falls back to `Value`) — such a record can't ride `Value` anyway.
        TypeExpr::Record {
            field_names,
            field_types,
        } => {
            let types: Vec<String> = field_types
                .iter()
                .map(|t| codegen_type_expr(t, interner))
                .collect();
            let body = if unboxable_marker_types(&types) {
                field_names
                    .iter()
                    .zip(types.iter())
                    .map(|(n, t)| format!("{}={}", interner.resolve(*n), t))
                    .collect::<Vec<_>>()
                    .join(",")
            } else {
                field_names
                    .iter()
                    .map(|s| interner.resolve(*s).to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            };
            // A named record is the SAME positional `LogosTuple` value (`Expr::Tuple` construction) — it just
            // carries field NAMES in the marker for `r's field` resolution — so its Rust type must be
            // `LogosTuple` too, matching the constructed value.
            format!("LogosTuple {}{} */", RECORD_FIELD_MARKER, body)
        }
        TypeExpr::Function { inputs, output } => {
            let inputs_str: Vec<String> = inputs
                .iter()
                .map(|i| codegen_type_expr(i, interner))
                .collect();
            let output_str = codegen_type_expr(output, interner);
            format!("impl Fn({}) -> {}", inputs_str.join(", "), output_str)
        }
        // Phase 43C: Refinement types use the base type for Rust type annotation
        // The constraint predicate is handled separately via debug_assert!
        TypeExpr::Refinement { base, .. } => codegen_type_expr(base, interner),
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
    registry: &TypeRegistry,
    interner: &Interner,
) -> Option<String> {
    use crate::analysis::types::{LogosType, TypeEnv};

    let mut env = TypeEnv::new();
    for (sym, ty) in params {
        env.register(*sym, LogosType::from_type_expr(ty, interner));
    }

    // The Rust return type for a returned expression. A struct FIELD access is
    // resolved directly from the base param's declared struct type via the
    // registry — the checker leaves such a param a free var and `LogosType`
    // can't carry a user type's name, so neither `type_env` nor the plain
    // `infer_expr` resolves `p's x`. Everything else goes through the
    // checker-parity renderer. `None` means unresolved (the caller then defaults
    // to `i64`, never the illegal placeholder `_`).
    fn resolve(
        expr: &Expr,
        env: &TypeEnv,
        params: &[(Symbol, &TypeExpr)],
        registry: &TypeRegistry,
        interner: &Interner,
    ) -> Option<String> {
        if let Expr::FieldAccess { object, field } = expr {
            if let Expr::Identifier(var) = object {
                if let Some(sym) = params
                    .iter()
                    .find(|(p, _)| p == var)
                    .and_then(|(_, ty)| user_type_symbol(ty))
                {
                    if let Some(TypeDef::Struct { fields, .. }) = registry.get(sym) {
                        if let Some(fd) = fields.iter().find(|f| f.name == *field) {
                            return Some(codegen_field_type(&fd.ty, interner));
                        }
                    }
                }
            }
        }
        logos_return_type_to_rust(&env.infer_expr(expr, interner), interner)
    }

    fn walk(
        stmts: &[Stmt],
        env: &mut TypeEnv,
        params: &[(Symbol, &TypeExpr)],
        registry: &TypeRegistry,
        interner: &Interner,
        found: &mut Option<String>,
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
                    // Keep the FIRST resolvable inference; a genuinely
                    // polymorphic body needs an annotation.
                    if found.is_none() {
                        *found = resolve(v, env, params, registry, interner);
                    }
                }
                Stmt::If {
                    then_block,
                    else_block,
                    ..
                } => {
                    walk(then_block, env, params, registry, interner, found);
                    if let Some(b) = else_block {
                        walk(b, env, params, registry, interner, found);
                    }
                }
                Stmt::While { body, .. } | Stmt::Repeat { body, .. } | Stmt::Zone { body, .. } => {
                    walk(body, env, params, registry, interner, found)
                }
                Stmt::Inspect { arms, .. } => {
                    for arm in arms {
                        walk(arm.body, env, params, registry, interner, found);
                    }
                }
                _ => {}
            }
        }
    }

    let mut found = None;
    walk(body, &mut env, params, registry, interner, &mut found);
    let has_return = found.is_some()
        || body
            .iter()
            .any(|s| matches!(s, Stmt::Return { value: Some(_) }));
    found.or_else(|| has_return.then(|| "i64".to_string()))
}

/// The interned type name of a (possibly wrapped) named/primitive type expr —
/// e.g. `Point` from `p: Point`, `mutable Point`, or a refinement over it.
fn user_type_symbol(ty: &TypeExpr) -> Option<Symbol> {
    match ty {
        TypeExpr::Named(s) | TypeExpr::Primitive(s) => Some(*s),
        TypeExpr::Mutable { inner } | TypeExpr::Persistent { inner } => user_type_symbol(inner),
        TypeExpr::Refinement { base, .. } => user_type_symbol(base),
        _ => None,
    }
}

/// Render a checker-inferred [`LogosType`](crate::analysis::types::LogosType) as
/// a Rust type string for a function signature — the source of truth for an
/// OMITTED return type. Unlike `LogosType::to_rust_type` (which has no interner
/// and so renders a `UserDefined` struct as the illegal placeholder `_`), this
/// resolves the struct's interned name (sanitizing a cross-module `::`), and
/// composes over the container types. Returns `None` when the type is genuinely
/// unresolved (`Unknown`, or a container of one), so the caller falls back to the
/// body heuristic rather than emit `_`.
pub(super) fn logos_return_type_to_rust(
    ty: &crate::analysis::types::LogosType,
    interner: &Interner,
) -> Option<String> {
    use crate::analysis::types::LogosType;
    Some(match ty {
        LogosType::Unknown => return None,
        LogosType::Function(..) => return None,
        LogosType::UserDefined(sym) => rust_type_ident(interner.resolve(*sym)),
        LogosType::Seq(inner) => {
            format!("LogosSeq<{}>", logos_return_type_to_rust(inner, interner)?)
        }
        LogosType::Set(inner) => format!("Set<{}>", logos_return_type_to_rust(inner, interner)?),
        LogosType::Option(inner) => {
            format!("Option<{}>", logos_return_type_to_rust(inner, interner)?)
        }
        LogosType::Map(k, v) => format!(
            "LogosMap<{}, {}>",
            logos_return_type_to_rust(k, interner)?,
            logos_return_type_to_rust(v, interner)?
        ),
        other => other.to_rust_type(),
    })
}

/// Map LOGOS type names to Rust types.
pub(crate) fn map_type_to_rust(ty: &str) -> String {
    match ty {
        "Int" | "Element" | "Event" | "View" => "i64".to_string(),
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
        "ComplexF" => "LogosComplexF".to_string(),
        "Modular" => "LogosModular".to_string(),
        "Algebraic" | "ExactReal" | "Hyperreal" | "Hypernatural" | "Ordinal" => {
            super::tower::VALUE.to_string()
        }
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
        // The counter CRDTs: a `ConvergentCount` only grows, a `Tally` also shrinks.
        "ConvergentCount" => "logicaffeine_data::crdt::GCounter".to_string(),
        "Tally" => "logicaffeine_data::crdt::PNCounter".to_string(),
        // A user-defined type — possibly a cross-module reference (`Geometry::Point`).
        // Sanitize the `::` path separator into a valid Rust identifier so the
        // declaration, construction, and every field/annotation reference agree.
        other => rust_type_ident(other),
    }
}

/// Map a LOGOS type name to a valid Rust identifier. Cross-module references are
/// interned as `Alias::Type` (discovery's namespace convention); Rust cannot use
/// `::` in a type identifier, so it lowers to `Alias__Type`. Plain names pass
/// through unchanged, so single-module programs are byte-identical to before.
pub(crate) fn rust_type_ident(name: &str) -> String {
    if name.contains("::") {
        name.replace("::", "__")
    } else {
        name.to_string()
    }
}

/// Generate a single struct definition with derives and visibility.
/// Phase 34: Now supports generic type parameters.
/// Phase 47: Now supports is_portable for Serialize/Deserialize derives.
/// Phase 49: Now supports is_shared for CRDT Merge impl.
pub(super) fn codegen_struct_def(
    name: Symbol,
    fields: &[FieldDef],
    generics: &[Symbol],
    is_portable: bool,
    is_shared: bool,
    interner: &Interner,
    indent: usize,
    c_abi_value_structs: &HashSet<Symbol>,
    c_abi_ref_structs: &HashSet<Symbol>,
) -> String {
    let ind = " ".repeat(indent);
    let mut output = String::new();

    // Build generic parameter string: <T, U> or empty
    let generic_str = if generics.is_empty() {
        String::new()
    } else {
        let params: Vec<&str> = generics.iter().map(|g| interner.resolve(*g)).collect();
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
        writeln!(
            output,
            "{}#[derive(Default, Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]",
            ind
        )
        .unwrap();
    } else {
        writeln!(output, "{}#[derive(Default, Debug, Clone, PartialEq)]", ind).unwrap();
    }
    writeln!(
        output,
        "{}pub struct {}{} {{",
        ind,
        rust_type_ident(interner.resolve(name)),
        generic_str
    )
    .unwrap();

    for field in fields {
        let vis = if field.is_public { "pub " } else { "" };
        // A field that some construction feeds a bignum-promoted value must be stored as `LogosInt`
        // (arbitrary precision), matching the interpreter's `Int` — see `bigint_promote::bigint_struct_fields`.
        let rust_type = if crate::codegen::expr::stores_logos_int(name, field.name) {
            "LogosInt".to_string()
        } else {
            codegen_field_type(&field.ty, interner)
        };
        let rust_type = super::absent::field(name, field.name, rust_type);
        writeln!(
            output,
            "{}    {}{}: {},",
            ind,
            vis,
            super::escape_rust_ident(interner.resolve(field.name)),
            rust_type
        )
        .unwrap();
    }

    writeln!(output, "{}}}\n", ind).unwrap();

    // Phase 49: Generate the Merge trait impl (additive, `Merge`/`Synced`) AND the inherent `join` (idempotent,
    // `Converge`) for Shared structs — both fold every field ≡ the VM's `crdt_merge_field`/`crdt_join_field`.
    if is_shared {
        output.push_str(&codegen_merge_impl(
            name, fields, generics, interner, indent,
        ));
        output.push_str(&codegen_join_impl(name, fields, generics, interner, indent));
    }

    // `Show <struct>` renders `Name { field: value }` — byte-identical to the interpreter oracle
    // (brace-delimited, fields SORTED by name) — and the struct rides a tuple element. Only for a
    // struct every field of which shows (see `showable_program_types`).
    if program_type_shows(name) {
        output.push_str(&codegen_show_impl(name, fields, generics, interner, indent));
        output.push_str(&codegen_boxable_impl(name, generics, interner, indent));
    }

    output
}

/// Whether the program's type `name` (registry entry `def`) becomes a Rust type in its `user_types`
/// module: one the program defines (`ProgramTypes::defined`) that is a struct with fields or type
/// parameters, or an enum with variants or type parameters. The one rule the emitter and
/// [`showable_program_types`] both read.
pub(super) fn emits_user_type(name: Symbol, def: &TypeDef) -> bool {
    let shaped = match def {
        TypeDef::Struct {
            fields, generics, ..
        } => !fields.is_empty() || !generics.is_empty(),
        TypeDef::Enum {
            variants, generics, ..
        } => !variants.is_empty() || !generics.is_empty(),
        _ => false,
    };
    shaped && PROGRAM.with(|p| p.borrow().defined.contains(&name))
}

/// The records and variants the program declares that get a `Showable` impl (and with it `Boxable`):
/// the greatest set in which every field of every member shows, where a field shows when its type is
/// a core showable scalar, a showable container over showable parameters, a type parameter (the impl
/// bounds it), or a member of the set. Greatest, so a variant holding itself (boxed) or a record
/// holding a list of itself shows; every member's impl compiles because each field it formats is a
/// member or core. A type with any other field (a CRDT, a function, a non-showing type) is left
/// out, and so is everything that holds it — `Show` on those stays unsupported rather than failing.
fn showable_program_types(registry: &TypeRegistry, interner: &Interner) -> HashSet<Symbol> {
    let fields_of = |def: &TypeDef| -> Vec<FieldType> {
        match def {
            TypeDef::Struct { fields, .. } => fields.iter().map(|f| f.ty.clone()).collect(),
            TypeDef::Enum { variants, .. } => variants
                .iter()
                .flat_map(|v| v.fields.iter().map(|f| f.ty.clone()))
                .collect(),
            _ => Vec::new(),
        }
    };
    let mut set: HashSet<Symbol> = registry
        .iter_types()
        .filter(|(sym, def)| emits_user_type(**sym, def))
        .map(|(sym, _)| *sym)
        .collect();
    loop {
        let unshowable: Vec<Symbol> = set
            .iter()
            .copied()
            .filter(|sym| {
                registry.get(*sym).map_or(true, |def| {
                    !fields_of(def)
                        .iter()
                        .all(|ty| field_type_is_showable(ty, &set, interner))
                })
            })
            .collect();
        if unshowable.is_empty() {
            return set;
        }
        for sym in unshowable {
            set.remove(&sym);
        }
    }
}

/// Whether a field of type `ty` shows, given the program types that do (`showable`).
fn field_type_is_showable(ty: &FieldType, showable: &HashSet<Symbol>, interner: &Interner) -> bool {
    match ty {
        FieldType::Primitive(sym) | FieldType::Named(sym) => {
            is_core_showable_scalar(interner.resolve(*sym)) || showable.contains(sym)
        }
        FieldType::Generic { base, params } => {
            let container = match interner.resolve(*base) {
                "Seq" | "List" | "Set" | "Map" | "Optional" | "Option" | "Result" => true,
                "Maybe" => is_builtin_maybe() || showable.contains(base),
                _ => showable.contains(base),
            };
            container
                && params
                    .iter()
                    .all(|p| field_type_is_showable(p, showable, interner))
        }
        FieldType::TypeParam(_) => true,
        FieldType::Tuple(items) => items
            .iter()
            .all(|t| field_type_is_showable(t, showable, interner)),
        FieldType::Record(fields) => fields
            .iter()
            .all(|(_, t)| field_type_is_showable(t, showable, interner)),
    }
}

/// LOGOS type names whose Rust lowering has a first-class `Showable` impl (see
/// `logicaffeine_system::io`). Kept tight: an omission only costs a struct its generated `Show`
/// (no regression), whereas a wrong inclusion would emit an impl that fails to compile.
fn is_core_showable_scalar(name: &str) -> bool {
    matches!(
        name,
        "Int"
            | "Nat"
            | "Bool"
            | "Text"
            | "Char"
            | "Real"
            | "Float"
            | "Rational"
            | "Decimal"
            | "Complex"
            | "Modular"
            | "Quantity"
            | "Money"
            | "Uuid"
            | "BigInt"
    )
}

/// Emit `impl Showable for <Struct>` producing `Name { field: value, … }` with fields sorted by name
/// (matching the interpreter oracle's deterministic order) and each value rendered via its own
/// `format_show`. An empty struct shows just its name. Generic params are bounded `: Showable`.
fn codegen_show_impl(
    name: Symbol,
    fields: &[FieldDef],
    generics: &[Symbol],
    interner: &Interner,
    indent: usize,
) -> String {
    let ind = " ".repeat(indent);
    // The oracle prints the bare LOGOS type name (`type_name`), never the namespaced Rust ident.
    let display_name = interner
        .resolve(name)
        .rsplit("::")
        .next()
        .unwrap_or_else(|| interner.resolve(name));
    let (bounds, ty) = impl_generics(
        name,
        generics,
        "logicaffeine_system::io::Showable",
        interner,
    );
    let mut output = String::new();

    let mut sorted: Vec<&FieldDef> = fields.iter().collect();
    sorted.sort_by(|a, b| interner.resolve(a.name).cmp(interner.resolve(b.name)));

    writeln!(
        output,
        "{}impl{} logicaffeine_system::io::Showable for {} {{",
        ind, bounds, ty
    )
    .unwrap();
    writeln!(
        output,
        "{}    fn format_show(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {{",
        ind
    )
    .unwrap();
    if sorted.is_empty() {
        writeln!(output, "{}        write!(f, \"{}\")", ind, display_name).unwrap();
    } else {
        writeln!(
            output,
            "{}        write!(f, \"{} {{{{ \")?;",
            ind, display_name
        )
        .unwrap();
        for (i, fld) in sorted.iter().enumerate() {
            let label = interner.resolve(fld.name);
            if i > 0 {
                writeln!(output, "{}        write!(f, \", \")?;", ind).unwrap();
            }
            writeln!(output, "{}        write!(f, \"{}: \")?;", ind, label).unwrap();
            writeln!(
                output,
                "{}        logicaffeine_system::io::Showable::format_show(&self.{}, f)?;",
                ind,
                super::escape_rust_ident(label)
            )
            .unwrap();
        }
        writeln!(output, "{}        write!(f, \" }}}}\")", ind).unwrap();
    }
    writeln!(output, "{}    }}", ind).unwrap();
    writeln!(output, "{}}}\n", ind).unwrap();

    output
}

/// The `impl<…>` bounds and the `Name<…>` application a generated impl for a program type uses: each
/// type parameter bounded by `bound`.
fn impl_generics(
    name: Symbol,
    generics: &[Symbol],
    bound: &str,
    interner: &Interner,
) -> (String, String) {
    let name_str = rust_type_ident(interner.resolve(name));
    if generics.is_empty() {
        return (String::new(), name_str);
    }
    let params: Vec<&str> = generics.iter().map(|g| interner.resolve(*g)).collect();
    let bounds = params
        .iter()
        .map(|p| format!("{p}: {bound}"))
        .collect::<Vec<_>>()
        .join(", ");
    (
        format!("<{bounds}>"),
        format!("{}<{}>", name_str, params.join(", ")),
    )
}

/// Emit `impl Showable for <Enum>` rendering a value the interpreter's way: a unit variant as its
/// name, a payload variant as `Variant(a, b)` with its fields in declaration order. A type the program
/// names `Option` shows as its payload or `nothing`, as the interpreter's `Option` does.
fn codegen_enum_show_impl(
    name: Symbol,
    variants: &[VariantDef],
    generics: &[Symbol],
    interner: &Interner,
    indent: usize,
) -> String {
    let ind = " ".repeat(indent);
    let (bounds, ty) = impl_generics(
        name,
        generics,
        "logicaffeine_system::io::Showable",
        interner,
    );
    let enum_str = rust_type_ident(interner.resolve(name));
    let transparent = interner.resolve(name).rsplit("::").next() == Some("Option");
    let mut output = String::new();
    writeln!(
        output,
        "{ind}impl{bounds} logicaffeine_system::io::Showable for {ty} {{"
    )
    .unwrap();
    writeln!(
        output,
        "{ind}    fn format_show(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {{"
    )
    .unwrap();
    writeln!(output, "{ind}        match self {{").unwrap();
    for variant in variants {
        let vname = interner.resolve(variant.name);
        let shown = vname.rsplit("::").next().unwrap_or(vname);
        if variant.fields.is_empty() {
            let text = if transparent { "nothing" } else { shown };
            writeln!(
                output,
                "{ind}            {enum_str}::{vname} => write!(f, \"{text}\"),"
            )
            .unwrap();
            continue;
        }
        let binds: Vec<(String, String)> = variant
            .fields
            .iter()
            .enumerate()
            .map(|(i, fd)| {
                (
                    super::escape_rust_ident(interner.resolve(fd.name)),
                    format!("__f{i}"),
                )
            })
            .collect();
        let (pattern, prelude) = VariantShape::of(variant, interner.resolve(name), interner)
            .pattern(&format!("{enum_str}::{vname}"), &binds, false);
        writeln!(output, "{ind}            {pattern} => {{").unwrap();
        if !prelude.is_empty() {
            writeln!(output, "{ind}                {prelude}").unwrap();
        }
        if transparent {
            writeln!(
                output,
                "{ind}                logicaffeine_system::io::Showable::format_show(__f0, f)"
            )
            .unwrap();
        } else {
            writeln!(output, "{ind}                write!(f, \"{shown}(\")?;").unwrap();
            for i in 0..variant.fields.len() {
                if i > 0 {
                    writeln!(output, "{ind}                write!(f, \", \")?;").unwrap();
                }
                writeln!(output, "{ind}                logicaffeine_system::io::Showable::format_show(__f{i}, f)?;").unwrap();
            }
            writeln!(output, "{ind}                write!(f, \")\")").unwrap();
        }
        writeln!(output, "{ind}            }}").unwrap();
    }
    writeln!(output, "{ind}        }}").unwrap();
    writeln!(output, "{ind}    }}").unwrap();
    writeln!(output, "{ind}}}\n").unwrap();
    output
}

/// Emit `impl Boxable for <Type>` so a value of it rides a tuple element as a `Value::Boxed`, rendered
/// by its own `Showable`.
fn codegen_boxable_impl(
    name: Symbol,
    generics: &[Symbol],
    interner: &Interner,
    indent: usize,
) -> String {
    let ind = " ".repeat(indent);
    let (bounds, ty) = impl_generics(
        name,
        generics,
        "logicaffeine_system::io::Showable + Clone + PartialEq + std::fmt::Debug + 'static",
        interner,
    );
    let mut output = String::new();
    writeln!(
        output,
        "{ind}impl{bounds} logicaffeine_data::Boxable for {ty} {{"
    )
    .unwrap();
    writeln!(
        output,
        "{ind}    fn render(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{"
    )
    .unwrap();
    writeln!(
        output,
        "{ind}        logicaffeine_system::io::Showable::format_show(self, f)"
    )
    .unwrap();
    writeln!(output, "{ind}    }}").unwrap();
    writeln!(output, "{ind}}}\n").unwrap();
    output
}

/// Whether a field's declared type is a bare `Int` (an `i64` in the emitted Rust) — the ONE plain kind that
/// folds arithmetically under a CRDT merge/join (`wrapping_add` / `max`), matching the VM's `crdt_merge_field`
/// / `crdt_join_field` `(Int, Int)` arms. Every other plain kind is last-writer-wins.
fn is_int_field(ty: &FieldType, interner: &Interner) -> bool {
    matches!(
        ty,
        FieldType::Named(s) | FieldType::Primitive(s) if interner.resolve(*s) == "Int"
    )
}

/// The per-field fold body shared by `merge` and `join` — the ONE place the VM's field-wise CRDT combine is
/// mirrored, so the two can never drift. For each field: a CRDT-typed field takes its lattice merge
/// (`self.f.merge(&other.f)`, idempotent — identical under merge AND join); a bare-`Int` field folds
/// arithmetically (`wrapping_add` for merge, `max` for the idempotent join); every other field is
/// last-writer-wins (`self.f = other.f.clone()`, the VM's `_ => incoming`).
fn crdt_combine_body(fields: &[FieldDef], interner: &Interner, ind: &str, is_join: bool) -> String {
    let mut body = String::new();
    for field in fields {
        let f = super::escape_rust_ident(interner.resolve(field.name));
        if is_crdt_field_type(&field.ty, interner) {
            writeln!(body, "{}        self.{}.merge(&other.{});", ind, f, f).unwrap();
        } else if is_int_field(&field.ty, interner) {
            if is_join {
                writeln!(
                    body,
                    "{}        self.{} = self.{}.max(other.{});",
                    ind, f, f, f
                )
                .unwrap();
            } else {
                writeln!(
                    body,
                    "{}        self.{} = self.{}.wrapping_add(other.{});",
                    ind, f, f, f
                )
                .unwrap();
            }
        } else {
            writeln!(body, "{}        self.{} = other.{}.clone();", ind, f, f).unwrap();
        }
    }
    body
}

/// Phase 49: Generate `impl logicaffeine_data::crdt::Merge` for a Shared struct — the ADDITIVE field-wise
/// combine (`Merge <src> into <tgt>` AND the `Synced<T>` network merge). Now folds EVERY field to match the
/// VM's `crdt_merge_field` exactly (it used to silently skip plain `Int`/`Text` fields).
pub(super) fn codegen_merge_impl(
    name: Symbol,
    fields: &[FieldDef],
    generics: &[Symbol],
    interner: &Interner,
    indent: usize,
) -> String {
    let ind = " ".repeat(indent);
    let name_str = rust_type_ident(interner.resolve(name));
    let mut output = String::new();
    let generic_str = if generics.is_empty() {
        String::new()
    } else {
        format!(
            "<{}>",
            generics
                .iter()
                .map(|g| interner.resolve(*g))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    writeln!(
        output,
        "{}impl{} logicaffeine_data::crdt::Merge for {}{} {{",
        ind, generic_str, name_str, generic_str
    )
    .unwrap();
    writeln!(output, "{}    fn merge(&mut self, other: &Self) {{", ind).unwrap();
    output.push_str(&crdt_combine_body(fields, interner, &ind, false));
    writeln!(output, "{}    }}", ind).unwrap();
    writeln!(output, "{}}}\n", ind).unwrap();
    output
}

/// The IDEMPOTENT lattice join (`Converge <src> into <tgt>`) — an INHERENT `fn join` (not a trait, so it never
/// clashes with `Merge::merge`). Identical to `merge` EXCEPT a bare-`Int` field takes per-field MAX, so applying
/// a redelivered state twice never double-counts — matching the VM's `crdt_join_field`.
pub(super) fn codegen_join_impl(
    name: Symbol,
    fields: &[FieldDef],
    generics: &[Symbol],
    interner: &Interner,
    indent: usize,
) -> String {
    let ind = " ".repeat(indent);
    let name_str = rust_type_ident(interner.resolve(name));
    let mut output = String::new();
    let generic_str = if generics.is_empty() {
        String::new()
    } else {
        format!(
            "<{}>",
            generics
                .iter()
                .map(|g| interner.resolve(*g))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    writeln!(
        output,
        "{}impl{} {}{} {{",
        ind, generic_str, name_str, generic_str
    )
    .unwrap();
    writeln!(output, "{}    pub fn join(&mut self, other: &Self) {{", ind).unwrap();
    output.push_str(&crdt_combine_body(fields, interner, &ind, true));
    writeln!(output, "{}    }}", ind).unwrap();
    writeln!(output, "{}}}\n", ind).unwrap();
    output
}

/// Phase 49: Check if a field type is a CRDT type that implements Merge.
pub(super) fn is_crdt_field_type(ty: &FieldType, interner: &Interner) -> bool {
    match ty {
        FieldType::Named(sym) => is_crdt_counter_name(interner.resolve(*sym)),
        FieldType::Generic { base, .. } => is_crdt_generic_name(interner.resolve(*base)),
        _ => false,
    }
}

/// The counter CRDTs, written as plain type names.
fn is_crdt_counter_name(name: &str) -> bool {
    matches!(name, "ConvergentCount" | "GCounter" | "Tally" | "PNCounter")
}

/// The CRDTs that take a type argument (`SharedSet of Text`).
fn is_crdt_generic_name(name: &str) -> bool {
    matches!(
        name,
        "LastWriteWins"
            | "LWWRegister"
            | "SharedSet"
            | "ORSet"
            | "SharedSet_AddWins"
            | "SharedSet_RemoveWins"
            | "SharedSequence"
            | "RGA"
            | "SharedSequence_YATA"
            | "CollaborativeSequence"
            | "SharedMap"
            | "ORMap"
            | "Divergent"
            | "MVRegister"
    )
}

/// Whether `name` is a type the language itself supplies rather than one a program declares: a scalar
/// the one type table maps to a Rust type of its own (`map_type_to_rust`), a built-in generic (the
/// language's own arity table, `builtin_generic_param_count`: the collections, `Option`/`Result`,
/// `Pair`/`Triple`, the collection CRDTs), or a CRDT.
pub(crate) fn is_language_type_name(name: &str) -> bool {
    map_type_to_rust(name) != rust_type_ident(name)
        || crate::analysis::registry::builtin_generic_param_count(name).is_some()
        || is_crdt_counter_name(name)
        || is_crdt_generic_name(name)
}

/// Phase 33/34: Generate enum definition with optional generic parameters.
/// Phase 47: Now supports is_portable for Serialize/Deserialize derives.
/// Phase 49: Now accepts is_shared parameter (enums don't generate Merge impl yet).
pub(super) fn codegen_enum_def(
    name: Symbol,
    variants: &[VariantDef],
    generics: &[Symbol],
    is_portable: bool,
    _is_shared: bool,
    interner: &Interner,
    indent: usize,
) -> String {
    let ind = " ".repeat(indent);
    let mut output = String::new();

    // Build generic parameter string: <T, U> or empty
    let generic_str = if generics.is_empty() {
        String::new()
    } else {
        let params: Vec<&str> = generics.iter().map(|g| interner.resolve(*g)).collect();
        format!("<{}>", params.join(", "))
    };

    // The RAW interned name — used for recursion detection, where the field type
    // is compared against this exact interned string.
    let enum_name_raw = interner.resolve(name);
    // The SANITIZED Rust ident — a cross-module reference (`Colors::Filter`) lowers
    // to `Colors__Filter` at EVERY emission site (decl, Default impl, wire impls),
    // exactly as the struct path does. Plain names pass through unchanged.
    let enum_name_str = rust_type_ident(enum_name_raw);
    let enum_name_str = enum_name_str.as_str();

    // Phase 47: Add Serialize, Deserialize derives if portable
    let derives = if is_portable {
        "#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]"
    } else {
        "#[derive(Debug, Clone, PartialEq)]"
    };
    writeln!(output, "{}{}", ind, derives).unwrap();
    writeln!(
        output,
        "{}pub enum {}{} {{",
        ind, enum_name_str, generic_str
    )
    .unwrap();

    // A field's Rust type, boxed when it holds the enum and its variant is not boxed whole. Recursion is detected
    // against the RAW interned name: the field type carries the same interned string, not the sanitized ident.
    let field_decl = |variant: &VariantDef, f: &FieldDef| -> String {
        // A variant field fed a bignum-promoted value is `LogosInt`, keyed by the variant name — json's
        // `JNum { n }` (`expr::stores_logos_int`: never a generic enum's).
        let rust_type = if crate::codegen::expr::stores_logos_int(variant.name, f.name) {
            "LogosInt".to_string()
        } else {
            codegen_field_type(&f.ty, interner)
        };
        let field_name = super::escape_rust_ident(interner.resolve(f.name));
        if field_is_boxed(&f.ty, enum_name_raw, interner) {
            format!("{}: Box<{}>", field_name, rust_type)
        } else {
            format!("{}: {}", field_name, rust_type)
        }
    };
    let mut payloads = String::new();
    for variant in variants {
        let variant_name = interner.resolve(variant.name);
        if variant.fields.is_empty() {
            writeln!(output, "{}    {},", ind, variant_name).unwrap();
            continue;
        }
        let fields_str: Vec<String> = variant
            .fields
            .iter()
            .map(|f| field_decl(variant, f))
            .collect();
        match VariantShape::of(variant, enum_name_raw, interner) {
            VariantShape::InPlace => {
                writeln!(
                    output,
                    "{}    {} {{ {} }},",
                    ind,
                    variant_name,
                    fields_str.join(", ")
                )
                .unwrap();
            }
            VariantShape::Boxed { payload } => {
                // The payload takes the enum's type parameters its fields name, so an unused one is no error.
                let params: Vec<&str> = generics
                    .iter()
                    .filter(|g| variant.fields.iter().any(|f| field_names_param(&f.ty, **g)))
                    .map(|g| interner.resolve(*g))
                    .collect();
                let payload_ty = if params.is_empty() {
                    payload.clone()
                } else {
                    format!("{}<{}>", payload, params.join(", "))
                };
                writeln!(output, "{}    {}(Box<{}>),", ind, variant_name, payload_ty).unwrap();
                writeln!(payloads, "{}{}", ind, derives).unwrap();
                writeln!(payloads, "{}#[allow(non_camel_case_types)]", ind).unwrap();
                writeln!(
                    payloads,
                    "{}pub struct {} {{ {} }}\n",
                    ind,
                    payload_ty,
                    fields_str
                        .iter()
                        .map(|f| format!("pub {f}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
                .unwrap();
            }
        }
    }

    writeln!(output, "{}}}\n", ind).unwrap();
    output.push_str(&payloads);

    // Generate Default impl for enum (defaults to first variant)
    // This is needed when the enum is used as a struct field and the struct derives Default
    // Only for non-generic enums — generic enums can't assume their type params implement Default
    if generics.is_empty() {
        if let Some(first_variant) = variants.first() {
            let first_variant_name = interner.resolve(first_variant.name);
            writeln!(
                output,
                "{}impl{} Default for {}{} {{",
                ind, generic_str, enum_name_str, generic_str
            )
            .unwrap();
            writeln!(output, "{}    fn default() -> Self {{", ind).unwrap();
            if first_variant.fields.is_empty() {
                writeln!(
                    output,
                    "{}        {}::{}",
                    ind, enum_name_str, first_variant_name
                )
                .unwrap();
            } else {
                let default_fields: Vec<(String, String)> = first_variant
                    .fields
                    .iter()
                    .map(|f| {
                        let field_name = super::escape_rust_ident(interner.resolve(f.name));
                        if field_is_boxed(&f.ty, enum_name_raw, interner) {
                            (field_name, "Box::new(Default::default())".to_string())
                        } else {
                            (field_name, "Default::default()".to_string())
                        }
                    })
                    .collect();
                let built = VariantShape::of(first_variant, enum_name_raw, interner).build(
                    &format!("{enum_name_str}::{first_variant_name}"),
                    &default_fields,
                );
                writeln!(output, "{}        {}", ind, built).unwrap();
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

    // `Show <variant>` renders the interpreter's way, and the enum rides a tuple element — for an
    // enum every variant field of which shows (see `showable_program_types`).
    if program_type_shows(name) {
        output.push_str(&codegen_enum_show_impl(
            name, variants, generics, interner, indent,
        ));
        output.push_str(&codegen_boxable_impl(name, generics, interner, indent));
    }

    output
}

/// Convert FieldType to Rust type string.
pub(super) fn codegen_field_type(ty: &FieldType, interner: &Interner) -> String {
    match ty {
        // A field names its type exactly as an annotation does: one table, `map_type_to_rust`.
        FieldType::Primitive(sym) | FieldType::Named(sym) => {
            map_type_to_rust(interner.resolve(*sym))
        }
        FieldType::Generic { base, params } => {
            let base_name = interner.resolve(*base);
            let param_strs: Vec<String> = params
                .iter()
                .map(|p| codegen_field_type(p, interner))
                .collect();

            // Phase 49c: Handle CRDT types with bias/algorithm modifiers
            match base_name {
                // SharedSet with explicit bias
                "SharedSet_RemoveWins" => {
                    return format!(
                        "logicaffeine_data::crdt::ORSet<{}, logicaffeine_data::crdt::RemoveWins>",
                        param_strs.join(", ")
                    );
                }
                "SharedSet_AddWins" => {
                    return format!(
                        "logicaffeine_data::crdt::ORSet<{}, logicaffeine_data::crdt::AddWins>",
                        param_strs.join(", ")
                    );
                }
                // SharedSequence with YATA algorithm
                "SharedSequence_YATA" | "CollaborativeSequence" => {
                    return format!("logicaffeine_data::crdt::YATA<{}>", param_strs.join(", "));
                }
                // A map to plain values: add-wins keys, each value a Lamport-stamped last-writer
                // register — a plain value cannot merge on its own.
                "SharedMap" | "ORMap" if param_strs.len() == 2 => {
                    return format!(
                        "logicaffeine_data::crdt::ORMap<{}, logicaffeine_data::crdt::LatestWrite<{}>>",
                        param_strs[0], param_strs[1]
                    );
                }
                _ => {}
            }

            // A dimensioned quantity erases its dimension at runtime — `Quantity of Length` is just
            // a `LogosQuantity` (the dimension was a compile-time-only refinement).
            if base_name == "Quantity" {
                return "LogosQuantity".to_string();
            }

            let base_str: String = match base_name {
                "List" | "Seq" => "LogosSeq".to_string(),
                "Set" => "Set".to_string(),
                "Map" => "LogosMap".to_string(),
                "Option" => "Option".to_string(),
                "Maybe" if is_builtin_maybe() => "Option".to_string(),
                "Result" => "Result".to_string(),
                // Phase 49: CRDT generic type
                "LastWriteWins" => "logicaffeine_data::crdt::LWWRegister".to_string(),
                // Phase 49b: New CRDT generic types (Wave 5) - default to AddWins for ORSet
                "SharedSet" | "ORSet" => "logicaffeine_data::crdt::ORSet".to_string(),
                "SharedSequence" | "RGA" => "logicaffeine_data::crdt::RGA".to_string(),
                "Divergent" | "MVRegister" => "logicaffeine_data::crdt::MVRegister".to_string(),
                // A user-defined generic base, possibly a cross-module reference.
                other => rust_type_ident(other),
            };
            format!("{}<{}>", base_str, param_strs.join(", "))
        }
        // Phase 34: Type parameter reference (T, U, etc.)
        FieldType::TypeParam(sym) => interner.resolve(*sym).to_string(),
        // A tuple field is the same positional `LogosTuple` a written `(A, B)` type is, carrying its
        // element (or named field) types in the marker — see `codegen_type_expr`'s tuple arms.
        FieldType::Tuple(items) => {
            let elems: Vec<String> = items
                .iter()
                .map(|t| codegen_field_type(t, interner))
                .collect();
            tuple_rust_type(&elems)
        }
        FieldType::Record(fields) => {
            let types: Vec<String> = fields
                .iter()
                .map(|(_, t)| codegen_field_type(t, interner))
                .collect();
            let body = if unboxable_marker_types(&types) {
                fields
                    .iter()
                    .zip(types.iter())
                    .map(|((n, _), t)| format!("{}={}", interner.resolve(*n), t))
                    .collect::<Vec<_>>()
                    .join(",")
            } else {
                fields
                    .iter()
                    .map(|(n, _)| interner.resolve(*n).to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            };
            format!("LogosTuple {}{} */", RECORD_FIELD_MARKER, body)
        }
    }
}

/// Phase 102: Check if a field type references the containing enum (recursive type).
/// Recursive types need to be wrapped in Box<T> for Rust to know the size.
/// Whether this compile holds a recursive enum's self-holding variants whole (`Opt::VariantBox`).
fn variant_box_on() -> bool {
    crate::optimize::active_config().is_on(crate::optimization::Opt::VariantBox)
}

/// Whether a field of the enum `enum_name` sits in a box of its own: a field that holds the enum, when its variant is
/// not boxed whole.
pub(crate) fn field_is_boxed(ty: &FieldType, enum_name: &str, interner: &Interner) -> bool {
    !variant_box_on() && is_recursive_field(ty, enum_name, interner)
}

/// How a variant of an enum holds its fields: in place, or (`Opt::VariantBox`, for a variant that holds the enum
/// itself) in one box around its payload, a struct of the fields.
pub(crate) enum VariantShape {
    InPlace,
    Boxed { payload: String },
}

impl VariantShape {
    /// The shape of `variant` of the enum interned as `enum_raw`.
    pub(crate) fn of(variant: &VariantDef, enum_raw: &str, interner: &Interner) -> VariantShape {
        if variant_box_on()
            && variant
                .fields
                .iter()
                .any(|f| is_recursive_field(&f.ty, enum_raw, interner))
        {
            crate::optimize::mark_fired(crate::optimization::Opt::VariantBox);
            VariantShape::Boxed {
                payload: variant_payload_ident(enum_raw, interner.resolve(variant.name)),
            }
        } else {
            VariantShape::InPlace
        }
    }

    /// A pattern for the variant at `path` binding each `(field, name)` to a reference, with `ref` spelled when the
    /// match is on a place; and the statements that bind them, for the boxed shape.
    pub(crate) fn pattern(
        &self,
        path: &str,
        binds: &[(String, String)],
        by_ref: bool,
    ) -> (String, String) {
        let r = if by_ref { "ref " } else { "" };
        match self {
            VariantShape::InPlace => {
                let fields: Vec<String> = binds
                    .iter()
                    .map(|(f, n)| {
                        if f == n {
                            format!("{r}{n}")
                        } else {
                            format!("{f}: {r}{n}")
                        }
                    })
                    .collect();
                (format!("{path} {{ {} }}", fields.join(", ")), String::new())
            }
            VariantShape::Boxed { .. } => {
                let prelude = binds
                    .iter()
                    .map(|(f, n)| format!("let {n} = &__fields.{f};"))
                    .collect::<Vec<_>>()
                    .join(" ");
                (format!("{path}({r}__fields)"), prelude)
            }
        }
    }

    /// A pattern for the variant at `path` binding its field `field` to a reference under the same name, and the
    /// statement that binds it, for the boxed shape.
    pub(crate) fn field_pattern(&self, path: &str, field: &str) -> (String, String) {
        match self {
            VariantShape::InPlace => (format!("{path} {{ {field}, .. }}"), String::new()),
            VariantShape::Boxed { .. } => (
                format!("{path}(__fields)"),
                format!("let {field} = &__fields.{field};"),
            ),
        }
    }

    /// The variant at `path` built from each `(field, value)`.
    pub(crate) fn build(&self, path: &str, values: &[(String, String)]) -> String {
        let fields: Vec<String> = values.iter().map(|(f, v)| format!("{f}: {v}")).collect();
        match self {
            VariantShape::InPlace => format!("{path} {{ {} }}", fields.join(", ")),
            VariantShape::Boxed { payload } => {
                format!("{path}(Box::new({payload} {{ {} }}))", fields.join(", "))
            }
        }
    }
}

/// The struct a boxed variant's fields live in: named for its enum and variant, past any name a program declares.
pub(crate) fn variant_payload_ident(enum_raw: &str, variant: &str) -> String {
    format!(
        "{}__{}__Fields",
        rust_type_ident(enum_raw),
        rust_type_ident(variant)
    )
}

/// Whether a field of type `ty` names the type parameter `param`.
fn field_names_param(ty: &FieldType, param: Symbol) -> bool {
    match ty {
        FieldType::TypeParam(p) => *p == param,
        FieldType::Primitive(s) | FieldType::Named(s) => *s == param,
        FieldType::Generic { base, params } => {
            *base == param || params.iter().any(|p| field_names_param(p, param))
        }
        FieldType::Tuple(items) => items.iter().any(|t| field_names_param(t, param)),
        FieldType::Record(fields) => fields.iter().any(|(_, t)| field_names_param(t, param)),
    }
}

pub(crate) fn is_recursive_field(ty: &FieldType, enum_name: &str, interner: &Interner) -> bool {
    match ty {
        FieldType::Primitive(sym) => interner.resolve(*sym) == enum_name,
        FieldType::Named(sym) => interner.resolve(*sym) == enum_name,
        FieldType::TypeParam(_) => false,
        FieldType::Tuple(items) => items
            .iter()
            .any(|t| is_recursive_field(t, enum_name, interner)),
        FieldType::Record(fields) => fields
            .iter()
            .any(|(_, t)| is_recursive_field(t, enum_name, interner)),
        FieldType::Generic { base, params } => {
            // Check if base matches or any type parameter contains the enum
            interner.resolve(*base) == enum_name
                || params
                    .iter()
                    .any(|p| is_recursive_field(p, enum_name, interner))
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
        Expr::NewVariant {
            enum_name,
            variant,
            fields,
        } => (*enum_name, *variant, fields),
        _ => return None,
    };

    // Look up the enum in the registry
    let enum_def = registry.get(enum_name)?;
    let (generics, variants) = match enum_def {
        TypeDef::Enum {
            generics, variants, ..
        } => (generics, variants),
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
    // For bound params, use the inferred type; an unbound (phantom) one carries no value, so any
    // type represents it — `i64`, the same one `codegen::instantiation` spells it as, and a type
    // with every bound a type parameter carries.
    let enum_str = interner.resolve(enum_name);
    let param_strs: Vec<String> = generics
        .iter()
        .map(|g| {
            type_param_types
                .get(g)
                .cloned()
                .unwrap_or_else(|| "i64".to_string())
        })
        .collect();

    Some(format!("{}<{}>", enum_str, param_strs.join(", ")))
}

/// The value of an integer expression whose machine type nothing decides but rustc's literal defaulting:
/// integer literals joined by `+`, `-` and `*`, every intermediate inside `i64` (the `BinaryOp` lowering emits
/// such a node raw, and anything that leaves `i64` is promoted instead).
///
/// A binder of one has no use that must fix its width: a `Let` read only through a widening to `Real`, a range
/// counter fed only to a multiply the range oracle proved in range. rustc defaults such a binder to `i32`, where
/// the multiply overflows, so the binder states the `i64` every other tier computes a LOGOS `Int` in.
pub(crate) fn type_silent_int(expr: &Expr) -> Option<i64> {
    use crate::ast::stmt::{BinaryOpKind, Literal};
    match expr {
        Expr::Literal(Literal::Number(n)) => Some(*n),
        Expr::BinaryOp { op, left, right } => {
            let (a, b) = (type_silent_int(left)?, type_silent_int(right)?);
            match op {
                BinaryOpKind::Add => a.checked_add(b),
                BinaryOpKind::Subtract => a.checked_sub(b),
                BinaryOpKind::Multiply => a.checked_mul(b),
                _ => None,
            }
        }
        _ => None,
    }
}

/// A [`type_silent_int`] expression spelled with its literals typed `i64`, in the `BinaryOp` lowering's shape.
pub(crate) fn type_silent_int_i64(expr: &Expr) -> Option<String> {
    use crate::ast::stmt::{BinaryOpKind, Literal};
    type_silent_int(expr)?;
    match expr {
        Expr::Literal(Literal::Number(n)) => Some(format!("{n}_i64")),
        Expr::BinaryOp { op, left, right } => {
            let op = match op {
                BinaryOpKind::Add => "+",
                BinaryOpKind::Subtract => "-",
                _ => "*",
            };
            Some(format!(
                "({} {} {})",
                type_silent_int_i64(left)?,
                op,
                type_silent_int_i64(right)?
            ))
        }
        _ => None,
    }
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
    // `item i of xs` takes its element's scalar type directly. The TypeEnv below is built from
    // Rust type strings and does NOT model borrow-slice params (`&[f64]` / `&mut [f64]`), so without
    // this a `Seq of Float` PARAMETER's element would fall back to i64 — breaking `abs(item i of a)`
    // (`(x as i64).abs()`) and float arithmetic over borrowed matrices.
    use crate::ast::stmt::BinaryOpKind;
    // Int arithmetic on two Int words is an Int word, whatever representation each operand's own read takes.
    if let Expr::BinaryOp {
        op:
            BinaryOpKind::Add
            | BinaryOpKind::Subtract
            | BinaryOpKind::Multiply
            | BinaryOpKind::Divide
            | BinaryOpKind::Modulo,
        left,
        right,
    } = expr
    {
        if infer_numeric_type(left, interner, variable_types) == "i64"
            && infer_numeric_type(right, interner, variable_types) == "i64"
        {
            return "i64";
        }
    }
    if let Expr::Index { collection, .. } = expr {
        match crate::codegen::expr::seq_element_rust_type(collection, variable_types) {
            Some("f64") => return "f64",
            Some("i64") => return "i64",
            _ => {}
        }
        // A narrowed item is read widened to the i64 every Int is.
        if let Expr::Identifier(sym) = collection {
            if variable_types
                .get(sym)
                .is_some_and(|t| crate::codegen::expr::item_read_widens(t))
            {
                return "i64";
            }
        }
    }
    // A call to a `-> Real` math.lg function yields an f64, but the TypeEnv below carries no
    // function signatures, so without this it falls back to "unknown" and a mixed `sin(x) * 100`
    // (Real × Int) would emit `f64 * {integer}` — a hard rustc error. Recognising the float return
    // here lets the binary-op emitter coerce the Int operand with `as f64`, so a `Real` can be
    // multiplied/added/… by an `Int` (literal or variable) directly.
    if let Expr::Call { function, .. } = expr {
        if let Some(ty) = crate::codegen::expr::declared_numeric_return(*function) {
            return ty;
        }
        if crate::codegen::expr::is_float_returning_math_call(*function, interner) {
            return "f64";
        }
    }
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
