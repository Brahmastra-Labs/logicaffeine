//! The types a program's values have, as functions of the program: what the offline specializer (`mix.logos`) asks
//! when it lifts one of the program's static values.
//!
//! A static value that reaches the residual is lifted to the expression that builds it. An empty collection is built
//! only with its type named (`a new Seq of CVal`), and a value carries no type: the type comes from where the value
//! sits. A constructor's field has the type its definition gives it, a function's answer its return type, and a
//! collection's element the type its own type names. Those types are the program's, so the program carries them:
//! [`lift_types_source`] writes three functions from the program's own type definitions and signatures, appended to
//! it before its analysis, and `mix` asks them through the self-interpreter, as it evaluates every static part:
//! - `liftedFieldType (fieldOwner, fieldName)`: the type of a constructor's collection field, `""` for any other;
//! - `liftedCollectionKind (collectionType)`: `"Seq"`, `"Set"` or `"Map"` for a collection type, `""` for any other;
//! - `liftedElementType (collectionType)`: the element type of a sequence or set, the value type of a map, `""` for any
//!   other type.
//!
//! The collection types are every one the program names in its constructors' fields and its functions' signatures,
//! with the element types of each. A program `mix` specializes defines no structs: its values are constructors of
//! enums, each field typed by its definition. A specialization to one program holds that program's types alone — the
//! compiler the second projection makes asks the self-interpreter's, which name none of the specializer's forms.
//!
//! Names are told apart by their length first and then by equality, as `optimize::core_builtins` tells builtins apart:
//! the language orders no `Text`.

use std::collections::{BTreeMap, BTreeSet};

use crate::optimize::mix_bta::catalog_shapes;

/// The function that answers a constructor's collection field's type.
pub const FIELD_TYPE: &str = "liftedFieldType";

/// The function that answers a collection type's kind.
pub const COLLECTION_KIND: &str = "liftedCollectionKind";

/// The function that answers a collection type's element type.
pub const ELEMENT_TYPE: &str = "liftedElementType";

/// A collection type's kind and its element type (the value type of a map).
enum Collection<'a> {
    Seq(&'a str),
    Set(&'a str),
    Map(&'a str),
}

/// `ty` as a collection, or `None` when it is no collection type. A map's key is a simple type: the first ` to `
/// after `Map of ` ends it.
fn collection(ty: &str) -> Option<Collection<'_>> {
    if let Some(element) = ty.strip_prefix("Seq of ") {
        return Some(Collection::Seq(element));
    }
    if let Some(element) = ty.strip_prefix("Set of ") {
        return Some(Collection::Set(element));
    }
    let entries = ty.strip_prefix("Map of ")?;
    let (key, value) = entries
        .split_once(" to ")
        .unwrap_or_else(|| panic!("`{ty}` names a map without its value type"));
    assert!(!key.contains(" of "), "`{ty}` keys a map by a compound type, which `mix` lifts no entries of");
    Some(Collection::Map(value))
}

/// `ty` as the Core IR spells a type: without `mutable`, parentheses or surrounding space.
fn normalized(ty: &str) -> String {
    let ty = ty.trim();
    let ty = ty.strip_prefix("mutable ").unwrap_or(ty);
    ty.chars().filter(|c| *c != '(' && *c != ')').collect::<String>().trim().to_string()
}

/// The types a `## To` header names: each parameter's — `(name: Type)`, or several in one group, `(a: T, b: U)` — and
/// its return type.
fn signature_types(header: &str) -> Vec<String> {
    let mut types = Vec::new();
    let body = header.trim_end().trim_end_matches(':');
    let (params, answer) = match body.split_once(" -> ") {
        Some((p, a)) => (p, Some(a)),
        None => (body, None),
    };
    let mut depth = 0usize;
    let mut start = None;
    for (i, c) in params.char_indices() {
        match c {
            '(' => {
                if depth == 0 {
                    start = Some(i + 1);
                }
                depth += 1;
            }
            ')' => {
                depth -= 1;
                if depth == 0 {
                    let group = start.map_or("", |s| &params[s..i]);
                    types.extend(group.split(',').filter_map(|p| p.split_once(':')).map(|(_, ty)| normalized(ty)));
                }
            }
            _ => {}
        }
    }
    if let Some(answer) = answer {
        let answer = answer.split(" is exported").next().unwrap_or(answer);
        types.push(normalized(answer));
    }
    types
}

/// Every constructor of `program`'s collection fields, by constructor then field, each with its type.
fn collection_fields(program: &str) -> BTreeMap<String, BTreeMap<String, String>> {
    for line in program.lines() {
        if let Some(definition) = line.strip_prefix("## A ").or_else(|| line.strip_prefix("## An ")) {
            assert!(
                definition.trim_end().ends_with("is one of:"),
                "`{line}`: a program `mix` specializes defines only enums, whose constructors' fields its lift types read"
            );
        }
    }
    let mut fields: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for (ctor, shape) in catalog_shapes(program) {
        for (field, ty) in shape {
            let ty = normalized(&ty);
            if collection(&ty).is_some() {
                fields.entry(ctor.clone()).or_default().insert(field, ty);
            }
        }
    }
    fields
}

/// Every collection type `program` names in `fields` and its functions' signatures, with the element types of each.
fn collection_types(program: &str, fields: &BTreeMap<String, BTreeMap<String, String>>) -> BTreeSet<String> {
    let mut pending: Vec<String> = fields.values().flat_map(|f| f.values().cloned()).collect();
    for line in program.lines().filter(|l| l.starts_with("## To ")) {
        pending.extend(signature_types(line));
    }
    let mut types = BTreeSet::new();
    while let Some(ty) = pending.pop() {
        let Some(c) = collection(&ty) else { continue };
        if types.insert(ty.clone()) {
            let (Collection::Seq(element) | Collection::Set(element) | Collection::Map(element)) = c;
            pending.push(element.to_string());
        }
    }
    types
}

/// `names` by their length, each length's names in order.
fn by_length<'a, T>(names: impl IntoIterator<Item = (&'a str, T)>) -> BTreeMap<usize, Vec<(&'a str, T)>> {
    let mut buckets: BTreeMap<usize, Vec<(&'a str, T)>> = BTreeMap::new();
    for (name, value) in names {
        assert!(
            name.is_ascii() && !name.contains(['"', '{', '}', ',', ':']),
            "`{name}` is no name the generated text can hold as a literal and measure as its reader does"
        );
        buckets.entry(name.len()).or_default().push((name, value));
    }
    buckets
}

/// A collection's kind, as [`COLLECTION_KIND`] answers it.
fn kind_of(c: Collection<'_>) -> &str {
    match c {
        Collection::Seq(_) => "Seq",
        Collection::Set(_) => "Set",
        Collection::Map(_) => "Map",
    }
}

/// A collection's element type, as [`ELEMENT_TYPE`] answers it.
fn element_of(c: Collection<'_>) -> &str {
    match c {
        Collection::Seq(element) | Collection::Set(element) | Collection::Map(element) => element,
    }
}

/// `function (collectionType) -> Text`: `answer` of each type of `types`, `""` for any other type.
fn collection_function(function: &str, types: &BTreeSet<String>, answer: for<'a> fn(Collection<'a>) -> &'a str) -> String {
    let mut out = format!("## To {function} (collectionType: Text) -> Text:\n");
    out.push_str("    Let collectionLength be length of collectionType.\n");
    for (len, group) in by_length(types.iter().map(|t| (t.as_str(), ()))) {
        out.push_str(&format!("    If collectionLength equals {len}:\n"));
        for (ty, ()) in group {
            let said = answer(collection(ty).expect("a collection type"));
            out.push_str(&format!("        If collectionType equals \"{ty}\":\n"));
            out.push_str(&format!("            Return \"{said}\".\n"));
        }
    }
    out.push_str("    Return \"\".\n");
    out
}

/// `program`'s lift types: [`FIELD_TYPE`], [`COLLECTION_KIND`] and [`ELEMENT_TYPE`], from its type definitions and
/// signatures, as LOGOS functions to append to it.
pub fn lift_types_source(program: &str) -> String {
    let fields = collection_fields(program);
    let types = collection_types(program, &fields);
    let mut out = String::new();

    out.push_str(&format!("## To {FIELD_TYPE} (fieldOwner: Text) and (fieldName: Text) -> Text:\n"));
    out.push_str("    Let ownerLength be length of fieldOwner.\n");
    for (len, owners) in by_length(fields.iter().map(|(c, f)| (c.as_str(), f))) {
        out.push_str(&format!("    If ownerLength equals {len}:\n"));
        for (owner, owned) in owners {
            out.push_str(&format!("        If fieldOwner equals \"{owner}\":\n"));
            for (field, ty) in owned {
                out.push_str(&format!("            If fieldName equals \"{field}\":\n"));
                out.push_str(&format!("                Return \"{ty}\".\n"));
            }
            out.push_str("            Return \"\".\n");
        }
    }
    out.push_str("    Return \"\".\n\n");

    out.push_str(&collection_function(COLLECTION_KIND, &types, kind_of));
    out.push('\n');
    out.push_str(&collection_function(ELEMENT_TYPE, &types, element_of));
    out.push('\n');
    out
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/optimize/mix_lift_types/tests.rs"]
mod tests;
