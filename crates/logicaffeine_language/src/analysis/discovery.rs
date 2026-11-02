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

use super::policy::{CapabilityDef, PolicyCondition, PolicyRegistry, PredicateDef};
use super::registry::{FieldDef, FieldType, TypeDef, TypeRegistry, VariantDef};
use crate::token::{BlockType, Span, Token, TokenType};
use logicaffeine_base::{Interner, Symbol};

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
        Self {
            tokens,
            pos: 0,
            interner,
        }
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
        let mut view_variants: Vec<VariantDef> = Vec::new();

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
            } else if self.check_block_header(BlockType::Session) {
                // `## Session` declares a RECORD, in the same words a record is declared in, and
                // it is registered HERE, from the tokens, so the parse that follows knows it. A
                // `## Store`'s record is not: the lowering types it by the rule that builds it and
                // registers it into the parser's own registry (`lgx_lower::stores`), which is the
                // registry the engines read once the parse is done (`Parser::into_type_registry`).
                self.advance(); // consume ## Session
                self.scan_session_block(&mut type_registry);
            } else if self.check_block_header(BlockType::Function) {
                // A `-> View` template declares a variant of the program's `__View`: a page's
                // lowered code holds a View as that DATA (`parser::lgx_lower::views`), and the
                // enum is registered HERE for the reason `## Session`'s record is — see that comment.
                self.advance(); // consume ## To
                if let Some(variant) = self.scan_view_template() {
                    view_variants.push(variant);
                }
            } else if self.check_block_header(BlockType::Requires) {
                // Requires blocks contain dependency metadata, not type definitions.
                // Skip to next block header.
                self.advance(); // consume ## Requires
                while self.pos < self.tokens.len() {
                    if matches!(
                        self.tokens.get(self.pos),
                        Some(Token {
                            kind: TokenType::BlockHeader { .. },
                            ..
                        })
                    ) {
                        break;
                    }
                    self.advance();
                }
            } else {
                self.advance();
            }
        }

        if !view_variants.is_empty() {
            let none = self
                .interner
                .intern(crate::parser::lgx_lower::empty_view_variant_name());
            view_variants.insert(
                0,
                VariantDef {
                    name: none,
                    fields: Vec::new(),
                },
            );
            type_registry.register(
                self.interner
                    .intern(crate::parser::lgx_lower::view_type_name()),
                TypeDef::Enum {
                    variants: view_variants,
                    generics: Vec::new(),
                    is_portable: false,
                    is_shared: false,
                },
            );
        }

        DiscoveryResult {
            types: type_registry,
            policies: policy_registry,
        }
    }

    /// `## To f (p: T) and (q: U) -> View:` with a markup body — the variant `__View_f` carrying
    /// one field per parameter, a `View` parameter as the enum itself. Read the way the parser
    /// reads a function header (name, `(name: Type)` groups joined by `and` or a preposition or
    /// separated by commas, `-> View`, `:`); anything else is not a template and declares nothing.
    /// The lowering refuses, by the template's name, any template this reading disagrees with.
    fn scan_view_template(&mut self) -> Option<VariantDef> {
        let name = self.header_word()?;
        let mut fields = Vec::new();
        while self.check_lparen() {
            self.advance();
            if self.check_rparen() {
                self.advance();
                break;
            }
            loop {
                let param = self.header_word()?;
                if !self.check_colon() {
                    return None;
                }
                self.advance();
                let ty = match self.consume_field_type() {
                    FieldType::Primitive(t) | FieldType::Named(t)
                        if self.interner.resolve(t) == "View" =>
                    {
                        FieldType::Named(
                            self.interner
                                .intern(crate::parser::lgx_lower::view_type_name()),
                        )
                    }
                    other => other,
                };
                fields.push(FieldDef {
                    name: param,
                    ty,
                    is_public: true,
                });
                if self.check_comma() {
                    self.advance();
                    continue;
                }
                break;
            }
            if !self.check_rparen() {
                return None;
            }
            self.advance();
            if self.check_and()
                || matches!(
                    self.peek(),
                    Some(Token {
                        kind: TokenType::Preposition(_) | TokenType::From,
                        ..
                    })
                )
            {
                self.advance();
            }
        }
        if !matches!(
            self.peek(),
            Some(Token {
                kind: TokenType::Arrow,
                ..
            })
        ) {
            return None;
        }
        self.advance();
        if !self.check_word("View") {
            return None;
        }
        self.advance();
        if !self.check_colon() {
            return None;
        }
        self.advance();
        let markup = matches!(
            self.peek(),
            Some(Token {
                kind: TokenType::Indent,
                ..
            })
        ) && matches!(
            self.tokens.get(self.pos + 1),
            Some(Token {
                kind: TokenType::RawBlock {
                    kind: crate::token::RawKind::View,
                    ..
                },
                ..
            })
        );
        if !markup {
            return None;
        }
        let variant = crate::parser::lgx_lower::view_variant_name(self.interner.resolve(name));
        Some(VariantDef {
            name: self.interner.intern(&variant),
            fields,
        })
    }

    /// A name in a function header, as the parser's `expect_identifier` reads it: a word's own
    /// symbol, or its lexeme.
    fn header_word(&mut self) -> Option<Symbol> {
        let token = self.peek()?;
        let sym = match &token.kind {
            TokenType::Noun(s) | TokenType::ProperName(s) | TokenType::Adjective(s) => *s,
            TokenType::Colon
            | TokenType::LParen
            | TokenType::RParen
            | TokenType::Arrow
            | TokenType::Newline
            | TokenType::Indent
            | TokenType::Dedent
            | TokenType::EOF
            | TokenType::BlockHeader { .. } => return None,
            _ => token.lexeme,
        };
        self.advance();
        Some(sym)
    }

    /// `## Session` — the fields a visitor carries between requests, registered as `Session`.
    ///
    /// The field syntax is a record's, so it is read by a record's own field scanner rather than
    /// a second one written beside it: two scanners for one syntax is how the two drift.
    fn scan_session_block(&mut self, registry: &mut TypeRegistry) {
        let name = self.interner.intern("Session");
        let mut spans = Vec::new();
        let mut fields = self.parse_struct_fields_with_params(&[], Some(&mut spans));
        if !fields.is_empty() {
            // ★ EVERY SESSION CARRIES A CSRF NONCE, declared or not. It is added HERE, with the
            // author's own fields, so it rides in the same cookie and through the same payload
            // encoder — there is no second place for it to be forgotten. See
            // `SESSION_NONCE_FIELD` for why the token is bound to a nonce rather than to the
            // session's contents.
            fields.push(crate::analysis::registry::FieldDef {
                name: self
                    .interner
                    .intern(crate::parser::lgx_lower::SESSION_NONCE_FIELD),
                ty: crate::analysis::registry::FieldType::Primitive(self.interner.intern("Text")),
                is_public: true,
            });
            for (field, span) in spans {
                registry.set_field_span(name, field, span);
            }
            registry.register(
                name,
                TypeDef::Struct {
                    fields,
                    generics: Vec::new(),
                    is_portable: false,
                    is_shared: false,
                },
            );
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
            if matches!(
                self.peek(),
                Some(Token {
                    kind: TokenType::BlockHeader { .. },
                    ..
                })
            ) {
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
            if matches!(
                self.peek(),
                Some(Token {
                    kind: TokenType::BlockHeader { .. },
                    ..
                })
            ) {
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
            let condition = self.parse_policy_condition();

            registry.register_predicate(PredicateDef {
                subject_type,
                predicate_name,
                condition,
            });

            self.skip_to_period();
        } else if self.check_word("can") {
            // "A User can publish the Document if..."
            self.advance(); // consume "can"

            // Get action name (e.g., "publish", "edit"). For a verb action take its
            // LEMMA — that is exactly what the `Check ... can <verb>` site resolves
            // (parser), so the registered and looked-up symbols agree. Using
            // `consume_noun_or_proper` first would capture the verb LEXEME instead and
            // every capability check would miss ("No capability 'Edit' defined").
            let action = match self.peek() {
                Some(Token {
                    kind: TokenType::Verb { lemma, .. },
                    ..
                }) => {
                    let sym = *lemma;
                    self.advance();
                    sym
                }
                _ => match self.consume_noun_or_proper() {
                    Some(sym) => sym,
                    None => return,
                },
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

            let condition = self.parse_policy_condition();

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
    fn parse_policy_condition(&mut self) -> PolicyCondition {
        // Left-fold the FULL n-ary chain. Returning after the first connective
        // silently dropped every conjunct/disjunct past the second atom, making
        // an AND-policy over-permissive and an OR-policy over-restrictive.
        let mut acc = self.parse_atomic_condition();

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
                let right = self.parse_atomic_condition();
                acc = PolicyCondition::And(Box::new(acc), Box::new(right));
            } else if self.check_word("OR") {
                self.advance();
                // Skip newlines after OR
                while self.check_newline() {
                    self.advance();
                }
                let right = self.parse_atomic_condition();
                acc = PolicyCondition::Or(Box::new(acc), Box::new(right));
            } else {
                break;
            }
        }

        acc
    }

    /// Phase 50: Parse an atomic condition
    fn parse_atomic_condition(&mut self) -> PolicyCondition {
        // Skip "The" article if present
        if self.check_article() {
            self.advance();
        }

        // Get the subject reference (e.g., "user" or "user's role")
        let subject_ref = match self.consume_noun_or_proper() {
            Some(sym) => sym,
            None => {
                return PolicyCondition::FieldEquals {
                    field: self.interner.intern("unknown"),
                    value: self.interner.intern("unknown"),
                    is_string_literal: false,
                }
            }
        };

        // Check if it's a field access ("'s role") or a predicate ("is admin")
        if self.check_possessive() {
            self.advance(); // consume "'s"

            // Get field name
            let field = match self.consume_noun_or_proper() {
                Some(sym) => sym,
                None => {
                    return PolicyCondition::FieldEquals {
                        field: self.interner.intern("unknown"),
                        value: self.interner.intern("unknown"),
                        is_string_literal: false,
                    }
                }
            };

            // Expect "equals"
            if self.check_word("equals") {
                self.advance();

                // First try an object-field RHS: `the document's owner`, giving a
                // cross-field comparison `self.<field> == object.<obj_field>`. Only a
                // literal/identifier RHS falls through to `FieldEquals`.
                let checkpoint = self.pos;
                if self.check_article() {
                    self.advance();
                }
                if let Some(obj_ref) = self.consume_noun_or_proper() {
                    if self.check_possessive() {
                        self.advance(); // consume "'s"
                        if let Some(obj_field) = self.consume_noun_or_proper() {
                            return PolicyCondition::SubjectFieldEqualsObjectField {
                                subject_field: field,
                                object: obj_ref,
                                object_field: obj_field,
                            };
                        }
                    }
                }
                // Not an object-field reference — rewind and take a plain value.
                self.pos = checkpoint;
                let (value, is_string_literal) = self.consume_value();

                return PolicyCondition::FieldEquals {
                    field,
                    value,
                    is_string_literal,
                };
            }
        } else if self.check_copula() {
            // "user is admin"
            self.advance(); // consume "is"

            // Get predicate name
            let predicate = match self.consume_noun_or_proper() {
                Some(sym) => sym,
                None => {
                    return PolicyCondition::FieldEquals {
                        field: self.interner.intern("unknown"),
                        value: self.interner.intern("unknown"),
                        is_string_literal: false,
                    }
                }
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
        if let Some(Token {
            kind: TokenType::StringLiteral(sym),
            ..
        }) = self.peek()
        {
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
        matches!(
            self.peek(),
            Some(Token {
                kind: TokenType::Possessive,
                ..
            })
        )
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
            if matches!(
                self.peek(),
                Some(Token {
                    kind: TokenType::BlockHeader { .. },
                    ..
                })
            ) {
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
                        let fields = self.parse_struct_fields_with_params(&type_params, None);
                        registry.register(
                            name_sym,
                            TypeDef::Struct {
                                fields,
                                generics: type_params,
                                is_portable,
                                is_shared,
                            },
                        );
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
                            registry.register(
                                name_sym,
                                TypeDef::Enum {
                                    variants,
                                    generics: type_params,
                                    is_portable,
                                    is_shared,
                                },
                            );
                            return;
                        }
                    }
                }

                if self.check_article() {
                    self.advance(); // consume a/an

                    // Look for type indicators
                    if self.check_word("generic") {
                        // `A Map of [K] and [V] is a generic collection.` declares its
                        // arity in the `of [..]` list — honor it (2 here) so the parser
                        // reads `Map of Int to Int` as two params. A bare `A Stack is a
                        // generic collection.` names no params and stays single-arity.
                        let param_count = if type_params.is_empty() {
                            1
                        } else {
                            type_params.len()
                        };
                        registry.register(name_sym, TypeDef::Generic { param_count });
                        self.skip_to_period();
                    } else if self.check_word("record")
                        || self.check_word("struct")
                        || self.check_word("structure")
                    {
                        registry.register(
                            name_sym,
                            TypeDef::Struct {
                                fields: vec![],
                                generics: vec![],
                                is_portable: false,
                                is_shared: false,
                            },
                        );
                        self.skip_to_period();
                    } else if self.check_word("sum")
                        || self.check_word("enum")
                        || self.check_word("choice")
                    {
                        registry.register(
                            name_sym,
                            TypeDef::Enum {
                                variants: vec![],
                                generics: vec![],
                                is_portable: false,
                                is_shared: false,
                            },
                        );
                        self.skip_to_period();
                    }
                }
            } else if !type_params.is_empty() {
                // "A Stack of [Things] is..." - old generic syntax, still supported
                registry.register(
                    name_sym,
                    TypeDef::Generic {
                        param_count: type_params.len(),
                    },
                );
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
            if matches!(
                self.peek(),
                Some(Token {
                    kind: TokenType::BlockHeader { .. },
                    ..
                })
            ) {
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

            // Try to parse variant name (noun or proper name) — registered by
            // RAW spelling: under the dynamic lexicon a capitalized variant
            // like `Ok` classifies as a common word whose canonical symbol is
            // the lowercased lemma, and a lemma-keyed registry would never
            // match the `When Ok (v):` arm's raw-lexeme lookup.
            let raw_lexeme = self.peek().map(|t| t.lexeme);
            if self.consume_noun_or_proper().is_some() {
                let variant_name = raw_lexeme.expect("peeked before consuming");
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

    /// Parse variant fields in natural syntax.
    /// Supports multiple syntaxes:
    /// - "with a radius, which is Int." (verbose natural)
    /// - "with radius Int" (concise natural - no article/comma)
    fn parse_variant_fields_natural_with_params(
        &mut self,
        type_params: &[Symbol],
    ) -> Vec<FieldDef> {
        let mut fields = Vec::new();

        // "with" has already been detected, consume it
        self.advance();

        loop {
            if self.check_article() && !self.article_is_variant_field_name(type_params) {
                self.advance();
            }

            if let Some(field_name) = self.consume_field_name() {
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

    /// Parse variant fields in concise syntax: "(radius: Int)" or "(width: Int, height: Int)"
    fn parse_variant_fields_concise_with_params(
        &mut self,
        type_params: &[Symbol],
    ) -> Vec<FieldDef> {
        let mut fields = Vec::new();

        // Consume "("
        self.advance();

        loop {
            // The word before `:` is the field's name.
            let field_name = if self.is_name_separator_at(self.pos + 1) {
                self.consume_field_name()
            } else {
                self.consume_noun_or_proper()
            };
            if let Some(field_name) = field_name {
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

    /// Parse struct fields in "has:" block
    /// Each field: "a [public] name, which is Type."
    ///
    /// `spans`, when given, collects each field's own name token — a check that must report AT a
    /// particular field (`## Session`'s field-kind refusal) reads it back from
    /// `TypeRegistry::field_span`; an ordinary struct has no such check and passes `None`.
    fn parse_struct_fields_with_params(
        &mut self,
        type_params: &[Symbol],
        mut spans: Option<&mut Vec<(Symbol, Span)>>,
    ) -> Vec<FieldDef> {
        let mut fields = Vec::new();

        while self.pos < self.tokens.len() {
            // Exit on dedent or next block
            if self.check_dedent() {
                self.advance();
                break;
            }
            if matches!(
                self.peek(),
                Some(Token {
                    kind: TokenType::BlockHeader { .. },
                    ..
                })
            ) {
                break;
            }

            // Skip newlines between fields
            if self.check_newline() {
                self.advance();
                continue;
            }

            // Parse field: "a [public] name, which is Type." or "name: Type." (no article). An
            // article directly before the separator is the field's name (`a: Int.`).
            let has_article = self.check_article() && !self.is_name_separator_at(self.pos + 1);
            if has_article {
                self.advance(); // consume "a"/"an"
            }

            // Check for "public" modifier — unless `public` is the field's own name (`public: Bool.`)
            let has_public_keyword =
                if self.check_word("public") && !self.is_name_separator_at(self.pos + 1) {
                    self.advance();
                    true
                } else {
                    false
                };
            // Visibility determined later based on syntax used
            let mut is_public = has_public_keyword;

            // A field's name is known by its position (`consume_field_name`): after an article, or
            // directly before the `:` of `name: Type` / the `,` of `name, which is Type`.
            let name_span = self.peek().map(|t| t.span).unwrap_or_default();
            let names_by_position = has_article || self.is_name_separator_at(self.pos + 1);
            let field_name = if names_by_position {
                self.consume_field_name()
            } else {
                self.consume_noun_or_proper()
            };
            if let Some(field_name) = field_name {
                // Three syntaxes, each public by default:
                // 1. "name: Type." (concise)
                // 2. "name, which is Type." (natural)
                // 3. "a name (Type)." (parenthesized)
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
                } else if has_article && self.check_lparen() {
                    is_public = true;
                    self.consume_field_type_with_params(type_params)
                } else if !has_article {
                    // No colon and no article - this wasn't a field, skip
                    continue;
                } else {
                    // Fallback: unknown type
                    FieldType::Primitive(self.interner.intern("Unknown"))
                };

                if let Some(spans) = spans.as_deref_mut() {
                    spans.push((field_name, name_span));
                }
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

    /// Parse a field type reference
    fn consume_field_type(&mut self) -> FieldType {
        if self.check_lparen() {
            return self.consume_parenthesized_field_type(&[]);
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
                return FieldType::Generic {
                    base: final_name,
                    params: vec![key_type, value_type],
                };
            }

            // Check for generic: "List of Int", "Seq of Text", "Map of K to V"
            if self.check_preposition("of") {
                // How many parameters this generic takes, from the ONE table the type-expression
                // parser reads — so `Result of Int and E` binds BOTH here, as it does everywhere
                // else. A field whose declared type silently loses a parameter takes its whole
                // record out of the layout tables, and nothing says so.
                let arity = crate::analysis::registry::builtin_generic_param_count(final_name_str)
                    .unwrap_or(1);

                self.advance();
                let mut params = Vec::with_capacity(arity);
                params.push(self.consume_field_type());
                // Later parameters follow a separator — `and` (`Result of T and E`), `to`
                // (`Map of K to V`) or a comma. A generic written with fewer than its arity keeps
                // what was actually given rather than consuming the sentence past its end.
                while params.len() < arity {
                    if self.check_and() || self.check_to() || self.check_comma() {
                        self.advance();
                        params.push(self.consume_field_type());
                    } else {
                        break;
                    }
                }
                return FieldType::Generic {
                    base: final_name,
                    params,
                };
            }

            // Phase 49b: "Divergent T" syntax (no "of" required)
            if final_name_str == "Divergent" {
                // Next token should be the inner type
                let param = self.consume_field_type();
                return FieldType::Generic {
                    base: final_name,
                    params: vec![param],
                };
            }

            // Check if primitive
            match final_name_str {
                "Int" | "Nat" | "Text" | "Bool" | "Real" | "Unit" | "Word8" | "Word16"
                | "Word32" | "Word64" | "Lanes8Word32" | "Lanes4Word32" | "Lanes16Word8"
                | "Lanes4Word64" | "Lanes16Word16" => FieldType::Primitive(final_name),
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
            Some(Token {
                kind: TokenType::Article(_),
                ..
            }) => true,
            // Also accept ProperName("A") / ProperName("An") which can occur at line starts
            Some(Token {
                kind: TokenType::ProperName(sym),
                ..
            }) => {
                let text = self.interner.resolve(*sym);
                text.eq_ignore_ascii_case("a") || text.eq_ignore_ascii_case("an")
            }
            _ => false,
        }
    }

    fn check_copula(&self) -> bool {
        match self.peek() {
            Some(Token {
                kind: TokenType::Is | TokenType::Are,
                ..
            }) => true,
            // Also match "is" when tokenized as a verb (common in declarative mode)
            Some(Token {
                kind: TokenType::Verb { lemma, .. },
                ..
            }) => {
                let word = self.interner.resolve(*lemma).to_lowercase();
                word == "is" || word == "are"
            }
            _ => false,
        }
    }

    fn check_preposition(&self, word: &str) -> bool {
        if let Some(Token {
            kind: TokenType::Preposition(sym),
            ..
        }) = self.peek()
        {
            self.interner.resolve(*sym) == word
        } else {
            false
        }
    }

    /// Take the word here as a field's name. A field's name is known by its POSITION, never by the
    /// part of speech the lexer gave the word: `first`/`second` are ordinals to it, `who` an
    /// interrogative, `a` an article, and each is an ordinary field name where a name stands. Any
    /// identifier-shaped word is accepted (the symbol [`Self::consume_noun_or_proper`] would give,
    /// where it accepts the word).
    fn consume_field_name(&mut self) -> Option<Symbol> {
        if let Some(sym) = self.consume_noun_or_proper() {
            return Some(sym);
        }
        let t = self.peek()?;
        let lexeme = self.interner.resolve(t.lexeme);
        let identifier = lexeme.chars().next().is_some_and(|c| c.is_alphabetic())
            && lexeme.chars().all(|c| c.is_alphanumeric() || c == '_');
        if !identifier {
            return None;
        }
        let sym = t.lexeme;
        self.advance();
        Some(sym)
    }

    /// Whether token `i` separates a field's name from its type: the `:` of `name: Type`, or the
    /// `,` of `name, which is Type` — never the `,` before `and` in a variant's field list.
    fn is_name_separator_at(&self, i: usize) -> bool {
        match self.tokens.get(i).map(|t| &t.kind) {
            Some(TokenType::Colon) => true,
            Some(TokenType::Comma) => !self
                .tokens
                .get(i + 1)
                .is_some_and(|t| self.interner.resolve(t.lexeme).eq_ignore_ascii_case("and")),
            _ => false,
        }
    }

    /// In a variant's `with` list, whether the article here is the field's own NAME (`with a Int
    /// and b Int`, `with a Seq of Int`) rather than the article before one (`with a radius Int`,
    /// `with a radius, which is Int`). A separator right after it makes it the name; one after the
    /// next word makes it an article. Otherwise both readings are tried against the type grammar:
    /// the article is the name only when skipping it leaves no field (a name, then a type ending at
    /// the list's boundary) and taking it as the name does.
    fn article_is_variant_field_name(&mut self, type_params: &[Symbol]) -> bool {
        let at = self.pos;
        if self.is_name_separator_at(at + 1) {
            return true;
        }
        if self.is_name_separator_at(at + 2) {
            return false;
        }
        !self.juxtaposed_field_parses_from(at + 1, type_params)
            && self.juxtaposed_field_parses_from(at, type_params)
    }

    /// Whether a field written `name Type` parses from token `start` and ends at the boundary of a
    /// variant's field list (`and`, `,`, `.`, `)`, or the line's end). The position is restored.
    fn juxtaposed_field_parses_from(&mut self, start: usize, type_params: &[Symbol]) -> bool {
        let saved = self.pos;
        self.pos = start;
        let parses = self.consume_field_name().is_some() && {
            let type_start = self.pos;
            let _ = self.consume_field_type_with_params(type_params);
            self.pos > type_start
                && (self.check_word("and")
                    || self.check_comma()
                    || self.check_period()
                    || self.check_rparen()
                    || self.check_newline()
                    || self.check_dedent()
                    || matches!(
                        self.peek(),
                        None | Some(Token {
                            kind: TokenType::EOF,
                            ..
                        })
                    ))
        };
        self.pos = saved;
        parses
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
            // ★ Accept a PRONOUN as a field name, by its lexeme. `an i, which is Int.` and
            // `an it, which is Option of Inner.` are ordinary declarations — `i` for an index and
            // `it` for an item are among the most natural short field names there are — and both
            // used to VANISH. With the article consumed and the name unaccepted, neither branch
            // of the field loop ran: the token was skipped on the next pass, the rest of the line
            // parsed as nothing, and the record simply did not have that field. No error, at
            // declaration OR at construction (`a new Row with it 4` set nothing), so the first
            // sign was a value missing from the far end of a wire.
            TokenType::Pronoun { .. } => {
                let sym = t.lexeme;
                self.advance();
                Some(sym)
            }
            // Temporal adverbs are names in a declaration too ("First", "next"), just as after
            // "a new"; dropping one silently hides its record layout from the static backends.
            TokenType::Adverb(_) | TokenType::TemporalAdverb(_) => {
                let sym = t.lexeme;
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
            _ => None,
        }
    }

    fn check_word(&self, word: &str) -> bool {
        if let Some(token) = self.peek() {
            // Check against the lexeme of the token
            self.interner
                .resolve(token.lexeme)
                .eq_ignore_ascii_case(word)
        } else {
            false
        }
    }

    fn skip_to_period(&mut self) {
        while self.pos < self.tokens.len() {
            if matches!(
                self.peek(),
                Some(Token {
                    kind: TokenType::Period,
                    ..
                })
            ) {
                self.advance();
                break;
            }
            self.advance();
        }
    }

    fn check_colon(&self) -> bool {
        matches!(
            self.peek(),
            Some(Token {
                kind: TokenType::Colon,
                ..
            })
        )
    }

    fn check_newline(&self) -> bool {
        matches!(
            self.peek(),
            Some(Token {
                kind: TokenType::Newline,
                ..
            })
        )
    }

    fn check_indent(&self) -> bool {
        matches!(
            self.peek(),
            Some(Token {
                kind: TokenType::Indent,
                ..
            })
        )
    }

    fn check_dedent(&self) -> bool {
        matches!(
            self.peek(),
            Some(Token {
                kind: TokenType::Dedent,
                ..
            })
        )
    }

    fn check_comma(&self) -> bool {
        matches!(
            self.peek(),
            Some(Token {
                kind: TokenType::Comma,
                ..
            })
        )
    }

    /// "and" — the separator between a multi-parameter generic's type arguments
    /// (`Result of T and E`, `Pair of A and B`, `Triple of A and B and C`).
    fn check_and(&self) -> bool {
        matches!(
            self.peek(),
            Some(Token {
                kind: TokenType::And,
                ..
            })
        )
    }

    fn check_period(&self) -> bool {
        matches!(
            self.peek(),
            Some(Token {
                kind: TokenType::Period,
                ..
            })
        )
    }

    fn check_either(&self) -> bool {
        matches!(
            self.peek(),
            Some(Token {
                kind: TokenType::Either,
                ..
            })
        )
    }

    fn check_lparen(&self) -> bool {
        matches!(
            self.peek(),
            Some(Token {
                kind: TokenType::LParen,
                ..
            })
        )
    }

    fn check_rparen(&self) -> bool {
        matches!(
            self.peek(),
            Some(Token {
                kind: TokenType::RParen,
                ..
            })
        )
    }

    /// Phase 49c: Check for AddWins token
    fn check_addwins(&self) -> bool {
        matches!(
            self.peek(),
            Some(Token {
                kind: TokenType::AddWins,
                ..
            })
        )
    }

    /// Phase 49c: Check for RemoveWins token
    fn check_removewins(&self) -> bool {
        matches!(
            self.peek(),
            Some(Token {
                kind: TokenType::RemoveWins,
                ..
            })
        )
    }

    /// Phase 49c: Check for YATA token
    fn check_yata(&self) -> bool {
        matches!(
            self.peek(),
            Some(Token {
                kind: TokenType::YATA,
                ..
            })
        )
    }

    /// Phase 49c: Check for "to" (either TokenType::To or preposition "to")
    fn check_to(&self) -> bool {
        match self.peek() {
            Some(Token {
                kind: TokenType::To,
                ..
            }) => true,
            Some(Token {
                kind: TokenType::Preposition(sym),
                ..
            }) => self.interner.resolve(*sym) == "to",
            _ => false,
        }
    }

    /// Phase 49c: Check for "from" (either TokenType::From or preposition "from")
    fn check_from(&self) -> bool {
        match self.peek() {
            Some(Token {
                kind: TokenType::From,
                ..
            }) => true,
            Some(Token {
                kind: TokenType::Preposition(sym),
                ..
            }) => self.interner.resolve(*sym) == "from",
            _ => false,
        }
    }

    /// Phase 47: Check for Portable token
    fn check_portable(&self) -> bool {
        matches!(
            self.peek(),
            Some(Token {
                kind: TokenType::Portable,
                ..
            })
        )
    }

    /// Phase 49: Check for Shared token
    fn check_shared(&self) -> bool {
        matches!(
            self.peek(),
            Some(Token {
                kind: TokenType::Shared,
                ..
            })
        )
    }

    // Phase 34: Bracket checks for type parameters
    fn check_lbracket(&self) -> bool {
        matches!(
            self.peek(),
            Some(Token {
                kind: TokenType::LBracket,
                ..
            })
        )
    }

    fn check_rbracket(&self) -> bool {
        matches!(
            self.peek(),
            Some(Token {
                kind: TokenType::RBracket,
                ..
            })
        )
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

    /// A parenthesized field type, the `(` next: one type in parentheses is that type (`Seq of (Seq
    /// of Int)`), a comma list a tuple (`(Int, Text)`), and `name: Type` entries a named tuple
    /// (`(quotient: Int, remainder: Int)`). An entry's name is known by its position, before `:`.
    fn consume_parenthesized_field_type(&mut self, type_params: &[Symbol]) -> FieldType {
        self.advance(); // consume "("
        let mut entries: Vec<(Option<Symbol>, FieldType)> = Vec::new();
        loop {
            let named = matches!(
                self.tokens.get(self.pos + 1),
                Some(Token {
                    kind: TokenType::Colon,
                    ..
                })
            ) && !self.check_lparen();
            let name = if named {
                let name = self.peek().map(|t| t.lexeme);
                self.advance(); // the entry's name
                self.advance(); // consume ":"
                name
            } else {
                None
            };
            let ty = self.consume_field_type_with_params(type_params);
            entries.push((name, ty));
            if self.check_comma() {
                self.advance();
                continue;
            }
            break;
        }
        if self.check_rparen() {
            self.advance(); // consume ")"
        }
        if entries.iter().all(|(name, _)| name.is_some()) {
            return FieldType::Record(
                entries
                    .into_iter()
                    .map(|(n, t)| (n.unwrap_or_default(), t))
                    .collect(),
            );
        }
        let mut types: Vec<FieldType> = entries.into_iter().map(|(_, t)| t).collect();
        if types.len() == 1 {
            return types
                .pop()
                .unwrap_or(FieldType::Primitive(self.interner.intern("Unknown")));
        }
        FieldType::Tuple(types)
    }

    /// Phase 34: Parse a field type reference, recognizing type parameters
    fn consume_field_type_with_params(&mut self, type_params: &[Symbol]) -> FieldType {
        if self.check_lparen() {
            return self.consume_parenthesized_field_type(type_params);
        }

        // Phase 34: Single-letter type params like "A" may be tokenized as Article
        // Check for Article that matches a type param first
        if let Some(Token {
            kind: TokenType::Article(_),
            lexeme,
            ..
        }) = self.peek()
        {
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
                return FieldType::Generic {
                    base: final_name,
                    params: vec![key_type, value_type],
                };
            }

            // Check for generic: "List of Int", "Seq of Text", "List of T", "Map of K to V"
            if self.check_preposition("of") {
                // How many parameters this generic takes, from the ONE table the type-expression
                // parser reads — so `Result of Int and E` binds BOTH here, as it does everywhere
                // else. A field whose declared type silently loses a parameter takes its whole
                // record out of the layout tables, and nothing says so.
                let arity = crate::analysis::registry::builtin_generic_param_count(final_name_str)
                    .unwrap_or(1);

                self.advance();
                let mut params = Vec::with_capacity(arity);
                params.push(self.consume_field_type_with_params(type_params));
                // Later parameters follow a separator — `and` (`Result of T and E`), `to`
                // (`Map of K to V`) or a comma. A generic written with fewer than its arity keeps
                // what was actually given rather than consuming the sentence past its end.
                while params.len() < arity {
                    if self.check_and() || self.check_to() || self.check_comma() {
                        self.advance();
                        params.push(self.consume_field_type_with_params(type_params));
                    } else {
                        break;
                    }
                }
                return FieldType::Generic {
                    base: final_name,
                    params,
                };
            }

            // Phase 49b: "Divergent T" syntax (no "of" required)
            if final_name_str == "Divergent" {
                // Next token should be the inner type
                let param = self.consume_field_type_with_params(type_params);
                return FieldType::Generic {
                    base: final_name,
                    params: vec![param],
                };
            }

            // Check if primitive
            match final_name_str {
                "Int" | "Nat" | "Text" | "Bool" | "Real" | "Unit" | "Word8" | "Word16"
                | "Word32" | "Word64" | "Lanes8Word32" | "Lanes4Word32" | "Lanes16Word8"
                | "Lanes4Word64" | "Lanes16Word16" => FieldType::Primitive(final_name),
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
#[path = "../../../logicaffeine_tests/unit/logicaffeine_language/analysis/discovery/tests.rs"]
mod tests;
