use crate::token::{Token, TokenType, BlockType};
use crate::intern::{Interner, Symbol};
use super::registry::{TypeRegistry, TypeDef, FieldDef, FieldType, VariantDef};
use super::dependencies::scan_dependencies;

#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;
#[cfg(not(target_arch = "wasm32"))]
use crate::project::Loader;

/// Discovery pass that scans tokens before main parsing to build a TypeRegistry.
///
/// This pass looks for type definitions in `## Definition` blocks:
/// - "A Stack is a generic collection." → Generic type
/// - "A User is a structure." → Struct type
/// - "A Shape is an enum." → Enum type
pub struct DiscoveryPass<'a> {
    tokens: &'a [Token],
    pos: usize,
    interner: &'a mut Interner,
}

impl<'a> DiscoveryPass<'a> {
    pub fn new(tokens: &'a [Token], interner: &'a mut Interner) -> Self {
        Self { tokens, pos: 0, interner }
    }

    /// Run discovery pass, returning populated TypeRegistry
    pub fn run(&mut self) -> TypeRegistry {
        let mut registry = TypeRegistry::with_primitives(self.interner);

        while self.pos < self.tokens.len() {
            // Look for Definition blocks
            if self.check_block_header(BlockType::Definition) {
                self.advance(); // consume ## Definition
                self.scan_definition_block(&mut registry);
            } else if self.check_block_header(BlockType::TypeDef) {
                // Inline type definition: ## A Point has: or ## A Color is one of:
                // The article is part of the block header, so don't skip it
                self.advance(); // consume ## A/An
                self.parse_type_definition_inline(&mut registry);
            } else {
                self.advance();
            }
        }

        registry
    }

    fn check_block_header(&self, expected: BlockType) -> bool {
        matches!(
            self.tokens.get(self.pos),
            Some(Token { kind: TokenType::BlockHeader { block_type }, .. })
            if *block_type == expected
        )
    }

    fn scan_definition_block(&mut self, registry: &mut TypeRegistry) {
        // Scan until next block header or EOF
        while self.pos < self.tokens.len() {
            if matches!(self.peek(), Some(Token { kind: TokenType::BlockHeader { .. }, .. })) {
                break;
            }

            // Look for "A [Name] is a..." pattern
            if self.check_article() {
                self.try_parse_type_definition(registry);
            } else {
                self.advance();
            }
        }
    }

    /// Parse inline type definition where article was part of block header (## A Point has:)
    fn parse_type_definition_inline(&mut self, registry: &mut TypeRegistry) {
        // Don't skip article - it was part of the block header
        self.parse_type_definition_body(registry);
    }

    fn try_parse_type_definition(&mut self, registry: &mut TypeRegistry) {
        self.advance(); // skip article
        self.parse_type_definition_body(registry);
    }

    fn parse_type_definition_body(&mut self, registry: &mut TypeRegistry) {
        if let Some(name_sym) = self.consume_noun_or_proper() {
            // Phase 34: Check for "of [T]" which indicates user-defined generic
            let type_params = if self.check_preposition("of") {
                self.advance(); // consume "of"
                self.parse_type_params()
            } else {
                vec![]
            };

            // Phase 47: Check for "is Portable and" pattern before "has:"
            let mut is_portable = false;
            if self.check_copula() {
                let copula_pos = self.pos;
                self.advance(); // consume is/are
                if self.check_portable() {
                    self.advance(); // consume "Portable"
                    is_portable = true;
                    // Expect "and" after Portable
                    if self.check_word("and") {
                        self.advance(); // consume "and"
                    }
                } else {
                    // Not a Portable pattern, restore position for other checks
                    self.pos = copula_pos;
                }
            }

            // Phase 31/34: Check for "has:" which indicates struct with fields
            // Pattern: "A Point has:" or "A Box of [T] has:" or "A Message is Portable and has:"
            if self.check_word("has") {
                self.advance(); // consume "has"
                if self.check_colon() {
                    self.advance(); // consume ":"
                    // Skip newline if present
                    if self.check_newline() {
                        self.advance();
                    }
                    if self.check_indent() {
                        self.advance(); // consume INDENT
                        let fields = self.parse_struct_fields_with_params(&type_params);
                        registry.register(name_sym, TypeDef::Struct { fields, generics: type_params, is_portable });
                        return;
                    }
                }
            }

            // Check for "is either:" or "is one of:" pattern (Phase 33/34: Sum types with variants)
            if self.check_copula() {
                self.advance(); // consume is/are

                // Phase 33: Check for "either:" or "one of:" pattern
                let is_enum_pattern = if self.check_either() {
                    self.advance(); // consume "either"
                    true
                } else if self.check_word("one") {
                    self.advance(); // consume "one"
                    if self.check_word("of") {
                        self.advance(); // consume "of"
                        true
                    } else {
                        false
                    }
                } else {
                    false
                };

                if is_enum_pattern {
                    if self.check_colon() {
                        self.advance(); // consume ":"
                        // Skip newline if present
                        if self.check_newline() {
                            self.advance();
                        }
                        if self.check_indent() {
                            self.advance(); // consume INDENT
                            let variants = self.parse_enum_variants_with_params(&type_params);
                            registry.register(name_sym, TypeDef::Enum { variants, generics: type_params, is_portable });
                            return;
                        }
                    }
                }

                if self.check_article() {
                    self.advance(); // consume a/an

                    // Look for type indicators
                    if self.check_word("generic") {
                        registry.register(name_sym, TypeDef::Generic { param_count: 1 });
                        self.skip_to_period();
                    } else if self.check_word("record") || self.check_word("struct") || self.check_word("structure") {
                        registry.register(name_sym, TypeDef::Struct { fields: vec![], generics: vec![], is_portable: false });
                        self.skip_to_period();
                    } else if self.check_word("sum") || self.check_word("enum") || self.check_word("choice") {
                        registry.register(name_sym, TypeDef::Enum { variants: vec![], generics: vec![], is_portable: false });
                        self.skip_to_period();
                    }
                }
            } else if !type_params.is_empty() {
                // "A Stack of [Things] is..." - old generic syntax, still supported
                registry.register(name_sym, TypeDef::Generic { param_count: type_params.len() });
                self.skip_to_period();
            }
        }
    }

    /// Phase 33/34: Parse enum variants in "is either:" block
    /// Each variant: "A VariantName." or "A VariantName with a field, which is Type."
    /// or concise: "A VariantName (field: Type)."
    fn parse_enum_variants_with_params(&mut self, type_params: &[Symbol]) -> Vec<VariantDef> {
        let mut variants = Vec::new();

        while self.pos < self.tokens.len() {
            // Exit on dedent or next block
            if self.check_dedent() {
                self.advance();
                break;
            }
            if matches!(self.peek(), Some(Token { kind: TokenType::BlockHeader { .. }, .. })) {
                break;
            }

            // Skip newlines between variants
            if self.check_newline() {
                self.advance();
                continue;
            }

            // Parse variant: "A VariantName [with fields | (field: Type)]." or bare "VariantName."
            // Optionally consume article (a/an) if present
            if self.check_article() {
                self.advance(); // consume "A"/"An"
            }

            // Try to parse variant name (noun or proper name)
            if let Some(variant_name) = self.consume_noun_or_proper() {
                // Check for payload fields
                let fields = if self.check_word("with") {
                    // Natural syntax: "A Circle with a radius, which is Int."
                    self.parse_variant_fields_natural_with_params(type_params)
                } else if self.check_lparen() {
                    // Concise syntax: "A Circle (radius: Int)."
                    self.parse_variant_fields_concise_with_params(type_params)
                } else {
                    // Unit variant: "A Point." or "Point."
                    vec![]
                };

                variants.push(VariantDef {
                    name: variant_name,
                    fields,
                });

                // Consume period
                if self.check_period() {
                    self.advance();
                }
            } else {
                self.advance(); // skip malformed token
            }
        }

        variants
    }

    /// Phase 33: Parse enum variants (backward compat wrapper)
    fn parse_enum_variants(&mut self) -> Vec<VariantDef> {
        self.parse_enum_variants_with_params(&[])
    }

    /// Parse variant fields in natural syntax.
    /// Supports multiple syntaxes:
    /// - "with a radius, which is Int." (verbose natural)
    /// - "with radius Int" (concise natural - no article/comma)
    fn parse_variant_fields_natural_with_params(&mut self, type_params: &[Symbol]) -> Vec<FieldDef> {
        let mut fields = Vec::new();

        // "with" has already been detected, consume it
        self.advance();

        loop {
            // Skip article (optional)
            if self.check_article() {
                self.advance();
            }

            // Get field name
            if let Some(field_name) = self.consume_noun_or_proper() {
                // Support multiple type annotation patterns:
                // 1. ", which is Type" (verbose)
                // 2. " Type" (concise - just a type name after field name)
                let ty = if self.check_comma() {
                    self.advance(); // consume ","
                    // Consume "which"
                    if self.check_word("which") {
                        self.advance();
                    }
                    // Consume "is"
                    if self.check_copula() {
                        self.advance();
                    }
                    self.consume_field_type_with_params(type_params)
                } else {
                    // Concise syntax: "radius Int" - type immediately follows field name
                    self.consume_field_type_with_params(type_params)
                };

                fields.push(FieldDef {
                    name: field_name,
                    ty,
                    is_public: true, // Variant fields are always public
                });

                // Check for "and" to continue: "and height Int"
                // May have comma before "and"
                if self.check_comma() {
                    self.advance(); // consume comma before "and"
                }
                if self.check_word("and") {
                    self.advance();
                    continue;
                }
            }
            break;
        }

        fields
    }

    /// Backward compat wrapper
    fn parse_variant_fields_natural(&mut self) -> Vec<FieldDef> {
        self.parse_variant_fields_natural_with_params(&[])
    }

    /// Parse variant fields in concise syntax: "(radius: Int)" or "(width: Int, height: Int)"
    fn parse_variant_fields_concise_with_params(&mut self, type_params: &[Symbol]) -> Vec<FieldDef> {
        let mut fields = Vec::new();

        // Consume "("
        self.advance();

        loop {
            // Get field name
            if let Some(field_name) = self.consume_noun_or_proper() {
                // Expect ": Type" pattern
                let ty = if self.check_colon() {
                    self.advance(); // consume ":"
                    self.consume_field_type_with_params(type_params)
                } else {
                    FieldType::Primitive(self.interner.intern("Unknown"))
                };

                fields.push(FieldDef {
                    name: field_name,
                    ty,
                    is_public: true, // Variant fields are always public
                });

                // Check for "," to continue
                if self.check_comma() {
                    self.advance();
                    continue;
                }
            }
            break;
        }

        // Consume ")"
        if self.check_rparen() {
            self.advance();
        }

        fields
    }

    /// Backward compat wrapper
    fn parse_variant_fields_concise(&mut self) -> Vec<FieldDef> {
        self.parse_variant_fields_concise_with_params(&[])
    }

    /// Parse struct fields in "has:" block
    /// Each field: "a [public] name, which is Type."
    fn parse_struct_fields_with_params(&mut self, type_params: &[Symbol]) -> Vec<FieldDef> {
        let mut fields = Vec::new();

        while self.pos < self.tokens.len() {
            // Exit on dedent or next block
            if self.check_dedent() {
                self.advance();
                break;
            }
            if matches!(self.peek(), Some(Token { kind: TokenType::BlockHeader { .. }, .. })) {
                break;
            }

            // Skip newlines between fields
            if self.check_newline() {
                self.advance();
                continue;
            }

            // Parse field: "a [public] name, which is Type." or "an x: Int."
            if self.check_article() {
                self.advance(); // consume "a"/"an"

                // Check for "public" modifier
                let has_public_keyword = if self.check_word("public") {
                    self.advance();
                    true
                } else {
                    false
                };
                // Visibility determined later based on syntax used
                let mut is_public = has_public_keyword;

                // Get field name
                if let Some(field_name) = self.consume_noun_or_proper() {
                    // Support both syntaxes:
                    // 1. "name: Type." (concise) - public by default (no visibility syntax)
                    // 2. "name, which is Type." (natural) - private unless "public" keyword
                    let ty = if self.check_colon() {
                        // Concise syntax: "x: Int" - public by default
                        is_public = true;
                        self.advance(); // consume ":"
                        self.consume_field_type_with_params(type_params)
                    } else if self.check_comma() {
                        // Natural syntax: uses has_public_keyword for visibility
                        self.advance(); // consume ","
                        // Consume "which"
                        if self.check_word("which") {
                            self.advance();
                        }
                        // Consume "is"
                        if self.check_copula() {
                            self.advance();
                        }
                        self.consume_field_type_with_params(type_params)
                    } else {
                        // Fallback: unknown type
                        FieldType::Primitive(self.interner.intern("Unknown"))
                    };

                    fields.push(FieldDef {
                        name: field_name,
                        ty,
                        is_public,
                    });

                    // Consume period
                    if self.check_period() {
                        self.advance();
                    }
                } else {
                    self.advance(); // skip malformed token
                }
            } else {
                self.advance();
            }
        }

        fields
    }

    /// Backward compat wrapper
    fn parse_struct_fields(&mut self) -> Vec<FieldDef> {
        self.parse_struct_fields_with_params(&[])
    }

    /// Parse a field type reference
    fn consume_field_type(&mut self) -> FieldType {
        if let Some(name) = self.consume_noun_or_proper() {
            // Check for generic: "List of Int", "Seq of Text"
            if self.check_preposition("of") {
                self.advance();
                let param = self.consume_field_type();
                return FieldType::Generic { base: name, params: vec![param] };
            }

            // Check if primitive
            let name_str = self.interner.resolve(name);
            match name_str {
                "Int" | "Nat" | "Text" | "Bool" | "Real" | "Unit" => FieldType::Primitive(name),
                _ => FieldType::Named(name),
            }
        } else {
            FieldType::Primitive(self.interner.intern("Unknown"))
        }
    }

    // Helper methods
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn advance(&mut self) {
        if self.pos < self.tokens.len() {
            self.pos += 1;
        }
    }

    fn check_article(&self) -> bool {
        match self.peek() {
            Some(Token { kind: TokenType::Article(_), .. }) => true,
            // Also accept ProperName("A") / ProperName("An") which can occur at line starts
            Some(Token { kind: TokenType::ProperName(sym), .. }) => {
                let text = self.interner.resolve(*sym);
                text.eq_ignore_ascii_case("a") || text.eq_ignore_ascii_case("an")
            }
            _ => false,
        }
    }

    fn check_copula(&self) -> bool {
        matches!(self.peek(), Some(Token { kind: TokenType::Is | TokenType::Are, .. }))
    }

    fn check_preposition(&self, word: &str) -> bool {
        if let Some(Token { kind: TokenType::Preposition(sym), .. }) = self.peek() {
            self.interner.resolve(*sym) == word
        } else {
            false
        }
    }

    fn consume_noun_or_proper(&mut self) -> Option<Symbol> {
        let t = self.peek()?;
        match &t.kind {
            TokenType::Noun(s) | TokenType::ProperName(s) => {
                let sym = *s;
                self.advance();
                Some(sym)
            }
            // Phase 31: Also accept Adjective as identifier (for field names like "x")
            TokenType::Adjective(s) => {
                let sym = *s;
                self.advance();
                Some(sym)
            }
            // Phase 47: Accept Performative as type name (for agent messages like "Command")
            TokenType::Performative(s) => {
                let sym = *s;
                self.advance();
                Some(sym)
            }
            // Phase 34: Accept special tokens as identifiers using their lexeme
            TokenType::Items | TokenType::Some => {
                let sym = t.lexeme;
                self.advance();
                Some(sym)
            }
            _ => None
        }
    }

    fn check_word(&self, word: &str) -> bool {
        if let Some(token) = self.peek() {
            // Check against the lexeme of the token
            self.interner.resolve(token.lexeme).eq_ignore_ascii_case(word)
        } else {
            false
        }
    }

    fn skip_to_period(&mut self) {
        while self.pos < self.tokens.len() {
            if matches!(self.peek(), Some(Token { kind: TokenType::Period, .. })) {
                self.advance();
                break;
            }
            self.advance();
        }
    }

    fn check_colon(&self) -> bool {
        matches!(self.peek(), Some(Token { kind: TokenType::Colon, .. }))
    }

    fn check_newline(&self) -> bool {
        matches!(self.peek(), Some(Token { kind: TokenType::Newline, .. }))
    }

    fn check_indent(&self) -> bool {
        matches!(self.peek(), Some(Token { kind: TokenType::Indent, .. }))
    }

    fn check_dedent(&self) -> bool {
        matches!(self.peek(), Some(Token { kind: TokenType::Dedent, .. }))
    }

    fn check_comma(&self) -> bool {
        matches!(self.peek(), Some(Token { kind: TokenType::Comma, .. }))
    }

    fn check_period(&self) -> bool {
        matches!(self.peek(), Some(Token { kind: TokenType::Period, .. }))
    }

    fn check_either(&self) -> bool {
        matches!(self.peek(), Some(Token { kind: TokenType::Either, .. }))
    }

    fn check_lparen(&self) -> bool {
        matches!(self.peek(), Some(Token { kind: TokenType::LParen, .. }))
    }

    fn check_rparen(&self) -> bool {
        matches!(self.peek(), Some(Token { kind: TokenType::RParen, .. }))
    }

    /// Phase 47: Check for Portable token
    fn check_portable(&self) -> bool {
        matches!(self.peek(), Some(Token { kind: TokenType::Portable, .. }))
    }

    // Phase 34: Bracket checks for type parameters
    fn check_lbracket(&self) -> bool {
        matches!(self.peek(), Some(Token { kind: TokenType::LBracket, .. }))
    }

    fn check_rbracket(&self) -> bool {
        matches!(self.peek(), Some(Token { kind: TokenType::RBracket, .. }))
    }

    /// Phase 34: Parse type parameters in brackets: "[T]" or "[A] and [B]"
    fn parse_type_params(&mut self) -> Vec<Symbol> {
        let mut params = Vec::new();

        loop {
            if self.check_lbracket() {
                self.advance(); // consume [
                if let Some(param) = self.consume_noun_or_proper() {
                    params.push(param);
                }
                if self.check_rbracket() {
                    self.advance(); // consume ]
                }
            }

            // Check for "and" separator for multi-param generics
            if self.check_word("and") {
                self.advance();
                continue;
            }
            break;
        }
        params
    }

    /// Phase 34: Parse a field type reference, recognizing type parameters
    fn consume_field_type_with_params(&mut self, type_params: &[Symbol]) -> FieldType {
        // Phase 34: Single-letter type params like "A" may be tokenized as Article
        // Check for Article that matches a type param first
        if let Some(Token { kind: TokenType::Article(_), lexeme, .. }) = self.peek() {
            let text = self.interner.resolve(*lexeme);
            // Find matching type param by name (case-insensitive for single letters)
            for &param_sym in type_params {
                let param_name = self.interner.resolve(param_sym);
                if text.eq_ignore_ascii_case(param_name) {
                    self.advance(); // consume the article token
                    return FieldType::TypeParam(param_sym);
                }
            }
        }

        if let Some(name) = self.consume_noun_or_proper() {
            // Check if this is a type parameter reference
            if type_params.contains(&name) {
                return FieldType::TypeParam(name);
            }

            // Check for generic: "List of Int", "Seq of Text", "List of T"
            if self.check_preposition("of") {
                self.advance();
                let param = self.consume_field_type_with_params(type_params);
                return FieldType::Generic { base: name, params: vec![param] };
            }

            // Check if primitive
            let name_str = self.interner.resolve(name);
            match name_str {
                "Int" | "Nat" | "Text" | "Bool" | "Real" | "Unit" => FieldType::Primitive(name),
                _ => FieldType::Named(name),
            }
        } else {
            FieldType::Primitive(self.interner.intern("Unknown"))
        }
    }
}

/// Phase 36: Recursive discovery with module imports.
///
/// This function scans a LOGOS source file for:
/// 1. Dependencies declared in the Abstract (Markdown links)
/// 2. Type definitions in ## Definition blocks
///
/// Dependencies are loaded recursively, and their types are merged into
/// the registry with namespace prefixes (e.g., "Geometry::Point").
#[cfg(not(target_arch = "wasm32"))]
pub fn discover_with_imports(
    file_path: &Path,
    source: &str,
    loader: &mut Loader,
    interner: &mut Interner,
) -> Result<TypeRegistry, String> {
    use crate::Lexer;
    use crate::mwe;

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
#[cfg(not(target_arch = "wasm32"))]
fn merge_registry(
    main: &mut TypeRegistry,
    namespace: &str,
    dep: TypeRegistry,
    interner: &mut Interner,
) {
    for (sym, def) in dep.iter_types() {
        let name = interner.resolve(*sym);
        // Skip primitives
        if ["Int", "Nat", "Text", "Bool", "Real", "Unit"].contains(&name) {
            continue;
        }
        // Create namespaced symbol: "Geometry::Point"
        let qualified = format!("{}::{}", namespace, name);
        let new_sym = interner.intern(&qualified);
        main.register(new_sym, def.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Lexer;
    use crate::mwe;

    fn make_tokens(source: &str, interner: &mut Interner) -> Vec<Token> {
        let mut lexer = Lexer::new(source, interner);
        let tokens = lexer.tokenize();
        let mwe_trie = mwe::build_mwe_trie();
        mwe::apply_mwe_pipeline(tokens, &mwe_trie, interner)
    }

    #[test]
    fn discovery_finds_generic_in_definition_block() {
        let source = "## Definition\nA Stack is a generic collection.";
        let mut interner = Interner::new();
        let tokens = make_tokens(source, &mut interner);

        let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
        let registry = discovery.run();

        let stack = interner.intern("Stack");
        assert!(registry.is_generic(stack), "Stack should be discovered as generic");
    }

    #[test]
    fn discovery_parses_struct_with_fields() {
        let source = r#"## Definition
A Point has:
    an x, which is Int.
    a y, which is Int.
"#;
        let mut interner = Interner::new();
        let tokens = make_tokens(source, &mut interner);

        let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
        let registry = discovery.run();

        let point = interner.intern("Point");
        assert!(registry.is_type(point), "Point should be registered");

        if let Some(TypeDef::Struct { fields, generics, .. }) = registry.get(point) {
            assert_eq!(fields.len(), 2, "Point should have 2 fields, got {:?}", fields);
            assert_eq!(interner.resolve(fields[0].name), "x");
            assert_eq!(interner.resolve(fields[1].name), "y");
            assert!(generics.is_empty(), "Point should have no generics");
        } else {
            panic!("Point should be a struct with fields");
        }
    }

    #[test]
    fn discovery_works_with_markdown_header() {
        // Phase 36: LOGOS files have `# Header` before `## Definition`
        let source = r#"# Geometry

## Definition
A Point has:
    an x, which is Int.
"#;
        let mut interner = Interner::new();
        let tokens = make_tokens(source, &mut interner);

        // Debug: print tokens to see what we're getting
        for (i, tok) in tokens.iter().enumerate() {
            eprintln!("Token {}: {:?}", i, tok.kind);
        }

        let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
        let registry = discovery.run();
        let point = interner.intern("Point");
        assert!(registry.is_type(point), "Point should be discovered even with # header");
    }

    #[test]
    fn discovery_parses_portable_enum() {
        let source = r#"## Definition
A Command is Portable and is either:
    a Start.
    a Stop.
    a Pause.
"#;
        let mut interner = Interner::new();
        let tokens = make_tokens(source, &mut interner);

        // Debug: print tokens to see what we're getting
        eprintln!("Tokens for portable enum:");
        for (i, tok) in tokens.iter().enumerate() {
            eprintln!("Token {}: {:?} ({})", i, tok.kind, interner.resolve(tok.lexeme));
        }

        let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
        let registry = discovery.run();

        let command = interner.intern("Command");
        assert!(registry.is_type(command), "Command should be registered as type");

        if let Some(TypeDef::Enum { variants, is_portable, .. }) = registry.get(command) {
            eprintln!("Command is_portable: {}", is_portable);
            eprintln!("Variants: {:?}", variants.iter().map(|v| interner.resolve(v.name)).collect::<Vec<_>>());
            assert!(*is_portable, "Command should be portable");
            assert_eq!(variants.len(), 3, "Command should have 3 variants");
        } else {
            panic!("Command should be an enum, got: {:?}", registry.get(command));
        }
    }
}
