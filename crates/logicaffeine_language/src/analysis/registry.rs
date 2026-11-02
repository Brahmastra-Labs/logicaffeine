//! Type registry for tracking user-defined types during parsing.
//!
//! The registry stores type definitions discovered during the discovery pass
//! and provides lookup during parsing and code generation. It supports:
//!
//! - **Primitives**: Nat, Int, Text, Bool (built-in)
//! - **Structs**: Named records with typed fields
//! - **Enums**: Sum types with variants (unit or tuple)
//! - **Generics**: Parameterized types (List of T, Pair of T and U)
//!
//! The registry is populated by [`super::DiscoveryPass`] before main parsing begins.

use crate::token::Span;
use indexmap::IndexMap;
use logicaffeine_base::{Interner, Symbol};
use std::collections::{HashMap, HashSet};

/// The structs the compiler hands to author code fully formed, rather than one an `## An X has:`
/// declaration writes — the one list [`TypeRegistry::with_primitives`] registers them from and
/// [`TypeRegistry::is_builtin_struct`] answers against, so a consumer never has to know their
/// names itself. `crates/logicaffeine_compile/src/vm/wasm/kind.rs`'s `builtin_struct_types()`
/// carries the same two shapes for the wasm backend's own boundary-type resolution.
pub const BUILTIN_STRUCT_NAMES: &[&str] = &["HttpError", "File"];

/// Type reference for struct fields (avoids circular deps with ast::TypeExpr)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldType {
    /// Primitive type name (Int, Nat, Text, Bool, etc.)
    Primitive(Symbol),
    /// User-defined type name
    Named(Symbol),
    /// Generic type with parameters (List of Int, Seq of Text)
    Generic {
        base: Symbol,
        params: Vec<FieldType>,
    },
    /// Phase 34: Type parameter reference (T, U, etc.)
    TypeParam(Symbol),
    /// An anonymous tuple: `(Int, Text)`.
    Tuple(Vec<FieldType>),
    /// A named tuple: `(quotient: Int, remainder: Int)`.
    Record(Vec<(Symbol, FieldType)>),
}

/// How many type parameters a BUILT-IN generic takes — the one table the whole compiler asks.
///
/// ★ IT LIVES HERE BECAUSE IT WAS ANSWERED IN THREE PLACES AND THEY DISAGREED. The type-expression
/// parser knew `Result` takes two; the two record-FIELD parsers knew only that `Map` does, so a
/// field typed `Result of Int and E` read as a ONE-parameter Result and silently dropped its error
/// type — and with it the whole record, since a field whose type will not resolve takes its struct
/// out of the layout tables. `Pair`/`Triple` fields went the same way. A field declaring a type the
/// language accepts everywhere else must mean the same thing here.
///
/// A registry lookup takes precedence for a USER generic; this answers for the built-ins only.
pub fn builtin_generic_param_count(name: &str) -> Option<usize> {
    Some(match name {
        "Result" => 2,
        "Option" | "Maybe" => 1,
        "Seq" | "List" | "Vec" => 1,
        "Set" | "HashSet" => 1,
        "Map" | "HashMap" => 2,
        "Pair" => 2,
        "Triple" => 3,
        "SharedSet" | "ORSet" | "SharedSet_AddWins" | "SharedSet_RemoveWins" => 1,
        "SharedSequence" | "RGA" | "SharedSequence_YATA" | "CollaborativeSequence" => 1,
        "SharedMap" | "ORMap" => 2,
        "Divergent" | "MVRegister" => 1,
        _ => return None,
    })
}

/// Field definition within a struct
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldDef {
    pub name: Symbol,
    pub ty: FieldType,
    pub is_public: bool,
}

/// Phase 33: Variant definition for sum types
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariantDef {
    pub name: Symbol,
    pub fields: Vec<FieldDef>, // Empty for unit variants
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeDef {
    /// Primitive type (Nat, Int, Text, Bool)
    Primitive,
    /// Struct with named fields and visibility
    /// Phase 34: Now includes optional type parameters
    /// Phase 47: Added is_portable for serde derives
    /// Phase 49: Added is_shared for CRDT Merge impl
    Struct {
        fields: Vec<FieldDef>,
        generics: Vec<Symbol>, // [T, U] for "A Pair of [T] and [U] has:"
        is_portable: bool,     // Phase 47: Derives Serialize/Deserialize
        is_shared: bool,       // Phase 49: Generates impl Merge
    },
    /// Phase 33: Enum with variants (unit or with payload)
    /// Phase 34: Now includes optional type parameters
    /// Phase 47: Added is_portable for serde derives
    /// Phase 49: Added is_shared for CRDT Merge impl
    Enum {
        variants: Vec<VariantDef>,
        generics: Vec<Symbol>, // [T] for "A Maybe of [T] is either:"
        is_portable: bool,     // Phase 47: Derives Serialize/Deserialize
        is_shared: bool,       // Phase 49: Generates impl Merge
    },
    /// Built-in generic type (List, Option, Result)
    Generic { param_count: usize },
    /// Type alias
    Alias { target: Symbol },
}

#[derive(Debug, Default, Clone)]
pub struct TypeRegistry {
    /// Every type, in the order it was registered: the builtins first, then each declaration as
    /// discovery reaches it. Iteration follows that order, so everything built by walking the
    /// registry (the bytecode's type tables, generated code, diagnostics) is the same on every run.
    types: IndexMap<Symbol, TypeDef>,
    /// Where a struct's own field was named in the source, keyed by `(type, field)`. Populated
    /// only where a check needs to report AT a field rather than at the record as a whole
    /// (`## Session`'s field-kind refusal); a field with no entry here has none to report.
    field_spans: HashMap<(Symbol, Symbol), Span>,
    /// [`BUILTIN_STRUCT_NAMES`], interned — a type registered here because the compiler always
    /// provides it, not because this program declared or imported it. A consumer that lists or
    /// emits "the types this program uses" (LSP completion, AOT/C codegen) must consult this
    /// before including one, or every program pays for `HttpError`/`File` whether it uses them
    /// or not.
    builtin_structs: HashSet<Symbol>,
}

impl TypeRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a type definition
    pub fn register(&mut self, name: Symbol, def: TypeDef) {
        self.types.insert(name, def);
    }

    /// Record where `field` of `ty` was named in the source.
    pub fn set_field_span(&mut self, ty: Symbol, field: Symbol, span: Span) {
        self.field_spans.insert((ty, field), span);
    }

    /// Where `field` of `ty` was named in the source, if recorded.
    pub fn field_span(&self, ty: Symbol, field: Symbol) -> Option<Span> {
        self.field_spans.get(&(ty, field)).copied()
    }

    /// Check if a symbol is a known type
    pub fn is_type(&self, name: Symbol) -> bool {
        self.types.contains_key(&name)
    }

    /// Whether `name` is one of [`BUILTIN_STRUCT_NAMES`] — a struct the compiler always
    /// registers, never one this program declared or imported.
    pub fn is_builtin_struct(&self, name: Symbol) -> bool {
        self.builtin_structs.contains(&name)
    }

    /// Check if a symbol is a generic type (takes parameters)
    pub fn is_generic(&self, name: Symbol) -> bool {
        match self.types.get(&name) {
            Some(TypeDef::Generic { .. }) => true,
            Some(TypeDef::Struct { generics, .. }) => !generics.is_empty(),
            Some(TypeDef::Enum { generics, .. }) => !generics.is_empty(),
            _ => false,
        }
    }

    /// Phase 34: Get type parameters for a user-defined generic type
    pub fn get_generics(&self, name: Symbol) -> Option<&[Symbol]> {
        match self.types.get(&name)? {
            TypeDef::Struct { generics, .. } => Some(generics),
            TypeDef::Enum { generics, .. } => Some(generics),
            _ => None,
        }
    }

    /// Get type definition
    pub fn get(&self, name: Symbol) -> Option<&TypeDef> {
        self.types.get(&name)
    }

    /// Iterate over all registered types (for codegen)
    pub fn iter_types(&self) -> impl Iterator<Item = (&Symbol, &TypeDef)> {
        self.types.iter()
    }

    /// Every enum declaring a variant named `variant_name`, in a STABLE, deterministic order
    /// — sorted by the enum's own `Symbol` (its intern index, fixed by the deterministic
    /// sequence discovery interns names in, never by `self.types`' `HashMap` iteration order,
    /// which is reseeded, and so reordered, every process run). Two (or more) enums sharing a
    /// variant name is a real, checkable condition: empty means no such variant anywhere, one
    /// entry means an ordinary unambiguous lookup, and more than one is an ambiguity a caller
    /// can act on (an `Inspect` prefers whichever enum it has already established; a bare
    /// construction with no such context has nothing to prefer and must refuse) — instead of a
    /// coin flip that used to silently compile the same source differently from run to run.
    pub fn find_variant_candidates(&self, variant_name: Symbol) -> Vec<(Symbol, &VariantDef)> {
        let mut found: Vec<(Symbol, &VariantDef)> = self
            .types
            .iter()
            .filter_map(|(enum_name, type_def)| match type_def {
                TypeDef::Enum { variants, .. } => variants
                    .iter()
                    .find(|v| v.name == variant_name)
                    .map(|v| (*enum_name, v)),
                _ => None,
            })
            .collect();
        found.sort_by_key(|(enum_name, _)| *enum_name);
        found
    }

    /// [`Self::find_variant_candidates`] with each twin pair (see [`Self::namespaced_twins`]) counted
    /// once, as its namespaced member: `Colors::Filter` and the bare `Filter` the assembler spliced
    /// beside it are one enum, so a variant both declare is not ambiguous.
    pub fn find_distinct_variant_candidates(
        &self,
        variant_name: Symbol,
        interner: &Interner,
    ) -> Vec<(Symbol, &VariantDef)> {
        let twins = self.namespaced_twins(interner);
        let candidates = self.find_variant_candidates(variant_name);
        let named: HashSet<Symbol> = candidates.iter().map(|(enum_name, _)| *enum_name).collect();
        candidates
            .into_iter()
            .filter(|(enum_name, _)| {
                !twins
                    .get(enum_name)
                    .is_some_and(|twin| named.contains(twin))
            })
            .collect()
    }

    /// Each bare type name that names a namespaced type, keyed by the bare name. The assembler
    /// registers an imported module's type under both its simple name (`Point`, from the inline
    /// splice) and its canonical `Geometry::Point`; when exactly one namespaced type is called
    /// `Point`, the bare spelling IS that type, and every consumer — the parser's variant
    /// resolution, the checker, the monomorphizer, the AOT emitter's alias — gives both spellings
    /// one identity. A simple name two namespaced types share stays ambiguous and maps to nothing,
    /// and a bare type is only ever the twin of a namespaced type of its own kind.
    pub fn namespaced_twins(&self, interner: &Interner) -> HashMap<Symbol, Symbol> {
        let mut by_simple: HashMap<&str, Vec<Symbol>> = HashMap::new();
        for (sym, def) in &self.types {
            if matches!(def, TypeDef::Struct { .. } | TypeDef::Enum { .. }) {
                if let Some((_, simple)) = interner.resolve(*sym).rsplit_once("::") {
                    by_simple.entry(simple).or_default().push(*sym);
                }
            }
        }
        let same_kind = |a: Option<&TypeDef>, b: Option<&TypeDef>| {
            matches!(
                (a, b),
                (Some(TypeDef::Struct { .. }), Some(TypeDef::Struct { .. }))
                    | (Some(TypeDef::Enum { .. }), Some(TypeDef::Enum { .. }))
            )
        };
        by_simple
            .into_iter()
            .filter_map(|(simple, full)| match full.as_slice() {
                [only] => interner
                    .lookup(simple)
                    .filter(|bare| same_kind(self.types.get(bare), self.types.get(only)))
                    .map(|bare| (bare, *only)),
                _ => None,
            })
            .collect()
    }

    /// The one identity a type named `name` has: its namespaced twin when the bare spelling names
    /// one (see [`Self::namespaced_twins`]), else itself.
    pub fn canonical_type(&self, name: Symbol, interner: &Interner) -> Symbol {
        self.namespaced_twins(interner)
            .get(&name)
            .copied()
            .unwrap_or(name)
    }

    /// Phase 33: Check if a symbol is a known enum variant.
    /// Returns `Some((enum_name, variant_def))` for the first candidate in
    /// [`Self::find_variant_candidates`]'s own deterministic order when the variant name is
    /// declared by more than one enum — callers that must tell the difference (an ambiguous
    /// bare construction) use `find_variant_candidates` directly instead.
    pub fn find_variant(&self, variant_name: Symbol) -> Option<(Symbol, &VariantDef)> {
        self.find_variant_candidates(variant_name)
            .into_iter()
            .next()
    }

    /// Phase 33: Check if a symbol is an enum variant
    pub fn is_variant(&self, name: Symbol) -> bool {
        self.find_variant(name).is_some()
    }

    /// The variant named `variant_name` declared by `enum_name` SPECIFICALLY — for a
    /// construction that already names its enum (`a new Failure from Outcome`), so the field
    /// list checked against is the ONE the author pointed at, never
    /// [`Self::find_variant`]'s own deterministic-first-candidate pick, which could be a
    /// same-named variant of a DIFFERENT enum with a different field shape entirely.
    pub fn variant_in(&self, enum_name: Symbol, variant_name: Symbol) -> Option<&VariantDef> {
        match self.types.get(&enum_name) {
            Some(TypeDef::Enum { variants, .. }) => {
                variants.iter().find(|v| v.name == variant_name)
            }
            _ => None,
        }
    }

    /// Every variant name `enum_name` declares, for a "did you mean" suggestion when `from
    /// <enum_name>` names a real enum that does not declare the variant asked for. `None` when
    /// `enum_name` is not an enum at all.
    pub fn variant_names_of(&self, enum_name: Symbol) -> Option<Vec<Symbol>> {
        match self.types.get(&enum_name) {
            Some(TypeDef::Enum { variants, .. }) => Some(variants.iter().map(|v| v.name).collect()),
            _ => None,
        }
    }

    /// Pre-register primitives and intrinsic generics
    pub fn with_primitives(interner: &mut Interner) -> Self {
        let mut reg = Self::new();

        // LOGOS Core Primitives
        reg.register(interner.intern("Nat"), TypeDef::Primitive);
        reg.register(interner.intern("Int"), TypeDef::Primitive);
        reg.register(interner.intern("Text"), TypeDef::Primitive);
        reg.register(interner.intern("Bool"), TypeDef::Primitive);
        reg.register(interner.intern("Boolean"), TypeDef::Primitive);
        reg.register(interner.intern("Unit"), TypeDef::Primitive);
        reg.register(interner.intern("Real"), TypeDef::Primitive); // Floating point
        reg.register(interner.intern("Char"), TypeDef::Primitive); // Character
        reg.register(interner.intern("Byte"), TypeDef::Primitive); // 8-bit unsigned (0-255)
                                                                   // Fixed-width machine words — the ℤ/2ⁿ wrapping ring (crypto substrate).
        reg.register(interner.intern("Word8"), TypeDef::Primitive);
        reg.register(interner.intern("Word16"), TypeDef::Primitive);
        reg.register(interner.intern("Word32"), TypeDef::Primitive);
        reg.register(interner.intern("Word64"), TypeDef::Primitive);
        // SIMD lane vectors — one AVX2 register each.
        reg.register(interner.intern("Lanes8Word32"), TypeDef::Primitive);
        reg.register(interner.intern("Lanes4Word32"), TypeDef::Primitive);
        reg.register(interner.intern("Lanes16Word8"), TypeDef::Primitive);
        reg.register(interner.intern("Lanes4Word64"), TypeDef::Primitive);
        reg.register(interner.intern("Lanes16Word16"), TypeDef::Primitive);

        // Intrinsic Generics
        reg.register(interner.intern("List"), TypeDef::Generic { param_count: 1 });
        reg.register(interner.intern("Seq"), TypeDef::Generic { param_count: 1 }); // Phase 30: Sequences
        reg.register(interner.intern("Map"), TypeDef::Generic { param_count: 2 }); // Phase 43D: Key-value maps
        reg.register(interner.intern("Set"), TypeDef::Generic { param_count: 1 }); // Set collection (HashSet)
        reg.register(
            interner.intern("Option"),
            TypeDef::Generic { param_count: 1 },
        );
        reg.register(
            interner.intern("Maybe"),
            TypeDef::Generic { param_count: 1 },
        );
        reg.register(
            interner.intern("Result"),
            TypeDef::Generic { param_count: 2 },
        );
        // A dimensioned quantity carries its dimension as a type parameter: `Quantity of Length`.
        // Bare `Quantity` (no `of`) still parses as the dimension-polymorphic form.
        reg.register(
            interner.intern("Quantity"),
            TypeDef::Generic { param_count: 1 },
        );

        // Every struct the compiler hands to author code fully formed, rather than one an
        // `## An X has:` declaration (the page's own source, or an imported stdlib module)
        // actually writes: `HttpError` (a typed fetch's `Err`, built by `http_error_expr`) and
        // `File` (a `When a file is dropped …: (file: File)` binding). Neither one's fields were
        // ever registered here, so `e's message`/`file's size` resolved to a struct NAME with no
        // field LAYOUT — "field access with no static slot" on the wasm AOT backend
        // (`HttpError`, `File`). `crates/logicaffeine_compile/src/vm/wasm/kind.rs`
        // `builtin_struct_types()` carries the SAME two shapes, in the SAME field order, for the
        // wasm backend's own boundary-type resolution; `assets/std/http.lg`'s own `An HttpError
        // has:` declaration is the field order's source of truth. Marked in `builtin_structs` so a
        // consumer can tell "the compiler always provides this" from "this program declared it".
        for name in BUILTIN_STRUCT_NAMES {
            reg.builtin_structs.insert(interner.intern(name));
        }
        reg.register(
            interner.intern("HttpError"),
            TypeDef::Struct {
                fields: vec![
                    FieldDef {
                        name: interner.intern("status"),
                        ty: FieldType::Primitive(interner.intern("Int")),
                        is_public: true,
                    },
                    FieldDef {
                        name: interner.intern("message"),
                        ty: FieldType::Primitive(interner.intern("Text")),
                        is_public: true,
                    },
                ],
                generics: Vec::new(),
                is_portable: false,
                is_shared: false,
            },
        );
        reg.register(
            interner.intern("File"),
            TypeDef::Struct {
                fields: vec![
                    FieldDef {
                        name: interner.intern("name"),
                        ty: FieldType::Primitive(interner.intern("Text")),
                        is_public: true,
                    },
                    FieldDef {
                        name: interner.intern("size"),
                        ty: FieldType::Primitive(interner.intern("Int")),
                        is_public: true,
                    },
                    FieldDef {
                        name: interner.intern("type"),
                        ty: FieldType::Primitive(interner.intern("Text")),
                        is_public: true,
                    },
                    FieldDef {
                        name: interner.intern("bytes"),
                        ty: FieldType::Generic {
                            base: interner.intern("Seq"),
                            params: vec![FieldType::Primitive(interner.intern("Int"))],
                        },
                        is_public: true,
                    },
                ],
                generics: Vec::new(),
                is_portable: false,
                is_shared: false,
            },
        );

        reg
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_language/analysis/registry/tests.rs"]
mod tests;
