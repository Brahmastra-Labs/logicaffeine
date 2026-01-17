//! Discovery extensions with module imports.
//!
//! Provides [`discover_with_imports`] which uses the [`Loader`](crate::loader::Loader)
//! for multi-file projects with namespace support.
//!
//! # Multi-File Type Discovery
//!
//! ```text
//! main.md                           geometry.md
//! ┌────────────────────────┐        ┌────────────────────────┐
//! │ ## Abstract            │        │ ## Definition          │
//! │ [Geometry](geometry.md)│───────▶│ A Point has x: Int,    │
//! └────────────────────────┘        │   y: Int.              │
//!                                   └────────────────────────┘
//!         ↓
//! TypeRegistry contains:
//!   - Geometry::Point { x: Int, y: Int }
//! ```

use std::path::Path;
use crate::loader::Loader;
use crate::{Interner, Lexer, mwe};
use super::{TypeRegistry, TypeDef, DiscoveryPass, scan_dependencies};

/// Recursive discovery with module imports.
///
/// Scans a LOGOS source file for:
/// 1. Dependencies declared in the Abstract (Markdown links)
/// 2. Type definitions in `## Definition` blocks
///
/// # Namespace Prefixing
///
/// Dependencies are loaded recursively, and their types are merged into
/// the registry with namespace prefixes (e.g., `Geometry::Point`).
///
/// # Arguments
///
/// * `file_path` - Path to the source file
/// * `source` - Source code content
/// * `loader` - Module loader for resolving imports
/// * `interner` - Symbol interner
///
/// # Example
///
/// ```ignore
/// let mut loader = Loader::new(&root_path);
/// let registry = discover_with_imports(
///     Path::new("main.md"),
///     source,
///     &mut loader,
///     &mut interner
/// )?;
/// // registry now contains types from main.md and all imports
/// ```
pub fn discover_with_imports(
    file_path: &Path,
    source: &str,
    loader: &mut Loader,
    interner: &mut Interner,
) -> Result<TypeRegistry, String> {
    let mut registry = TypeRegistry::with_primitives(interner);

    // 1. Scan for dependencies in the abstract
    let deps = scan_dependencies(source);

    // 2. For each dependency, recursively discover types
    for dep in deps {
        let module_source = loader.resolve(file_path, &dep.uri)?;
        let dep_content = module_source.content.clone();
        let dep_path = module_source.path.clone();

        // Recursively discover types in the dependency
        let dep_registry = discover_with_imports(
            &dep_path,
            &dep_content,
            loader,
            interner
        )?;

        // Merge with namespace prefix
        merge_registry(&mut registry, &dep.alias, dep_registry, interner);
    }

    // 3. Scan local definitions using existing DiscoveryPass
    let mut lexer = Lexer::new(source, interner);
    let tokens = lexer.tokenize();
    let mwe_trie = mwe::build_mwe_trie();
    let tokens = mwe::apply_mwe_pipeline(tokens, &mwe_trie, interner);

    let mut discovery = DiscoveryPass::new(&tokens, interner);
    let local_registry = discovery.run();

    // Merge local types (without namespace prefix)
    for (sym, def) in local_registry.iter_types() {
        // Skip primitives (already in registry)
        let name = interner.resolve(*sym);
        if !["Int", "Nat", "Text", "Bool", "Real", "Unit"].contains(&name) {
            registry.register(*sym, def.clone());
        }
    }

    Ok(registry)
}

/// Merges types from a dependency registry into the main registry with namespace prefix.
fn merge_registry(
    main: &mut TypeRegistry,
    namespace: &str,
    dep: TypeRegistry,
    interner: &mut Interner,
) {
    for (sym, def) in dep.iter_types() {
        // Get original name
        let orig_name = interner.resolve(*sym);

        // Skip primitives
        if ["Int", "Nat", "Text", "Bool", "Real", "Unit"].contains(&orig_name) {
            continue;
        }

        // Create namespaced name: "Geometry::Point"
        let namespaced = format!("{}::{}", namespace, orig_name);
        let namespaced_sym = interner.intern(&namespaced);

        // Clone the typedef with the new namespaced symbol
        let namespaced_def = match def {
            TypeDef::Struct { fields, generics, is_portable, is_shared } => TypeDef::Struct {
                fields: fields.clone(),
                generics: generics.clone(),
                is_portable: *is_portable,
                is_shared: *is_shared,
            },
            TypeDef::Enum { variants, generics, is_portable, is_shared } => TypeDef::Enum {
                variants: variants.clone(),
                generics: generics.clone(),
                is_portable: *is_portable,
                is_shared: *is_shared,
            },
            TypeDef::Alias { target } => TypeDef::Alias {
                target: target.clone(),
            },
            TypeDef::Generic { param_count } => TypeDef::Generic {
                param_count: *param_count,
            },
            TypeDef::Primitive => TypeDef::Primitive,
        };

        main.register(namespaced_sym, namespaced_def);
    }
}
