use std::collections::HashMap;
use crate::intern::{Interner, Symbol};

/// Type reference for struct fields (avoids circular deps with ast::TypeExpr)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldType {
    /// Primitive type name (Int, Nat, Text, Bool, etc.)
    Primitive(Symbol),
    /// User-defined type name
    Named(Symbol),
    /// Generic type with parameters (List of Int, Seq of Text)
    Generic { base: Symbol, params: Vec<FieldType> },
    /// Phase 34: Type parameter reference (T, U, etc.)
    TypeParam(Symbol),
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
    pub fields: Vec<FieldDef>,  // Empty for unit variants
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeDef {
    /// Primitive type (Nat, Int, Text, Bool)
    Primitive,
    /// Struct with named fields and visibility
    /// Phase 34: Now includes optional type parameters
    Struct {
        fields: Vec<FieldDef>,
        generics: Vec<Symbol>,  // [T, U] for "A Pair of [T] and [U] has:"
    },
    /// Phase 33: Enum with variants (unit or with payload)
    /// Phase 34: Now includes optional type parameters
    Enum {
        variants: Vec<VariantDef>,
        generics: Vec<Symbol>,  // [T] for "A Maybe of [T] is either:"
    },
    /// Built-in generic type (List, Option, Result)
    Generic { param_count: usize },
    /// Type alias
    Alias { target: Symbol },
}

#[derive(Debug, Default, Clone)]
pub struct TypeRegistry {
    types: HashMap<Symbol, TypeDef>,
}

impl TypeRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a type definition
    pub fn register(&mut self, name: Symbol, def: TypeDef) {
        self.types.insert(name, def);
    }

    /// Check if a symbol is a known type
    pub fn is_type(&self, name: Symbol) -> bool {
        self.types.contains_key(&name)
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

    /// Phase 33: Check if a symbol is a known enum variant
    /// Returns Some((enum_name, variant_def)) if found
    pub fn find_variant(&self, variant_name: Symbol) -> Option<(Symbol, &VariantDef)> {
        for (enum_name, type_def) in &self.types {
            if let TypeDef::Enum { variants, .. } = type_def {
                for variant in variants {
                    if variant.name == variant_name {
                        return Some((*enum_name, variant));
                    }
                }
            }
        }
        None
    }

    /// Phase 33: Check if a symbol is an enum variant
    pub fn is_variant(&self, name: Symbol) -> bool {
        self.find_variant(name).is_some()
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

        // Intrinsic Generics
        reg.register(interner.intern("List"), TypeDef::Generic { param_count: 1 });
        reg.register(interner.intern("Seq"), TypeDef::Generic { param_count: 1 });  // Phase 30: Sequences
        reg.register(interner.intern("Map"), TypeDef::Generic { param_count: 2 });  // Phase 43D: Key-value maps
        reg.register(interner.intern("Option"), TypeDef::Generic { param_count: 1 });
        reg.register(interner.intern("Result"), TypeDef::Generic { param_count: 2 });

        reg
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_stores_and_retrieves() {
        let mut interner = Interner::new();
        let mut registry = TypeRegistry::new();
        let foo = interner.intern("Foo");
        registry.register(foo, TypeDef::Primitive);
        assert!(registry.is_type(foo));
        assert!(!registry.is_generic(foo));
    }
}
