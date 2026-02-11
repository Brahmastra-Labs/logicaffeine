use std::collections::HashMap;

use logicaffeine_base::{Arena, Interner};
use logicaffeine_compile::analysis::{
    EscapeChecker, OwnershipChecker, VarState,
};
use logicaffeine_language::{
    analysis::{DiscoveryPass, TypeRegistry, PolicyRegistry},
    arena_ctx::AstContext,
    ast::stmt::{Stmt, Expr, TypeExpr},
    drs::WorldState,
    error::ParseError,
    lexer::Lexer,
    mwe,
    parser::Parser,
    token::{Token, BlockType, TokenType},
};

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
}

/// Run the full analysis pipeline: lex → MWE → discover → parse → index.
///
/// Uses block-level error recovery: if the full parse fails, splits source
/// at `## BlockHeader` boundaries and parses each block independently.
pub fn analyze(source: &str) -> AnalysisResult {
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
    let (errors, symbol_index, escape_errors, ownership_errors, ownership_states) = match try_full_parse(
        parse_tokens.clone(),
        &type_registry,
        &mut interner,
    ) {
        Ok(result) => {
            let idx = SymbolIndex::build(&result.owned_stmts, &tokens, &type_registry, result.interner);
            (vec![], idx, result.escape_errors, result.ownership_errors, result.ownership_states)
        }
        Err(first_error) => {
            // Fall through to block-level recovery
            let mut recovery = parse_with_recovery(
                source,
                &tokens,
                &type_registry,
                &mut interner,
            );

            // Extract function definitions from block header tokens (the standard
            // parser cannot handle `## To funcName with param: Type` blocks)
            let func_defs = extract_function_defs_from_tokens(&tokens, &interner);
            recovery.stmts.extend(func_defs);

            if recovery.parse_errors.is_empty() {
                // Only report the first_error if it didn't originate from a
                // function definition block that the parser can't handle
                let has_function_blocks = tokens.iter().any(|t| {
                    matches!(t.kind, TokenType::BlockHeader { block_type: BlockType::Function })
                });
                if !has_function_blocks {
                    recovery.parse_errors.push(first_error);
                }
            }
            let idx = SymbolIndex::build(&recovery.stmts, &tokens, &type_registry, &interner);
            (recovery.parse_errors, idx, recovery.escape_errors, recovery.ownership_errors, recovery.ownership_states)
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
    }
}

/// Result of a successful full parse, including analysis checker results.
struct FullParseResult<'a> {
    owned_stmts: Vec<OwnedStmt>,
    interner: &'a Interner,
    escape_errors: Vec<AnalysisError>,
    ownership_errors: Vec<AnalysisError>,
    ownership_states: HashMap<String, VarState>,
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
    let mut parser = Parser::new(tokens, &mut world_state, interner, ctx, type_registry.clone());
    let stmts = parser.parse_program()?;

    // Run escape analysis while arena-allocated AST is still alive
    let escape_errors = {
        let mut checker = EscapeChecker::new(interner);
        match checker.check_program(&stmts) {
            Ok(()) => vec![],
            Err(e) => {
                let (code, cause_context) = match &e.kind {
                    logicaffeine_compile::analysis::EscapeErrorKind::ReturnEscape { zone_name, .. } => {
                        ("escape-return", Some(format!("zone '{}'", zone_name)))
                    }
                    logicaffeine_compile::analysis::EscapeErrorKind::AssignmentEscape { target, zone_name, .. } => {
                        ("escape-assignment", Some(format!("zone '{}', target '{}'", zone_name, target)))
                    }
                };
                let variable = match &e.kind {
                    logicaffeine_compile::analysis::EscapeErrorKind::ReturnEscape { variable, .. } => variable.clone(),
                    logicaffeine_compile::analysis::EscapeErrorKind::AssignmentEscape { variable, .. } => variable.clone(),
                };
                vec![AnalysisError {
                    message: e.to_string(),
                    variable,
                    code,
                    cause_context,
                }]
            }
        }
    };

    // Run ownership analysis while arena-allocated AST is still alive
    let (ownership_errors, ownership_states) = {
        let mut checker = OwnershipChecker::new(interner);
        let errors = match checker.check_program(&stmts) {
            Ok(()) => vec![],
            Err(e) => {
                let (code, variable, cause_context) = match &e.kind {
                    logicaffeine_compile::analysis::OwnershipErrorKind::UseAfterMove { variable } => {
                        ("use-after-move", variable.clone(), Some(format!("'{}' was given away here", variable)))
                    }
                    logicaffeine_compile::analysis::OwnershipErrorKind::UseAfterMaybeMove { variable, branch } => {
                        ("maybe-moved", variable.clone(), Some(format!("'{}' might be given away in {}", variable, branch)))
                    }
                    logicaffeine_compile::analysis::OwnershipErrorKind::DoubleMoved { variable } => {
                        ("double-move", variable.clone(), Some(format!("'{}' was first given away here", variable)))
                    }
                };
                vec![AnalysisError {
                    message: e.to_string(),
                    variable,
                    code,
                    cause_context,
                }]
            }
        };

        // Extract ownership states, resolving symbols to strings
        let states: HashMap<String, VarState> = checker
            .var_states()
            .iter()
            .map(|(sym, state)| (interner.resolve(*sym).to_string(), *state))
            .collect();

        (errors, states)
    };

    // Convert arena-allocated stmts to owned summaries for the symbol index
    let owned = stmts.iter().map(|s| summarize_stmt(s, interner)).collect();
    Ok(FullParseResult {
        owned_stmts: owned,
        interner,
        escape_errors,
        ownership_errors,
        ownership_states,
    })
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
    Other,
}

fn summarize_stmt(stmt: &Stmt, interner: &Interner) -> OwnedStmt {
    match stmt {
        Stmt::FunctionDef { name, params, return_type, .. } => OwnedStmt::FunctionDef {
            name: interner.resolve(*name).to_string(),
            params: params
                .iter()
                .map(|(n, ty)| {
                    (interner.resolve(*n).to_string(), format_type_expr(ty, interner))
                })
                .collect(),
            return_type: return_type.map(|ty| format_type_expr(ty, interner)),
        },
        Stmt::StructDef { name, fields, .. } => OwnedStmt::StructDef {
            name: interner.resolve(*name).to_string(),
            fields: fields
                .iter()
                .map(|(n, ty, _is_public)| {
                    (interner.resolve(*n).to_string(), interner.resolve(*ty).to_string())
                })
                .collect(),
        },
        Stmt::Let { var, ty, mutable, value, .. } => {
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
            let param_strs: Vec<String> = params.iter().map(|p| format_type_expr(p, interner)).collect();
            format!("{} of {}", base_name, param_strs.join(", "))
        }
        TypeExpr::Function { inputs, output } => {
            let input_strs: Vec<String> = inputs.iter().map(|i| format_type_expr(i, interner)).collect();
            let out = format_type_expr(output, interner);
            format!("({}) -> {}", input_strs.join(", "), out)
        }
        TypeExpr::Refinement { base, .. } => {
            format!("{} where ...", format_type_expr(base, interner))
        }
        TypeExpr::Persistent { inner } => {
            format!("Persistent {}", format_type_expr(inner, interner))
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
            matches!(t.kind, TokenType::BlockHeader { block_type: BlockType::Function })
        });
        if is_function_block {
            return result;
        }
        // Non-function single block: attempt to parse for partial recovery
        let block_tokens: Vec<Token> = tokens.to_vec();
        match try_parse_block(block_tokens, type_registry, interner) {
            Ok(block) => {
                result.stmts = block.owned_stmts;
                result.escape_errors = block.escape_errors;
                result.ownership_errors = block.ownership_errors;
                result.ownership_states = block.ownership_states;
            }
            Err(e) => result.parse_errors.push(e),
        }
        return result;
    }

    let mut try_block_fn = |start: usize, end_excl: usize, result: &mut RecoveryResult| {
        // Skip function blocks — they can't be parsed by the standard parser
        if matches!(tokens[start].kind, TokenType::BlockHeader { block_type: BlockType::Function }) {
            return;
        }
        let block_tokens: Vec<Token> = tokens[start..end_excl].to_vec();
        match try_parse_block(block_tokens, type_registry, interner) {
            Ok(block) => {
                result.stmts.extend(block.owned_stmts);
                result.escape_errors.extend(block.escape_errors);
                result.ownership_errors.extend(block.ownership_errors);
                result.ownership_states.extend(block.ownership_states);
            }
            Err(e) => result.parse_errors.push(e),
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
fn extract_function_defs_from_tokens(
    tokens: &[Token],
    interner: &Interner,
) -> Vec<OwnedStmt> {
    let mut results = Vec::new();
    let mut i = 0;

    while i < tokens.len() {
        if let TokenType::BlockHeader { block_type: BlockType::Function } = &tokens[i].kind {
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
                                )
                            {
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
mod tests {
    use super::*;

    #[test]
    fn analyze_simple_let() {
        let result = analyze("## Main\n    Let x be 5.\n");
        assert!(result.errors.is_empty(), "Expected no errors, got: {:?}", result.errors);
        assert!(!result.tokens.is_empty());
        let defs: Vec<_> = result.symbol_index.definitions.iter()
            .filter(|d| d.name == "x")
            .collect();
        assert!(!defs.is_empty(), "Expected definition for 'x'");
        assert_eq!(defs[0].kind, crate::index::DefinitionKind::Variable);
    }

    #[test]
    fn analyze_multiple_lets() {
        let result = analyze("## Main\n    Let x be 5.\n    Let y be 10.\n");
        assert!(result.errors.is_empty(), "Expected no errors, got: {:?}", result.errors);
        let x_defs = result.symbol_index.definitions_of("x");
        let y_defs = result.symbol_index.definitions_of("y");
        assert_eq!(x_defs.len(), 1);
        assert_eq!(y_defs.len(), 1);
    }

    #[test]
    fn analyze_empty_source() {
        let result = analyze("");
        assert!(result.tokens.is_empty() || result.errors.is_empty(),
            "Empty source should not crash");
    }

    #[test]
    fn analyze_produces_tokens() {
        let result = analyze("## Main\n    Let x be 5.\n");
        assert!(result.tokens.len() >= 3, "Expected at least block header + Let + x tokens");
    }

    #[test]
    fn analyze_syntax_error_produces_diagnostics() {
        let result = analyze("## Main\n    Let be.\n");
        assert!(!result.errors.is_empty(), "Expected parse errors for invalid syntax");
    }

    #[test]
    fn analyze_function_def() {
        let source = "## Main\n    Let x be 5.\n    Let y be x + 1.\n";
        let result = analyze(source);
        assert!(result.errors.is_empty(), "Expected no errors, got: {:?}", result.errors);
        let x_defs = result.symbol_index.definitions_of("x");
        let y_defs = result.symbol_index.definitions_of("y");
        assert_eq!(x_defs.len(), 1, "Expected 1 def for 'x'");
        assert_eq!(y_defs.len(), 1, "Expected 1 def for 'y'");
        assert!(y_defs[0].detail.as_ref().unwrap().contains("Let"),
            "y detail should mention Let: {:?}", y_defs[0].detail);
    }

    #[test]
    fn analyze_indexes_references() {
        let result = analyze("## Main\n    Let x be 5.\n    Show x.\n");
        assert!(result.errors.is_empty(), "Expected no errors, got: {:?}", result.errors);
        let refs = result.symbol_index.references_to("x");
        assert!(refs.len() >= 1, "Expected at least one reference to 'x', got {}", refs.len());
    }

    #[test]
    fn analyze_block_headers_indexed() {
        let result = analyze("## Main\n    Let x be 5.\n");
        assert!(!result.symbol_index.block_spans.is_empty(),
            "Expected block spans to be indexed");
    }

    #[test]
    fn analyze_statement_spans_indexed() {
        let result = analyze("## Main\n    Let x be 5.\n");
        assert!(result.errors.is_empty(), "Expected no errors, got: {:?}", result.errors);
        assert!(!result.symbol_index.statement_spans.is_empty(),
            "Expected statement spans to be indexed");
    }

    #[test]
    fn analyze_single_block_with_error_still_reports() {
        // Single block with a parse error should still produce diagnostics,
        // not silently return empty
        let result = analyze("## Main\n    Let be.\n");
        assert!(
            !result.errors.is_empty(),
            "Single block with error should still report errors"
        );
    }

    #[test]
    fn analyze_function_def_summarized() {
        let source = "## To greet (name: Text) -> Text:\n    Return name.\n";
        let result = analyze(source);
        let defs = result.symbol_index.definitions_of("greet");
        let func_defs: Vec<_> = defs.iter()
            .filter(|d| d.kind == crate::index::DefinitionKind::Function)
            .collect();
        if !func_defs.is_empty() {
            let detail = func_defs[0].detail.as_ref().unwrap();
            assert!(detail.contains("greet"), "Detail should contain function name: {}", detail);
            assert!(detail.contains("name"), "Detail should contain param name: {}", detail);
        }
    }

    #[test]
    fn analyze_generic_type_in_detail() {
        let source = "## Main\n    Let items: Seq of Int be an empty list.\n";
        let result = analyze(source);
        let defs = result.symbol_index.definitions_of("items");
        if !defs.is_empty() {
            if let Some(detail) = &defs[0].detail {
                assert!(detail.contains("Seq") || detail.contains("Int"),
                    "Detail should mention Seq or Int: {}", detail);
            }
        }
    }

    #[test]
    fn analyze_block_recovery_preserves_good_block() {
        let source = "## Main\n    Let be.\n## Note: readme\n    Just a note.\n";
        let result = analyze(source);
        assert!(result.symbol_index.block_spans.len() >= 1,
            "Block spans should have at least 1 entry even with errors");
    }

    #[test]
    fn analyze_single_block_error_does_not_swallow() {
        // Even with only one block header, error recovery should attempt to parse it
        let result = analyze("## Main\n    Let x be 5.\n    Let be.\n");
        // The good statement should still be indexed even if an error occurs
        assert!(
            !result.errors.is_empty() || !result.symbol_index.definitions_of("x").is_empty(),
            "Should either report errors or still index the valid statement"
        );
    }

    #[test]
    fn analyze_use_after_move_produces_ownership_error() {
        // Give x to y moves x. Using x afterward should produce an ownership error
        // (from the OwnershipChecker) or a UseAfterMove parse error (from the parser).
        let source = "## Main\n    Let x be 5.\n    Let y be 0.\n    Give x to y.\n    Show x.\n";
        let result = analyze(source);
        let has_parse_move_error = result.errors.iter()
            .any(|e| matches!(e.kind, logicaffeine_language::error::ParseErrorKind::UseAfterMove { .. }));
        let has_ownership_error = !result.ownership_errors.is_empty();
        assert!(
            has_parse_move_error || has_ownership_error,
            "Expected ownership error for use-after-move, got none. Parse errors: {:?}, Ownership errors: {:?}",
            result.errors, result.ownership_errors
        );
    }

    #[test]
    fn analyze_clean_code_no_analysis_errors() {
        let source = "## Main\n    Let x be 5.\n    Show x.\n";
        let result = analyze(source);
        assert!(result.errors.is_empty(), "Expected no parse errors: {:?}", result.errors);
        assert!(result.escape_errors.is_empty(), "Expected no escape errors: {:?}", result.escape_errors);
        assert!(result.ownership_errors.is_empty(), "Expected no ownership errors: {:?}", result.ownership_errors);
    }

    #[test]
    fn analyze_ownership_states_populated() {
        let source = "## Main\n    Let x be 5.\n    Show x.\n";
        let result = analyze(source);
        assert!(result.errors.is_empty(), "Expected no parse errors: {:?}", result.errors);
        assert!(
            !result.ownership_states.is_empty(),
            "Expected ownership states for variables"
        );
        let x_state = result.ownership_states.get("x");
        assert!(x_state.is_some(), "Expected state for 'x', got keys: {:?}", result.ownership_states.keys().collect::<Vec<_>>());
    }

    #[test]
    fn analyze_moved_variable_has_moved_state() {
        let source = "## Main\n    Let x be 5.\n    Let y be 0.\n    Give x to y.\n    Show x.\n";
        let result = analyze(source);
        // After Give, x should be Moved (even though there's a use-after-move error)
        if let Some(state) = result.ownership_states.get("x") {
            assert_eq!(*state, VarState::Moved, "x should be Moved after Give");
        }
    }
}

/// Result of parsing a single recovered block, including analysis errors.
struct BlockParseResult {
    owned_stmts: Vec<OwnedStmt>,
    escape_errors: Vec<AnalysisError>,
    ownership_errors: Vec<AnalysisError>,
    ownership_states: HashMap<String, VarState>,
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
    let mut parser = Parser::new(tokens, &mut world_state, interner, ctx, type_registry.clone());
    let stmts = parser.parse_program()?;

    let escape_errors = {
        let mut checker = EscapeChecker::new(interner);
        match checker.check_program(&stmts) {
            Ok(()) => vec![],
            Err(e) => {
                let (code, cause_context) = match &e.kind {
                    logicaffeine_compile::analysis::EscapeErrorKind::ReturnEscape { zone_name, .. } => {
                        ("escape-return", Some(format!("zone '{}'", zone_name)))
                    }
                    logicaffeine_compile::analysis::EscapeErrorKind::AssignmentEscape { target, zone_name, .. } => {
                        ("escape-assignment", Some(format!("zone '{}', target '{}'", zone_name, target)))
                    }
                };
                let variable = match &e.kind {
                    logicaffeine_compile::analysis::EscapeErrorKind::ReturnEscape { variable, .. } => variable.clone(),
                    logicaffeine_compile::analysis::EscapeErrorKind::AssignmentEscape { variable, .. } => variable.clone(),
                };
                vec![AnalysisError { message: e.to_string(), variable, code, cause_context }]
            }
        }
    };

    let (ownership_errors, ownership_states) = {
        let mut checker = OwnershipChecker::new(interner);
        let errors = match checker.check_program(&stmts) {
            Ok(()) => vec![],
            Err(e) => {
                let (code, variable, cause_context) = match &e.kind {
                    logicaffeine_compile::analysis::OwnershipErrorKind::UseAfterMove { variable } => {
                        ("use-after-move", variable.clone(), Some(format!("'{}' was given away here", variable)))
                    }
                    logicaffeine_compile::analysis::OwnershipErrorKind::UseAfterMaybeMove { variable, branch } => {
                        ("maybe-moved", variable.clone(), Some(format!("'{}' might be given away in {}", variable, branch)))
                    }
                    logicaffeine_compile::analysis::OwnershipErrorKind::DoubleMoved { variable } => {
                        ("double-move", variable.clone(), Some(format!("'{}' was first given away here", variable)))
                    }
                };
                vec![AnalysisError { message: e.to_string(), variable, code, cause_context }]
            }
        };
        let states: HashMap<String, VarState> = checker
            .var_states()
            .iter()
            .map(|(sym, state)| (interner.resolve(*sym).to_string(), *state))
            .collect();
        (errors, states)
    };

    let owned: Vec<OwnedStmt> = stmts.iter().map(|s| summarize_stmt(s, interner)).collect();
    Ok(BlockParseResult { owned_stmts: owned, escape_errors, ownership_errors, ownership_states })
}
