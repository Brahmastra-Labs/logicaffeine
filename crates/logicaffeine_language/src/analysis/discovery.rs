//! Discovery pass for type and policy extraction.
//!
//! Runs before main parsing to scan tokens for type and policy definitions.
//! Populates [`TypeRegistry`] and [`PolicyRegistry`] for use during parsing.
//!
//! # Discovery Targets
//!
//! | Block | Pattern | Result |
//! |-------|---------|--------|
//! | `## Definition` | "A Stack is a generic collection." | `TypeDef::Generic` |
//! | `## Definition` | "A User is a structure." | `TypeDef::Struct` |
//! | `## Definition` | "A Shape is an enum." | `TypeDef::Enum` |
//! | `## Policy` | "A user can publish if they are admin." | `CapabilityDef` |
//!
//! # Key Function
//!
//! [`DiscoveryPass::run`] - Execute the discovery pass and return registries.

use crate::token::{Token, TokenType, BlockType};
use logicaffeine_base::{Interner, Symbol};
use super::registry::{TypeRegistry, TypeDef, FieldDef, FieldType, VariantDef};
use super::policy::{PolicyRegistry, PredicateDef, CapabilityDef, PolicyCondition};
use super::dependencies::scan_dependencies;

/// Result of running the discovery pass
pub struct DiscoveryResult {
    pub types: TypeRegistry,
    pub policies: PolicyRegistry,
}

/// Discovery pass that scans tokens before main parsing to build a TypeRegistry.
///
/// This pass looks for type definitions in `## Definition` blocks:
/// - "A Stack is a generic collection." → Generic type
/// - "A User is a structure." → Struct type
/// - "A Shape is an enum." → Enum type
///
/// Phase 50: Also scans `## Policy` blocks for security predicates and capabilities.
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
    /// (Backward compatible - returns only TypeRegistry)
    pub fn run(&mut self) -> TypeRegistry {
        self.run_full().types
    }

    /// Phase 50: Run discovery pass, returning both TypeRegistry and PolicyRegistry
    pub fn run_full(&mut self) -> DiscoveryResult {
        let mut type_registry = TypeRegistry::with_primitives(self.interner);
        let mut policy_registry = PolicyRegistry::new();

        while self.pos < self.tokens.len() {
            // Look for Definition blocks
            if self.check_block_header(BlockType::Definition) {
                self.advance(); // consume ## Definition
                self.scan_definition_block(&mut type_registry);
            } else if self.check_block_header(BlockType::TypeDef) {
                // Inline type definition: ## A Point has: or ## A Color is one of:
                // The article is part of the block header, so don't skip it
                self.advance(); // consume ## A/An
                self.parse_type_definition_inline(&mut type_registry);
            } else if self.check_block_header(BlockType::Policy) {
                // Phase 50: Security policy definitions
                self.advance(); // consume ## Policy
                self.scan_policy_block(&mut policy_registry);
            } else if self.check_block_header(BlockType::Requires) {
                // Requires blocks contain dependency metadata, not type definitions.
                // Skip to next block header.
                self.advance(); // consume ## Requires
                while self.pos < self.tokens.len() {
                    if matches!(self.tokens.get(self.pos), Some(Token { kind: TokenType::BlockHeader { .. }, .. })) {
                        break;
                    }
                    self.advance();
                }
            } else {
                self.advance();
            }
        }

        DiscoveryResult {
            types: type_registry,
            policies: policy_registry,
        }
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

    /// Phase 50: Scan policy block for predicate and capability definitions
    /// Patterns:
    /// - "A User is admin if the user's role equals \"admin\"."
    /// - "A User can publish the Document if the user is admin OR the user equals the document's owner."
    fn scan_policy_block(&mut self, registry: &mut PolicyRegistry) {
        while self.pos < self.tokens.len() {
            if matches!(self.peek(), Some(Token { kind: TokenType::BlockHeader { .. }, .. })) {
                break;
            }

            // Skip newlines and indentation
            if self.check_newline() || self.check_indent() || self.check_dedent() {
                self.advance();
                continue;
            }

            // Look for "A [Type] is [predicate] if..." or "A [Type] can [action] ..."
            if self.check_article() {
                self.try_parse_policy_definition(registry);
            } else {
                self.advance();
            }
        }
    }

    /// Phase 50: Parse a policy definition
    fn try_parse_policy_definition(&mut self, registry: &mut PolicyRegistry) {
        self.advance(); // consume article

        // Get subject type name (e.g., "User")
        let subject_type = match self.consume_noun_or_proper() {
            Some(sym) => sym,
            None => return,
        };

        // Determine if predicate ("is admin") or capability ("can publish")
        if self.check_copula() {
            // "A User is admin if..."
            self.advance(); // consume "is"

            // Get predicate name (e.g., "admin")
            let predicate_name = match self.consume_noun_or_proper() {
                Some(sym) => sym,
                None => return,
            };

            // Expect "if"
            if !self.check_word("if") {
                self.skip_to_period();
                return;
            }
            self.advance(); // consume "if"

            // Handle multi-line condition (colon followed by indented lines)
            if self.check_colon() {
                self.advance();
            }
            if self.check_newline() {
                self.advance();
            }
            if self.check_indent() {
                self.advance();
            }

            // Parse condition
            let condition = self.parse_policy_condition(subject_type, None);

            registry.register_predicate(PredicateDef {
                subject_type,
                predicate_name,
                condition,
            });

            self.skip_to_period();
        } else if self.check_word("can") {
            // "A User can publish the Document if..."
            self.advance(); // consume "can"

            // Get action name (e.g., "publish")
            let action = match self.consume_noun_or_proper() {
                Some(sym) => sym,
                None => {
                    // Try verb token
                    if let Some(Token { kind: TokenType::Verb { lemma, .. }, .. }) = self.peek() {
                        let sym = *lemma;
                        self.advance();
                        sym
                    } else {
                        return;
                    }
                }
            };

            // Skip "the" article if present
            if self.check_article() {
                self.advance();
            }

            // Get object type (e.g., "Document")
            let object_type = match self.consume_noun_or_proper() {
                Some(sym) => sym,
                None => return,
            };

            // Expect "if"
            if !self.check_word("if") {
                self.skip_to_period();
                return;
            }
            self.advance(); // consume "if"

            // Parse condition (may include colon for multi-line)
            if self.check_colon() {
                self.advance();
            }
            if self.check_newline() {
                self.advance();
            }
            if self.check_indent() {
                self.advance();
            }

            let condition = self.parse_policy_condition(subject_type, Some(object_type));

            registry.register_capability(CapabilityDef {
                subject_type,
                action,
                object_type,
                condition,
            });

            // Skip to end of definition (may span multiple lines)
            self.skip_policy_definition();
        } else {
            self.skip_to_period();
        }
    }

    /// Phase 50: Parse a policy condition
    /// Handles: field comparisons, predicate references, and OR/AND combinators
    fn parse_policy_condition(&mut self, subject_type: Symbol, object_type: Option<Symbol>) -> PolicyCondition {
        let first = self.parse_atomic_condition(subject_type, object_type);

        // Check for OR/AND combinators
        loop {
            // Skip newlines between conditions
            while self.check_newline() {
                self.advance();
            }

            // Handle ", AND" or ", OR" patterns
            if self.check_comma() {
                self.advance(); // consume comma
                // Skip whitespace after comma
                while self.check_newline() {
                    self.advance();
                }
            }

            if self.check_word("AND") {
                self.advance();
                // Skip newlines after AND
                while self.check_newline() {
                    self.advance();
                }
                let right = self.parse_atomic_condition(subject_type, object_type);
                return PolicyCondition::And(Box::new(first), Box::new(right));
            } else if self.check_word("OR") {
                self.advance();
                // Skip newlines after OR
                while self.check_newline() {
                    self.advance();
                }
                let right = self.parse_atomic_condition(subject_type, object_type);
                return PolicyCondition::Or(Box::new(first), Box::new(right));
            } else {
                break;
            }
        }

        first
    }

    /// Phase 50: Parse an atomic condition
    fn parse_atomic_condition(&mut self, subject_type: Symbol, object_type: Option<Symbol>) -> PolicyCondition {
        // Skip "The" article if present
        if self.check_article() {
            self.advance();
        }

        // Get the subject reference (e.g., "user" or "user's role")
        let subject_ref = match self.consume_noun_or_proper() {
            Some(sym) => sym,
            None => return PolicyCondition::FieldEquals {
                field: self.interner.intern("unknown"),
                value: self.interner.intern("unknown"),
                is_string_literal: false,
            },
        };

        // Check if it's a field access ("'s role") or a predicate ("is admin")
        if self.check_possessive() {
            self.advance(); // consume "'s"

            // Get field name
            let field = match self.consume_noun_or_proper() {
                Some(sym) => sym,
                None => return PolicyCondition::FieldEquals {
                    field: self.interner.intern("unknown"),
                    value: self.interner.intern("unknown"),
                    is_string_literal: false,
                },
            };

            // Expect "equals"
            if self.check_word("equals") {
                self.advance();

                // Get value (string literal or identifier)
                let (value, is_string_literal) = self.consume_value();

                return PolicyCondition::FieldEquals { field, value, is_string_literal };
            }
        } else if self.check_copula() {
            // "user is admin"
            self.advance(); // consume "is"

            // Get predicate name
            let predicate = match self.consume_noun_or_proper() {
                Some(sym) => sym,
                None => return PolicyCondition::FieldEquals {
                    field: self.interner.intern("unknown"),
                    value: self.interner.intern("unknown"),
                    is_string_literal: false,
                },
            };

            return PolicyCondition::Predicate {
                subject: subject_ref,
                predicate,
            };
        } else if self.check_word("equals") {
            // "user equals the document's owner"
            self.advance(); // consume "equals"

            // Skip "the" if present
            if self.check_article() {
                self.advance();
            }

            // Check for object field reference: "document's owner"
            if let Some(obj_ref) = self.consume_noun_or_proper() {
                if self.check_possessive() {
                    self.advance(); // consume "'s"
                    if let Some(field) = self.consume_noun_or_proper() {
                        return PolicyCondition::ObjectFieldEquals {
                            subject: subject_ref,
                            object: obj_ref,
                            field,
                        };
                    }
                }
            }
        }

        // Fallback: unknown condition
        PolicyCondition::FieldEquals {
            field: self.interner.intern("unknown"),
            value: self.interner.intern("unknown"),
            is_string_literal: false,
        }
    }

    /// Consume a value (string literal or identifier), returning the symbol and whether it was a string literal
    fn consume_value(&mut self) -> (Symbol, bool) {
        if let Some(Token { kind: TokenType::StringLiteral(sym), .. }) = self.peek() {
            let s = *sym;
            self.advance();
            (s, true)
        } else if let Some(sym) = self.consume_noun_or_proper() {
            (sym, false)
        } else {
            (self.interner.intern("unknown"), false)
        }
    }

    /// Check for possessive marker ('s)
    fn check_possessive(&self) -> bool {
        matches!(self.peek(), Some(Token { kind: TokenType::Possessive, .. }))
    }

    /// Skip to end of a multi-line policy definition
    fn skip_policy_definition(&mut self) {
        let mut depth = 0;
        while self.pos < self.tokens.len() {
            if self.check_indent() {
                depth += 1;
            } else if self.check_dedent() {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            if self.check_period() && depth == 0 {
                self.advance();
                break;
            }
            if matches!(self.peek(), Some(Token { kind: TokenType::BlockHeader { .. }, .. })) {
                break;
            }
            self.advance();
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
        // Phase 47/49: Check for pre-type modifiers: "A portable Config has:" or "A shared Config has:"
        let mut is_portable = false;
        let mut is_shared = false;
        loop {
            if self.check_portable() {
                is_portable = true;
                self.advance();
            } else if self.check_shared() {
                is_shared = true;
                self.advance();
            } else {
                break;
            }
        }

        if let Some(name_sym) = self.consume_noun_or_proper() {
            // Phase 34: Check for "of [T]" which indicates user-defined generic
            let type_params = if self.check_preposition("of") {
                self.advance(); // consume "of"
                self.parse_type_params()
            } else {
                vec![]
            };
            if self.check_copula() {
                let copula_pos = self.pos;
                self.advance(); // consume is/are

                // Check for modifiers in any order (e.g., "is Shared and Portable and")
                loop {
                    if self.check_portable() {
                        self.advance(); // consume "Portable"
                        is_portable = true;
                        if self.check_word("and") {
                            self.advance(); // consume "and"
                        }
                    } else if self.check_shared() {
                        self.advance(); // consume "Shared"
                        is_shared = true;
                        if self.check_word("and") {
                            self.advance(); // consume "and"
                        }
                    } else {
                        break;
                    }
                }

                // If no modifiers were found, restore position
                if !is_portable && !is_shared {
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
                        registry.register(name_sym, TypeDef::Struct { fields, generics: type_params, is_portable, is_shared });
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
                            registry.register(name_sym, TypeDef::Enum { variants, generics: type_params, is_portable, is_shared });
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
                        registry.register(name_sym, TypeDef::Struct { fields: vec![], generics: vec![], is_portable: false, is_shared: false });
                        self.skip_to_period();
                    } else if self.check_word("sum") || self.check_word("enum") || self.check_word("choice") {
                        registry.register(name_sym, TypeDef::Enum { variants: vec![], generics: vec![], is_portable: false, is_shared: false });
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
                } else if self.check_colon() {
                    self.advance(); // consume ":"
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

            // Parse field: "a [public] name, which is Type." or "name: Type." (no article)
            // Check for article (optional for concise syntax)
            let has_article = self.check_article();
            if has_article {
                self.advance(); // consume "a"/"an"
            }

            // Check for "public" modifier
            let has_public_keyword = if self.check_word("public") {
                self.advance();
                true
            } else {
                false
            };
            // Visibility determined later based on syntax used
            let mut is_public = has_public_keyword;

            // Get field name - try to parse if we had article OR if next token looks like identifier
            if let Some(field_name) = self.consume_noun_or_proper() {
                // Support both syntaxes:
                // 1. "name: Type." (concise) - public by default
                // 2. "name, which is Type." (natural) - public by default
                let ty = if self.check_colon() {
                    // Concise syntax: "x: Int" - public by default
                    is_public = true;
                    self.advance(); // consume ":"
                    self.consume_field_type_with_params(type_params)
                } else if self.check_comma() {
                    // Natural syntax: "name, which is Type" - also public by default
                    is_public = true;
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
                } else if !has_article {
                    // No colon and no article - this wasn't a field, skip
                    continue;
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
            } else if !has_article {
                // Didn't have article and couldn't get field name - skip this token
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
        // Bug fix: Handle parenthesized type expressions: "Seq of (Seq of Int)"
        if self.check_lparen() {
            self.advance(); // consume "("
            let inner_type = self.consume_field_type();
            if self.check_rparen() {
                self.advance(); // consume ")"
            }
            return inner_type;
        }

        // Skip article if present (e.g., "a Tally" -> "Tally")
        if self.check_article() {
            self.advance();
        }

        if let Some(name) = self.consume_noun_or_proper() {
            let name_str = self.interner.resolve(name);

            // Phase 49c: Check for bias/algorithm modifier on SharedSet: "SharedSet (AddWins) of T"
            let modified_name = if name_str == "SharedSet" || name_str == "ORSet" {
                if self.check_lparen() {
                    self.advance(); // consume "("
                    let modifier = if self.check_removewins() {
                        self.advance(); // consume "RemoveWins"
                        Some("SharedSet_RemoveWins")
                    } else if self.check_addwins() {
                        self.advance(); // consume "AddWins"
                        Some("SharedSet_AddWins")
                    } else {
                        None
                    };
                    if self.check_rparen() {
                        self.advance(); // consume ")"
                    }
                    modifier.map(|m| self.interner.intern(m))
                } else {
                    None
                }
            } else if name_str == "SharedSequence" {
                // Phase 49c: Check for algorithm modifier on SharedSequence: "SharedSequence (YATA) of T"
                if self.check_lparen() {
                    self.advance(); // consume "("
                    let modifier = if self.check_yata() {
                        self.advance(); // consume "YATA"
                        Some("SharedSequence_YATA")
                    } else {
                        None
                    };
                    if self.check_rparen() {
                        self.advance(); // consume ")"
                    }
                    modifier.map(|m| self.interner.intern(m))
                } else {
                    None
                }
            } else {
                None
            };

            // Use modified name if we found a modifier, otherwise use original
            let final_name = modified_name.unwrap_or(name);
            let final_name_str = self.interner.resolve(final_name);

            // Phase 49c: Handle "SharedMap from K to V" / "ORMap from K to V" syntax
            if (final_name_str == "SharedMap" || final_name_str == "ORMap") && self.check_from() {
                self.advance(); // consume "from"
                let key_type = self.consume_field_type();
                // Expect "to" (can be TokenType::To or preposition)
                if self.check_to() {
                    self.advance(); // consume "to"
                }
                let value_type = self.consume_field_type();
                return FieldType::Generic { base: final_name, params: vec![key_type, value_type] };
            }

            // Check for generic: "List of Int", "Seq of Text", "Map of K to V"
            if self.check_preposition("of") {
                // Check if this is a Map type that needs two params (before we start mutating)
                let is_map_type = final_name_str == "Map" || final_name_str == "HashMap";

                self.advance();
                let first_param = self.consume_field_type();

                // For Map/HashMap, check for "to" separator to parse second type parameter
                if is_map_type && self.check_to() {
                    self.advance(); // consume "to"
                    let second_param = self.consume_field_type();
                    return FieldType::Generic { base: final_name, params: vec![first_param, second_param] };
                }

                return FieldType::Generic { base: final_name, params: vec![first_param] };
            }

            // Phase 49b: "Divergent T" syntax (no "of" required)
            if final_name_str == "Divergent" {
                // Next token should be the inner type
                let param = self.consume_field_type();
                return FieldType::Generic { base: final_name, params: vec![param] };
            }

            // Check if primitive
            match final_name_str {
                "Int" | "Nat" | "Text" | "Bool" | "Real" | "Unit" => FieldType::Primitive(final_name),
                _ => FieldType::Named(final_name),
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
        match self.peek() {
            Some(Token { kind: TokenType::Is | TokenType::Are, .. }) => true,
            // Also match "is" when tokenized as a verb (common in declarative mode)
            Some(Token { kind: TokenType::Verb { lemma, .. }, .. }) => {
                let word = self.interner.resolve(*lemma).to_lowercase();
                word == "is" || word == "are"
            }
            _ => false,
        }
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
            // Phase 49/50: Accept Verb tokens as identifiers
            // - Uppercase verbs like "Setting" are type names
            // - Lowercase verbs like "trusted", "privileged" are predicate names
            // Use lexeme to preserve the original word (not lemma which strips suffixes)
            TokenType::Verb { .. } => {
                let sym = t.lexeme;
                self.advance();
                Some(sym)
            }
            // Phase 49b: Accept CRDT type tokens as type names
            TokenType::Tally => {
                self.advance();
                Some(self.interner.intern("Tally"))
            }
            TokenType::SharedSet => {
                self.advance();
                Some(self.interner.intern("SharedSet"))
            }
            TokenType::SharedSequence => {
                self.advance();
                Some(self.interner.intern("SharedSequence"))
            }
            TokenType::CollaborativeSequence => {
                self.advance();
                Some(self.interner.intern("CollaborativeSequence"))
            }
            TokenType::SharedMap => {
                self.advance();
                Some(self.interner.intern("SharedMap"))
            }
            TokenType::Divergent => {
                self.advance();
                Some(self.interner.intern("Divergent"))
            }
            // Phase 49: Accept Ambiguous tokens (e.g., "name" could be verb or noun)
            // Use lexeme to get the original word
            TokenType::Ambiguous { .. } => {
                let sym = t.lexeme;
                self.advance();
                Some(sym)
            }
            // Escape hatch keyword can be a type/identifier name
            TokenType::Escape => {
                let sym = t.lexeme;
                self.advance();
                Some(sym)
            }
            // Phrasal verb particles can be identifiers (out, up, down, etc.)
            TokenType::Particle(_) => {
                let sym = t.lexeme;
                self.advance();
                Some(sym)
            }
            // Prepositions can be identifiers in code context (from, into, etc.)
            TokenType::Preposition(_) => {
                let sym = t.lexeme;
                self.advance();
                Some(sym)
            }
            // Phase 103: Accept Focus tokens as identifiers (e.g., "Just" for Maybe variants)
            TokenType::Focus(_) => {
                let sym = t.lexeme;
                self.advance();
                Some(sym)
            }
            // Phase 103: Accept Nothing token as identifier (for Maybe/Option variants)
            TokenType::Nothing => {
                let sym = t.lexeme;
                self.advance();
                Some(sym)
            }
            // Phase 103: Accept Article tokens as type parameter names (L, R, A, etc.)
            TokenType::Article(_) => {
                let sym = t.lexeme;
                self.advance();
                Some(sym)
            }
            // Phase 103: Accept Either token as type name (for Either type definition)
            TokenType::Either => {
                let sym = t.lexeme;
                self.advance();
                Some(sym)
            }
            // Calendar unit tokens can be type/variant/field names (Day, Week, Month, Year)
            TokenType::CalendarUnit(_) => {
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

    /// Phase 49c: Check for AddWins token
    fn check_addwins(&self) -> bool {
        matches!(self.peek(), Some(Token { kind: TokenType::AddWins, .. }))
    }

    /// Phase 49c: Check for RemoveWins token
    fn check_removewins(&self) -> bool {
        matches!(self.peek(), Some(Token { kind: TokenType::RemoveWins, .. }))
    }

    /// Phase 49c: Check for YATA token
    fn check_yata(&self) -> bool {
        matches!(self.peek(), Some(Token { kind: TokenType::YATA, .. }))
    }

    /// Phase 49c: Check for "to" (either TokenType::To or preposition "to")
    fn check_to(&self) -> bool {
        match self.peek() {
            Some(Token { kind: TokenType::To, .. }) => true,
            Some(Token { kind: TokenType::Preposition(sym), .. }) => {
                self.interner.resolve(*sym) == "to"
            }
            _ => false,
        }
    }

    /// Phase 49c: Check for "from" (either TokenType::From or preposition "from")
    fn check_from(&self) -> bool {
        match self.peek() {
            Some(Token { kind: TokenType::From, .. }) => true,
            Some(Token { kind: TokenType::Preposition(sym), .. }) => {
                self.interner.resolve(*sym) == "from"
            }
            _ => false,
        }
    }

    /// Phase 47: Check for Portable token
    fn check_portable(&self) -> bool {
        matches!(self.peek(), Some(Token { kind: TokenType::Portable, .. }))
    }

    /// Phase 49: Check for Shared token
    fn check_shared(&self) -> bool {
        matches!(self.peek(), Some(Token { kind: TokenType::Shared, .. }))
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
        // Bug fix: Handle parenthesized type expressions: "Seq of (Seq of Int)"
        if self.check_lparen() {
            self.advance(); // consume "("
            let inner_type = self.consume_field_type_with_params(type_params);
            if self.check_rparen() {
                self.advance(); // consume ")"
            }
            return inner_type;
        }

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
            // Article didn't match a type param, skip it (e.g., "a Tally" -> "Tally")
            self.advance();
        }

        if let Some(name) = self.consume_noun_or_proper() {
            // Check if this is a type parameter reference
            if type_params.contains(&name) {
                return FieldType::TypeParam(name);
            }

            let name_str = self.interner.resolve(name);

            // Phase 49c: Check for bias/algorithm modifier on SharedSet: "SharedSet (AddWins) of T"
            let modified_name = if name_str == "SharedSet" || name_str == "ORSet" {
                if self.check_lparen() {
                    self.advance(); // consume "("
                    let modifier = if self.check_removewins() {
                        self.advance(); // consume "RemoveWins"
                        Some("SharedSet_RemoveWins")
                    } else if self.check_addwins() {
                        self.advance(); // consume "AddWins"
                        Some("SharedSet_AddWins")
                    } else {
                        None
                    };
                    if self.check_rparen() {
                        self.advance(); // consume ")"
                    }
                    modifier.map(|m| self.interner.intern(m))
                } else {
                    None
                }
            } else if name_str == "SharedSequence" {
                // Phase 49c: Check for algorithm modifier on SharedSequence: "SharedSequence (YATA) of T"
                if self.check_lparen() {
                    self.advance(); // consume "("
                    let modifier = if self.check_yata() {
                        self.advance(); // consume "YATA"
                        Some("SharedSequence_YATA")
                    } else {
                        None
                    };
                    if self.check_rparen() {
                        self.advance(); // consume ")"
                    }
                    modifier.map(|m| self.interner.intern(m))
                } else {
                    None
                }
            } else {
                None
            };

            // Use modified name if we found a modifier, otherwise use original
            let final_name = modified_name.unwrap_or(name);
            let final_name_str = self.interner.resolve(final_name);

            // Phase 49c: Handle "SharedMap from K to V" / "ORMap from K to V" syntax
            if (final_name_str == "SharedMap" || final_name_str == "ORMap") && self.check_from() {
                self.advance(); // consume "from"
                let key_type = self.consume_field_type_with_params(type_params);
                // Expect "to" (can be TokenType::To or preposition)
                if self.check_to() {
                    self.advance(); // consume "to"
                }
                let value_type = self.consume_field_type_with_params(type_params);
                return FieldType::Generic { base: final_name, params: vec![key_type, value_type] };
            }

            // Check for generic: "List of Int", "Seq of Text", "List of T", "Map of K to V"
            if self.check_preposition("of") {
                // Check if this is a Map type that needs two params (before we start mutating)
                let is_map_type = final_name_str == "Map" || final_name_str == "HashMap";

                self.advance();
                let first_param = self.consume_field_type_with_params(type_params);

                // For Map/HashMap, check for "to" separator to parse second type parameter
                if is_map_type && self.check_to() {
                    self.advance(); // consume "to"
                    let second_param = self.consume_field_type_with_params(type_params);
                    return FieldType::Generic { base: final_name, params: vec![first_param, second_param] };
                }

                return FieldType::Generic { base: final_name, params: vec![first_param] };
            }

            // Phase 49b: "Divergent T" syntax (no "of" required)
            if final_name_str == "Divergent" {
                // Next token should be the inner type
                let param = self.consume_field_type_with_params(type_params);
                return FieldType::Generic { base: final_name, params: vec![param] };
            }

            // Check if primitive
            match final_name_str {
                "Int" | "Nat" | "Text" | "Bool" | "Real" | "Unit" => FieldType::Primitive(final_name),
                _ => FieldType::Named(final_name),
            }
        } else {
            FieldType::Primitive(self.interner.intern("Unknown"))
        }
    }
}

// Note: discover_with_imports is defined in the main crate since it needs
// access to the project::Loader which is part of the compile system.

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

    #[test]
    fn discovery_parses_lww_int_field() {
        let source = r#"## Definition
A Setting is Shared and has:
    a volume, which is LastWriteWins of Int.
"#;
        let mut interner = Interner::new();
        let tokens = make_tokens(source, &mut interner);

        // Debug: print tokens
        eprintln!("Tokens for LWW of Int:");
        for (i, tok) in tokens.iter().enumerate() {
            eprintln!("{:3}: {:?} ({})", i, tok.kind, interner.resolve(tok.lexeme));
        }

        let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
        let registry = discovery.run();

        let setting = interner.intern("Setting");
        assert!(registry.is_type(setting), "Setting should be registered");

        if let Some(TypeDef::Struct { fields, is_shared, .. }) = registry.get(setting) {
            eprintln!("is_shared: {}", is_shared);
            eprintln!("Fields: {:?}", fields.len());
            for f in fields {
                eprintln!("  field: {} = {:?}", interner.resolve(f.name), f.ty);
            }
            assert!(*is_shared, "Setting should be shared");
            assert_eq!(fields.len(), 1, "Setting should have 1 field");
        } else {
            panic!("Setting should be a struct, got: {:?}", registry.get(setting));
        }
    }
}
