use crate::pipeline::OwnedStmt;
use logicaffeine_base::Interner;
use logicaffeine_language::{
    analysis::TypeRegistry,
    token::{BlockType, Span, Token, TokenType},
};
use std::collections::HashMap;

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
    /// `## Component App` / `## Page /todos` — the thing an LGX file is made of.
    Component,
    /// A field of a component's `State:`, or one of its props.
    StateField,
    /// A `To …` inside a component's `Actions:`.
    Action,
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
    /// Names mentioned inside a `View:`, collected while walking definitions and resolved once
    /// every definition exists — a template can name a component declared further down the file.
    template_mentions: Vec<(String, Span)>,
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

        // Phase 4.25: Reads inside interpolated text. `"The first {n} numbers"` is one string
        // token, so the walk above never sees `n` — and an unused-variable hint then told the
        // author a variable they print is never used.
        index.index_interpolation_reads(tokens, source);

        // Phase 4.5: The references a token walk CANNOT see. A `View:` body is one `RawBlock`
        // token, so `{count}` is invisible to the walk above; the parser recorded the names and
        // their file spans on the way past, and they are resolved HERE because a template may
        // name something declared further down the file.
        index.index_template_mentions();

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
                def.doc = logicaffeine_language::teach::doc_for_header_at(source, line_start);
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
                if resolve_token_name(candidate, interner)
                    .map(|n| n == name)
                    .unwrap_or(false)
                {
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
                OwnedStmt::FunctionDef {
                    name,
                    params,
                    return_type,
                } => {
                    let span =
                        find_token_span_for_name(tokens, name, interner).unwrap_or(Span::default());
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
                            tokens,
                            param_name,
                            interner,
                            search_after,
                        )
                        .unwrap_or(span);
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
                // ★ A COMPONENT AND ITS PARTS, spans carried rather than hunted. Everything the
                // editor can answer about a page — the outline, hover, go-to-definition, rename —
                // reads this index, and until a component put anything IN it the answer to every
                // one of those questions inside a `## Component` was nothing at all.
                OwnedStmt::ComponentDef {
                    name,
                    kind,
                    name_span,
                    state,
                    props,
                    actions,
                    mentions,
                } => {
                    self.add_definition(Definition {
                        name: name.clone(),
                        kind: DefinitionKind::Component,
                        span: *name_span,
                        detail: Some(format!("{name} ({kind})")),
                        scope: ScopeInfo::default(),
                        mutable: None,
                        doc: None,
                    });
                    for (field, ty, derived, span) in state {
                        let shown = ty.clone().unwrap_or_else(|| "?".to_string());
                        self.add_definition(Definition {
                            name: field.clone(),
                            kind: DefinitionKind::StateField,
                            span: *span,
                            detail: Some(if *derived {
                                format!("{field}: {shown} (derived)")
                            } else {
                                format!("{field}: {shown}")
                            }),
                            scope: ScopeInfo::default(),
                            // A derived field is recomputed, never assigned — the same distinction
                            // `Set` enforces, surfaced where the editor can warn before the
                            // compiler has to.
                            mutable: Some(!*derived),
                            doc: None,
                        });
                    }
                    for (prop, ty, span) in props {
                        self.add_definition(Definition {
                            name: prop.clone(),
                            kind: DefinitionKind::StateField,
                            span: *span,
                            detail: Some(format!("{prop}: {ty} (prop)")),
                            scope: ScopeInfo::default(),
                            mutable: Some(false),
                            doc: None,
                        });
                    }
                    self.template_mentions
                        .extend(mentions.iter().map(|(n, sp)| (n.clone(), *sp)));
                    for (action, params, span) in actions {
                        let shown: Vec<String> =
                            params.iter().map(|(n, t)| format!("{n}: {t}")).collect();
                        self.add_definition(Definition {
                            name: action.clone(),
                            kind: DefinitionKind::Action,
                            span: *span,
                            detail: Some(format!("To {action}({})", shown.join(", "))),
                            scope: ScopeInfo::default(),
                            mutable: None,
                            doc: None,
                        });
                    }
                }
                OwnedStmt::StructDef { name, fields } => {
                    let span =
                        find_token_span_for_name(tokens, name, interner).unwrap_or(Span::default());
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
                            tokens,
                            field_name,
                            interner,
                            field_search_after,
                        )
                        .unwrap_or(span);
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
                OwnedStmt::Let {
                    name,
                    ty,
                    inferred_type,
                    mutable,
                } => {
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
                    let span =
                        find_token_span_for_name(tokens, name, interner).unwrap_or(Span::default());
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

            let found = find_token_span_for_name(tokens, &name, interner);
            // A compiler-registered builtin (`HttpError`, `File`) is in EVERY registry whether
            // this program ever names it or not — indexing it unconditionally would crowd every
            // completion and hover with two structs (and their fields) most pages never touch.
            // Index it only when the program's own tokens actually name it; an author-declared
            // struct still gets its default span when the search comes up empty, as before.
            if type_registry.is_builtin_struct(*sym) && found.is_none() {
                continue;
            }
            let span = found.unwrap_or(Span::default());

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
                            tokens,
                            &field_name,
                            interner,
                            field_search_after,
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
                        let variant_span =
                            find_token_span_for_name(tokens, &variant_name, interner)
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

                    // An LGX header's IDENTITY lives in its block type, not its lexeme: a route
                    // is its METHOD and PATH together (`GET /api/todos` and `POST /api/todos`
                    // are different routes), a page is its path, a redirect is the pair. Taking
                    // the lexeme gave the outline a name that said none of it.
                    let name = match block_type {
                        BlockType::Route { method, path } => {
                            format!("{} {}", interner.resolve(*method), interner.resolve(*path))
                        }
                        BlockType::Page { path } => interner.resolve(*path).to_string(),
                        BlockType::Redirect { from, to } => {
                            format!("{} -> {}", interner.resolve(*from), interner.resolve(*to))
                        }
                        BlockType::ErrorPage { status } => format!("Error page {status}"),
                        // `## Component Card` / `## Store Session` / `## Directive tip` carry
                        // their name in the NEXT token, not in the block type — the lexeme is
                        // the header word itself, so the outline showed `##Component` for every
                        // component in the file.
                        BlockType::Component | BlockType::Store | BlockType::Directive => tokens
                            .get(i + 1)
                            .map(|t| interner.resolve(t.lexeme).to_string())
                            .filter(|n| !n.is_empty() && !n.starts_with('#'))
                            .unwrap_or_else(|| interner.resolve(tokens[i].lexeme).to_string()),
                        _ => interner.resolve(tokens[i].lexeme).to_string(),
                    };
                    self.block_spans
                        .push((name, *block_type, Span::new(start, end)));
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
                    | TokenType::Converge
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

    fn index_template_mentions(&mut self) {
        for (name, span) in std::mem::take(&mut self.template_mentions) {
            // Unresolved mentions are dropped rather than recorded: the scan that produced them
            // is deliberately loose (it also sees filter names and bare words), and a reference
            // to nothing helps nobody — it would only make "find all references" answer with
            // places that mean something else.
            let Some(def_idx) = self.nearest_def(&name, None) else {
                continue;
            };
            self.references.push(Reference {
                name,
                span,
                definition_idx: Some(def_idx),
            });
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

    /// Record each name read inside an interpolated string's `{…}` holes (`{{` is a literal
    /// brace, holes nest), at its own span in the file. Only names that resolve to a
    /// definition count — a hole may also hold format specs and built-in words.
    fn index_interpolation_reads(&mut self, tokens: &[Token], source: &str) {
        for token in tokens {
            if !matches!(token.kind, TokenType::InterpolatedString(_)) {
                continue;
            }
            let Some(raw) = source.get(token.span.start..token.span.end) else {
                continue;
            };
            for hole in interpolation_holes(raw) {
                let text = &raw[hole.clone()];
                let mut at = 0;
                while at < text.len() {
                    let rest = &text[at..];
                    let Some(first) = rest
                        .char_indices()
                        .find(|(_, c)| c.is_alphabetic() || *c == '_')
                    else {
                        break;
                    };
                    let word_start = at + first.0;
                    let len = text[word_start..]
                        .char_indices()
                        .find(|(_, c)| !(c.is_alphanumeric() || *c == '_'))
                        .map_or(text.len() - word_start, |(i, _)| i);
                    let word = &text[word_start..word_start + len];
                    let offset = token.span.start + hole.start + word_start;
                    let block = self.block_for_offset(offset);
                    if let Some(def_idx) = self.nearest_def(word, block) {
                        self.references.push(Reference {
                            name: word.to_string(),
                            span: Span::new(offset, offset + len),
                            definition_idx: Some(def_idx),
                        });
                    }
                    at = word_start + len;
                }
            }
        }
    }

    /// Record every `f(…)` and `Call f …` site whose name resolves to a
    /// function definition. The definition's own header (`## To f (…)`) also
    /// puts the name before a paren — excluded by span identity.
    fn index_call_sites(&mut self, tokens: &[Token], interner: &Interner) {
        for (i, token) in tokens.iter().enumerate() {
            let Some(name) = resolve_token_name(token, interner) else {
                continue;
            };

            let is_call_form =
                matches!(tokens.get(i + 1).map(|t| &t.kind), Some(TokenType::LParen))
                    || matches!(
                        i.checked_sub(1)
                            .and_then(|p| tokens.get(p))
                            .map(|t| &t.kind),
                        Some(TokenType::Call)
                    );
            if !is_call_form {
                continue;
            }

            let Some(callee) = self.name_to_defs.get(name).and_then(|indices| {
                indices
                    .iter()
                    .copied()
                    .find(|&ix| self.definitions[ix].kind == DefinitionKind::Function)
            }) else {
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
        self.references.iter().filter(|r| r.name == name).collect()
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
        self.block_spans
            .get(block_idx)
            .map(|(name, _, _)| name.as_str())
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_lsp/index/tests.rs"]
mod tests;

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
pub fn find_token_span_for_name_pub(
    tokens: &[Token],
    name: &str,
    interner: &Interner,
) -> Option<Span> {
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
                && resolve_token_name(t, interner)
                    .map(|n| n == name)
                    .unwrap_or(false)
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

/// The byte ranges of the `{…}` holes in an interpolated string's source text — the text
/// between each hole's braces. `{{` is a literal brace, and holes nest (`{f({x})}`), the same
/// rules the parser reads them by.
fn interpolation_holes(raw: &str) -> Vec<std::ops::Range<usize>> {
    let bytes = raw.as_bytes();
    let mut holes = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'{' if bytes.get(i + 1) == Some(&b'{') => i += 2,
            b'}' if bytes.get(i + 1) == Some(&b'}') => i += 2,
            b'{' => {
                let mut depth = 1;
                let mut j = i + 1;
                while j < bytes.len() && depth > 0 {
                    match bytes[j] {
                        b'{' => depth += 1,
                        b'}' => depth -= 1,
                        _ => {}
                    }
                    if depth > 0 {
                        j += 1;
                    }
                }
                if depth != 0 {
                    break;
                }
                holes.push(i + 1..j);
                i = j + 1;
            }
            _ => i += 1,
        }
    }
    holes
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_lsp/index/interpolation_hole_tests.rs"]
mod interpolation_hole_tests;
