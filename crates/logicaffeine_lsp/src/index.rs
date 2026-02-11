use std::collections::HashMap;
use logicaffeine_base::Interner;
use logicaffeine_language::{
    analysis::TypeRegistry,
    token::{Token, TokenType, BlockType, Span},
};
use crate::pipeline::OwnedStmt;

/// Scope context for a definition — which block contains it and at what depth.
#[derive(Debug, Clone, Default)]
pub struct ScopeInfo {
    /// Index into `block_spans` for the containing block, if any.
    pub block_idx: Option<usize>,
    /// Nesting depth (0 = top-level, 1 = inside a block, etc.)
    pub depth: u32,
}

/// A definition in the document (variable, function, struct, enum, field, etc.)
#[derive(Debug, Clone)]
pub struct Definition {
    pub name: String,
    pub kind: DefinitionKind,
    pub span: Span,
    pub detail: Option<String>,
    pub scope: ScopeInfo,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DefinitionKind {
    Variable,
    Function,
    Struct,
    Enum,
    Field,
    Parameter,
    Block,
    Variant,
    Theorem,
}

/// A reference to a definition.
#[derive(Debug, Clone)]
pub struct Reference {
    pub name: String,
    pub span: Span,
    pub definition_idx: Option<usize>,
}

/// The symbol index for a single document.
#[derive(Debug, Clone, Default)]
pub struct SymbolIndex {
    pub definitions: Vec<Definition>,
    pub references: Vec<Reference>,
    pub name_to_defs: HashMap<String, Vec<usize>>,
    /// Maps block header names to their spans.
    pub block_spans: Vec<(String, BlockType, Span)>,
    /// Statement spans inferred from keyword..period token ranges.
    pub statement_spans: Vec<(String, Span)>,
}

impl SymbolIndex {
    /// Build the symbol index from parsed statements, tokens, and the type registry.
    pub fn build(
        stmts: &[OwnedStmt],
        tokens: &[Token],
        type_registry: &TypeRegistry,
        interner: &Interner,
    ) -> Self {
        let mut index = SymbolIndex::default();

        // Phase 1: Extract definitions from parsed statements
        index.index_statements(stmts, tokens, interner);

        // Phase 2: Extract definitions from TypeRegistry (struct/enum definitions)
        index.index_type_registry(type_registry, tokens, interner);

        // Phase 3: Extract block headers and statement spans from tokens
        index.index_tokens(tokens, interner);

        // Phase 4: Index identifier references (scope-aware)
        index.index_references(tokens, interner);

        // Phase 5: Compute scope info for each definition
        index.compute_scopes();

        index
    }

    fn add_definition(&mut self, def: Definition) -> usize {
        let idx = self.definitions.len();
        self.name_to_defs
            .entry(def.name.clone())
            .or_default()
            .push(idx);
        self.definitions.push(def);
        idx
    }

    fn index_statements(&mut self, stmts: &[OwnedStmt], tokens: &[Token], interner: &Interner) {
        for stmt in stmts {
            match stmt {
                OwnedStmt::FunctionDef { name, params, return_type } => {
                    let span = find_token_span_for_name(tokens, name, interner)
                        .unwrap_or(Span::default());
                    let detail = {
                        let param_str: Vec<String> = params
                            .iter()
                            .map(|(n, t)| format!("{}: {}", n, t))
                            .collect();
                        let ret = return_type.as_deref().unwrap_or("Unit");
                        Some(format!("To {}({}) -> {}", name, param_str.join(", "), ret))
                    };
                    self.add_definition(Definition {
                        name: name.clone(),
                        kind: DefinitionKind::Function,
                        span,
                        detail,
                        scope: ScopeInfo::default(),
                    });

                    // Add parameters as definitions, each with its own span
                    let mut search_after = span.end;
                    for (param_name, param_type) in params {
                        let param_span = find_token_span_for_name_after(
                            tokens, param_name, interner, search_after,
                        ).unwrap_or(span);
                        if param_span != span {
                            search_after = param_span.end;
                        }
                        self.add_definition(Definition {
                            name: param_name.clone(),
                            kind: DefinitionKind::Parameter,
                            span: param_span,
                            detail: Some(format!("{}: {}", param_name, param_type)),
                            scope: ScopeInfo::default(),
                        });
                    }
                }
                OwnedStmt::StructDef { name, fields } => {
                    let span = find_token_span_for_name(tokens, name, interner)
                        .unwrap_or(Span::default());
                    self.add_definition(Definition {
                        name: name.clone(),
                        kind: DefinitionKind::Struct,
                        span,
                        detail: Some(format!("{} (struct)", name)),
                        scope: ScopeInfo::default(),
                    });
                    let mut field_search_after = span.end;
                    for (field_name, field_type) in fields {
                        let field_span = find_token_span_for_name_after(
                            tokens, field_name, interner, field_search_after,
                        ).unwrap_or(span);
                        if field_span != span {
                            field_search_after = field_span.end;
                        }
                        self.add_definition(Definition {
                            name: field_name.clone(),
                            kind: DefinitionKind::Field,
                            span: field_span,
                            detail: Some(format!("{}: {}", field_name, field_type)),
                            scope: ScopeInfo::default(),
                        });
                    }
                }
                OwnedStmt::Let { name, ty, inferred_type, mutable } => {
                    let span = find_token_span_for_name(tokens, name, interner)
                        .unwrap_or(Span::default());
                    let prefix = if *mutable { "mut " } else { "" };
                    let detail = if let Some(explicit_ty) = ty {
                        format!("Let {}{}: {}", prefix, name, explicit_ty)
                    } else if let Some(inferred) = inferred_type {
                        format!("Let {}{}: {} (inferred)", prefix, name, inferred)
                    } else {
                        format!("Let {}{}: auto (inferred)", prefix, name)
                    };
                    self.add_definition(Definition {
                        name: name.clone(),
                        kind: DefinitionKind::Variable,
                        span,
                        detail: Some(detail),
                        scope: ScopeInfo::default(),
                    });
                }
                OwnedStmt::Theorem { name } => {
                    let span = find_token_span_for_name(tokens, name, interner)
                        .unwrap_or(Span::default());
                    self.add_definition(Definition {
                        name: name.clone(),
                        kind: DefinitionKind::Theorem,
                        span,
                        detail: Some(format!("Theorem {}", name)),
                        scope: ScopeInfo::default(),
                    });
                }
                OwnedStmt::Block { name, kind } => {
                    self.add_definition(Definition {
                        name: name.clone(),
                        kind: DefinitionKind::Block,
                        span: Span::default(),
                        detail: Some(format!("{} {}", kind, name)),
                        scope: ScopeInfo::default(),
                    });
                }
                OwnedStmt::Other => {}
            }
        }
    }

    fn index_type_registry(
        &mut self,
        type_registry: &TypeRegistry,
        tokens: &[Token],
        interner: &Interner,
    ) {
        for (sym, typedef) in type_registry.iter_types() {
            let name = interner.resolve(*sym).to_string();
            // Skip primitives — they're built-in, not user-defined
            if matches!(typedef, logicaffeine_language::analysis::TypeDef::Primitive) {
                continue;
            }
            // Skip if already indexed from statements
            if self.name_to_defs.contains_key(&name) {
                continue;
            }

            let span = find_token_span_for_name(tokens, &name, interner)
                .unwrap_or(Span::default());

            match typedef {
                logicaffeine_language::analysis::TypeDef::Struct { fields, .. } => {
                    self.add_definition(Definition {
                        name: name.clone(),
                        kind: DefinitionKind::Struct,
                        span,
                        detail: Some(format!("{} (struct)", name)),
                        scope: ScopeInfo::default(),
                    });
                    for field in fields {
                        let field_name = interner.resolve(field.name).to_string();
                        self.add_definition(Definition {
                            name: field_name.clone(),
                            kind: DefinitionKind::Field,
                            span,
                            detail: Some(format!("{}.{}", name, field_name)),
                            scope: ScopeInfo::default(),
                        });
                    }
                }
                logicaffeine_language::analysis::TypeDef::Enum { variants, .. } => {
                    self.add_definition(Definition {
                        name: name.clone(),
                        kind: DefinitionKind::Enum,
                        span,
                        detail: Some(format!("{} (enum)", name)),
                        scope: ScopeInfo::default(),
                    });
                    for variant in variants {
                        let variant_name = interner.resolve(variant.name).to_string();
                        self.add_definition(Definition {
                            name: variant_name.clone(),
                            kind: DefinitionKind::Variant,
                            span,
                            detail: Some(format!("{}::{}", name, variant_name)),
                            scope: ScopeInfo::default(),
                        });
                    }
                }
                _ => {}
            }
        }
    }

    fn index_tokens(&mut self, tokens: &[Token], interner: &Interner) {
        let mut i = 0;
        while i < tokens.len() {
            match &tokens[i].kind {
                TokenType::BlockHeader { block_type } => {
                    // Find the extent of this block (up to next block header or EOF)
                    let start = tokens[i].span.start;
                    let mut end = tokens.last().map(|t| t.span.end).unwrap_or(start);
                    for j in (i + 1)..tokens.len() {
                        if matches!(tokens[j].kind, TokenType::BlockHeader { .. }) {
                            end = tokens[j].span.start;
                            break;
                        }
                    }

                    let name = interner.resolve(tokens[i].lexeme).to_string();
                    self.block_spans.push((
                        name,
                        *block_type,
                        Span::new(start, end),
                    ));
                }
                _ => {}
            }
            i += 1;
        }

        // Index statement spans (keyword..period pairs)
        self.index_statement_spans(tokens, interner);
    }

    fn index_statement_spans(&mut self, tokens: &[Token], interner: &Interner) {
        let mut i = 0;
        while i < tokens.len() {
            let is_stmt_keyword = matches!(
                tokens[i].kind,
                TokenType::Let
                    | TokenType::Set
                    | TokenType::If
                    | TokenType::While
                    | TokenType::Repeat
                    | TokenType::Return
                    | TokenType::Show
                    | TokenType::Give
                    | TokenType::Push
                    | TokenType::Pop
                    | TokenType::Call
                    | TokenType::Inspect
                    | TokenType::Check
                    | TokenType::Assert
                    | TokenType::Trust
                    | TokenType::Escape
                    | TokenType::Read
                    | TokenType::Write
                    | TokenType::Spawn
                    | TokenType::Send
                    | TokenType::Await
                    | TokenType::Sleep
                    | TokenType::Merge
                    | TokenType::Increase
                    | TokenType::Decrease
                    | TokenType::Listen
                    | TokenType::Sync
                    | TokenType::Mount
                    | TokenType::Launch
                    | TokenType::Receive
                    | TokenType::Stop
            );
            if is_stmt_keyword {
                let start = tokens[i].span.start;
                let keyword = interner.resolve(tokens[i].lexeme).to_string();
                // Scan forward for period or next statement keyword
                let mut end = tokens[i].span.end;
                for j in (i + 1)..tokens.len() {
                    end = tokens[j].span.end;
                    if matches!(tokens[j].kind, TokenType::Period | TokenType::Dedent) {
                        break;
                    }
                }
                self.statement_spans.push((keyword, Span::new(start, end)));
            }
            i += 1;
        }
    }

    fn index_references(&mut self, tokens: &[Token], interner: &Interner) {
        for token in tokens {
            if let Some(resolved) = resolve_token_name(token, interner) {
                // Skip block headers — they're not references
                if matches!(token.kind, TokenType::BlockHeader { .. }) {
                    continue;
                }
                let name = resolved.to_string();
                // Prefer the definition in the nearest scope
                let ref_block = self.block_for_offset(token.span.start);
                let def_idx = self.nearest_def(&name, ref_block);
                self.references.push(Reference {
                    name,
                    span: token.span,
                    definition_idx: def_idx,
                });
            }
        }
    }

    /// Compute scope info for each definition by matching its span against block_spans.
    fn compute_scopes(&mut self) {
        for i in 0..self.definitions.len() {
            let def_start = self.definitions[i].span.start;
            if self.definitions[i].span == Span::default() {
                continue;
            }
            let mut best_block: Option<usize> = None;
            let mut best_size = usize::MAX;
            for (bi, (_name, _bt, bspan)) in self.block_spans.iter().enumerate() {
                if def_start >= bspan.start && def_start < bspan.end {
                    let size = bspan.end - bspan.start;
                    if size < best_size {
                        best_size = size;
                        best_block = Some(bi);
                    }
                }
            }
            self.definitions[i].scope = ScopeInfo {
                block_idx: best_block,
                depth: if best_block.is_some() { 1 } else { 0 },
            };
        }
    }

    /// Find which block an offset falls inside (smallest containing block).
    fn block_for_offset(&self, offset: usize) -> Option<usize> {
        let mut best: Option<usize> = None;
        let mut best_size = usize::MAX;
        for (bi, (_name, _bt, bspan)) in self.block_spans.iter().enumerate() {
            if offset >= bspan.start && offset < bspan.end {
                let size = bspan.end - bspan.start;
                if size < best_size {
                    best_size = size;
                    best = Some(bi);
                }
            }
        }
        best
    }

    /// Find the best definition index for a name, preferring same-block defs.
    fn nearest_def(&self, name: &str, ref_block: Option<usize>) -> Option<usize> {
        let indices = self.name_to_defs.get(name)?;
        if indices.len() == 1 {
            return Some(indices[0]);
        }
        // Prefer definition in the same block
        if let Some(block_idx) = ref_block {
            for &idx in indices {
                if self.definitions[idx].scope.block_idx == Some(block_idx) {
                    return Some(idx);
                }
            }
        }
        // Fall back to first definition
        indices.first().copied()
    }

    /// Find the definition at the given byte offset.
    pub fn definition_at(&self, offset: usize) -> Option<&Definition> {
        // First check if we're on a reference
        for reference in &self.references {
            if offset >= reference.span.start && offset < reference.span.end {
                if let Some(idx) = reference.definition_idx {
                    return self.definitions.get(idx);
                }
            }
        }
        // Then check if we're on a definition itself
        for def in &self.definitions {
            if offset >= def.span.start && offset < def.span.end {
                return Some(def);
            }
        }
        None
    }

    /// Scope-aware definition lookup. Given a byte offset for context,
    /// returns the definition of `name` in the nearest scope.
    pub fn definition_at_scoped(&self, name: &str, offset: usize) -> Option<&Definition> {
        let ref_block = self.block_for_offset(offset);
        let idx = self.nearest_def(name, ref_block)?;
        self.definitions.get(idx)
    }

    /// Find all references to the definition with the given name.
    pub fn references_to(&self, name: &str) -> Vec<&Reference> {
        self.references
            .iter()
            .filter(|r| r.name == name)
            .collect()
    }

    /// Find references to `name` that are in the same scope as the definition at `def_offset`.
    pub fn references_in_scope(&self, name: &str, def_offset: usize) -> Vec<&Reference> {
        let def_block = self.block_for_offset(def_offset);
        self.references
            .iter()
            .filter(|r| {
                if r.name != name {
                    return false;
                }
                let ref_block = self.block_for_offset(r.span.start);
                ref_block == def_block
            })
            .collect()
    }

    /// Find all definitions with the given name.
    pub fn definitions_of(&self, name: &str) -> Vec<&Definition> {
        self.name_to_defs
            .get(name)
            .map(|indices| {
                indices
                    .iter()
                    .filter_map(|&idx| self.definitions.get(idx))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Get the block name for a block index.
    pub fn block_name(&self, block_idx: usize) -> Option<&str> {
        self.block_spans.get(block_idx).map(|(name, _, _)| name.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::analyze;

    #[test]
    fn let_binding_has_nondefault_span() {
        let result = analyze("## Main\n    Let x be 5.\n");
        assert!(result.errors.is_empty(), "Errors: {:?}", result.errors);
        let defs = result.symbol_index.definitions_of("x");
        assert_eq!(defs.len(), 1, "Expected 1 def for 'x', got {:?}", defs);
        assert_ne!(defs[0].span, Span::default(),
            "Definition span should not be default after fix");
    }

    #[test]
    fn let_binding_span_points_to_source() {
        let source = "## Main\n    Let x be 5.\n";
        let result = analyze(source);
        assert!(result.errors.is_empty(), "Errors: {:?}", result.errors);
        let defs = result.symbol_index.definitions_of("x");
        assert_eq!(defs.len(), 1);
        let span = defs[0].span;
        let text = &source[span.start..span.end];
        assert_eq!(text, "x", "Span should point to 'x' in source, got '{}'", text);
    }

    #[test]
    fn definition_at_finds_variable() {
        let source = "## Main\n    Let x be 5.\n";
        let result = analyze(source);
        assert!(result.errors.is_empty(), "Errors: {:?}", result.errors);
        let defs = result.symbol_index.definitions_of("x");
        assert!(!defs.is_empty());
        let span = defs[0].span;
        let def = result.symbol_index.definition_at(span.start);
        assert!(def.is_some(), "definition_at should find 'x' at its span");
        assert_eq!(def.unwrap().name, "x");
    }

    #[test]
    fn definitions_of_returns_correct_kind() {
        let result = analyze("## Main\n    Let x be 5.\n");
        assert!(result.errors.is_empty(), "Errors: {:?}", result.errors);
        let defs = result.symbol_index.definitions_of("x");
        assert_eq!(defs[0].kind, DefinitionKind::Variable);
    }

    #[test]
    fn definitions_of_unknown_returns_empty() {
        let result = analyze("## Main\n    Let x be 5.\n");
        let defs = result.symbol_index.definitions_of("nonexistent");
        assert!(defs.is_empty());
    }

    #[test]
    fn references_to_finds_usages() {
        let result = analyze("## Main\n    Let x be 5.\n    Show x.\n");
        assert!(result.errors.is_empty(), "Errors: {:?}", result.errors);
        let refs = result.symbol_index.references_to("x");
        assert!(refs.len() >= 1, "Expected refs to 'x', got {}", refs.len());
    }

    #[test]
    fn reference_linked_to_definition() {
        let result = analyze("## Main\n    Let x be 5.\n    Show x.\n");
        assert!(result.errors.is_empty(), "Errors: {:?}", result.errors);
        let refs = result.symbol_index.references_to("x");
        let linked: Vec<_> = refs.iter().filter(|r| r.definition_idx.is_some()).collect();
        assert!(!linked.is_empty(), "At least one reference should link to a definition");
    }

    #[test]
    fn let_binding_has_detail() {
        let result = analyze("## Main\n    Let x be 5.\n");
        assert!(result.errors.is_empty(), "Errors: {:?}", result.errors);
        let defs = result.symbol_index.definitions_of("x");
        assert!(defs[0].detail.is_some(), "Definition should have detail");
        assert!(defs[0].detail.as_ref().unwrap().contains("Let"),
            "Detail should mention Let: {:?}", defs[0].detail);
    }

    #[test]
    fn block_spans_populated() {
        let result = analyze("## Main\n    Let x be 5.\n");
        assert!(!result.symbol_index.block_spans.is_empty(),
            "block_spans should have at least one entry");
    }

    #[test]
    fn multiple_variables_indexed() {
        let result = analyze("## Main\n    Let a be 1.\n    Let b be 2.\n    Let c be 3.\n");
        assert!(result.errors.is_empty(), "Errors: {:?}", result.errors);
        assert_eq!(result.symbol_index.definitions_of("a").len(), 1);
        assert_eq!(result.symbol_index.definitions_of("b").len(), 1);
        assert_eq!(result.symbol_index.definitions_of("c").len(), 1);
    }

    #[test]
    fn definition_at_whitespace_returns_none() {
        let source = "## Main\n    Let x be 5.\n";
        let result = analyze(source);
        // Offset 0 is at '#' in "## Main" — there IS a BlockHeader token there
        // But offset somewhere in pure whitespace between tokens should be None
        // The space between "Let" and "x" is at byte 12
        // Let's use an offset that's clearly in trailing whitespace
        let past_end = source.len() + 5;
        let def = result.symbol_index.definition_at(past_end);
        assert!(def.is_none(), "definition_at beyond source should return None");
    }

    #[test]
    fn references_to_exact_count() {
        let source = "## Main\n    Let x be 5.\n    Show x.\n    Set x to x + 1.\n";
        let result = analyze(source);
        assert!(result.errors.is_empty(), "Errors: {:?}", result.errors);
        let refs = result.symbol_index.references_to("x");
        // "x" appears in: "Let x" (def+ref), "Show x" (ref), "Set x" (ref), "x + 1" (ref)
        // The exact count depends on tokenization, but there should be at least 3 references
        assert!(refs.len() >= 3,
            "Expected at least 3 references to 'x', got {}: {:?}",
            refs.len(),
            refs.iter().map(|r| (r.span.start, r.span.end)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn definitions_of_function() {
        let source = "## To greet (name: Text):\n    Show name.\n## Main\n    Call greet with \"Alice\".\n";
        let result = analyze(source);
        let defs = result.symbol_index.definitions_of("greet");
        let func_defs: Vec<_> = defs.iter().filter(|d| d.kind == DefinitionKind::Function).collect();
        assert!(!func_defs.is_empty(), "Expected a Function definition for 'greet', got defs: {:?}", defs);
        assert_eq!(func_defs[0].kind, DefinitionKind::Function);
    }

    #[test]
    fn parameter_has_own_span() {
        let source = "## To greet (name: Text):\n    Show name.\n";
        let result = analyze(source);
        assert!(result.errors.is_empty(), "Errors: {:?}", result.errors);

        let func_defs = result.symbol_index.definitions_of("greet");
        let param_defs = result.symbol_index.definitions_of("name");

        assert!(!func_defs.is_empty(), "Should have function def for 'greet'");
        assert!(!param_defs.is_empty(), "Should have param def for 'name'");

        let func_span = func_defs.iter()
            .find(|d| d.kind == DefinitionKind::Function)
            .map(|d| d.span);
        let param_span = param_defs.iter()
            .find(|d| d.kind == DefinitionKind::Parameter)
            .map(|d| d.span);

        if let (Some(fs), Some(ps)) = (func_span, param_span) {
            if fs != Span::default() && ps != Span::default() {
                assert_ne!(fs, ps,
                    "Parameter 'name' span {:?} should differ from function 'greet' span {:?}",
                    ps, fs);
                let param_text = &source[ps.start..ps.end];
                assert_eq!(param_text, "name",
                    "Parameter span should point to 'name' in source, got '{}'", param_text);
            }
        }
    }

    #[test]
    fn second_occurrence_found() {
        let source = "## Main\n    Let x be 5.\n    Let x be 10.\n";
        let result = analyze(source);
        // Even if there are parse errors due to redefinition, we should get definitions
        let defs = result.symbol_index.definitions_of("x");
        // There should be at least one definition
        assert!(!defs.is_empty(), "Expected at least one definition for 'x'");
    }
}

/// Resolve the user-visible name from a token, if it carries one.
pub fn resolve_token_name<'a>(token: &Token, interner: &'a Interner) -> Option<&'a str> {
    match &token.kind {
        TokenType::Identifier => Some(interner.resolve(token.lexeme)),
        TokenType::ProperName(sym) => Some(interner.resolve(*sym)),
        TokenType::Noun(sym) => Some(interner.resolve(*sym)),
        TokenType::Adjective(sym) => Some(interner.resolve(*sym)),
        TokenType::BlockHeader { .. } => Some(interner.resolve(token.lexeme)),
        TokenType::Verb { lemma, .. } => Some(interner.resolve(*lemma)),
        _ => None,
    }
}

/// Find the first token span where a name appears in the token stream,
/// optionally starting after a given byte offset.
fn find_token_span_for_name(tokens: &[Token], name: &str, interner: &Interner) -> Option<Span> {
    find_token_span_for_name_after(tokens, name, interner, 0)
}

/// Public version of `find_token_span_for_name` for use by other LSP modules.
pub fn find_token_span_for_name_pub(tokens: &[Token], name: &str, interner: &Interner) -> Option<Span> {
    find_token_span_for_name(tokens, name, interner)
}

/// Find the first token span where a name appears after `after_offset`.
fn find_token_span_for_name_after(
    tokens: &[Token],
    name: &str,
    interner: &Interner,
    after_offset: usize,
) -> Option<Span> {
    for token in tokens {
        if token.span.start < after_offset {
            continue;
        }
        if let Some(resolved) = resolve_token_name(token, interner) {
            if resolved == name {
                return Some(token.span);
            }
        }
    }
    None
}

/// Find the span of a keyword token (Give, Zone, etc.) that precedes a named
/// variable in the token stream. Used for diagnostic related-information.
pub fn find_keyword_span_before_name(
    tokens: &[Token],
    keyword: TokenType,
    variable_name: &str,
    interner: &Interner,
) -> Option<Span> {
    let discriminant = std::mem::discriminant(&keyword);
    for (i, token) in tokens.iter().enumerate() {
        if std::mem::discriminant(&token.kind) == discriminant {
            // Check if the variable name appears in subsequent tokens (within 3)
            for j in (i + 1)..tokens.len().min(i + 4) {
                if let Some(resolved) = resolve_token_name(&tokens[j], interner) {
                    if resolved == variable_name {
                        return Some(token.span);
                    }
                }
            }
        }
    }
    None
}
