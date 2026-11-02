use std::collections::HashMap;

use logicaffeine_base::{Arena, Interner};
use logicaffeine_compile::analysis::{EscapeChecker, OwnershipChecker, VarState};
use logicaffeine_language::{
    analysis::{DiscoveryPass, PolicyRegistry, TypeRegistry},
    arena_ctx::AstContext,
    ast::stmt::{Expr, Stmt, StmtId, TypeExpr},
    drs::WorldState,
    error::ParseError,
    lexer::Lexer,
    mwe,
    parser::{NestedStmtSpans, Parser},
    token::{BlockType, Span, Token, TokenType},
};

use logicaffeine_compile::vm::compiler::Refusal;
use logicaffeine_language::lgx::check::Finding;
use logicaffeine_language::lint::Finding as CopyFinding;

use crate::index::SymbolIndex;

/// A lightweight owned analysis error from escape/ownership checking.
#[derive(Debug, Clone)]
pub struct AnalysisError {
    /// Socratic error message
    pub message: String,
    /// Variable name for span resolution
    pub variable: String,
    /// Stable diagnostic code for code actions
    pub code: &'static str,
    /// Optional cause context for related information
    pub cause_context: Option<String>,
    /// The erring statement's span — the innermost statement at fault, nested
    /// ones included — when known.
    pub use_span: Option<Span>,
    /// The causing statement's span (the Give that moved it), when known.
    pub cause_span: Option<Span>,
}

/// Result of running the full analysis pipeline on a document.
pub struct AnalysisResult {
    pub tokens: Vec<Token>,
    pub interner: Interner,
    pub type_registry: TypeRegistry,
    pub policy_registry: PolicyRegistry,
    pub errors: Vec<ParseError>,
    pub escape_errors: Vec<AnalysisError>,
    pub ownership_errors: Vec<AnalysisError>,
    pub ownership_states: HashMap<String, VarState>,
    pub symbol_index: SymbolIndex,
    /// The LGX template checker's findings — accessibility and structure warnings on a `View:` —
    /// at their spans in the file.
    pub lgx_findings: Vec<Finding>,
    /// The value-semantics copy lint's findings, at their spans in the file — see
    /// `logicaffeine_language::lint`.
    pub copy_findings: Vec<CopyFinding>,
    /// Every refusal the browser build of this program meets, each at the construct it refuses.
    /// Whole-program: computed from a full parse only, since whether a `## Route` or a mounted
    /// component lowers depends on the declarations beside it.
    pub refusals: Vec<Refusal>,
}

/// Run the full analysis pipeline: lex → MWE → discover → parse → index.
///
/// Uses block-level error recovery: if the full parse fails, splits source
/// at `## BlockHeader` boundaries and parses each block independently.
/// Blank the `private` header modifier — replace `private ` with spaces in a
/// `## To … private …` / `## A[n] private …` header so the parser (which has no
/// notion of privacy) names the definition by its REAL name instead of naming it
/// `private`, and so a private definition raises no spurious diagnostic. Byte
/// length is preserved, so every span the analysis reports still lines up with
/// the original document.
fn blank_privacy_markers(source: &str) -> String {
    const BLANK: &str = "        "; // "private " is 8 bytes
    let mut out = String::with_capacity(source.len());
    for line in source.split_inclusive('\n') {
        match privacy_marker_offset(line) {
            Some(at) => {
                out.push_str(&line[..at]);
                out.push_str(BLANK);
                out.push_str(&line[at + BLANK.len()..]);
            }
            None => out.push_str(line),
        }
    }
    out
}

/// Byte offset of a `private ` modifier within a header line, when it sits in a
/// real modifier position (`## To [native] private …`, `## A[n] private …`).
/// `None` for prose, a `private` inside a name, or a string.
fn privacy_marker_offset(line: &str) -> Option<usize> {
    let indent = line.len() - line.trim_start().len();
    let t = &line[indent..];
    if let Some(rest) = t.strip_prefix("## To ") {
        let mut pos = indent + "## To ".len();
        let rest = match rest.strip_prefix("native ") {
            Some(r) => {
                pos += "native ".len();
                r
            }
            None => rest,
        };
        return rest.starts_with("private ").then_some(pos);
    }
    for prefix in ["## A ", "## An "] {
        if let Some(rest) = t.strip_prefix(prefix) {
            if rest.starts_with("private ") {
                return Some(indent + prefix.len());
            }
        }
    }
    None
}

pub fn analyze(source: &str) -> AnalysisResult {
    let blanked = blank_privacy_markers(source);
    let source = blanked.as_str();
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(source, &mut interner);
    let tokens = lexer.tokenize();

    let mwe_trie = mwe::build_mwe_trie();
    let tokens = mwe::apply_mwe_pipeline(tokens, &mwe_trie, &mut interner);

    let (type_registry, policy_registry) = {
        let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
        let result = discovery.run_full();
        (result.types, result.policies)
    };

    let parse_tokens = tokens.clone();

    // Try full parse first (optimistic fast path)
    let (
        errors,
        symbol_index,
        escape_errors,
        ownership_errors,
        ownership_states,
        lgx_findings,
        copy_findings,
        refusals,
    ) = match try_full_parse(parse_tokens.clone(), &type_registry, &mut interner) {
        Ok(result) => {
            let idx = SymbolIndex::build(
                &result.owned_stmts,
                &tokens,
                &type_registry,
                result.interner,
                source,
            );
            (
                result.type_errors,
                idx,
                result.escape_errors,
                result.ownership_errors,
                result.ownership_states,
                result.lgx_findings,
                result.copy_findings,
                result.refusals,
            )
        }
        Err(first_error) => {
            // Fall through to block-level recovery
            let mut recovery = parse_with_recovery(source, &tokens, &type_registry, &mut interner);

            // Extract function definitions from block header tokens (the standard
            // parser cannot handle `## To funcName with param: Type` blocks)
            let func_defs = extract_function_defs_from_tokens(&tokens, &interner);
            recovery.stmts.extend(func_defs);

            // The full parse saw the whole program; its error is the truest
            // diagnosis for the region it names. The per-sentence reparse
            // only sees the excised sentence and can degrade a specific
            // teaching (IsValueEquality) to a generic ExpectedStatement —
            // swap the same-line generic back for the specific one (the two
            // anchor differently within the sentence, so lines, not spans,
            // are the join).
            let line_of = |offset: usize| {
                source[..offset.min(source.len())]
                    .bytes()
                    .filter(|&b| b == b'\n')
                    .count()
            };
            let first_error_line = line_of(first_error.span.start);
            let same_line = recovery
                .parse_errors
                .iter_mut()
                .find(|e| line_of(e.span.start) == first_error_line);
            if let Some(slot) = same_line {
                *slot = first_error;
            } else {
                // The full parse read the whole program — function bodies
                // included, which the per-block recovery does not reparse — so
                // its error stands whatever the recovery found elsewhere.
                let at = recovery
                    .parse_errors
                    .partition_point(|e| e.span.start <= first_error.span.start);
                recovery.parse_errors.insert(at, first_error);
            }
            let idx =
                SymbolIndex::build(&recovery.stmts, &tokens, &type_registry, &interner, source);
            (
                recovery.parse_errors,
                idx,
                recovery.escape_errors,
                recovery.ownership_errors,
                recovery.ownership_states,
                recovery.lgx_findings,
                recovery.copy_findings,
                Vec::new(),
            )
        }
    };

    AnalysisResult {
        tokens,
        interner,
        type_registry,
        policy_registry,
        errors,
        escape_errors,
        ownership_errors,
        ownership_states,
        symbol_index,
        lgx_findings,
        copy_findings,
        refusals,
    }
}

/// Result of a successful full parse, including analysis checker results.
struct FullParseResult<'a> {
    owned_stmts: Vec<OwnedStmt>,
    interner: &'a Interner,
    /// Typechecker findings mapped onto real statement spans.
    type_errors: Vec<ParseError>,
    escape_errors: Vec<AnalysisError>,
    ownership_errors: Vec<AnalysisError>,
    ownership_states: HashMap<String, VarState>,
    lgx_findings: Vec<Finding>,
    copy_findings: Vec<CopyFinding>,
    refusals: Vec<Refusal>,
}

/// Attempt a full parse of the token stream.
fn try_full_parse<'a>(
    tokens: Vec<Token>,
    type_registry: &TypeRegistry,
    interner: &'a mut Interner,
) -> Result<FullParseResult<'a>, ParseError> {
    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();
    let stmt_arena: Arena<Stmt> = Arena::new();
    let imperative_expr_arena: Arena<Expr> = Arena::new();
    let type_expr_arena: Arena<TypeExpr> = Arena::new();

    let ctx = AstContext::with_types(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
        &stmt_arena,
        &imperative_expr_arena,
        &type_expr_arena,
    );

    let mut world_state = WorldState::new();
    let mut parser = Parser::new(
        tokens,
        &mut world_state,
        interner,
        ctx,
        type_registry.clone(),
    );
    let stmts = parser.parse_program()?;
    let stmt_spans = parser.stmt_spans().to_vec();
    let nested_spans = parser.nested_stmt_spans().clone();
    let lgx_findings = parser.lgx_findings().to_vec();
    let copy_findings = parser.value_copy_findings().to_vec();
    let type_registry = parser.into_type_registry();

    // The browser build's own refusals — the same list the bytecode compiler stops at the first
    // of — located in THIS file: the editor parses without the stdlib prelude, so a span here is
    // already a file offset.
    let refusals =
        logicaffeine_compile::vm::compiler::lgx_refusals(&stmts, interner, Some(&type_registry));

    // Typecheck while the arena AST is alive; every failing top-level
    // statement reports, anchored on the innermost statement at fault — a
    // nested one through its trail, else the top-level statement itself.
    let type_errors = {
        let (_env, indexed) =
            logicaffeine_compile::analysis::check_program_collect(&stmts, interner, &type_registry);
        indexed
            .into_iter()
            .map(|found| {
                let mut kind = found.error.to_parse_error_kind(interner);
                // The bridge cannot see the registry; fill in the fields that
                // DO exist so the message and quickfix can name them.
                if let logicaffeine_language::error::ParseErrorKind::FieldNotFound {
                    type_name,
                    available,
                    ..
                } = &mut kind
                {
                    *available = fields_of_type(&type_registry, type_name, interner);
                }
                ParseError {
                    kind,
                    span: statement_span(
                        &found.trail,
                        found.stmt_index,
                        &stmt_spans,
                        &nested_spans,
                    )
                    .unwrap_or_default(),
                }
            })
            .collect()
    };

    // Escape and ownership analysis while the arena AST is still alive.
    let escape_errors = escape_findings(&stmts, interner, &stmt_spans, &nested_spans);
    let (ownership_errors, ownership_states) =
        ownership_findings(&stmts, interner, &stmt_spans, &nested_spans);

    // Convert arena-allocated stmts to owned summaries for the symbol index
    let owned = stmts.iter().map(|s| summarize_stmt(s, interner)).collect();
    Ok(FullParseResult {
        owned_stmts: owned,
        interner,
        type_errors,
        escape_errors,
        ownership_errors,
        ownership_states,
        lgx_findings,
        copy_findings,
        refusals,
    })
}

/// Where a finding lands: the innermost statement of its trail that the parser
/// read (a nested one, through `Parser::nested_stmt_spans`), else its
/// top-level statement (through `Parser::stmt_spans`).
fn statement_span(
    trail: &[StmtId],
    stmt_index: Option<usize>,
    stmt_spans: &[Span],
    nested_spans: &NestedStmtSpans,
) -> Option<Span> {
    trail
        .iter()
        .find_map(|id| nested_spans.get(*id))
        .or_else(|| stmt_index.and_then(|i| stmt_spans.get(i)).copied())
}

/// Every escape finding in `stmts`, anchored on the statement at fault.
fn escape_findings(
    stmts: &[Stmt<'_>],
    interner: &Interner,
    stmt_spans: &[Span],
    nested_spans: &NestedStmtSpans,
) -> Vec<AnalysisError> {
    use logicaffeine_compile::analysis::EscapeErrorKind;
    EscapeChecker::new(interner)
        .check_program_collect(stmts)
        .into_iter()
        .map(|finding| {
            let (code, variable, cause_context) = match &finding.error.kind {
                EscapeErrorKind::ReturnEscape {
                    variable,
                    zone_name,
                } => (
                    "escape-return",
                    variable.clone(),
                    Some(format!("zone '{}'", zone_name)),
                ),
                EscapeErrorKind::AssignmentEscape {
                    variable,
                    target,
                    zone_name,
                } => (
                    "escape-assignment",
                    variable.clone(),
                    Some(format!("zone '{}', target '{}'", zone_name, target)),
                ),
            };
            AnalysisError {
                message: finding.error.to_string(),
                variable,
                code,
                cause_context,
                use_span: statement_span(
                    &finding.trail,
                    Some(finding.stmt_index),
                    stmt_spans,
                    nested_spans,
                ),
                cause_span: None,
            }
        })
        .collect()
}

/// Every ownership finding in `stmts`, anchored on the use at fault and linked
/// to the statement that gave the value away, plus the variable states the
/// checker ended with.
fn ownership_findings(
    stmts: &[Stmt<'_>],
    interner: &Interner,
    stmt_spans: &[Span],
    nested_spans: &NestedStmtSpans,
) -> (Vec<AnalysisError>, HashMap<String, VarState>) {
    use logicaffeine_compile::analysis::OwnershipErrorKind;
    let mut checker = OwnershipChecker::new(interner);
    let errors = checker
        .check_program_collect(stmts)
        .into_iter()
        .map(|finding| {
            let (code, variable, cause_context) = match &finding.error.kind {
                OwnershipErrorKind::UseAfterMove { variable } => (
                    "use-after-move",
                    variable.clone(),
                    Some(format!("'{}' was given away here", variable)),
                ),
                OwnershipErrorKind::UseAfterMaybeMove { variable, given_in } => (
                    "maybe-moved",
                    variable.clone(),
                    Some(format!(
                        "'{}' might be given away in {}",
                        variable, given_in
                    )),
                ),
                OwnershipErrorKind::DoubleMoved { variable } => (
                    "double-move",
                    variable.clone(),
                    Some(format!("'{}' was first given away here", variable)),
                ),
            };
            let cause_trail: Vec<StmtId> = finding.cause.into_iter().collect();
            AnalysisError {
                message: finding.error.to_string(),
                variable,
                code,
                cause_context,
                use_span: statement_span(
                    &finding.trail,
                    Some(finding.stmt_index),
                    stmt_spans,
                    nested_spans,
                ),
                cause_span: statement_span(
                    &cause_trail,
                    finding.cause_stmt_index,
                    stmt_spans,
                    nested_spans,
                ),
            }
        })
        .collect();
    let states = checker
        .var_states()
        .iter()
        .map(|(sym, state)| (interner.resolve(*sym).to_string(), *state))
        .collect();
    (errors, states)
}

/// The field names a struct type declares, for did-you-mean and message text.
fn fields_of_type(registry: &TypeRegistry, type_name: &str, interner: &Interner) -> Vec<String> {
    for (sym, typedef) in registry.iter_types() {
        if interner.resolve(*sym) == type_name {
            if let logicaffeine_language::analysis::TypeDef::Struct { fields, .. } = typedef {
                return fields
                    .iter()
                    .map(|f| interner.resolve(f.name).to_string())
                    .collect();
            }
        }
    }
    Vec::new()
}

/// Lightweight summary of a statement for symbol indexing.
/// This avoids lifetime issues with arena-allocated AST nodes.
#[derive(Debug, Clone)]
pub enum OwnedStmt {
    FunctionDef {
        name: String,
        params: Vec<(String, String)>,
        return_type: Option<String>,
    },
    StructDef {
        name: String,
        fields: Vec<(String, String)>,
    },
    Let {
        name: String,
        ty: Option<String>,
        inferred_type: Option<String>,
        mutable: bool,
    },
    Theorem {
        name: String,
    },
    Block {
        name: String,
        kind: String,
    },
    /// `## Component App` — its heading, its state, its props and its actions.
    ///
    /// ★ IT CARRIES ITS OWN SPANS, unlike every variant above, which hands the indexer a NAME and
    /// lets it hunt the token stream for a match. That works for a `To …` heading and stops working
    /// here: a `View:` body is ONE `RawBlock` token, so a name used only in a template is not in
    /// the token stream at all — and a name that appears both in `State:` and in the view would be
    /// matched by position rather than by meaning. The parser already resolved all of this (a
    /// `StateField`, an `ActionDef` and the `ComponentDef` each carry a `Span`, and template spans
    /// are mapped back to FILE offsets by `RawMap`), so the answer is carried rather than re-guessed.
    ComponentDef {
        name: String,
        /// What the heading says it is — "component", "page", "store", "error page".
        kind: String,
        name_span: Span,
        /// `(name, type-or-inferred, derived?, span)`.
        state: Vec<(String, Option<String>, bool, Span)>,
        props: Vec<(String, String, Span)>,
        actions: Vec<(String, Vec<(String, String)>, Span)>,
        /// Every identifier the `View:` mentions, at its span in the FILE — the references a
        /// token walk cannot see, because the whole template is one `RawBlock` token.
        mentions: Vec<(String, Span)>,
    },
    Other,
}

fn summarize_stmt(stmt: &Stmt, interner: &Interner) -> OwnedStmt {
    match stmt {
        Stmt::ComponentDef(def) => {
            use logicaffeine_language::ast::component::ComponentKind;
            OwnedStmt::ComponentDef {
                name: interner.resolve(def.name).to_string(),
                kind: match &def.kind {
                    ComponentKind::Component => "component",
                    ComponentKind::Page { .. } => "page",
                    _ => "component",
                }
                .to_string(),
                name_span: def.span,
                state: def
                    .state
                    .iter()
                    .map(|f| {
                        (
                            interner.resolve(f.name).to_string(),
                            f.ty.map(|t| format_type_expr(t, interner))
                                .or_else(|| infer_type_from_expr(f.init, interner)),
                            f.derived,
                            f.span,
                        )
                    })
                    .collect(),
                props: def
                    .props
                    .iter()
                    .map(|(n, t)| {
                        (
                            interner.resolve(*n).to_string(),
                            format_type_expr(t, interner),
                            def.span,
                        )
                    })
                    .collect(),
                mentions: def
                    .view
                    .as_ref()
                    .map(|v| {
                        v.mentions
                            .iter()
                            .map(|(n, sp)| (interner.resolve(*n).to_string(), *sp))
                            .collect()
                    })
                    .unwrap_or_default(),
                actions: def
                    .actions
                    .iter()
                    .map(|a| {
                        (
                            interner.resolve(a.name).to_string(),
                            a.params
                                .iter()
                                .map(|(n, t)| {
                                    (
                                        interner.resolve(*n).to_string(),
                                        format_type_expr(t, interner),
                                    )
                                })
                                .collect(),
                            a.span,
                        )
                    })
                    .collect(),
            }
        }
        Stmt::FunctionDef {
            name,
            params,
            return_type,
            ..
        } => OwnedStmt::FunctionDef {
            name: interner.resolve(*name).to_string(),
            params: params
                .iter()
                .map(|(n, ty)| {
                    (
                        interner.resolve(*n).to_string(),
                        format_type_expr(ty, interner),
                    )
                })
                .collect(),
            return_type: return_type.map(|ty| format_type_expr(ty, interner)),
        },
        Stmt::StructDef { name, fields, .. } => OwnedStmt::StructDef {
            name: interner.resolve(*name).to_string(),
            fields: fields
                .iter()
                .map(|(n, ty, _is_public)| {
                    (
                        interner.resolve(*n).to_string(),
                        interner.resolve(*ty).to_string(),
                    )
                })
                .collect(),
        },
        Stmt::Let {
            var,
            ty,
            mutable,
            value,
            ..
        } => {
            let explicit_ty = ty.map(|t| format_type_expr(t, interner));
            let inferred_type = if explicit_ty.is_none() {
                infer_type_from_expr(value, interner)
            } else {
                None
            };
            OwnedStmt::Let {
                name: interner.resolve(*var).to_string(),
                ty: explicit_ty,
                inferred_type,
                mutable: *mutable,
            }
        }
        Stmt::Theorem(t) => OwnedStmt::Theorem {
            name: t.name.clone(),
        },
        _ => OwnedStmt::Other,
    }
}

fn infer_type_from_expr(expr: &Expr, interner: &Interner) -> Option<String> {
    use logicaffeine_language::ast::stmt::Literal;
    match expr {
        Expr::Literal(lit) => match lit {
            Literal::Number(_) => Some("Int".to_string()),
            Literal::Float(_) => Some("Real".to_string()),
            Literal::Text(_) => Some("Text".to_string()),
            Literal::Boolean(_) => Some("Bool".to_string()),
            Literal::Nothing => Some("Unit".to_string()),
            Literal::Char(_) => Some("Char".to_string()),
            Literal::Duration(_) => Some("Duration".to_string()),
            Literal::Date(_) => Some("Date".to_string()),
            Literal::Moment(_) => Some("Moment".to_string()),
            _ => None,
        },
        Expr::New { type_name, .. } => Some(interner.resolve(*type_name).to_string()),
        Expr::List(_) => Some("Seq".to_string()),
        Expr::Call { function, .. } => Some(format!("{}(..)", interner.resolve(*function))),
        Expr::Copy { .. } => Some("copy".to_string()),
        Expr::Length { .. } => Some("Int".to_string()),
        Expr::Contains { .. } => Some("Bool".to_string()),
        _ => None,
    }
}

fn format_type_expr(ty: &TypeExpr, interner: &Interner) -> String {
    match ty {
        TypeExpr::Primitive(sym) => interner.resolve(*sym).to_string(),
        TypeExpr::Named(sym) => interner.resolve(*sym).to_string(),
        TypeExpr::Generic { base, params } => {
            let base_name = interner.resolve(*base);
            let param_strs: Vec<String> = params
                .iter()
                .map(|p| format_type_expr(p, interner))
                .collect();
            format!("{} of {}", base_name, param_strs.join(", "))
        }
        TypeExpr::Function { inputs, output } => {
            let input_strs: Vec<String> = inputs
                .iter()
                .map(|i| format_type_expr(i, interner))
                .collect();
            let out = format_type_expr(output, interner);
            format!("({}) -> {}", input_strs.join(", "), out)
        }
        TypeExpr::Refinement { base, .. } => {
            format!("{} where ...", format_type_expr(base, interner))
        }
        TypeExpr::Persistent { inner } => {
            format!("Persistent {}", format_type_expr(inner, interner))
        }
        TypeExpr::Mutable { inner } => {
            format!("mutable {}", format_type_expr(inner, interner))
        }
        TypeExpr::Tuple(elems) => {
            let parts: Vec<String> = elems
                .iter()
                .map(|t| format_type_expr(t, interner))
                .collect();
            format!("({})", parts.join(", "))
        }
        TypeExpr::Record {
            field_names,
            field_types,
        } => {
            let parts: Vec<String> = field_names
                .iter()
                .zip(field_types.iter())
                .map(|(n, t)| {
                    format!(
                        "{}: {}",
                        interner.resolve(*n),
                        format_type_expr(t, interner)
                    )
                })
                .collect();
            format!("({})", parts.join(", "))
        }
    }
}

/// Result of block-level recovery parsing, including analysis errors.
struct RecoveryResult {
    stmts: Vec<OwnedStmt>,
    parse_errors: Vec<ParseError>,
    escape_errors: Vec<AnalysisError>,
    ownership_errors: Vec<AnalysisError>,
    ownership_states: HashMap<String, VarState>,
    lgx_findings: Vec<Finding>,
    copy_findings: Vec<CopyFinding>,
}

/// Parse with block-level recovery.
///
/// Splits source at `## BlockHeader` boundaries and parses each block
/// independently, collecting successful parses and errors separately.
/// Function definition blocks (`## To`) are skipped — they are handled
/// separately by `extract_function_defs_from_tokens`.
fn parse_with_recovery(
    _source: &str,
    tokens: &[Token],
    type_registry: &TypeRegistry,
    interner: &mut Interner,
) -> RecoveryResult {
    let mut result = RecoveryResult {
        stmts: vec![],
        parse_errors: vec![],
        escape_errors: vec![],
        ownership_errors: vec![],
        ownership_states: HashMap::new(),
        lgx_findings: vec![],
        copy_findings: vec![],
    };

    // Find block header positions in the token stream
    let mut block_boundaries: Vec<usize> = vec![0]; // first token
    for (i, tok) in tokens.iter().enumerate() {
        if matches!(tok.kind, TokenType::BlockHeader { .. }) && i > 0 {
            block_boundaries.push(i);
        }
    }

    if block_boundaries.len() <= 1 {
        // Single block — check if it's a function block (handled elsewhere)
        let is_function_block = tokens.first().map_or(false, |t| {
            matches!(
                t.kind,
                TokenType::BlockHeader {
                    block_type: BlockType::Function
                }
            )
        });
        if is_function_block {
            return result;
        }
        // Non-function single block: attempt to parse for partial recovery
        let block_tokens: Vec<Token> = tokens.to_vec();
        let (block, errors) =
            parse_block_excising_bad_sentences(block_tokens, type_registry, interner);
        result.parse_errors.extend(errors);
        if let Some(block) = block {
            result.stmts = block.owned_stmts;
            result.escape_errors = block.escape_errors;
            result.ownership_errors = block.ownership_errors;
            result.ownership_states = block.ownership_states;
            result.lgx_findings = block.lgx_findings;
            result.copy_findings = block.copy_findings;
        }
        return result;
    }

    let mut try_block_fn = |start: usize, end_excl: usize, result: &mut RecoveryResult| {
        // Skip function blocks — they can't be parsed by the standard parser
        if matches!(
            tokens[start].kind,
            TokenType::BlockHeader {
                block_type: BlockType::Function
            }
        ) {
            return;
        }
        let block_tokens: Vec<Token> = tokens[start..end_excl].to_vec();
        let (block, errors) =
            parse_block_excising_bad_sentences(block_tokens, type_registry, interner);
        result.parse_errors.extend(errors);
        if let Some(block) = block {
            result.stmts.extend(block.owned_stmts);
            result.escape_errors.extend(block.escape_errors);
            result.ownership_errors.extend(block.ownership_errors);
            result.ownership_states.extend(block.ownership_states);
            result.lgx_findings.extend(block.lgx_findings);
            result.copy_findings.extend(block.copy_findings);
        }
    };

    for window in block_boundaries.windows(2) {
        let start = window[0];
        let end = window[1];
        try_block_fn(start, end, &mut result);
    }

    // Parse the last block
    let last_start = *block_boundaries.last().unwrap();
    try_block_fn(last_start, tokens.len(), &mut result);

    result
}

/// Extract function definitions from `## To` block header tokens.
///
/// The standard parser cannot handle function definition blocks, so we extract
/// function name and parameters directly from the token stream.
fn extract_function_defs_from_tokens(tokens: &[Token], interner: &Interner) -> Vec<OwnedStmt> {
    let mut results = Vec::new();
    let mut i = 0;

    while i < tokens.len() {
        if let TokenType::BlockHeader {
            block_type: BlockType::Function,
        } = &tokens[i].kind
        {
            let mut j = i + 1;
            let mut func_name: Option<String> = None;
            let mut params: Vec<(String, String)> = Vec::new();
            let mut after_with = false;
            let mut current_param_name: Option<String> = None;
            let mut return_type: Option<String> = None;

            while j < tokens.len() {
                match &tokens[j].kind {
                    TokenType::Newline | TokenType::Indent | TokenType::BlockHeader { .. } => break,
                    TokenType::Colon => {
                        if let Some(param_name) = current_param_name.take() {
                            // Next token after colon is the type
                            j += 1;
                            let type_name = if j < tokens.len()
                                && !matches!(
                                    tokens[j].kind,
                                    TokenType::Newline
                                        | TokenType::Indent
                                        | TokenType::BlockHeader { .. }
                                        | TokenType::Colon
                                ) {
                                interner.resolve(tokens[j].lexeme).to_string()
                            } else {
                                "auto".to_string()
                            };
                            params.push((param_name, type_name));
                        }
                    }
                    TokenType::Arrow => {
                        // Next token is the return type
                        j += 1;
                        if j < tokens.len()
                            && !matches!(
                                tokens[j].kind,
                                TokenType::Newline
                                    | TokenType::Indent
                                    | TokenType::BlockHeader { .. }
                            )
                        {
                            return_type = Some(interner.resolve(tokens[j].lexeme).to_string());
                        }
                    }
                    TokenType::And | TokenType::Comma => {
                        // Parameter separator
                    }
                    TokenType::LParen => {
                        // `(n: Int)` groups open the parameter list exactly
                        // like the prose `with` does.
                        if func_name.is_some() {
                            after_with = true;
                        }
                    }
                    TokenType::RParen => {}
                    _ => {
                        let text = interner.resolve(tokens[j].lexeme);
                        if text == "with" && func_name.is_some() {
                            after_with = true;
                        } else if func_name.is_none() {
                            func_name = Some(text.to_string());
                        } else if after_with && current_param_name.is_none() {
                            current_param_name = Some(text.to_string());
                        }
                    }
                }
                j += 1;
            }

            if let Some(name) = func_name {
                results.push(OwnedStmt::FunctionDef {
                    name,
                    params,
                    return_type,
                });
            }
        }
        i += 1;
    }

    results
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_lsp/pipeline/tests.rs"]
mod tests;

/// Result of parsing a single recovered block, including analysis errors.
/// Parse a block, excising broken SENTENCES one at a time so every
/// independently bad statement reports and the good ones stay analyzed.
///
/// On failure the sentence containing the error span (a `Period`-delimited
/// token run) is removed — spans on the surviving tokens are original, so
/// later errors and the symbol index stay source-accurate — and the block is
/// reparsed. Excision stops when a failing sentence carries block structure
/// (`Indent`/`Dedent`, where cutting would unbalance the tree), when the
/// error lies outside any sentence, or after a hard iteration cap.
fn parse_block_excising_bad_sentences(
    mut block_tokens: Vec<Token>,
    type_registry: &TypeRegistry,
    interner: &mut Interner,
) -> (Option<BlockParseResult>, Vec<ParseError>) {
    const MAX_EXCISIONS: usize = 32;
    let mut errors = Vec::new();

    // Mid-stream block slices carry no EOF; the parser walks off the end of
    // an EOF-less stream, so terminate the slice before any parse attempt.
    if !matches!(block_tokens.last().map(|t| &t.kind), Some(TokenType::EOF)) {
        let end = block_tokens.last().map(|t| t.span.end).unwrap_or(0);
        let empty = interner.intern("");
        block_tokens.push(Token::new(
            TokenType::EOF,
            empty,
            logicaffeine_language::token::Span::new(end, end),
        ));
    }

    for _ in 0..=MAX_EXCISIONS {
        match try_parse_block(block_tokens.clone(), type_registry, interner) {
            Ok(block) => return (Some(block), errors),
            Err(e) => {
                let sentence = sentence_token_range(&block_tokens, e.span.start);
                errors.push(e);
                match sentence {
                    Some((start, end)) => {
                        let cuts_structure = block_tokens[start..=end]
                            .iter()
                            .any(|t| matches!(t.kind, TokenType::Indent | TokenType::Dedent));
                        if cuts_structure {
                            return (None, errors);
                        }
                        block_tokens.drain(start..=end);
                    }
                    None => return (None, errors),
                }
            }
        }
    }
    (None, errors)
}

/// The token range (inclusive) of the `Period`-delimited sentence containing
/// `offset`. Sentences start after the previous `Period` (or the block
/// header) and end at their own `Period`.
fn sentence_token_range(tokens: &[Token], offset: usize) -> Option<(usize, usize)> {
    let mut start = None;
    for (i, token) in tokens.iter().enumerate() {
        if matches!(
            token.kind,
            TokenType::Indent | TokenType::Dedent | TokenType::Newline | TokenType::EOF
        ) || matches!(token.kind, TokenType::BlockHeader { .. })
        {
            continue;
        }
        if start.is_none() {
            start = Some(i);
        }
        if matches!(token.kind, TokenType::Period) {
            let s = start.take().unwrap();
            if tokens[s].span.start <= offset && offset <= token.span.end {
                return Some((s, i));
            }
        }
    }
    // An error past the last period belongs to the trailing unterminated run.
    if let Some(s) = start {
        if tokens[s].span.start <= offset {
            return Some((s, tokens.len().saturating_sub(1)));
        }
    }
    None
}

struct BlockParseResult {
    owned_stmts: Vec<OwnedStmt>,
    escape_errors: Vec<AnalysisError>,
    ownership_errors: Vec<AnalysisError>,
    ownership_states: HashMap<String, VarState>,
    lgx_findings: Vec<Finding>,
    copy_findings: Vec<CopyFinding>,
}

fn try_parse_block(
    tokens: Vec<Token>,
    type_registry: &TypeRegistry,
    interner: &mut Interner,
) -> Result<BlockParseResult, ParseError> {
    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();
    let stmt_arena: Arena<Stmt> = Arena::new();
    let imperative_expr_arena: Arena<Expr> = Arena::new();
    let type_expr_arena: Arena<TypeExpr> = Arena::new();

    let ctx = AstContext::with_types(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
        &stmt_arena,
        &imperative_expr_arena,
        &type_expr_arena,
    );

    let mut world_state = WorldState::new();
    let mut parser = Parser::new(
        tokens,
        &mut world_state,
        interner,
        ctx,
        type_registry.clone(),
    );
    // A per-block recovery reparse sees only this block's own statements — the whole-file checks
    // `parse_program` otherwise runs at its end (`Mount` names something real, a page mounts
    // itself, LGX resolution) would report false refusals for names the REST of the file declares.
    // `fragment_mode` skips exactly those four; see its own doc comment for what a fragment still
    // does NOT get (cross-block LGX checks stay absent, not wrong, until the next whole-file parse).
    parser.fragment_mode = true;
    let stmts = parser.parse_program()?;
    let stmt_spans = parser.stmt_spans().to_vec();
    let nested_spans = parser.nested_stmt_spans().clone();
    let lgx_findings = parser.lgx_findings().to_vec();
    let copy_findings = parser.value_copy_findings().to_vec();
    drop(parser);

    let escape_errors = escape_findings(&stmts, interner, &stmt_spans, &nested_spans);
    let (ownership_errors, ownership_states) =
        ownership_findings(&stmts, interner, &stmt_spans, &nested_spans);

    let owned: Vec<OwnedStmt> = stmts.iter().map(|s| summarize_stmt(s, interner)).collect();
    Ok(BlockParseResult {
        owned_stmts: owned,
        escape_errors,
        ownership_errors,
        ownership_states,
        lgx_findings,
        copy_findings,
    })
}
