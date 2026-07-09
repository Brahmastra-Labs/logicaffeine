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
    /// `Some(false)` = an immutable `Let` (readonly); only Variables carry this.
    pub mutable: Option<bool>,
    /// Literate documentation: the `## Note` block directly above this
    /// definition's `##` header (functions, types, theorems).
    pub doc: Option<String>,
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

/// One function call site, for the call hierarchy.
#[derive(Debug, Clone)]
pub struct CallSite {
    /// Definition index of the ENCLOSING function — `None` for calls from
    /// `## Main` or other non-function blocks (no hierarchy item to hang
    /// them on).
    pub caller: Option<usize>,
    /// Definition index of the called function.
    pub callee: usize,
    pub span: Span,
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
    /// Function call sites (`f(…)` and `Call f …`), caller-resolved.
    pub call_sites: Vec<CallSite>,
}

impl SymbolIndex {
    /// Build the symbol index from parsed statements, tokens, the type
    /// registry, and the source text (literate docs read the prose).
    pub fn build(
        stmts: &[OwnedStmt],
        tokens: &[Token],
        type_registry: &TypeRegistry,
        interner: &Interner,
        source: &str,
    ) -> Self {
        let mut index = SymbolIndex::default();

        // Phase 1: Extract definitions from parsed statements
        index.index_statements(stmts, tokens, interner);

        // Phase 2: Extract definitions from TypeRegistry (struct/enum definitions)
        index.index_type_registry(type_registry, tokens, interner);

        // Phase 3: Extract block headers and statement spans from tokens
        index.index_tokens(tokens, interner);

        // Phase 3.5: Attach literate documentation (`## Note` above a header)
        index.index_docs(source);

        // Phase 4: Index identifier references (scope-aware)
        index.index_references(tokens, interner);

        // Phase 5: Compute scope info for each definition
        index.compute_scopes();

        // Phase 6: Function call sites for the call hierarchy
        index.index_call_sites(tokens, interner);

        index
    }

    /// Attach each function/type/theorem definition's literate documentation:
    /// the `## Note` block sitting directly above its `##` header line.
    fn index_docs(&mut self, source: &str) {
        for def in &mut self.definitions {
            if !matches!(
                def.kind,
                DefinitionKind::Function
                    | DefinitionKind::Struct
                    | DefinitionKind::Enum
                    | DefinitionKind::Theorem
            ) {
                continue;
            }
            let anchor = def.span.start.min(source.len());
            let line_start = source[..anchor].rfind('\n').map(|i| i + 1).unwrap_or(0);
            let is_header_line = source[line_start..]
                .lines()
                .next()
                .is_some_and(|l| l.trim_start().starts_with("## "));
            if is_header_line {
                def.doc =
                    logicaffeine_language::teach::doc_for_header_at(source, line_start);
            }
        }
    }

    /// The binding token of the NEXT unconsumed `Let <name>` /
    /// `Let mutable <name>` in the stream — each Let statement claims its own
    /// binding site exactly once.
    fn claim_let_binding(
        tokens: &[Token],
        name: &str,
        interner: &Interner,
        taken: &mut std::collections::HashSet<usize>,
    ) -> Option<Span> {
        for (i, token) in tokens.iter().enumerate() {
            if !matches!(token.kind, TokenType::Let) || taken.contains(&i) {
                continue;
            }
            // The binding name sits within the next couple of tokens
            // (`Let x`, `Let mutable x`).
            for candidate in tokens.iter().skip(i + 1).take(3) {
                if matches!(candidate.kind, TokenType::Be | TokenType::Colon) {
                    break;
                }
                if resolve_token_name(candidate, interner).map(|n| n == name).unwrap_or(false) {
                    taken.insert(i);
                    return Some(candidate.span);
                }
            }
        }
        None
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
        let mut taken_lets: std::collections::HashSet<usize> = std::collections::HashSet::new();
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
                        mutable: None,
                        doc: None,
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
                            mutable: None,
                            doc: None,
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
                        mutable: None,
                        doc: None,
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
                            mutable: None,
                            doc: None,
                        });
                    }
                }
                OwnedStmt::Let { name, ty, inferred_type, mutable } => {
                    // Each re-Let of the same name anchors on its OWN
                    // binding token (the name right after its `Let`), never
                    // the first occurrence — shadow warnings and renames
                    // depend on the distinction.
                    let span = Self::claim_let_binding(tokens, name, interner, &mut taken_lets)
                        .or_else(|| find_token_span_for_name(tokens, name, interner))
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
                        mutable: Some(*mutable),
                        doc: None,
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
                        mutable: None,
                        doc: None,
                    });
                }
                OwnedStmt::Block { name, kind } => {
                    self.add_definition(Definition {
                        name: name.clone(),
                        kind: DefinitionKind::Block,
                        span: Span::default(),
                        detail: Some(format!("{} {}", kind, name)),
                        scope: ScopeInfo::default(),
                        mutable: None,
                        doc: None,
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
                        mutable: None,
                        doc: None,
                    });
                    let mut field_search_after = span.end;
                    for field in fields {
                        let field_name = interner.resolve(field.name).to_string();
                        // The field's OWN token span — reusing the struct
                        // header's span would collide in every span-keyed
                        // map (highlighting painted struct names as fields).
                        let field_span = find_token_span_for_name_after(
                            tokens, &field_name, interner, field_search_after,
                        )
                        .unwrap_or(span);
                        if field_span != span {
                            field_search_after = field_span.end;
                        }
                        self.add_definition(Definition {
                            name: field_name.clone(),
                            kind: DefinitionKind::Field,
                            span: field_span,
                            detail: Some(format!("{}.{}", name, field_name)),
                            scope: ScopeInfo::default(),
                            mutable: None,
                            doc: None,
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
                        mutable: None,
                        doc: None,
                    });
                    for variant in variants {
                        let variant_name = interner.resolve(variant.name).to_string();
                        let variant_span = find_token_span_for_name(tokens, &variant_name, interner)
                            .unwrap_or(span);
                        self.add_definition(Definition {
                            name: variant_name.clone(),
                            kind: DefinitionKind::Variant,
                            span: variant_span,
                            detail: Some(format!("{}::{}", name, variant_name)),
                            scope: ScopeInfo::default(),
                            mutable: None,
                            doc: None,
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
                    | TokenType::Require
                    | TokenType::Requires
                    | TokenType::Ensures
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

    /// Record every `f(…)` and `Call f …` site whose name resolves to a
    /// function definition. The definition's own header (`## To f (…)`) also
    /// puts the name before a paren — excluded by span identity.
    fn index_call_sites(&mut self, tokens: &[Token], interner: &Interner) {
        for (i, token) in tokens.iter().enumerate() {
            let Some(name) = resolve_token_name(token, interner) else { continue };

            let is_call_form = matches!(
                tokens.get(i + 1).map(|t| &t.kind),
                Some(TokenType::LParen)
            ) || matches!(
                i.checked_sub(1).and_then(|p| tokens.get(p)).map(|t| &t.kind),
                Some(TokenType::Call)
            );
            if !is_call_form {
                continue;
            }

            let Some(callee) = self
                .name_to_defs
                .get(name)
                .and_then(|indices| {
                    indices
                        .iter()
                        .copied()
                        .find(|&ix| self.definitions[ix].kind == DefinitionKind::Function)
                })
            else {
                continue;
            };
            if self.definitions[callee].span == token.span {
                continue; // the definition's own signature, not a call
            }

            self.call_sites.push(CallSite {
                caller: self.enclosing_function(token.span.start),
                callee,
                span: token.span,
            });
        }
    }

    /// The function definition whose block contains `offset`, if any.
    pub fn enclosing_function(&self, offset: usize) -> Option<usize> {
        let block = self
            .block_spans
            .iter()
            .filter(|(_, block_type, span)| {
                *block_type == BlockType::Function && span.start <= offset && offset < span.end
            })
            .min_by_key(|(_, _, span)| span.end - span.start)?;
        let block_span = block.2;
        self.definitions.iter().position(|d| {
            d.kind == DefinitionKind::Function
                && block_span.start <= d.span.start
                && d.span.start < block_span.end
        })
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
        // Definitions are named by their SURFACE form ("greet"), not the
        // lexicon's normalized lemma ("Greet") — an English word used as an
        // identifier must resolve by what the author wrote.
        TokenType::Verb { .. } => Some(interner.resolve(token.lexeme)),
        // A lexically ambiguous word ("name": verb or noun) is still an
        // identifier at the surface level.
        TokenType::Ambiguous { .. } => Some(interner.resolve(token.lexeme)),
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

/// The LAST occurrence of a name — the use-site approximation for
/// use-after-move/escape diagnostics when no statement span is known (the
/// complaint is always about a later use, never the binding itself).
pub fn find_last_token_span_for_name(
    tokens: &[Token],
    name: &str,
    interner: &Interner,
) -> Option<Span> {
    tokens
        .iter()
        .rev()
        .find(|t| {
            !matches!(t.kind, TokenType::BlockHeader { .. })
                && resolve_token_name(t, interner).map(|n| n == name).unwrap_or(false)
        })
        .map(|t| t.span)
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

/// Find the span of the keyword statement (Give, Zone, …) that CAUSED an
/// error on `variable_name`. Used for diagnostic related-information.
///
/// The variable must sit in OBJECT position — the token immediately after
/// the keyword — so `Give y to x` never counts as the Give that moved `x`
/// (there, `x` is the recipient). When `before_offset` is given, the LAST
/// matching statement before that use site wins: the most recent move is
/// the cause, not the first one in the file.
pub fn find_keyword_span_before_name(
    tokens: &[Token],
    keyword: TokenType,
    variable_name: &str,
    interner: &Interner,
) -> Option<Span> {
    find_cause_keyword_span(tokens, keyword, variable_name, interner, usize::MAX)
}

/// [`find_keyword_span_before_name`] bounded to causes before a use site.
pub fn find_cause_keyword_span(
    tokens: &[Token],
    keyword: TokenType,
    variable_name: &str,
    interner: &Interner,
    before_offset: usize,
) -> Option<Span> {
    let discriminant = std::mem::discriminant(&keyword);
    let mut best: Option<Span> = None;
    for (i, token) in tokens.iter().enumerate() {
        if std::mem::discriminant(&token.kind) != discriminant {
            continue;
        }
        if token.span.start >= before_offset {
            break;
        }
        let object_matches = tokens
            .get(i + 1)
            .and_then(|t| resolve_token_name(t, interner))
            .map(|resolved| resolved == variable_name)
            .unwrap_or(false);
        if object_matches {
            best = Some(token.span);
        }
    }
    best
}
