use std::collections::{HashMap, HashSet};
use std::fmt::Write;

use crate::analysis::registry::{FieldType, TypeDef, TypeRegistry};
use crate::analysis::types::RustNames;
use crate::ast::stmt::{
    BinaryOpKind, Block, Expr, Literal, MatchArm, Pattern, ReadSource, Stmt, TypeExpr, ZoneSource,
};
use crate::intern::{Interner, Symbol};

use super::context::{emit_refinement_check, RefinementContext, VariableCapabilities};
use super::detection::{
    calls_async_function, collect_expr_identifiers, collect_stmt_identifiers, expr_debug_prefix,
    expr_reads_any_collection, parse_aos_tag, requires_async_stmt,
};
use super::expr::{
    codegen_assertion, codegen_expr_boxed_with_types, codegen_expr_boxed_with_types_oracle,
    codegen_expr_boxed_with_types_oracle_tolerant, codegen_expr_with_async,
    codegen_expr_with_async_and_strings, codegen_expr_with_async_oracle,
    collect_string_concat_operands, is_definitely_string_expr_with_vars, is_rational_expr,
    set_movable_last_use,
};

/// How many times `target` is read in `e`, counting only the expression shapes a
/// promoted-`Int` accumulator RHS is built from (arithmetic, calls, unary not).
/// Any shape not modeled here returns `2` — a poison that makes the total ≥ 2, so
/// the move-on-last-use hint is disabled rather than risk a use-after-move on a
/// subtree we did not fully account for. A result of exactly `1` therefore proves
/// `target` occurs once, in a fully-understood position, and may be moved.
fn count_target_reads(e: &Expr, target: Symbol) -> usize {
    match e {
        Expr::Identifier(s) => usize::from(*s == target),
        Expr::Literal(_) => 0,
        Expr::BinaryOp { left, right, .. } => {
            count_target_reads(left, target) + count_target_reads(right, target)
        }
        Expr::Not { operand } => count_target_reads(operand, target),
        Expr::Call { args, .. } => args.iter().map(|a| count_target_reads(a, target)).sum(),
        _ => 2,
    }
}
use super::escape_rust_ident;
use super::i64_map::{dense_type_name, is_logos_map_type, map_rust_type};
use super::peephole::{
    body_declares_var, body_has_early_exit, body_modifies_var, body_mutates_collection,
    body_resizes_collection, collect_expr_symbols, emit_bounds_hint_header,
    emit_bounds_hint_preheader, exprs_equal, is_counter_increment, plan_bounds_hints,
    simplify_1based_index, try_emit_byte_compare_window, try_emit_drain_tail_in_while,
    BoundsHintPlan,
};
use super::types::{
    codegen_type_expr, infer_numeric_type, infer_rust_type_from_expr, infer_variant_type_annotation,
};

/// Escape a LOGOS text value for a Rust `"..."` string-literal body (no surrounding
/// quotes). Kept identical to the string-literal escaping in `codegen::expr` so the
/// self-append fast paths (`push_str`, `write!`-with-`&str`) render byte-for-byte the
/// same literal a normal `String::from("…")` would.
/// A LOGOS Text as the body of a Rust string literal — the ONE escaper.
///
/// There were two: this, and an identical copy inlined in the expression emitter. Two copies is
/// how three sites ended up without any — `Let s be "a\"b" + n.` emitted
/// `format!("{}{}", "a"b", 5)`, which does not compile, while the same text on its own was fine.
/// Every place that spells a literal into generated Rust calls THIS.
pub(crate) fn rust_string_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            _ => out.push(ch),
        }
    }
    out
}

/// Escape a single `char` for a Rust `'c'` char-literal body (no surrounding quotes).
fn rust_char_escape(ch: char) -> String {
    match ch {
        '\n' => "\\n".to_string(),
        '\r' => "\\r".to_string(),
        '\t' => "\\t".to_string(),
        '\\' => "\\\\".to_string(),
        '\'' => "\\'".to_string(),
        _ => ch.to_string(),
    }
}

/// The `(Vec<T>, init-expr)` pair for a de-Rc'd Seq declaration, or `None`
/// when the initializer is not a fresh scalar-Seq allocation (in which case
/// the normal `LogosSeq` path runs). Mirrors `fresh_scalar_seq_elem` in the
/// de-Rc eligibility analysis — only those forms are ever marked de-Rc.
fn derc_vec_decl<'a>(
    value: &Expr<'a>,
    interner: &Interner,
    synced_vars: &HashSet<Symbol>,
    boxed_fields: &HashSet<(String, String, String)>,
    registry: &TypeRegistry,
    async_functions: &HashSet<Symbol>,
    ctx: &RefinementContext<'a>,
) -> Option<(String, String)> {
    match value {
        Expr::New {
            type_name,
            type_args,
            init_fields,
        } if init_fields.is_empty() => match interner.resolve(*type_name) {
            "Seq" | "List" | "Vec" => {
                let elem = codegen_type_expr(type_args.first()?, interner);
                Some((format!("Vec<{}>", elem), "Vec::new()".to_string()))
            }
            _ => None,
        },
        Expr::WithCapacity {
            value: inner,
            capacity,
        } => {
            if let Expr::New {
                type_name,
                type_args,
                ..
            } = inner
            {
                if matches!(interner.resolve(*type_name), "Seq" | "List" | "Vec") {
                    let elem = codegen_type_expr(type_args.first()?, interner);
                    let cap = codegen_expr_boxed_with_types(
                        capacity,
                        interner,
                        synced_vars,
                        boxed_fields,
                        registry,
                        async_functions,
                        ctx.get_string_vars(),
                        ctx.get_variable_types(),
                        ctx.get_fast_div(),
                    );
                    return Some((
                        format!("Vec<{}>", elem),
                        format!("Vec::with_capacity(({}) as usize)", cap),
                    ));
                }
            }
            None
        }
        // Phase 4: `Let r be f(...)` where `f` returns an OWNED `Vec<T>` (its
        // return type already de-Rc'd in `fn_returns_map`). The result is a
        // uniquely-owned fresh value — bind it as `Vec<T>` so every later access
        // on `r` indexes directly instead of borrowing a RefCell.
        Expr::Call { function, .. } => {
            let ret = ctx.get_fn_return(function)?;
            if !ret.starts_with("Vec<") {
                return None;
            }
            let vec_ty = ret.clone();
            let init = codegen_expr_boxed_with_types(
                value,
                interner,
                synced_vars,
                boxed_fields,
                registry,
                async_functions,
                ctx.get_string_vars(),
                ctx.get_variable_types(),
                ctx.get_fast_div(),
            );
            Some((vec_ty, init))
        }
        // A homogeneous scalar-literal list de-Rc's to a plain `Vec<T>` literal — the NTT power
        // table + coefficient array, and any `[…]` of one scalar literal kind, indexed without a
        // RefCell borrow. Eligibility mirrors `fresh_scalar_seq_elem` via the shared helper.
        Expr::List(items) => {
            let elem = crate::codegen::detection::homogeneous_scalar_literal_elem(items)?;
            let item_strs: Vec<String> = items
                .iter()
                .map(|i| {
                    codegen_expr_boxed_with_types(
                        i,
                        interner,
                        synced_vars,
                        boxed_fields,
                        registry,
                        async_functions,
                        ctx.get_string_vars(),
                        ctx.get_variable_types(),
                        ctx.get_fast_div(),
                    )
                })
                .collect();
            Some((
                format!("Vec<{}>", elem),
                format!("vec![{}]", item_strs.join(", ")),
            ))
        }
        _ => None,
    }
}

/// The `LogosI64Map` constructor for a `Map of Int to Int` declared via `value`
/// (`a new Map of Int to Int` or `… with capacity n`); `None` for any other
/// initializer (the normal `LogosMap` codegen runs). Only called when the alias
/// analysis already selected the variable for `LogosI64Map`.
fn i64_map_ctor<'a>(
    value: &Expr<'a>,
    ty: &str,
    dense_lo: Option<i64>,
    interner: &Interner,
    synced_vars: &HashSet<Symbol>,
    boxed_fields: &HashSet<(String, String, String)>,
    registry: &TypeRegistry,
    async_functions: &HashSet<Symbol>,
    ctx: &RefinementContext<'a>,
) -> Option<String> {
    match value {
        Expr::New {
            type_name,
            init_fields,
            ..
        } if init_fields.is_empty()
            && matches!(interner.resolve(*type_name), "Map" | "HashMap") =>
        {
            Some(format!("{}::new()", ty))
        }
        Expr::WithCapacity {
            value: inner,
            capacity,
        } => {
            if let Expr::New { type_name, .. } = inner {
                if matches!(interner.resolve(*type_name), "Map" | "HashMap") {
                    let cap = codegen_expr_boxed_with_types(
                        capacity,
                        interner,
                        synced_vars,
                        boxed_fields,
                        registry,
                        async_functions,
                        ctx.get_string_vars(),
                        ctx.get_variable_types(),
                        ctx.get_fast_div(),
                    );
                    // A dense map is a direct-addressed array offset by the proven
                    // window `lo` (currently 0) and sized `capacity + 1` slots: the
                    // bound proof gives `lo <= key <= lo + capacity`, so the highest
                    // index `key - lo` is `capacity`, which needs `capacity + 1`
                    // slots. A non-dense `LogosI64Map`/`Set` takes the plain capacity.
                    return Some(match dense_lo {
                        Some(lo) => {
                            format!("{}::with_bounds({}, (({}) + 1) as usize)", ty, lo, cap)
                        }
                        None => format!("{}::with_capacity(({}) as usize)", ty, cap),
                    });
                }
            }
            None
        }
        _ => None,
    }
}

/// The 0-based `usize` offset a store index proven in range names: a rebased counter as it is, any other
/// index with its 1 taken away.
fn store_offset(
    index: &Expr,
    zero_based: Option<&str>,
    interner: &Interner,
    ctx: &RefinementContext,
) -> String {
    match zero_based {
        Some(s) => format!("{} as usize", s),
        None => simplify_1based_index(index, interner, true, ctx.get_variable_types()),
    }
}

/// O5: emit `assert_unchecked` bounds hints for every indexed access in `stmt`
/// the bounds-elision oracle proves in range (a relational induction bound
/// `i <= length(arr)` or a concrete interval). Paired with a `debug_assert!`
/// so an unsound proof panics loudly in debug. No-op when the oracle is absent.
/// Only Identifier collections (a name is needed for `.len()`).
fn emit_oracle_index_hints<'a>(
    stmt: &Stmt<'a>,
    ctx: &RefinementContext<'a>,
    interner: &Interner,
    registry: &TypeRegistry,
    indent_str: &str,
) -> String {
    // Kill switch (A/B): `LOGOS_ORACLE_HINTS=0` suppresses the oracle bounds
    // hints entirely (the access keeps its runtime bounds check).
    if !crate::optimize::active_config().is_on(crate::optimization::Opt::OracleHints) {
        return String::new();
    }
    let oracle = match ctx.oracle() {
        Some(o) => o,
        None => return String::new(),
    };
    let mut accesses: Vec<(&Expr, &Expr)> = Vec::new();
    collect_index_accesses_in_stmt(stmt, &mut accesses);
    if accesses.is_empty() {
        return String::new();
    }
    let names = RustNames::new(interner);
    let mut out = String::new();
    let mut seen: HashSet<usize> = HashSet::new();
    for (coll, idx) in accesses {
        let arr = match coll {
            Expr::Identifier(s) => *s,
            _ => continue,
        };
        // An affine read-only array is deleted — its reads are pure arithmetic,
        // so there is no `.len()` to bound-check and no hint to emit.
        if ctx.affine_array(arr).is_some() {
            continue;
        }
        // A list of the exact extension's values is read through its model, which checks every
        // index itself; there is no slice for a hint to bound.
        if ctx
            .get_variable_types()
            .get(&arr)
            .is_some_and(|t| t == super::tower::VALUE)
        {
            continue;
        }
        // An index held as an exact number is read as the integer it equals, checked as it converts; there is
        // no machine word to bound.
        let typing = super::rational::Typing {
            interner,
            variable_types: ctx.get_variable_types(),
            registry,
        };
        if super::rational::read_as_exact(idx, typing) {
            continue;
        }
        // An AoS-interleaved member folds into a `[[T; W]; N]` backing. The hint
        // must name the BACKING (the member's own name no longer exists) and
        // bound its ROW dimension (`backing.len()` == N) — that elides the
        // rolled-regime row bounds-check just like C's, since the column is a
        // constant < W.
        let aos_backing = parse_aos_tag(ctx.get_variable_types().get(&arr)).map(|t| t.backing);
        if !seen.insert(idx as *const Expr as usize) {
            continue;
        }
        if !oracle.index_provably_in_bounds(coll, idx) {
            continue;
        }
        // Discharge any `% m` element-bound precondition this elision leaned on
        // with a hard `assert!(m > 0)` (LLVM hoists the loop-invariant check out
        // of the loop, so it costs one comparison per region entry). For
        // `m <= 0` the `% m`-filled array is empty and this access would be out
        // of bounds: the assert panics there — exactly where the program would
        // have — instead of the elision becoming UB.
        for &m in oracle.index_positivity_guards(idx) {
            let mn = names.ident(m);
            writeln!(
                out,
                "{}assert!(({mn}) > 0, \"LOGOS positivity guard: element bound on `% {mn}` requires {mn} > 0\");",
                indent_str, mn = mn
            )
            .unwrap();
        }
        // Match the ACCESS's 0-based index form: a `__zero_based_i64` counter
        // indexes directly (`arr[i]`), everything else is 1-based (`arr[i-1]`).
        // The oracle reasons about the 1-based source; codegen may rebase.
        let i0 = match super::expr::zero_based_index_str(idx, ctx.get_variable_types(), interner) {
            Some(s) => s,
            None => simplify_1based_index(idx, interner, false, ctx.get_variable_types()),
        };
        let arr_name = aos_backing.unwrap_or_else(|| names.ident(arr));
        let cond = format!("({}) >= 0 && ({}) < ({}.len() as i64)", i0, i0, arr_name);
        writeln!(out, "{}debug_assert!({}, \"LOGOS oracle bounds hint violated: indexing `{}` out of range\");", indent_str, cond, arr_name).unwrap();
        writeln!(
            out,
            "{}unsafe {{ std::hint::assert_unchecked({}); }}",
            indent_str, cond
        )
        .unwrap();
    }
    if !out.is_empty() {
        crate::optimize::mark_fired(crate::optimization::Opt::OracleHints);
    }
    out
}

fn collect_index_accesses_in_stmt<'a>(
    stmt: &'a Stmt<'a>,
    out: &mut Vec<(&'a Expr<'a>, &'a Expr<'a>)>,
) {
    match stmt {
        Stmt::Let { value, .. } | Stmt::Set { value, .. } => collect_index_in_expr(value, out),
        Stmt::SetIndex {
            collection,
            index,
            value,
        } => {
            out.push((collection, index));
            collect_index_in_expr(index, out);
            collect_index_in_expr(value, out);
        }
        Stmt::If { cond, .. } | Stmt::While { cond, .. } => collect_index_in_expr(cond, out),
        Stmt::Show { object, .. } | Stmt::Give { object, .. } => collect_index_in_expr(object, out),
        Stmt::Return { value: Some(e) } => collect_index_in_expr(e, out),
        Stmt::Push { value, .. } | Stmt::Add { value, .. } | Stmt::Remove { value, .. } => {
            collect_index_in_expr(value, out)
        }
        _ => {}
    }
}

fn collect_index_in_expr<'a>(e: &'a Expr<'a>, out: &mut Vec<(&'a Expr<'a>, &'a Expr<'a>)>) {
    match e {
        Expr::Index { collection, index } => {
            out.push((collection, index));
            collect_index_in_expr(collection, out);
            collect_index_in_expr(index, out);
        }
        Expr::BinaryOp { left, right, .. } => {
            collect_index_in_expr(left, out);
            collect_index_in_expr(right, out);
        }
        Expr::Not { operand } => collect_index_in_expr(operand, out),
        Expr::Call { args, .. } => {
            for a in args.iter() {
                collect_index_in_expr(a, out);
            }
        }
        Expr::Length { collection } => collect_index_in_expr(collection, out),
        _ => {}
    }
}

/// AOT BCE-hoist: a row-major affine-offset bounds guard for one array in one
/// loop. The index `item (offset + counter + 1) of arr` is BILINEAR (e.g.
/// `c[i*n+j]`) — the `i*n` product defeats the linear bounds prover — but from
/// the inner loop's view `offset` (`i*n`) is loop-INVARIANT, so the emitted
/// index `offset + counter` is monotone in the counter. A single preheader
/// `assert!` that `offset >= 0` and the MAX index `offset + (limit - 1) < len`
/// lets the per-iteration access be elided soundly with no static nonlinear
/// proof and no UB (it PANICS, never UB, on an out-of-range program).
pub(crate) struct AffineOffsetGuard<'a> {
    arr_sym: Symbol,
    /// The loop-invariant offset (`i * n`).
    offset: &'a Expr<'a>,
    /// The emitted index `offset + counter` (`i*n + j`), for the body hint.
    emitted_index: &'a Expr<'a>,
    /// Every access index `(offset + counter) + 1` into the array, all of which the guard proves.
    covered: Vec<&'a Expr<'a>>,
}

/// Match the 1-based row-major index `(offset + counter) + 1` whose emitted
/// index is `offset + counter`. Returns `(offset, offset_plus_counter)` where
/// `offset` is the operand that is NOT the bare counter.
fn match_affine_offset_index<'a>(
    index: &'a Expr<'a>,
    counter: Symbol,
) -> Option<(&'a Expr<'a>, &'a Expr<'a>)> {
    if let Expr::BinaryOp {
        op: BinaryOpKind::Add,
        left,
        right,
    } = index
    {
        if matches!(right, Expr::Literal(Literal::Number(1))) {
            if let Expr::BinaryOp {
                op: BinaryOpKind::Add,
                left: a,
                right: b,
            } = left
            {
                if matches!(b, Expr::Identifier(s) if *s == counter) {
                    return Some((a, left));
                }
                if matches!(a, Expr::Identifier(s) if *s == counter) {
                    return Some((b, left));
                }
            }
        }
    }
    None
}

/// Plan the per-iteration `assert_unchecked` hints for row-major affine-offset accesses in an exclusive `while
/// counter < limit` loop body: each access the loop's versioning guard proved in range
/// ([`crate::optimize::facts::Fact::InBounds`], `optimize::version`), top-level in the body, to a `Vec`/slice that is
/// not resized or rebound, with an offset fixed over the loop (not mentioning the counter, none of its symbols
/// mutated). An access without the fact gets no hint and keeps its check. One guard per array.
pub(crate) fn plan_affine_offset_guards<'a>(
    body: &'a [Stmt<'a>],
    counter_sym: Symbol,
    is_exclusive: bool,
    var_types: &HashMap<Symbol, String>,
) -> Vec<AffineOffsetGuard<'a>> {
    if !is_exclusive || body_has_early_exit(body) {
        return Vec::new();
    }
    let mut guards: Vec<AffineOffsetGuard> = Vec::new();
    let mut seen: HashSet<Symbol> = HashSet::new();
    for stmt in body {
        let mut accesses: Vec<(&Expr, &Expr)> = Vec::new();
        collect_index_accesses_in_stmt(stmt, &mut accesses);
        for (coll, index) in accesses {
            let arr_sym = match coll {
                Expr::Identifier(s) => *s,
                _ => continue,
            };
            if !crate::optimize::facts::holds_expr(crate::optimize::facts::Fact::InBounds, index) {
                continue;
            }
            let Some((offset, emitted)) = match_affine_offset_index(index, counter_sym) else {
                continue;
            };
            let base_ty = var_types
                .get(&arr_sym)
                .map(|t| t.split("|__hl:").next().unwrap_or(t.as_str()));
            let qualifies = matches!(
                base_ty,
                Some(t) if t.starts_with("Vec<") || t.starts_with("LogosSeq")
                    || t.starts_with("&[") || t.starts_with("&mut [")
            );
            if !qualifies
                || body_resizes_collection(body, arr_sym)
                || body_modifies_var(body, arr_sym)
            {
                continue;
            }
            let mut osyms = Vec::new();
            collect_expr_symbols(offset, &mut osyms);
            if osyms.contains(&counter_sym)
                || osyms
                    .iter()
                    .any(|s| body_modifies_var(body, *s) || body_mutates_collection(body, *s))
            {
                continue;
            }
            if seen.insert(arr_sym) {
                guards.push(AffineOffsetGuard {
                    arr_sym,
                    offset,
                    emitted_index: emitted,
                    covered: vec![index],
                });
            } else if let Some(g) = guards
                .iter_mut()
                .find(|g| g.arr_sym == arr_sym && exprs_equal(g.offset, offset))
            {
                g.covered.push(index);
            }
        }
    }
    guards
}

/// Emit the per-iteration `assert_unchecked` for each affine-offset guard, at
/// the top of the loop body. Sound because the preheader `assert!` proved the
/// max index in range and the index is monotone in the (non-negative) counter.
pub(crate) fn emit_affine_offset_header(
    guards: &[AffineOffsetGuard],
    interner: &Interner,
    synced_vars: &HashSet<Symbol>,
    async_functions: &HashSet<Symbol>,
    var_types: &HashMap<Symbol, String>,
    body_indent: &str,
    output: &mut String,
) {
    let names = RustNames::new(interner);
    for g in guards {
        let arr = names.ident(g.arr_sym);
        let e = codegen_expr_with_async(
            g.emitted_index,
            interner,
            synced_vars,
            async_functions,
            var_types,
        );
        writeln!(
            output,
            "{}unsafe {{ std::hint::assert_unchecked(({e}) >= 0 && ({e}) < ({arr}.len() as i64)); }}",
            body_indent, e = e, arr = arr
        ).unwrap();
    }
}

/// Emit a `Select` as a raw `tokio::select!` — the default (free-running) lowering and the
/// no-arm-ready fallback of deterministic replay.
#[allow(clippy::too_many_arguments)]
fn write_tokio_select<'a>(
    output: &mut String,
    branches: &[crate::ast::stmt::SelectBranch<'a>],
    interner: &Interner,
    indent: usize,
    mutable_vars: &HashSet<Symbol>,
    ctx: &mut RefinementContext<'a>,
    lww_fields: &HashSet<(String, String)>,
    mv_fields: &HashSet<(String, String)>,
    synced_vars: &mut HashSet<Symbol>,
    var_caps: &HashMap<Symbol, VariableCapabilities>,
    async_functions: &HashSet<Symbol>,
    pipe_vars: &HashSet<Symbol>,
    boxed_fields: &HashSet<(String, String, String)>,
    registry: &TypeRegistry,
    type_env: &crate::analysis::types::TypeEnv,
) {
    use crate::ast::stmt::SelectBranch;
    let indent_str = "    ".repeat(indent);
    writeln!(output, "{}tokio::select! {{", indent_str).unwrap();
    for branch in branches {
        match branch {
            SelectBranch::Receive { var, pipe, body } => {
                let var_name = interner.resolve(*var);
                let pipe_str = codegen_expr_with_async(
                    pipe,
                    interner,
                    synced_vars,
                    async_functions,
                    ctx.get_variable_types(),
                );
                let is_local_pipe = if let Expr::Identifier(sym) = pipe {
                    pipe_vars.contains(sym)
                } else {
                    false
                };
                let suffix = if is_local_pipe { "_rx" } else { "" };
                writeln!(
                    output,
                    "{}    {} = {}{}.recv() => {{",
                    indent_str, var_name, pipe_str, suffix
                )
                .unwrap();
                writeln!(
                    output,
                    "{}        if let Some({}) = {} {{",
                    indent_str, var_name, var_name
                )
                .unwrap();
                for stmt in *body {
                    let stmt_code = codegen_stmt(
                        stmt,
                        interner,
                        indent + 3,
                        mutable_vars,
                        ctx,
                        lww_fields,
                        mv_fields,
                        synced_vars,
                        var_caps,
                        async_functions,
                        pipe_vars,
                        boxed_fields,
                        registry,
                        type_env,
                    );
                    write!(output, "{}", stmt_code).unwrap();
                }
                writeln!(output, "{}        }}", indent_str).unwrap();
                writeln!(output, "{}    }}", indent_str).unwrap();
            }
            SelectBranch::Timeout { milliseconds, body } => {
                let dur = match milliseconds {
                    Expr::Literal(Literal::Duration(_)) => codegen_expr_with_async(
                        milliseconds,
                        interner,
                        synced_vars,
                        async_functions,
                        ctx.get_variable_types(),
                    ),
                    Expr::Literal(Literal::Span { months, days }) => {
                        let secs = ((*months as i64) * 30 + (*days as i64)) * 86_400;
                        format!("std::time::Duration::from_secs({}u64)", secs.max(0))
                    }
                    _ => {
                        let n = codegen_expr_with_async(
                            milliseconds,
                            interner,
                            synced_vars,
                            async_functions,
                            ctx.get_variable_types(),
                        );
                        format!("std::time::Duration::from_secs({} as u64)", n)
                    }
                };
                writeln!(
                    output,
                    "{}    _ = tokio::time::sleep({}) => {{",
                    indent_str, dur
                )
                .unwrap();
                for stmt in *body {
                    let stmt_code = codegen_stmt(
                        stmt,
                        interner,
                        indent + 2,
                        mutable_vars,
                        ctx,
                        lww_fields,
                        mv_fields,
                        synced_vars,
                        var_caps,
                        async_functions,
                        pipe_vars,
                        boxed_fields,
                        registry,
                        type_env,
                    );
                    write!(output, "{}", stmt_code).unwrap();
                }
                writeln!(output, "{}    }}", indent_str).unwrap();
            }
        }
    }
    writeln!(output, "{}}}", indent_str).unwrap();
}

/// The future a Mode-A launch spawns. Each argument `(rendered, is_literal)` that is not a literal is
/// evaluated where the launch stands and moved into the task, so a named value handed to two tasks is
/// cloned once for each before either starts — never moved into the first — and in the order the
/// interpreter evaluates them; a literal, which can neither move nor observe anything, stays in the
/// call. `tail` follows the call (`.await`, a `;` to discard the result).
fn launched_future(function: &str, args: &[(String, bool)], tail: &str) -> String {
    let mut lets = String::new();
    let mut call_args = Vec::with_capacity(args.len());
    for (i, (value, literal)) in args.iter().enumerate() {
        if *literal {
            call_args.push(value.clone());
        } else {
            lets.push_str(&format!("let __logos_arg{i} = {value}; "));
            call_args.push(format!("__logos_arg{i}"));
        }
    }
    let future = format!(
        "async move {{ {function}({}){tail} }}",
        call_args.join(", ")
    );
    if lets.is_empty() {
        future
    } else {
        format!("{{ {lets}{future} }}")
    }
}

/// Emit a copy-on-write guard before an in-place mutation of `collection`, when the one gate
/// ([`super::expr::cow_handle`]) says its buffer may be shared.
pub(super) fn emit_cow(
    collection: &Expr,
    ctx: &RefinementContext,
    names: &RustNames,
    indent_str: &str,
    output: &mut String,
) {
    if let Some(name) = super::expr::cow_handle(collection, ctx.get_variable_types(), names) {
        writeln!(output, "{}{}.cow();", indent_str, name).unwrap();
    }
}

/// If a loop body's sole write to a registered scratch buffer identifies it, return that buffer and its
/// scalarization info. Only the buffer's own fill loop pushes it (post-fill reads are proven read-only by
/// `detect_scratch_buffers`), so any loop that pushes a registered scratch buffer IS its fill loop.
fn scratch_fill_target<'a>(
    body: &[Stmt<'a>],
    ctx: &RefinementContext<'a>,
) -> Option<(Symbol, super::affine_array::ScratchInfo)> {
    for s in body {
        if let Stmt::Push {
            collection: Expr::Identifier(c),
            ..
        } = s
        {
            if let Some(info) = ctx.scratch_buffer(*c) {
                return Some((*c, info.clone()));
            }
        }
    }
    None
}

/// `Opt::StrideFill`: a loop storing one value into a bit-packed sequence at a fixed stride ([`super::bits::StrideFill`])
/// as one strided fill, where the loop runs at all. The counter is left where the loop leaves it: the last value
/// stored at, then the loop's own `Set j to j + step`, emitted as the loop would emit it.
#[allow(clippy::too_many_arguments)]
fn try_emit_stride_fill<'a>(
    cond: &'a Expr<'a>,
    body: Block<'a>,
    interner: &Interner,
    indent: usize,
    mutable_vars: &HashSet<Symbol>,
    ctx: &mut RefinementContext<'a>,
    lww_fields: &HashSet<(String, String)>,
    mv_fields: &HashSet<(String, String)>,
    synced_vars: &mut HashSet<Symbol>,
    var_caps: &HashMap<Symbol, VariableCapabilities>,
    async_functions: &HashSet<Symbol>,
    pipe_vars: &HashSet<Symbol>,
    boxed_fields: &HashSet<(String, String, String)>,
    registry: &TypeRegistry,
    type_env: &crate::analysis::types::TypeEnv,
    indent_str: &str,
) -> Option<String> {
    let config = crate::optimize::active_config();
    if !config.is_on(crate::optimization::Opt::StrideFill)
        || !config.is_on(crate::optimization::Opt::Unchecked)
    {
        return None;
    }
    let fill = super::bits::match_stride_fill(
        cond,
        body,
        ctx.get_variable_types(),
        ctx.oracle()?,
        interner,
    )?;
    crate::optimize::mark_fired(crate::optimization::Opt::StrideFill);
    crate::optimize::mark_fired(crate::optimization::Opt::Unchecked);
    let names = RustNames::new(interner);
    let types = ctx.get_variable_types();
    let counter = names.ident(fill.counter);
    let bits = names.ident(fill.bits);
    let emit = |e: &Expr| codegen_expr_with_async(e, interner, synced_vars, async_functions, types);
    let cond_str = codegen_expr_with_async_oracle(
        cond,
        interner,
        synced_vars,
        async_functions,
        types,
        ctx.oracle(),
    );
    let cond_str = truthy_cond_wrap(cond, cond_str, interner, types);
    let (bound, step, value) = (emit(fill.bound), emit(fill.step), emit(fill.value));
    let first = simplify_1based_index(fill.index, interner, false, types);
    let below = if fill.strict { " - 1" } else { "" };
    let inner = "    ".repeat(indent + 1);
    let mut out = String::new();
    writeln!(out, "{indent_str}if {cond_str} {{").unwrap();
    writeln!(out, "{inner}let __sf_step: i64 = {step};").unwrap();
    writeln!(
        out,
        "{inner}debug_assert!(({first}) >= 0, \"LOGOS oracle BCE: strided fill out of range\");"
    )
    .unwrap();
    writeln!(
        out,
        "{inner}let __sf_more = unsafe {{ {bits}.fill_stride_through_unchecked(({first}) as usize, ({first}).wrapping_add((({bound}) as i64{below}).wrapping_sub({counter})) as usize, __sf_step as usize, {value}) }} as i64;"
    )
    .unwrap();
    writeln!(out, "{inner}{counter} = {counter} + __sf_more * __sf_step;").unwrap();
    out.push_str(&codegen_stmt(
        fill.increment,
        interner,
        indent + 1,
        mutable_vars,
        ctx,
        lww_fields,
        mv_fields,
        synced_vars,
        var_caps,
        async_functions,
        pipe_vars,
        boxed_fields,
        registry,
        type_env,
    ));
    writeln!(out, "{indent_str}}}").unwrap();
    Some(out)
}

/// The `While` arm of [`codegen_stmt`]: the loop walker: it recurses into the body, so it sits on the stack at every nesting level.
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn codegen_while_stmt<'a>(
    mut output: String,
    stmt: &Stmt<'a>,
    cond: &&'a Expr<'a>,
    body: &Block<'a>,
    interner: &Interner,
    indent: usize,
    mutable_vars: &HashSet<Symbol>,
    ctx: &mut RefinementContext<'a>,
    lww_fields: &HashSet<(String, String)>,
    mv_fields: &HashSet<(String, String)>,
    synced_vars: &mut HashSet<Symbol>,
    var_caps: &HashMap<Symbol, VariableCapabilities>,
    async_functions: &HashSet<Symbol>,
    pipe_vars: &HashSet<Symbol>,
    boxed_fields: &HashSet<(String, String, String)>,
    registry: &TypeRegistry,
    type_env: &crate::analysis::types::TypeEnv,
    indent_str: &str,
) -> String {
    // bcmp idiom: a fixed-window byte-compare loop collapses to a slice
    // inequality (LLVM `bcmp`) — try it before the scalar while codegen.
    if let Some(code) = try_emit_byte_compare_window(
        stmt,
        interner,
        indent,
        ctx.get_variable_types(),
        ctx.oracle(),
    ) {
        return code;
    }
    if let Some(code) = try_emit_stride_fill(
        *cond,
        *body,
        interner,
        indent,
        mutable_vars,
        ctx,
        lww_fields,
        mv_fields,
        synced_vars,
        var_caps,
        async_functions,
        pipe_vars,
        boxed_fields,
        registry,
        type_env,
        indent_str,
    ) {
        output.push_str(&code);
        return output;
    }
    // decreasing is compile-time only, ignored at runtime

    // OPT: Loop bounds hoisting.
    // Collect all `length of X` symbols from condition AND body where X is
    // not modified in the loop. Emit hoisted `let x_len = (x.len() as i64);`
    // bindings before the loop. Register a sentinel in variable_types so that
    // Expr::Length codegen emits the hoisted name directly (no string replacement).
    let mut all_length_syms_raw = extract_length_expr_syms(cond);
    collect_length_syms_from_stmts(body, &mut all_length_syms_raw);
    let mut seen = HashSet::new();
    let all_length_syms: Vec<Symbol> = all_length_syms_raw
        .into_iter()
        .filter(|s| seen.insert(*s))
        .collect();

    let mut hoisted_syms: Vec<(Symbol, Option<String>)> = Vec::new();
    for len_sym in &all_length_syms {
        // An affine read-only array is deleted; `length of` it is its trip
        // count (no `.len()` to hoist). Leave its `__affine_array:` tag intact.
        if ctx.affine_array(*len_sym).is_some() {
            continue;
        }
        if !body_mutates_collection(body, *len_sym)
            && !body_modifies_var(body, *len_sym)
            && !body_declares_var(body, *len_sym)
        {
            let name = interner.resolve(*len_sym);
            let hoisted_name = format!("{}_len", name);
            writeln!(
                output,
                "{}let {} = ({}.len() as i64);",
                indent_str, hoisted_name, name
            )
            .unwrap();
            let old_type = ctx.get_variable_types().get(len_sym).cloned();
            // Append hoisted sentinel to existing type string. Expr::Length checks
            // for |__hl: suffix to emit the hoisted name. All other type checks
            // use starts_with() so the suffix doesn't interfere.
            let new_type = match &old_type {
                Some(existing) => format!("{}|__hl:{}", existing, hoisted_name),
                None => format!("|__hl:{}", hoisted_name),
            };
            ctx.register_variable_type(*len_sym, new_type);
            hoisted_syms.push((*len_sym, old_type));
        }
    }

    // OPT-9: bounds hints for arrays indexed by a unit-stepping while
    // counter (`while counter <= bound:` / `< bound:`). Unlike the
    // for-range site, this loop shape guarantees nothing by
    // construction, so everything plan_bounds_hints assumes must be
    // checked here: the counter takes every value up to the bound
    // (unit increment last, no other counter writes) and the bound is
    // loop-invariant.
    let mut while_bounds_hints: Vec<BoundsHintPlan> = Vec::new();
    let mut affine_guards: Vec<AffineOffsetGuard> = Vec::new();
    if let Expr::BinaryOp {
        op: op @ (BinaryOpKind::LtEq | BinaryOpKind::Lt),
        left: Expr::Identifier(counter_sym),
        right: bound_expr,
    } = cond
    {
        let body_sans_inc =
            if !body.is_empty() && is_counter_increment(&body[body.len() - 1], *counter_sym) {
                Some(&body[..body.len() - 1])
            } else {
                None
            };
        if let Some(body_sans_inc) = body_sans_inc {
            let counter_clean = !body_modifies_var(body_sans_inc, *counter_sym);
            let mut bound_syms = Vec::new();
            collect_expr_symbols(bound_expr, &mut bound_syms);
            let bound_invariant = bound_syms
                .iter()
                .all(|s| !body_modifies_var(body, *s) && !body_mutates_collection(body, *s));
            if counter_clean && bound_invariant {
                let bound_str = codegen_expr_with_async(
                    bound_expr,
                    interner,
                    synced_vars,
                    async_functions,
                    ctx.get_variable_types(),
                );
                while_bounds_hints = plan_bounds_hints(
                    body_sans_inc,
                    *counter_sym,
                    matches!(op, BinaryOpKind::Lt),
                    false,
                    bound_expr,
                    &bound_str,
                    ctx.get_variable_types(),
                );
                emit_bounds_hint_preheader(&while_bounds_hints, interner, &indent_str, &mut output);

                // Row-major affine-offset accesses the loop's versioning guard proved in range get a
                // per-iteration hint in the body (`plan_affine_offset_guards`).
                affine_guards = plan_affine_offset_guards(
                    body_sans_inc,
                    *counter_sym,
                    matches!(op, BinaryOpKind::Lt),
                    ctx.get_variable_types(),
                );
            }
        }
    }

    // Scratch-buffer allocation hoisting: a loop-local buffer that is
    // freshly allocated, fully overwritten by a copy of a source slice,
    // used/mutated in place, and never escapes the iteration is hoisted
    // to ONE allocation before the loop and reused (`clear()` +
    // `extend_from_slice`) each iteration — eliminating one malloc/free
    // per iteration (fannkuch's `perm`: ~n! of them). Value-identical to
    // the per-iteration `.to_vec()`; gated on the de-Rc non-escape proof.
    let scratch_hoisted: Vec<Symbol> =
        super::peephole::detect_scratch_hoist_in_body(body, interner, ctx)
            .into_iter()
            .map(|(dst, elem_type)| {
                writeln!(
                    output,
                    "{}let mut {}: Vec<{}> = Vec::new();",
                    indent_str,
                    interner.resolve(dst),
                    elem_type
                )
                .unwrap();
                ctx.register_variable_type(dst, format!("Vec<{}>", elem_type));
                ctx.register_scratch_hoist(dst);
                dst
            })
            .collect();

    // O1: scope-extract per-handle borrows around the whole while
    // loop when the alias oracle proves it sound. The cond is part of
    // the access scan; opened before cond emission so the condition
    // also indexes slices.
    let hoist_plan = super::hoist::plan_borrow_hoist(stmt, Some(cond), body, ctx, interner);
    super::hoist::emit_hoist_open(&hoist_plan, interner, &indent_str, ctx, &mut output);

    let cond_str = codegen_expr_with_async_oracle(
        cond,
        interner,
        synced_vars,
        async_functions,
        ctx.get_variable_types(),
        ctx.oracle(),
    );
    let cond_str = truthy_cond_wrap(cond, cond_str, interner, ctx.get_variable_types());
    let loop_start = output.len();
    writeln!(output, "{}while {} {{", indent_str, cond_str).unwrap();
    emit_bounds_hint_header(
        &while_bounds_hints,
        interner,
        &"    ".repeat(indent + 1),
        &mut output,
    );
    emit_affine_offset_header(
        &affine_guards,
        interner,
        synced_vars,
        async_functions,
        ctx.get_variable_types(),
        &"    ".repeat(indent + 1),
        &mut output,
    );
    ctx.push_scope();

    // OPT-5: Extract sentinel context from while condition.
    // For `While counter < limit:`, detect `Set counter to limit` inside If blocks
    // as sentinel exits and replace with `break`.
    let sentinel_ctx: Option<(Symbol, &Expr)> = match cond {
        Expr::BinaryOp {
            op: BinaryOpKind::Lt,
            left,
            right,
        } => {
            if let Expr::Identifier(sym) = left {
                Some((*sym, *right))
            } else {
                None
            }
        }
        _ => None,
    };

    // Peephole: process body statements with peephole optimizations.
    let body_refs: Vec<&Stmt> = body.iter().collect();
    let mut bi = 0;
    while bi < body_refs.len() {
        if let Some((code, skip)) = super::peephole::try_block_peepholes(
            &body_refs,
            bi,
            interner,
            indent + 1,
            mutable_vars,
            ctx,
            lww_fields,
            mv_fields,
            synced_vars,
            var_caps,
            async_functions,
            pipe_vars,
            boxed_fields,
            registry,
            type_env,
        ) {
            output.push_str(&code);
            bi += 1 + skip;
            continue;
        }
        // Drain-tail optimization: If a branch in the while body is a
        // loop-invariant sequential drain (push-from-array + increment), emit
        // extend_from_slice + break instead of element-by-element push.
        if let Some(code) = try_emit_drain_tail_in_while(
            body_refs[bi],
            cond,
            interner,
            indent + 1,
            mutable_vars,
            ctx,
            lww_fields,
            mv_fields,
            synced_vars,
            var_caps,
            async_functions,
            pipe_vars,
            boxed_fields,
            registry,
            type_env,
        ) {
            output.push_str(&code);
            bi += 1;
            continue;
        }
        // OPT-5: Sentinel → break transformation.
        // If this statement is an If whose then_block ends with `Set counter to limit`,
        // emit break instead of the sentinel set.
        if let Some((counter_sym, limit_expr)) = &sentinel_ctx {
            if let Some(code) = try_emit_sentinel_break(
                body_refs[bi],
                *counter_sym,
                limit_expr,
                interner,
                indent + 1,
                mutable_vars,
                ctx,
                lww_fields,
                mv_fields,
                synced_vars,
                var_caps,
                async_functions,
                pipe_vars,
                boxed_fields,
                registry,
                type_env,
            ) {
                // This peephole emits the If inline (bypassing codegen_stmt),
                // so the oracle bounds hints for index accesses in its guard
                // (string_search's `text[i+j]`) must be emitted here too.
                output.push_str(&emit_oracle_index_hints(
                    body_refs[bi],
                    ctx,
                    interner,
                    registry,
                    &"    ".repeat(indent + 1),
                ));
                output.push_str(&code);
                bi += 1;
                continue;
            }
        }
        output.push_str(&codegen_stmt(
            body_refs[bi],
            interner,
            indent + 1,
            mutable_vars,
            ctx,
            lww_fields,
            mv_fields,
            synced_vars,
            var_caps,
            async_functions,
            pipe_vars,
            boxed_fields,
            registry,
            type_env,
        ));
        bi += 1;
    }
    ctx.pop_scope();

    // Restore original variable_types for hoisted symbols.
    // variable_types is a flat HashMap (not scoped), so push/pop_scope doesn't clean it.
    for (sym, old_type) in hoisted_syms {
        if let Some(old) = old_type {
            ctx.register_variable_type(sym, old);
        } else {
            ctx.get_variable_types_mut().remove(&sym);
        }
    }

    // Stop treating the hoisted scratch buffers as reused once their
    // loop closes, so a same-named buffer in a sibling loop is not
    // mis-rewritten into a refill of a buffer that was never hoisted.
    for dst in scratch_hoisted {
        ctx.clear_scratch_hoist(dst);
    }

    writeln!(output, "{}}}", indent_str).unwrap();
    super::peephole::version_on_bounds_hints(
        &mut output,
        loop_start,
        &while_bounds_hints,
        body,
        ctx.oracle(),
        interner,
        &indent_str,
    );
    // Close the borrow-hoist scope and restore slice-shadowed types.
    super::hoist::emit_hoist_close(&hoist_plan, &indent_str, ctx, &mut output);
    output
}

/// Mark the body's accesses `item counter of arr` that a literal range proves in range: `counter` runs over
/// `lo..=hi` with `1 <= lo`, `arr` is a fixed-size `[T; N]` with `hi <= N`, and nothing in the body binds
/// `counter` anew, so every such access names an item. A closure or a nested function is another scope, where
/// `counter` may name a parameter; its accesses keep the rule.
fn mark_fixed_array_counter_accesses(
    counter: Symbol,
    iterable: &Expr,
    body: &[Stmt],
    var_types: &HashMap<Symbol, String>,
) {
    let Expr::Range { start, end } = iterable else {
        return;
    };
    let (Expr::Literal(Literal::Number(lo)), Expr::Literal(Literal::Number(hi))) = (start, end)
    else {
        return;
    };
    if *lo < 1 || crate::optimize::WriteSet::of_block(body).may_write(counter) {
        return;
    }
    let fixed_len = |arr: Symbol| -> Option<i64> {
        let t = var_types.get(&arr)?;
        let t = t.split("|__hl:").next().unwrap_or(t.as_str());
        t.strip_prefix('[')?
            .strip_suffix(']')?
            .rsplit_once("; ")?
            .1
            .trim()
            .parse()
            .ok()
    };
    use logicaffeine_language::ast_depth::{walk_nodes, Visited, Walk};
    walk_nodes(body, |node| {
        let (arr, index) = match node {
            Visited::Stmt(Stmt::FunctionDef { .. }) | Visited::Expr(Expr::Closure { .. }) => {
                return Walk::Past
            }
            Visited::Expr(Expr::Index {
                collection: Expr::Identifier(arr),
                index,
            }) => (*arr, *index),
            Visited::Stmt(Stmt::SetIndex {
                collection: Expr::Identifier(arr),
                index,
                ..
            }) => (*arr, *index),
            _ => return Walk::Into,
        };
        if matches!(index, Expr::Identifier(s) if *s == counter)
            && fixed_len(arr).is_some_and(|n| *hi <= n)
        {
            super::expr::mark_index_in_range(index);
        }
        Walk::Into
    });
}

/// The `Repeat` arm of [`codegen_stmt`]: an arm of [`codegen_stmt`], in a frame of its own — see [`codegen_let_stmt`].
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn codegen_repeat_stmt<'a>(
    mut output: String,
    repeat: &Stmt<'a>,
    pattern: &Pattern,
    iterable: &&'a Expr<'a>,
    body: &Block<'a>,
    interner: &Interner,
    indent: usize,
    mutable_vars: &HashSet<Symbol>,
    ctx: &mut RefinementContext<'a>,
    lww_fields: &HashSet<(String, String)>,
    mv_fields: &HashSet<(String, String)>,
    synced_vars: &mut HashSet<Symbol>,
    var_caps: &HashMap<Symbol, VariableCapabilities>,
    async_functions: &HashSet<Symbol>,
    pipe_vars: &HashSet<Symbol>,
    boxed_fields: &HashSet<(String, String, String)>,
    registry: &TypeRegistry,
    type_env: &crate::analysis::types::TypeEnv,
    indent_str: &str,
    names: &RustNames,
) -> String {
    use crate::ast::stmt::Pattern;

    // The loop's bindings. A body that sets one (`Set t to t + 10`) changes it for the rest of that pass,
    // and the next pass binds the next item, so Rust needs that binding `mut`.
    let written = crate::optimize::WriteSet::of_block(body);
    let binding = |sym: Symbol| {
        let name = interner.resolve(sym);
        if written.may_write(sym) {
            format!("mut {name}")
        } else {
            name.to_string()
        }
    };
    let pattern_str = match pattern {
        Pattern::Identifier(sym) => binding(*sym),
        Pattern::Tuple(syms) => {
            let names = syms.iter().map(|s| binding(*s)).collect::<Vec<_>>().join(", ");
            format!("({})", names)
        }
    };

    // A `Repeat` over a value of the exact extension binds each of its items, a `LogosValue`.
    if super::tower::is_tower_expr(iterable, ctx.get_variable_types(), interner) {
        let items = super::tower::statement_value(
            iterable,
            interner,
            synced_vars,
            async_functions,
            boxed_fields,
            registry,
            ctx.get_variable_types(),
        );
        let bound: Vec<Symbol> = match pattern {
            Pattern::Identifier(sym) => {
                writeln!(
                    output,
                    "{}for {} in logicaffeine_data::failure::answered(({}).items()) {{",
                    indent_str, pattern_str, items
                )
                .unwrap();
                vec![*sym]
            }
            Pattern::Tuple(syms) => {
                writeln!(
                    output,
                    "{}for __tower_item in logicaffeine_data::failure::answered(({}).items()) {{",
                    indent_str, items
                )
                .unwrap();
                for (k, sym) in syms.iter().enumerate() {
                    writeln!(
                        output,
                        "{}    let {}: {} = logicaffeine_data::failure::answered(__tower_item.item(&LogosValue::Int({}_i64)));",
                        indent_str,
                        binding(*sym),
                        super::tower::VALUE,
                        k + 1
                    )
                    .unwrap();
                }
                syms.clone()
            }
        };
        ctx.push_scope();
        let shadowed: Vec<(Symbol, Option<String>)> = bound
            .iter()
            .map(|sym| (*sym, ctx.get_variable_types().get(sym).cloned()))
            .collect();
        for sym in &bound {
            ctx.register_variable_type(*sym, super::tower::VALUE.to_string());
        }
        for s in body.iter() {
            output.push_str(&codegen_stmt(
                s,
                interner,
                indent + 1,
                mutable_vars,
                ctx,
                lww_fields,
                mv_fields,
                synced_vars,
                var_caps,
                async_functions,
                pipe_vars,
                boxed_fields,
                registry,
                type_env,
            ));
        }
        for (sym, before) in shadowed {
            match before {
                Some(t) => ctx.register_variable_type(sym, t),
                None => {
                    ctx.get_variable_types_mut().remove(&sym);
                }
            }
        }
        ctx.pop_scope();
        writeln!(output, "{}}}", indent_str).unwrap();
        return output;
    }

    // A `Repeat` over a value-or-nothing list iterates the list it holds; one over a list of them binds each item as one.
    let held = match pattern {
        Pattern::Identifier(item) => {
            let ecx = operand_ctx(interner, ctx, synced_vars, async_functions, boxed_fields, registry);
            super::absent::iteration(iterable, &ecx).map(|(items, ty)| (*item, items, ty))
        }
        Pattern::Tuple(_) => None,
    };
    if let Some((item, items, ty)) = held {
        writeln!(output, "{}for {} in {} {{", indent_str, pattern_str, items).unwrap();
        ctx.push_scope();
        let before = ctx.get_variable_types().get(&item).cloned();
        ctx.register_variable_type(item, ty);
        for s in body.iter() {
            output.push_str(&codegen_stmt(
                s,
                interner,
                indent + 1,
                mutable_vars,
                ctx,
                lww_fields,
                mv_fields,
                synced_vars,
                var_caps,
                async_functions,
                pipe_vars,
                boxed_fields,
                registry,
                type_env,
            ));
        }
        match before {
            Some(t) => ctx.register_variable_type(item, t),
            None => {
                ctx.get_variable_types_mut().remove(&item);
            }
        }
        ctx.pop_scope();
        writeln!(output, "{}}}", indent_str).unwrap();
        return output;
    }

    // Indexed-buffer zero-init loop: dropped — the `let mut buf: [T; N] = [0; N]` the O3 `Let`
    // handler already emitted zeroes every slot, and the subsequent `Set item i of buf` writes hit
    // the array directly. (A push-fill of a `[T; N]` array would be wrong: the fill counter is
    // compile-time, so a runtime loop would set slot 0 repeatedly.)
    if super::affine_array::is_indexed_init_loop(body, |s| ctx.is_indexed_buffer(s)) {
        return output;
    }

    // Scratch-buffer scalarization: a counted loop whose sole job is to fill a registered
    // fixed-size, non-escaping buffer is emitted in place as `let w: [T; N] = from_fn(|__k| { let
    // j = LO + __k; <body-lets>; <push-val> })` — a single stack array — instead of a `Vec` that a
    // per-entry push loop reallocates on every entry. All non-push body statements (intermediate
    // `Let`s) become the closure's leading `let`s; the sole `Push`'s value is the tail expression.
    if let Pattern::Identifier(loop_var) = pattern {
        if let Some((w, info)) = scratch_fill_target(body, ctx) {
            let wname = names.ident(w);
            writeln!(
                output,
                "{}let {}{}: [{}; {}] = ::std::array::from_fn(|__k: usize| {{",
                indent_str,
                if info.mutated { "mut " } else { "" },
                wname,
                info.elem_ty,
                info.len
            )
            .unwrap();
            writeln!(
                output,
                "{}    let {} = ({} as i64) + (__k as i64);",
                indent_str,
                interner.resolve(*loop_var),
                info.lo
            )
            .unwrap();
            ctx.push_scope();
            ctx.register_variable_type(*loop_var, "i64".to_string());
            let mut push_val: Option<&Expr> = None;
            for s in body.iter() {
                match s {
                    Stmt::Push {
                        collection: Expr::Identifier(c),
                        value,
                    } if *c == w => {
                        push_val = Some(value);
                    }
                    other => output.push_str(&codegen_stmt(
                        other,
                        interner,
                        indent + 1,
                        mutable_vars,
                        ctx,
                        lww_fields,
                        mv_fields,
                        synced_vars,
                        var_caps,
                        async_functions,
                        pipe_vars,
                        boxed_fields,
                        registry,
                        type_env,
                    )),
                }
            }
            let val = codegen_expr_boxed_with_types_oracle(
                push_val.expect("scratch fill loop has exactly one push"),
                interner,
                synced_vars,
                boxed_fields,
                registry,
                async_functions,
                ctx.get_string_vars(),
                ctx.get_variable_types(),
                ctx.get_fast_div(),
                ctx.oracle(),
            );
            writeln!(output, "{}    {}", indent_str, val).unwrap();
            ctx.pop_scope();
            writeln!(output, "{}}});", indent_str).unwrap();
            return output;
        }
    }

    let iter_str = codegen_expr_with_async(
        iterable,
        interner,
        synced_vars,
        async_functions,
        ctx.get_variable_types(),
    );
    if let Pattern::Identifier(counter) = pattern {
        mark_fixed_array_counter_accesses(*counter, iterable, body, ctx.get_variable_types());
    }

    // OPT-PAR: Automatic parallelization for commutative reduction patterns.
    // Detect: Repeat for x in coll: Set acc = acc + expr(x)
    // Emit: acc += coll.par_iter().copied().map(|x| expr).sum::<i64>()
    let par_emitted = if let Pattern::Identifier(var_sym) = pattern {
        if let Some(par_code) = try_emit_parallel_reduction(
            *var_sym,
            &pattern_str,
            &iter_str,
            iterable,
            body,
            interner,
            indent,
            ctx,
            synced_vars,
            async_functions,
        ) {
            output.push_str(&par_code);
            true
        } else {
            false
        }
    } else {
        false
    };

    if par_emitted {
        // Parallel code already emitted — skip normal loop codegen
    } else if body
        .iter()
        .any(|s| requires_async_stmt(s) || calls_async_function(s, async_functions))
    {
        // Use while-let with explicit iterator for async compatibility
        writeln!(
            output,
            "{}let mut __iter = ({}).into_iter();",
            indent_str, iter_str
        )
        .unwrap();
        writeln!(
            output,
            "{}while let Some({}) = __iter.next() {{",
            indent_str, pattern_str
        )
        .unwrap();
    } else {
        // Determine iteration strategy based on collection type
        let is_logos_seq = if let Expr::Identifier(coll_sym) = iterable {
            ctx.get_variable_types()
                .get(coll_sym)
                .map(|s| s.split("|__hl:").next().unwrap_or(s.as_str()))
                .map_or(false, |t| t.starts_with("LogosSeq"))
        } else {
            false
        };
        // `TextBytes` is the zero-copy `&[u8]` byte-view (Opt::TextBytesView): iterating it
        // yields each byte widened to the `i64` the body expects, over the borrowed slice.
        let is_text_bytes = if let Expr::Identifier(coll_sym) = iterable {
            ctx.get_variable_types()
                .get(coll_sym)
                .map(|s| s.split("|__hl:").next().unwrap_or(s.as_str()))
                .map_or(false, |t| t == "TextBytes")
        } else {
            false
        };

        // ★ `For each c in <Text>` WALKS CHARACTERS. A `String` is not an iterator, so the default
        // `.clone()` below emitted Rust that does not compile — and this sentence is how the whole
        // standard library reads text: `htmlEscape` escapes one character at a time, `isIntText`
        // validates one at a time, `urlDecode` scans for an escape. None of them could be built
        // for the AOT tier, and nothing said so, because the refusal is invalid Rust in a string
        // the compiler returns as `Ok`.
        //
        // Each `c` is a one-character TEXT, which is what the body expects: LOGOS compares it with
        // `c is equal to "&"` and concatenates it. CHARACTERS, not bytes — a check mark is three
        // UTF-8 bytes and one character, and a byte walk would hand the body a fragment that is
        // not a character at all.
        let iterates_text = matches!(
            super::types::infer_logos_type(iterable, interner, ctx.get_variable_types()),
            crate::analysis::types::LogosType::String
        );
        if let (Pattern::Identifier(_), Expr::Range { start, end }) = (pattern, iterable) {
            let emit = |e: &Expr| {
                let ecx = super::expr::ExprCtx {
                    async_functions,
                    variable_types: ctx.get_variable_types(),
                    ..super::expr::ExprCtx::bare(interner, synced_vars)
                };
                super::rational::integer_operand(e, crate::semantics::collections::IntegerPlace::Range, &ecx)
                    .unwrap_or_else(|| {
                        codegen_expr_with_async(
                            e,
                            interner,
                            synced_vars,
                            async_functions,
                            ctx.get_variable_types(),
                        )
                    })
            };
            let (start_str, end_str) = super::expr::range_bound_strs(start, end, emit);
            let range =
                super::expr::counted_loop_range(start, end, &start_str, &end_str, ctx.oracle());
            writeln!(output, "{}for {} in {} {{", indent_str, pattern_str, range).unwrap();
        } else if iterates_text {
            // The bound name is a `String`, and saying so is what makes the rest of the body
            // compile: a non-Copy loop variable read twice needs the `.clone()` the identifier
            // emitter adds only for a variable whose type it knows. `htmlEscape`'s
            // `Let mutable piece be c.` followed by `If c is equal to "&"` is exactly that shape.
            if let Pattern::Identifier(sym) = pattern {
                ctx.register_variable_type(*sym, "String".to_string());
            }
            writeln!(
                output,
                "{}for {} in {}.chars().map(|__c| __c.to_string()).collect::<Vec<String>>() {{",
                indent_str, pattern_str, iter_str
            )
            .unwrap();
        } else if is_text_bytes {
            // ── OPT TextBytesView (iteration) ───────────────────────────────
            // `for b in sb` over the borrowed `&[u8]` view yields each byte widened
            // to `i64` — no `Vec<i64>`, no clone. The loop variable is an `i64`.
            if let Pattern::Identifier(sym) = pattern {
                ctx.register_variable_type(*sym, "i64".to_string());
            }
            writeln!(
                output,
                "{}for {} in {}.iter().map(|&__x| __x as i64) {{",
                indent_str, pattern_str, iter_str
            )
            .unwrap();
        } else if is_logos_seq {
            // A `Seq` nothing in the loop can change is iterated through a shared borrow of its buffer, which is the
            // snapshot every tier iterates (`optimize::iteration`); `.copied()` for a `Copy` element, the lazy
            // per-element `.cloned()` otherwise. Any other loop iterates an owned copy taken on entry.
            let coll = match iterable {
                Expr::Identifier(xs) => Some(*xs),
                _ => None,
            };
            let lent = crate::optimize::facts::holds_stmt(
                crate::optimize::facts::Fact::LentIteration,
                repeat,
            );
            if lent {
                crate::optimize::mark_fired(crate::optimization::Opt::Borrow);
                let seq_ty = coll
                    .and_then(|xs| ctx.get_variable_types().get(&xs))
                    .cloned()
                    .unwrap_or_default();
                let held = seq_ty.split("|__hl:").next().unwrap_or(&seq_ty);
                let elem = held
                    .strip_prefix("LogosSeq<")
                    .and_then(|t| t.strip_suffix('>'))
                    .unwrap_or("_");
                let each = if is_copy_type(elem) {
                    "copied"
                } else {
                    "cloned"
                };
                writeln!(
                    output,
                    "{}for {} in {}.borrow().iter().{}() {{",
                    indent_str, pattern_str, iter_str, each
                )
                .unwrap();
            } else {
                writeln!(
                    output,
                    "{}for {} in {}.to_vec() {{",
                    indent_str, pattern_str, iter_str
                )
                .unwrap();
            }
        } else {
            // Optimization: for known Vec<T>/&[T] with Copy element type,
            // use .iter().copied() instead of .clone() to avoid copying the entire collection.
            // For slices, must use .iter() since .clone() on &[T] clones the reference, not the data.
            let (use_iter_copied, use_iter_cloned) = if let Expr::Identifier(coll_sym) = iterable {
                if let Some(coll_type_raw) = ctx.get_variable_types().get(coll_sym) {
                    // Strip |__hl: hoisting suffix so strip_suffix(']') works correctly.
                    let coll_type = coll_type_raw
                        .split("|__hl:")
                        .next()
                        .unwrap_or(coll_type_raw.as_str());
                    if coll_type.starts_with('[') {
                        // O3 fixed array `[T; N]` — iterate by borrow so the array is not consumed.
                        let elem = coll_type
                            .strip_prefix('[')
                            .and_then(|s| s.split(';').next())
                            .unwrap_or("_")
                            .trim();
                        if is_copy_type(elem) {
                            (true, false)
                        } else {
                            (false, true)
                        }
                    } else if coll_type.starts_with("&[") {
                        // Borrowed slice `&[T]` or fixed-array ref `&[T; N]`: must use .iter() (can't
                        // move/clone a borrowed slice). Take the element type before any `; N` length so
                        // a `&[i64; 3]` param iterates by `.copied()` exactly like the `&[i64]` slice.
                        let inner = coll_type
                            .strip_prefix("&[")
                            .and_then(|s| s.strip_suffix(']'))
                            .unwrap_or("_");
                        let elem = inner.split(';').next().unwrap_or(inner).trim();
                        if is_copy_type(elem) {
                            (true, false)
                        } else {
                            (false, true)
                        }
                    } else if coll_type.starts_with("Vec")
                        && has_copy_element_type(coll_type)
                        && !body_mutates_collection(body, *coll_sym)
                    {
                        (true, false)
                    } else if coll_type.starts_with("Vec")
                        && !body_mutates_collection(body, *coll_sym)
                    {
                        // Non-Copy element type, but body doesn't mutate the collection:
                        // use .iter().cloned() for lazy per-element cloning instead of
                        // .clone() which copies the entire collection upfront.
                        (false, true)
                    } else {
                        (false, false)
                    }
                } else {
                    (false, false)
                }
            } else {
                (false, false)
            };

            if use_iter_copied {
                writeln!(
                    output,
                    "{}for {} in {}.iter().copied() {{",
                    indent_str, pattern_str, iter_str
                )
                .unwrap();
            } else if use_iter_cloned {
                writeln!(
                    output,
                    "{}for {} in {}.iter().cloned() {{",
                    indent_str, pattern_str, iter_str
                )
                .unwrap();
            } else {
                // Clone the collection before iterating to avoid moving it.
                // This allows the collection to be reused after the loop.
                writeln!(
                    output,
                    "{}for {} in {}.clone() {{",
                    indent_str, pattern_str, iter_str
                )
                .unwrap();
            }
        }
    }
    if !par_emitted {
        ctx.push_scope();
        // A loop over a list of rationals binds each item as the rational it is.
        if let Pattern::Identifier(pvar) = pattern {
            if let Some(list) = super::rational::rational_type_of(iterable, super::rational::Typing { interner, variable_types: ctx.get_variable_types(), registry }) {
                if let crate::analysis::types::LogosType::Seq(item) = super::rational::named(&list) {
                    ctx.register_variable_type(*pvar, item.to_rust_type());
                }
            }
        }
        // Register a VALUE-TYPE loop variable's element type so `is_money_expr`/`is_quantity_expr`/
        // etc. recognize it — this is what lets `Set total to total + m` reduce over a `Seq of Money`
        // (else `m` is untyped and `total + m` lowers to a raw Rust `+` → E0369). Surgical: fires
        // ONLY for the exact-value element types, so scalar/struct loops are entirely unchanged.
        if let (Pattern::Identifier(pvar), Expr::Identifier(coll_sym)) = (pattern, iterable) {
            if let Some(ct) = ctx.get_variable_types().get(coll_sym).cloned() {
                let inner = ct.split("|__hl:").next().unwrap_or(ct.as_str());
                let elem = inner
                    .strip_prefix("LogosSeq<")
                    .and_then(|s| s.strip_suffix('>'))
                    .or_else(|| inner.strip_prefix("Vec<").and_then(|s| s.strip_suffix('>')))
                    // A `Seq of T` PARAMETER arrives as a borrowed slice `&[T]` (or `&[T; N]`),
                    // which the two owned forms above do not match — and a parameter is exactly
                    // where a loop over a table of records comes from.
                    .or_else(|| {
                        inner
                            .strip_prefix("&[")
                            .and_then(|s| s.strip_suffix(']'))
                            .map(|s| s.split(';').next().unwrap_or(s))
                    })
                    .or_else(|| {
                        inner
                            .strip_prefix('[')
                            .and_then(|s| s.strip_suffix(']'))
                            .map(|s| s.split(';').next().unwrap_or(s))
                    })
                    .map(str::trim);
                if let Some(e) = elem {
                    if matches!(
                        e,
                        "LogosMoney"
                            | "LogosQuantity"
                            | "LogosModular"
                            | "LogosUuid"
                            | "LogosComplex"
                            | "LogosDecimal"
                    ) {
                        ctx.register_variable_type(*pvar, e.to_string());
                    } else if registry.iter_types().any(|(sym, def)| {
                        matches!(def, TypeDef::Struct { .. })
                            && crate::codegen::types::rust_type_ident(interner.resolve(*sym)) == e
                    }) && !ctx.get_variable_types().contains_key(pvar)
                    {
                        // ★ A RECORD loop element must be typed too, and for the same reason the
                        // note above gives for `String`: without a type, a FIELD READ of the row
                        // (`t's text`) is emitted as a bare move, so a sibling branch that uses the
                        // whole row fails to build — "use of partially moved value". That is the
                        // ordinary "change one row, keep the others" loop, and it could not be
                        // compiled for the AOT tier at all. The type is also what tells the
                        // concatenation path that `p's left + p's right` is TEXT rather than a
                        // numeric `+`, which is the same blind spot in its other position.
                        ctx.register_variable_type(*pvar, e.to_string());
                    } else if e == "String" && !ctx.get_variable_types().contains_key(pvar) {
                        // A `String` loop element (`Repeat for part in sfParts` over a `Seq of Text`)
                        // must be typed so a call-arg read of it (`removePrefix(part, …)`) clones by
                        // value — the Set/OPT-1C call codegen moves an UNKNOWN-typed identifier, and
                        // a loop variable read across sibling branches would then be a use-after-move.
                        ctx.register_variable_type(*pvar, "String".to_string());
                    } else if e.starts_with("LogosTuple ")
                        && !ctx.get_variable_types().contains_key(pvar)
                    {
                        // A tuple element keeps its marker, so `item N of t` inside the loop gives
                        // each element back as itself.
                        ctx.register_variable_type(*pvar, e.to_string());
                    }
                }
            }
        }
        // Peephole: process body statements with swap, seq-copy, and rotate-left detection
        {
            let body_refs: Vec<&Stmt> = body.iter().collect();
            let mut bi = 0;
            while bi < body_refs.len() {
                if let Some((code, skip)) = super::peephole::try_block_peepholes(
                    &body_refs,
                    bi,
                    interner,
                    indent + 1,
                    mutable_vars,
                    ctx,
                    lww_fields,
                    mv_fields,
                    synced_vars,
                    var_caps,
                    async_functions,
                    pipe_vars,
                    boxed_fields,
                    registry,
                    type_env,
                ) {
                    output.push_str(&code);
                    bi += 1 + skip;
                    continue;
                }
                output.push_str(&codegen_stmt(
                    body_refs[bi],
                    interner,
                    indent + 1,
                    mutable_vars,
                    ctx,
                    lww_fields,
                    mv_fields,
                    synced_vars,
                    var_caps,
                    async_functions,
                    pipe_vars,
                    boxed_fields,
                    registry,
                    type_env,
                ));
                bi += 1;
            }
        }
        ctx.pop_scope();
        writeln!(output, "{}}}", indent_str).unwrap();
    }
    output
}

/// The `Inspect` arm of [`codegen_stmt`]: an arm of [`codegen_stmt`], in a frame of its own — see [`codegen_let_stmt`].
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn codegen_inspect_stmt<'a>(
    mut output: String,
    target: &&'a Expr<'a>,
    arms: &[MatchArm<'a>],
    interner: &Interner,
    indent: usize,
    mutable_vars: &HashSet<Symbol>,
    ctx: &mut RefinementContext<'a>,
    lww_fields: &HashSet<(String, String)>,
    mv_fields: &HashSet<(String, String)>,
    synced_vars: &mut HashSet<Symbol>,
    var_caps: &HashMap<Symbol, VariableCapabilities>,
    async_functions: &HashSet<Symbol>,
    pipe_vars: &HashSet<Symbol>,
    boxed_fields: &HashSet<(String, String, String)>,
    registry: &TypeRegistry,
    type_env: &crate::analysis::types::TypeEnv,
    indent_str: &str,
) -> String {
    let target_str = codegen_expr_with_async(
        target,
        interner,
        synced_vars,
        async_functions,
        ctx.get_variable_types(),
    );

    // ★ THE SCRUTINEE IS BOUND FIRST, so its temporaries die at the semicolon.
    //
    // An `Inspect` emits a bare `match` with no trailing semicolon, and nothing is emitted after a
    // function's last statement — so a TERMINAL `Inspect` becomes the function's TAIL EXPRESSION.
    // A `Seq` argument at a borrow-role index is emitted `&*xs.borrow()`, and in edition 2021 (the
    // edition every generated crate pins) a tail expression's temporaries are dropped AFTER the
    // block's locals. The `Ref` guard therefore outlived the local it borrowed:
    // `error[E0597]: does not live long enough`, in generated code, for a program the tree-walker
    // and the VM both run.
    //
    // Binding to a local ends the borrow at the `let`, which is where it always belonged. It costs
    // nothing — the match still reads the same value — and it makes the emission independent of
    // whether this `Inspect` happens to be last, which is the property that made the bug so hard
    // to see: the same statement was correct anywhere but the end.
    //
    // (Rust 2024's RFC 3606 changes the tail-expression rule and would have made this unnecessary;
    // the generated crates are 2021 and this must not depend on that.)
    // ★ ONLY A SCRUTINEE THAT PRODUCES A TEMPORARY IS HOISTED, and the first version of this fix
    // got that wrong in a way worth recording. Binding unconditionally — `let __inspect_target =
    // e;` — MOVES a place expression that `match e { … }` merely matched on, so every `Inspect e:`
    // followed by a later use of `e` became `error[E0382]: borrow of moved value`. It turned one
    // borrow bug into a move bug across the whole partial-evaluation corpus.
    //
    // A CALL is the shape that needs it: its result is a temporary, and a `Seq` argument inside it
    // is emitted `&*xs.borrow()`, whose guard would otherwise outlive the block's locals when the
    // `Inspect` is the function's tail expression. A CONSTRUCTION is a temporary too, and a struct
    // literal is not a scrutinee Rust accepts (`match Json::JNum { n } {`). A variable or a field read
    // owns no temporary and is left exactly as it was.
    let hoist = matches!(
        target,
        Expr::Call { .. } | Expr::CallExpr { .. } | Expr::New { .. } | Expr::NewVariant { .. }
    );
    // A lent scrutinee (`&T`, a lent parameter or binding, `codegen::lent`) is matched through the reference: its arms bind
    // by `ref`, so matching the place `*x` moves nothing.
    let lent_scrutinee = matches!(target, Expr::Identifier(t)
        if ctx.get_variable_types().get(t).is_some_and(|ty| super::expr::is_lent_record(ty)));
    if hoist {
        // One fixed name is safe: a NESTED `Inspect` shadows it, and the outer match has already
        // evaluated its own scrutinee by the time any inner one is declared.
        writeln!(
            output,
            "{}let __inspect_target = {};",
            indent_str, target_str
        )
        .unwrap();
        writeln!(output, "{}match __inspect_target {{", indent_str).unwrap();
    } else if lent_scrutinee {
        writeln!(output, "{}match *{} {{", indent_str, target_str).unwrap();
    } else {
        writeln!(output, "{}match {} {{", indent_str, target_str).unwrap();
    }

    for arm in arms {
        // Phase 102: Track which bindings come from boxed fields for inner Inspects
        // Use NAMES (strings) not symbols, because parser may create different symbols
        // for the same identifier in different syntactic positions.
        // Must be per-arm so bindings from other arms don't leak into this arm's scope.
        let mut inner_boxed_binding_names: HashSet<String> = HashSet::new();
        // The RAW interned enum name keys the boxed-field lookup: its keys are built from the same raw name.
        if let (Some(variant), Some(enum_name)) =
            (arm.variant, enum_name_of_arm(arm, registry, interner))
        {
            for (field, binding) in arm.bindings.iter() {
                let key = (
                    enum_name.clone(),
                    interner.resolve(variant).to_string(),
                    interner.resolve(*field).to_string(),
                );
                if boxed_fields.contains(&key) {
                    inner_boxed_binding_names.insert(interner.resolve(*binding).to_string());
                }
            }
        }
        let (pattern, prelude) = inspect_arm_pattern(arm, registry, interner);
        writeln!(output, "{}    {} => {{", indent_str, pattern).unwrap();
        if !prelude.is_empty() {
            writeln!(output, "{}        {}", indent_str, prelude).unwrap();
        }

        ctx.push_scope();

        // A binding of one of the program's enum types that the arm only inspects or lends on (`codegen::lent`) is
        // bound as a borrow of the matched field — no copy of the subtree it names.
        let mut lent_bindings: HashSet<Symbol> = HashSet::new();

        // Register pattern-bound variable types from Inspect arms so that
        // type inference (e.g. for mixed Int/Real coercion) works correctly.
        if let Some(variant_sym) = arm.variant {
            if let Some((_enum_name, variant_def)) = registry.find_variant(variant_sym) {
                for (field_sym, binding_sym) in &arm.bindings {
                    if let Some(field_def) =
                        variant_def.fields.iter().find(|f| f.name == *field_sym)
                    {
                        let rust_type = super::types::codegen_field_type(&field_def.ty, interner);
                        let lendable = field_is_user_enum(&field_def.ty, registry)
                            && !mutable_vars.contains(binding_sym)
                            && super::lent::only_borrowed_uses(
                                arm.body,
                                *binding_sym,
                                interner,
                                &|f, j| {
                                    super::expr::lends_record_at(f, j, ctx.get_variable_types())
                                },
                            );
                        if lendable {
                            lent_bindings.insert(*binding_sym);
                            ctx.register_variable_type(*binding_sym, format!("&{}", rust_type));
                        } else {
                            ctx.register_variable_type(*binding_sym, rust_type);
                        }
                    }
                }
            }
        }
        for sym in &lent_bindings {
            inner_boxed_binding_names.remove(interner.resolve(*sym));
        }

        // An arm's bindings are owned copies of the matched fields, rebound `mut` when the arm
        // writes one — pushes to it, sets its items, lends it to a `mutable` parameter — since
        // each write first copies the binding on write. Matched by name, as the boxed set is.
        let written = |name: &str| {
            if mutable_vars.iter().any(|s| interner.resolve(*s) == name) {
                "mut "
            } else {
                ""
            }
        };

        // Generate explicit dereferences for boxed bindings at the start of the arm
        // With ref bindings, boxed fields need double deref: ref -> Box -> inner
        for binding_name in &inner_boxed_binding_names {
            let ident = escape_rust_ident(binding_name);
            writeln!(
                output,
                "{}        let {}{} = (**{}).clone();",
                indent_str,
                written(binding_name),
                ident,
                ident
            )
            .unwrap();
        }

        // Clone non-boxed ref bindings to get owned values; a lent binding stays the `ref` borrow the pattern made,
        // seen through its box when the field is boxed.
        if let Some(_) = arm.variant {
            for (_field, binding) in &arm.bindings {
                let binding_name = interner.resolve(*binding);
                let ident = escape_rust_ident(binding_name);
                if lent_bindings.contains(binding) {
                    if arm_binding_is_boxed(
                        arm,
                        *binding,
                        enum_name_of_arm(arm, registry, interner).as_deref(),
                        boxed_fields,
                        interner,
                    ) {
                        writeln!(
                            output,
                            "{}        let {} = &**{};",
                            indent_str, ident, ident
                        )
                        .unwrap();
                    }
                } else if !inner_boxed_binding_names.contains(binding_name) {
                    writeln!(
                        output,
                        "{}        let {}{} = {}.clone();",
                        indent_str,
                        written(binding_name),
                        ident,
                        ident
                    )
                    .unwrap();
                }
            }
        }

        for stmt in arm.body {
            // Phase 102: Handle inner Inspect statements with boxed bindings
            // Note: Since we now dereference boxed bindings at the start of the arm,
            // inner matches don't need the `*` dereference operator.
            let inner_stmt_code = if let Stmt::Inspect {
                target: inner_target,
                ..
            } = stmt
            {
                // Check if the inner target is a boxed binding (already dereferenced above)
                // Use name comparison since symbols may differ between binding and reference
                if let Expr::Identifier(sym) = inner_target {
                    let target_name = interner.resolve(*sym);
                    if inner_boxed_binding_names.contains(target_name) {
                        // Generate match (binding was already dereferenced at arm start)
                        let mut inner_output = String::new();
                        writeln!(
                            inner_output,
                            "{}match {} {{",
                            "    ".repeat(indent + 2),
                            escape_rust_ident(target_name)
                        )
                        .unwrap();

                        if let Stmt::Inspect {
                            arms: inner_arms, ..
                        } = stmt
                        {
                            for inner_arm in inner_arms.iter() {
                                let (pattern, prelude) =
                                    inspect_arm_pattern(inner_arm, registry, interner);
                                let inner_indent = "    ".repeat(indent + 2);
                                writeln!(inner_output, "{inner_indent}    {pattern} => {{")
                                    .unwrap();
                                if !prelude.is_empty() {
                                    writeln!(inner_output, "{inner_indent}        {prelude}")
                                        .unwrap();
                                }

                                ctx.push_scope();
                                for inner_stmt in inner_arm.body {
                                    inner_output.push_str(&codegen_stmt(
                                        inner_stmt,
                                        interner,
                                        indent + 4,
                                        mutable_vars,
                                        ctx,
                                        lww_fields,
                                        mv_fields,
                                        synced_vars,
                                        var_caps,
                                        async_functions,
                                        pipe_vars,
                                        boxed_fields,
                                        registry,
                                        type_env,
                                    ));
                                }
                                ctx.pop_scope();
                                writeln!(inner_output, "{}    }}", "    ".repeat(indent + 2))
                                    .unwrap();
                            }
                        }
                        writeln!(inner_output, "{}}}", "    ".repeat(indent + 2)).unwrap();
                        inner_output
                    } else {
                        codegen_stmt(
                            stmt,
                            interner,
                            indent + 2,
                            mutable_vars,
                            ctx,
                            lww_fields,
                            mv_fields,
                            synced_vars,
                            var_caps,
                            async_functions,
                            pipe_vars,
                            boxed_fields,
                            registry,
                            type_env,
                        )
                    }
                } else {
                    codegen_stmt(
                        stmt,
                        interner,
                        indent + 2,
                        mutable_vars,
                        ctx,
                        lww_fields,
                        mv_fields,
                        synced_vars,
                        var_caps,
                        async_functions,
                        pipe_vars,
                        boxed_fields,
                        registry,
                        type_env,
                    )
                }
            } else {
                codegen_stmt(
                    stmt,
                    interner,
                    indent + 2,
                    mutable_vars,
                    ctx,
                    lww_fields,
                    mv_fields,
                    synced_vars,
                    var_caps,
                    async_functions,
                    pipe_vars,
                    boxed_fields,
                    registry,
                    type_env,
                )
            };
            output.push_str(&inner_stmt_code);
        }
        ctx.pop_scope();
        writeln!(output, "{}    }}", indent_str).unwrap();
    }

    writeln!(output, "{}}}", indent_str).unwrap();
    output
}

/// Whether a field's declared type is one of the program's own enums.
fn field_is_user_enum(ty: &FieldType, registry: &TypeRegistry) -> bool {
    let name = match ty {
        FieldType::Named(s) | FieldType::Primitive(s) => *s,
        FieldType::Generic { base, .. } => *base,
        _ => return false,
    };
    matches!(registry.get(name), Some(TypeDef::Enum { .. }))
}

/// The enum an `Inspect` arm matches: the one the source names, or the one the registry says owns the variant.
fn enum_name_of_arm(
    arm: &MatchArm,
    registry: &TypeRegistry,
    interner: &Interner,
) -> Option<String> {
    arm.enum_name
        .map(|e| interner.resolve(e).to_string())
        .or_else(|| {
            arm.variant.and_then(|v| {
                registry
                    .find_variant(v)
                    .map(|(e, _)| interner.resolve(e).to_string())
            })
        })
}

/// The pattern an `Inspect` arm matches with, binding each of its names by `ref` into the scrutinee, and the
/// statements that bind the names a variant boxed whole holds in its box ([`super::types::VariantShape`]).
fn inspect_arm_pattern(
    arm: &MatchArm,
    registry: &TypeRegistry,
    interner: &Interner,
) -> (String, String) {
    let Some(variant) = arm.variant else {
        return ("_".to_string(), String::new());
    };
    let variant_name = interner.resolve(variant);
    // ★ AN UNNAMED ARM IS RESOLVED FROM THE REGISTRY, not left bare. `Inspect` arms carry the enum only when the
    // source named it, and a COMPILER-WRITTEN arm (the derived JSON decoder's `When JOk (v)`) has nothing to carry —
    // so the pattern came out as a bare `JErr { … }`, which Rust reads as a struct that does not exist. The
    // tree-walker resolves a variant by name and never noticed; only a BUILD did, with an error pointing at
    // generated code the author never wrote. The path is the SANITIZED ident, so a cross-module enum
    // (`Colors::Filter`) matches as `Colors__Filter::Variant`.
    let enum_name = enum_name_of_arm(arm, registry, interner);
    let path = match enum_name.as_deref() {
        Some(e) => format!("{}::{}", super::types::rust_type_ident(e), variant_name),
        None => variant_name.to_string(),
    };
    if arm.bindings.is_empty() {
        return (path, String::new());
    }
    if variant_name == "Some" {
        // `Option::Some` maps to Rust's std `Option` — a TUPLE variant (`Some(T)`, matching the `Some(value)`
        // constructor emission), so it needs tuple-pattern syntax `Some(ref v)`, never the struct-variant
        // `Some { value: ref v }` a user enum would use (that is E0769). `None` rides the unit-variant arm above.
        let binding = escape_rust_ident(interner.resolve(arm.bindings[0].1));
        return (format!("Some(ref {binding})"), String::new());
    }
    let binds: Vec<(String, String)> = arm
        .bindings
        .iter()
        .map(|(f, b)| {
            (
                escape_rust_ident(interner.resolve(*f)),
                escape_rust_ident(interner.resolve(*b)),
            )
        })
        .collect();
    let variant_def = match arm.enum_name {
        Some(e) => registry.variant_in(e, variant),
        None => registry.find_variant(variant).map(|(_, v)| v),
    };
    let shape = match (variant_def, enum_name.as_deref()) {
        (Some(v), Some(e)) => super::types::VariantShape::of(v, e, interner),
        _ => super::types::VariantShape::InPlace,
    };
    shape.pattern(&path, &binds, true)
}

/// Whether the field `binding` is bound from in `arm` is stored boxed (a recursive field).
fn arm_binding_is_boxed(
    arm: &MatchArm,
    binding: Symbol,
    enum_name: Option<&str>,
    boxed_fields: &HashSet<(String, String, String)>,
    interner: &Interner,
) -> bool {
    let (Some(enum_name), Some(variant)) = (enum_name, arm.variant) else {
        return false;
    };
    arm.bindings.iter().any(|(field, b)| {
        *b == binding
            && boxed_fields.contains(&(
                enum_name.to_string(),
                interner.resolve(variant).to_string(),
                interner.resolve(*field).to_string(),
            ))
    })
}

/// The `Concurrent` arm of [`codegen_stmt`]: an arm of [`codegen_stmt`], in a frame of its own — see [`codegen_let_stmt`].
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn codegen_concurrent_stmt<'a>(
    mut output: String,
    tasks: &Block<'a>,
    interner: &Interner,
    indent: usize,
    mutable_vars: &HashSet<Symbol>,
    ctx: &mut RefinementContext<'a>,
    lww_fields: &HashSet<(String, String)>,
    mv_fields: &HashSet<(String, String)>,
    synced_vars: &mut HashSet<Symbol>,
    var_caps: &HashMap<Symbol, VariableCapabilities>,
    async_functions: &HashSet<Symbol>,
    pipe_vars: &HashSet<Symbol>,
    boxed_fields: &HashSet<(String, String, String)>,
    registry: &TypeRegistry,
    type_env: &crate::analysis::types::TypeEnv,
    indent_str: &str,
) -> String {
    // Collect Let statements to generate tuple destructuring
    let let_bindings: Vec<_> = tasks
        .iter()
        .filter_map(|s| {
            if let Stmt::Let { var, .. } = s {
                Some(interner.resolve(*var).to_string())
            } else {
                None
            }
        })
        .collect();

    // Collect variables DEFINED in this block (to exclude from cloning)
    let defined_vars: HashSet<Symbol> = tasks
        .iter()
        .filter_map(|s| {
            if let Stmt::Let { var, .. } = s {
                Some(*var)
            } else {
                None
            }
        })
        .collect();

    // Check if there are intra-block dependencies (a later task uses a var from earlier task)
    // If so, fall back to sequential execution
    let mut has_intra_dependency = false;
    let mut seen_defs: HashSet<Symbol> = HashSet::new();
    for s in *tasks {
        // Check if this task uses any variable defined by previous tasks in this block
        let mut used_in_task: HashSet<Symbol> = HashSet::new();
        collect_stmt_identifiers(s, &mut used_in_task);
        for used_var in &used_in_task {
            if seen_defs.contains(used_var) {
                has_intra_dependency = true;
                break;
            }
        }
        // Track variables defined by this task
        if let Stmt::Let { var, .. } = s {
            seen_defs.insert(*var);
        }
        if has_intra_dependency {
            break;
        }
    }

    // Collect ALL variables used in task expressions (not just Call args)
    // Exclude variables defined within this block
    let mut used_syms: HashSet<Symbol> = HashSet::new();
    for s in *tasks {
        collect_stmt_identifiers(s, &mut used_syms);
    }
    // Remove variables that are defined in this block
    for def_var in &defined_vars {
        used_syms.remove(def_var);
    }
    let used_vars: HashSet<String> = used_syms
        .iter()
        .map(|sym| interner.resolve(*sym).to_string())
        .collect();

    // If there are intra-block dependencies, execute sequentially
    if has_intra_dependency {
        // Generate sequential Let bindings
        for stmt in *tasks {
            output.push_str(&codegen_stmt(
                stmt,
                interner,
                indent,
                mutable_vars,
                ctx,
                lww_fields,
                mv_fields,
                synced_vars,
                var_caps,
                async_functions,
                pipe_vars,
                boxed_fields,
                registry,
                type_env,
            ));
        }
    } else {
        // Generate concurrent execution with tokio::join!
        if !let_bindings.is_empty() {
            // Generate tuple destructuring for concurrent Let bindings
            writeln!(
                output,
                "{}let ({}) = tokio::join!(",
                indent_str,
                let_bindings.join(", ")
            )
            .unwrap();
        } else {
            writeln!(output, "{}tokio::join!(", indent_str).unwrap();
        }

        for (i, stmt) in tasks.iter().enumerate() {
            // For Let statements, generate only the VALUE so the async block returns it
            // For Call statements, generate the call with .await
            let inner_code = match stmt {
                Stmt::Let { value, .. } => {
                    // Return the value expression directly (not "let x = value;")
                    // Phase 54+: Use codegen_expr_with_async to handle all nested async calls
                    codegen_expr_with_async(
                        value,
                        interner,
                        synced_vars,
                        async_functions,
                        ctx.get_variable_types(),
                    )
                }
                Stmt::Call { function, args } => {
                    let func_name = interner.resolve(*function);
                    let args_str: Vec<String> = args
                        .iter()
                        .map(|a| {
                            codegen_expr_with_async(
                                a,
                                interner,
                                synced_vars,
                                async_functions,
                                ctx.get_variable_types(),
                            )
                        })
                        .collect();
                    // Only add .await for async functions
                    let await_suffix = if async_functions.contains(function) {
                        ".await"
                    } else {
                        ""
                    };
                    format!("{}({}){}", func_name, args_str.join(", "), await_suffix)
                }
                _ => {
                    // Fallback for other statement types
                    let inner = codegen_stmt(
                        stmt,
                        interner,
                        indent + 1,
                        mutable_vars,
                        ctx,
                        lww_fields,
                        mv_fields,
                        synced_vars,
                        var_caps,
                        async_functions,
                        pipe_vars,
                        boxed_fields,
                        registry,
                        type_env,
                    );
                    inner.trim().to_string()
                }
            };

            // For tasks that use shared variables, wrap in a block that clones them
            if !used_vars.is_empty() && i < tasks.len() - 1 {
                // Clone variables for all tasks except the last one
                let clones: Vec<String> = used_vars
                    .iter()
                    .map(|v| format!("let {} = {}.clone();", v, v))
                    .collect();
                write!(
                    output,
                    "{}    {{ {} async move {{ {} }} }}",
                    indent_str,
                    clones.join(" "),
                    inner_code
                )
                .unwrap();
            } else {
                // Last task can use original variables
                write!(output, "{}    async {{ {} }}", indent_str, inner_code).unwrap();
            }

            if i < tasks.len() - 1 {
                writeln!(output, ",").unwrap();
            } else {
                writeln!(output).unwrap();
            }
        }

        writeln!(output, "{});", indent_str).unwrap();
    }
    output
}

/// The `Parallel` arm of [`codegen_stmt`]: an arm of [`codegen_stmt`], in a frame of its own — see [`codegen_let_stmt`].
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn codegen_parallel_stmt<'a>(
    mut output: String,
    tasks: &Block<'a>,
    interner: &Interner,
    indent: usize,
    mutable_vars: &HashSet<Symbol>,
    ctx: &mut RefinementContext<'a>,
    lww_fields: &HashSet<(String, String)>,
    mv_fields: &HashSet<(String, String)>,
    synced_vars: &mut HashSet<Symbol>,
    var_caps: &HashMap<Symbol, VariableCapabilities>,
    async_functions: &HashSet<Symbol>,
    pipe_vars: &HashSet<Symbol>,
    boxed_fields: &HashSet<(String, String, String)>,
    registry: &TypeRegistry,
    type_env: &crate::analysis::types::TypeEnv,
    indent_str: &str,
) -> String {
    if tasks.len() < 2 {
        for stmt in tasks.iter() {
            output.push_str(&codegen_stmt(
                stmt,
                interner,
                indent,
                mutable_vars,
                ctx,
                lww_fields,
                mv_fields,
                synced_vars,
                var_caps,
                async_functions,
                pipe_vars,
                boxed_fields,
                registry,
                type_env,
            ));
        }
        return output;
    }
    let names = RustNames::new(interner);
    // Each task as the body of a pool closure, and the pattern its value binds: a `Let` returns its value into
    // its name, any other task returns nothing and binds `_`.
    let leaves: Vec<(String, String)> = tasks
        .iter()
        .map(|stmt| match stmt {
            Stmt::Let {
                var,
                value,
                mutable,
                ..
            } => {
                let code = super::types::type_silent_int_i64(value).unwrap_or_else(|| {
                    codegen_expr_with_async(
                        value,
                        interner,
                        synced_vars,
                        async_functions,
                        ctx.get_variable_types(),
                    )
                });
                let name = names.ident(*var);
                let pattern = if *mutable || mutable_vars.contains(var) {
                    format!("mut {name}")
                } else {
                    name.to_string()
                };
                (format!("{{ {code} }}"), pattern)
            }
            _ => {
                let inner = codegen_stmt(
                    stmt,
                    interner,
                    indent + 1,
                    mutable_vars,
                    ctx,
                    lww_fields,
                    mv_fields,
                    synced_vars,
                    var_caps,
                    async_functions,
                    pipe_vars,
                    boxed_fields,
                    registry,
                    type_env,
                );
                (format!("{{ {} }}", inner.trim()), "_".to_string())
            }
        })
        .collect();
    // The tasks run as one balanced tree of `rayon::join`s on the pool: every task may run beside every other,
    // each closure borrows what it reads, and the tree's value is a nested pair the pattern mirrors.
    fn tree(leaves: &[(String, String)]) -> (String, String) {
        if let [(code, pattern)] = leaves {
            return (code.clone(), pattern.clone());
        }
        let (left, right) = leaves.split_at(leaves.len() / 2);
        let ((lc, lp), (rc, rp)) = (tree(left), tree(right));
        let pattern = if lp == "_" && rp == "_" {
            "_".to_string()
        } else {
            format!("({lp}, {rp})")
        };
        (format!("rayon::join(|| {lc}, || {rc})"), pattern)
    }
    let (left, right) = leaves.split_at(leaves.len() / 2);
    let ((lc, lp), (rc, rp)) = (tree(left), tree(right));
    if lp == "_" && rp == "_" {
        writeln!(output, "{indent_str}rayon::join(").unwrap();
    } else {
        writeln!(output, "{indent_str}let ({lp}, {rp}) = rayon::join(").unwrap();
    }
    writeln!(output, "{indent_str}    || {lc},").unwrap();
    writeln!(output, "{indent_str}    || {rc}").unwrap();
    writeln!(output, "{indent_str});").unwrap();
    output
}

/// The `Zone` arm of [`codegen_stmt`]: an arm of [`codegen_stmt`], in a frame of its own — see [`codegen_let_stmt`].
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn codegen_zone_stmt<'a>(
    mut output: String,
    name: &Symbol,
    capacity: &Option<usize>,
    source_file: &Option<ZoneSource>,
    body: &Block<'a>,
    interner: &Interner,
    indent: usize,
    mutable_vars: &HashSet<Symbol>,
    ctx: &mut RefinementContext<'a>,
    lww_fields: &HashSet<(String, String)>,
    mv_fields: &HashSet<(String, String)>,
    synced_vars: &mut HashSet<Symbol>,
    var_caps: &HashMap<Symbol, VariableCapabilities>,
    async_functions: &HashSet<Symbol>,
    pipe_vars: &HashSet<Symbol>,
    boxed_fields: &HashSet<(String, String, String)>,
    registry: &TypeRegistry,
    type_env: &crate::analysis::types::TypeEnv,
    indent_str: &str,
) -> String {
    let zone_name = interner.resolve(*name);

    // Generate zone creation based on type
    if let Some(src) = source_file {
        // Memory-mapped file zone — literal path is quoted; a variable path
        // is passed by reference (`new_mapped` takes `AsRef<Path>`).
        use crate::ast::stmt::ZoneSource;
        let path_expr = match src {
            ZoneSource::Literal(p) => format!("\"{}\"", interner.resolve(*p)),
            ZoneSource::Variable(v) => format!("&{}", interner.resolve(*v)),
        };
        writeln!(
            output,
            "{}let {} = logicaffeine_system::memory::Zone::new_mapped({}).expect(\"Failed to map file\");",
            indent_str, zone_name, path_expr
        ).unwrap();
    } else {
        // Heap arena zone
        let cap = capacity.unwrap_or(4096); // Default 4KB
        writeln!(
            output,
            "{}let {} = logicaffeine_system::memory::Zone::new_heap({});",
            indent_str, zone_name, cap
        )
        .unwrap();
    }

    // The zone is a labeled block a `Break` or a `Return` inside it leaves (`control`).
    let exit = super::control::break_label().expect("a zone is a boundary");
    writeln!(output, "{indent_str}'{exit}: {{").unwrap();
    ctx.push_scope();

    // Generate body statements
    for stmt in *body {
        output.push_str(&codegen_stmt(
            stmt,
            interner,
            indent + 1,
            mutable_vars,
            ctx,
            lww_fields,
            mv_fields,
            synced_vars,
            var_caps,
            async_functions,
            pipe_vars,
            boxed_fields,
            registry,
            type_env,
        ));
    }

    ctx.pop_scope();
    writeln!(output, "{}}}", indent_str).unwrap();
    output
}

/// The `If` arm of [`codegen_stmt`]: an arm of [`codegen_stmt`], in a frame of its own — see [`codegen_let_stmt`].
#[inline(never)]
#[allow(clippy::too_many_arguments)]
/// Opt::WideAccumulator analysis — the vars safe to widen to `i128` (see the Opt doc). A qualifier
/// has EXACTLY one `Let X be 0` (outside any loop), is incremented ONLY by `Set X to X + 1` all inside
/// ONE counted loop with an i64 bound (⟹ X ≤ trip ≤ i64::MAX), and is read ONLY by a bare `Show X`.
/// ANY other appearance of X disqualifies it — conservative, so we never widen a var whose value is
/// needed elsewhere (which would at worst be a loud i128/i64 type error, never a silent wrong answer).
fn collect_count_accumulators(
    stmts: &[Stmt],
    bignum_vars: &std::collections::HashSet<Symbol>,
    restrict_reads: bool,
    allow_length_bound: bool,
    pure_calls: &std::collections::HashSet<Symbol>,
) -> std::collections::HashSet<Symbol> {
    use std::collections::{HashMap, HashSet};
    fn is_zero(e: &Expr) -> bool {
        matches!(e, Expr::Literal(Literal::Number(0)))
    }
    fn is_plus1(e: &Expr, x: Symbol) -> bool {
        matches!(e, Expr::BinaryOp { op: BinaryOpKind::Add, left, right }
            if matches!(left, Expr::Identifier(l) if *l == x)
                && matches!(right, Expr::Literal(Literal::Number(1))))
    }
    // A `while <counter> </<= <bound>` whose bound is provably i64 (a literal, a length, or a
    // non-bignum var), so its trip count is ≤ i64::MAX.
    fn counted_i64(cond: &Expr, bignum: &HashSet<Symbol>, allow_length_bound: bool) -> bool {
        if let Expr::BinaryOp { op, left, right } = cond {
            if matches!(op, BinaryOpKind::Lt | BinaryOpKind::LtEq)
                && matches!(left, Expr::Identifier(_))
            {
                return match right {
                    Expr::Literal(Literal::Number(_)) => true,
                    Expr::Identifier(b) => !bignum.contains(b),
                    Expr::Length { .. } => allow_length_bound,
                    _ => false,
                };
            }
        }
        false
    }
    struct S {
        zero_let: HashSet<Symbol>,                // has a top-level `Let X be 0`
        bad_let: HashSet<Symbol>,                 // >1 Let, or a Let that isn't a top-level `be 0`
        inc_loop: HashMap<Symbol, Option<usize>>, // X → the ONE counted-loop id it's `+1`-incremented in (None = multiple/bad)
        disq: HashSet<Symbol>,                    // appeared somewhere disallowed
        bail: bool, // an unrecognized statement kind is present → widen nothing (safe)
    }
    // Every name `e` reads, at any depth: an item's index, a call's argument and a field's owner read their names
    // as surely as an operand does, and a widened accumulator read there would meet an i64 it cannot add to.
    fn disq_reads(e: &Expr, s: &mut S, restrict_reads: bool) {
        if !restrict_reads {
            return;
        }
        let mut stack = vec![e];
        while let Some(x) = stack.pop() {
            if let Expr::Identifier(r) = x {
                s.disq.insert(*r);
            }
            stack.extend(logicaffeine_language::ast_depth::expr_children(x));
        }
    }
    // `loop_ctx = Some(id)` ⟺ we are directly inside a depth-1 counted-i64 loop (its body); None
    // otherwise (top level, a nested loop, or a non-counted loop) — increments there are not clean.
    fn walk(
        stmts: &[Stmt],
        loop_ctx: Option<usize>,
        loop_depth: usize,
        next_id: &mut usize,
        bignum: &HashSet<Symbol>,
        restrict_reads: bool,
        allow_length_bound: bool,
        s: &mut S,
    ) {
        for st in stmts {
            match st {
                Stmt::Let { var, value, .. } => {
                    if loop_ctx.is_none() && is_zero(value) {
                        if !s.zero_let.insert(*var) {
                            s.bad_let.insert(*var);
                        }
                    } else {
                        s.bad_let.insert(*var);
                    }
                    disq_reads(value, s, restrict_reads); // any var READ in a Let RHS is a non-Show read
                }
                Stmt::Set { target, value } => {
                    if let Some(id) = loop_ctx {
                        if is_plus1(value, *target) {
                            match s.inc_loop.get(target) {
                                None => {
                                    s.inc_loop.insert(*target, Some(id));
                                }
                                Some(Some(prev)) if *prev == id => {}
                                _ => {
                                    s.inc_loop.insert(*target, None);
                                }
                            }
                            continue; // the `X` read inside `X + 1` is the allowed one
                        }
                    }
                    s.disq.insert(*target);
                    disq_reads(value, s, restrict_reads);
                }
                Stmt::Show { object, .. } => {
                    if !matches!(object, Expr::Identifier(_)) {
                        disq_reads(object, s, restrict_reads); // only a BARE `Show X` is an allowed read
                    }
                }
                Stmt::If {
                    cond,
                    then_block,
                    else_block,
                } => {
                    disq_reads(cond, s, restrict_reads);
                    walk(
                        then_block,
                        loop_ctx,
                        loop_depth,
                        next_id,
                        bignum,
                        restrict_reads,
                        allow_length_bound,
                        s,
                    );
                    if let Some(eb) = else_block {
                        walk(
                            eb,
                            loop_ctx,
                            loop_depth,
                            next_id,
                            bignum,
                            restrict_reads,
                            allow_length_bound,
                            s,
                        );
                    }
                }
                Stmt::While { cond, body, .. } => {
                    disq_reads(cond, s, restrict_reads);
                    let inner = if loop_depth == 0 && counted_i64(cond, bignum, allow_length_bound)
                    {
                        let id = *next_id;
                        *next_id += 1;
                        Some(id)
                    } else {
                        None
                    };
                    walk(
                        body,
                        inner,
                        loop_depth + 1,
                        next_id,
                        bignum,
                        restrict_reads,
                        allow_length_bound,
                        s,
                    );
                }
                Stmt::Repeat { iterable, body, .. } => {
                    disq_reads(iterable, s, restrict_reads);
                    walk(
                        body,
                        None,
                        loop_depth + 1,
                        next_id,
                        bignum,
                        restrict_reads,
                        allow_length_bound,
                        s,
                    );
                }
                Stmt::Zone { body, .. } => {
                    walk(
                        body,
                        None,
                        loop_depth + 1,
                        next_id,
                        bignum,
                        restrict_reads,
                        allow_length_bound,
                        s,
                    );
                }
                Stmt::Inspect { target, arms, .. } => {
                    disq_reads(target, s, restrict_reads);
                    for arm in arms.iter() {
                        walk(
                            arm.body,
                            None,
                            loop_depth,
                            next_id,
                            bignum,
                            restrict_reads,
                            allow_length_bound,
                            s,
                        );
                    }
                }
                Stmt::Return { value } => {
                    if let Some(v) = value {
                        disq_reads(v, s, restrict_reads);
                    }
                }
                Stmt::Push { value, collection } => {
                    disq_reads(value, s, restrict_reads);
                    disq_reads(collection, s, restrict_reads);
                }
                Stmt::Call { args, .. } => {
                    // The effect walk admitted only proven-pure callees and arguments.
                    for arg in args.iter() {
                        disq_reads(arg, s, restrict_reads);
                    }
                }
                Stmt::Break => {}
                // A function definition at the top level is not part of THIS body's executable code
                // (its body is analyzed on its own); skip so a program with helper fns still widens Main.
                Stmt::FunctionDef { .. } => {}
                _ => {
                    // An unrecognized statement kind might read a candidate in a way we don't model —
                    // widen NOTHING in this body rather than risk it (safe + rare).
                    s.bail = true;
                }
            }
        }
    }
    // Expression calls can mutate a count, its induction variable, or its bound.
    // Without an effect summary none of those visible updates is a complete proof.
    let mut effectful = false;
    logicaffeine_language::ast_depth::walk_nodes(stmts, |node| {
        use logicaffeine_language::ast_depth::{Visited, Walk};
        match node {
            Visited::Stmt(Stmt::FunctionDef { .. }) | Visited::Expr(Expr::Closure { .. }) => {
                Walk::Past
            }
            Visited::Expr(Expr::Call { function, .. })
            | Visited::Stmt(Stmt::Call { function, .. })
                if !pure_calls.contains(function) =>
            {
                effectful = true;
                Walk::Past
            }
            Visited::Expr(Expr::CallExpr { .. } | Expr::Escape { .. }) => {
                effectful = true;
                Walk::Past
            }
            _ => Walk::Into,
        }
    });
    if effectful {
        return HashSet::new();
    }
    let mut s = S {
        zero_let: HashSet::new(),
        bad_let: HashSet::new(),
        inc_loop: HashMap::new(),
        disq: HashSet::new(),
        bail: false,
    };
    let mut next_id = 0usize;
    walk(
        stmts,
        None,
        0,
        &mut next_id,
        bignum_vars,
        restrict_reads,
        allow_length_bound,
        &mut s,
    );
    if s.bail {
        return HashSet::new();
    }
    s.zero_let
        .iter()
        .copied()
        .filter(|x| {
            !s.bad_let.contains(x)
                && !s.disq.contains(x)
                && matches!(s.inc_loop.get(x), Some(Some(_)))
        })
        .collect()
}

pub(crate) fn collect_wide_accumulators(
    stmts: &[Stmt],
    bignum_vars: &std::collections::HashSet<Symbol>,
    pure_calls: &std::collections::HashSet<Symbol>,
) -> std::collections::HashSet<Symbol> {
    collect_count_accumulators(stmts, bignum_vars, true, false, pure_calls)
}

/// Integer accumulators whose `+1` cannot overflow: they start at zero and are changed only inside
/// one i64-bounded counted loop, so their value is at most that loop's trip count. Unlike the
/// WideAccumulator set, reads need not be restricted because this proof changes no storage type.
pub(crate) fn collect_bounded_count_accumulators(
    stmts: &[Stmt],
    bignum_vars: &std::collections::HashSet<Symbol>,
) -> std::collections::HashSet<Symbol> {
    collect_bounded_count_accumulators_with_purity(stmts, bignum_vars, &Default::default())
}

/// The same count proof, with calls admitted by the whole program's shared purity analysis.
pub(crate) fn collect_bounded_count_accumulators_with_purity(
    stmts: &[Stmt],
    bignum_vars: &std::collections::HashSet<Symbol>,
    pure_calls: &std::collections::HashSet<Symbol>,
) -> std::collections::HashSet<Symbol> {
    use std::collections::{HashMap, HashSet};

    fn is_plus1(e: &Expr, x: Symbol) -> bool {
        matches!(e, Expr::BinaryOp { op: BinaryOpKind::Add, left, right }
            if matches!(left, Expr::Identifier(l) if *l == x)
                && matches!(right, Expr::Literal(Literal::Number(1))))
    }

    fn counter_advances_once(body: &[Stmt], counter: Symbol) -> bool {
        fn walk(block: &[Stmt], counter: Symbol, direct: bool, count: &mut usize, bad: &mut bool) {
            for stmt in block {
                match stmt {
                    Stmt::Set { target, value } if *target == counter => {
                        if direct && is_plus1(value, counter) {
                            *count += 1;
                        } else {
                            *bad = true;
                        }
                    }
                    Stmt::Let { var, .. } if *var == counter => *bad = true,
                    Stmt::If {
                        then_block,
                        else_block,
                        ..
                    } => {
                        walk(then_block, counter, false, count, bad);
                        if let Some(eb) = else_block {
                            walk(eb, counter, false, count, bad);
                        }
                    }
                    Stmt::While { body, .. }
                    | Stmt::Repeat { body, .. }
                    | Stmt::Zone { body, .. } => walk(body, counter, false, count, bad),
                    Stmt::Inspect { arms, .. } => {
                        for arm in arms.iter() {
                            walk(arm.body, counter, false, count, bad);
                        }
                    }
                    Stmt::FunctionDef { .. } => {}
                    _ => {}
                }
            }
        }
        let (mut count, mut bad) = (0, false);
        walk(body, counter, true, &mut count, &mut bad);
        !bad && count == 1
    }

    fn certified_loop(
        cond: &Expr,
        body: &[Stmt],
        bignum: &HashSet<Symbol>,
        nonnegative: &HashSet<Symbol>,
        positive: &HashSet<Symbol>,
    ) -> bool {
        let Expr::BinaryOp { op, left, right } = cond else {
            return false;
        };
        let Expr::Identifier(counter) = left else {
            return false;
        };
        if !nonnegative.contains(counter) || !counter_advances_once(body, *counter) {
            return false;
        }
        match (op, right) {
            (BinaryOpKind::Lt, Expr::Literal(Literal::Number(_))) => true,
            (BinaryOpKind::Lt, Expr::Identifier(bound)) => !bignum.contains(bound),
            (BinaryOpKind::Lt, Expr::Length { .. }) => true,
            (BinaryOpKind::LtEq, Expr::Literal(Literal::Number(bound))) => {
                *bound < i64::MAX || positive.contains(counter)
            }
            (BinaryOpKind::LtEq, Expr::Identifier(bound)) => {
                positive.contains(counter) && !bignum.contains(bound)
            }
            (BinaryOpKind::LtEq, Expr::Length { .. }) => true,
            _ => false,
        }
    }

    fn plus1_sites(block: &[Stmt], out: &mut HashMap<Symbol, usize>) {
        for stmt in block {
            match stmt {
                Stmt::Set { target, value } if is_plus1(value, *target) => {
                    *out.entry(*target).or_insert(0) += 1;
                }
                Stmt::If {
                    then_block,
                    else_block,
                    ..
                } => {
                    plus1_sites(then_block, out);
                    if let Some(eb) = else_block {
                        plus1_sites(eb, out);
                    }
                }
                Stmt::Zone { body, .. } => plus1_sites(body, out),
                Stmt::Inspect { arms, .. } => {
                    for arm in arms.iter() {
                        plus1_sites(arm.body, out);
                    }
                }
                // A nested loop's sites do not run at most once per outer iteration.
                Stmt::While { .. } | Stmt::Repeat { .. } | Stmt::FunctionDef { .. } => {}
                _ => {}
            }
        }
    }

    fn visit_loops(
        block: &[Stmt],
        bignum: &HashSet<Symbol>,
        nonnegative: &HashSet<Symbol>,
        positive: &HashSet<Symbol>,
        certified: &mut HashSet<Symbol>,
    ) {
        let mut positive = positive.clone();
        for stmt in block {
            match stmt {
                Stmt::While { cond, body, .. } => {
                    if certified_loop(cond, body, bignum, nonnegative, &positive) {
                        let mut sites = HashMap::new();
                        plus1_sites(body, &mut sites);
                        certified.extend(
                            sites
                                .into_iter()
                                .filter_map(|(target, n)| (n == 1).then_some(target)),
                        );
                    }
                    visit_loops(body, bignum, nonnegative, &positive, certified);
                }
                Stmt::If {
                    then_block,
                    else_block,
                    ..
                } => {
                    visit_loops(then_block, bignum, nonnegative, &positive, certified);
                    if let Some(eb) = else_block {
                        visit_loops(eb, bignum, nonnegative, &positive, certified);
                    }
                }
                Stmt::Repeat { body, .. } | Stmt::Zone { body, .. } => {
                    visit_loops(body, bignum, nonnegative, &positive, certified)
                }
                Stmt::Inspect { arms, .. } => {
                    for arm in arms.iter() {
                        visit_loops(arm.body, bignum, nonnegative, &positive, certified);
                    }
                }
                Stmt::FunctionDef { .. } => {}
                _ => {}
            }
            // Only a dominating positive literal, preserved by +1, proves a
            // strictly positive start. Writes in branches or loops invalidate it.
            match stmt {
                Stmt::Let { var, value, .. } | Stmt::Set { target: var, value } if matches!(value, Expr::Literal(Literal::Number(n)) if *n > 0) =>
                {
                    positive.insert(*var);
                }
                Stmt::Set { target, value } if is_plus1(value, *target) => {}
                _ => {
                    logicaffeine_language::ast_depth::walk_nodes(
                        std::slice::from_ref(stmt),
                        |node| {
                            use logicaffeine_language::ast_depth::{Visited, Walk};
                            match node {
                                Visited::Stmt(Stmt::FunctionDef { .. })
                                | Visited::Expr(Expr::Closure { .. }) => Walk::Past,
                                Visited::Stmt(
                                    Stmt::Let { var, .. } | Stmt::Set { target: var, .. },
                                ) => {
                                    positive.remove(var);
                                    Walk::Into
                                }
                                _ => Walk::Into,
                            }
                        },
                    );
                }
            }
        }
    }

    let candidates = collect_count_accumulators(stmts, bignum_vars, false, true, pure_calls);
    let nonnegative = super::expr::collect_always_nonneg(stmts);
    let mut certified = HashSet::new();
    visit_loops(
        stmts,
        bignum_vars,
        &nonnegative,
        &HashSet::new(),
        &mut certified,
    );
    candidates.intersection(&certified).copied().collect()
}

fn codegen_if_stmt<'a>(
    mut output: String,
    cond: &&'a Expr<'a>,
    then_block: &Block<'a>,
    else_block: &Option<Block<'a>>,
    interner: &Interner,
    indent: usize,
    mutable_vars: &HashSet<Symbol>,
    ctx: &mut RefinementContext<'a>,
    lww_fields: &HashSet<(String, String)>,
    mv_fields: &HashSet<(String, String)>,
    synced_vars: &mut HashSet<Symbol>,
    var_caps: &HashMap<Symbol, VariableCapabilities>,
    async_functions: &HashSet<Symbol>,
    pipe_vars: &HashSet<Symbol>,
    boxed_fields: &HashSet<(String, String, String)>,
    registry: &TypeRegistry,
    type_env: &crate::analysis::types::TypeEnv,
    indent_str: &str,
) -> String {
    // Opt::ProbeFuse: `If m contains k:` over a branch that reads `item k of m` asks the map once.
    if let Some(fusion) = probe_fusion(
        cond,
        then_block,
        interner,
        synced_vars,
        async_functions,
        ctx,
    ) {
        let fused_head = format!(
            "{}if let Some::<{}>({}) = {}.get(&({})) {{",
            indent_str, fusion.value_ty, fusion.probe.name, fusion.map, fusion.key
        );
        let (out, served) = codegen_if_arms(
            output,
            fused_head.clone(),
            then_block,
            Some(fusion.probe),
            else_block,
            interner,
            indent,
            mutable_vars,
            ctx,
            lww_fields,
            mv_fields,
            synced_vars,
            var_caps,
            async_functions,
            pipe_vars,
            boxed_fields,
            registry,
            type_env,
            indent_str,
        );
        if served == 0 {
            let plain_head = format!(
                "{}if {}.logos_contains(&({})) {{",
                indent_str, fusion.map, fusion.key
            );
            return out.replacen(&fused_head, &plain_head, 1);
        }
        crate::optimize::mark_fired(crate::optimization::Opt::ProbeFuse);
        return out;
    }
    // Opt::BranchlessAccumulate: `if <cond> { X = X + <literal k> }` (no else, X a plain integer
    // accumulator) -> `X = logos_add_i64(X, (<cond>) as i64 * k)`. Removes the data-dependent branch
    // that blocks loop vectorization. Byte-identical: `cond ∈ {0,1}`, evaluated exactly once either way.
    if else_block.is_none()
        && then_block.len() == 1
        && crate::optimize::active_config().is_on(crate::optimization::Opt::BranchlessAccumulate)
    {
        if let Some(Stmt::Set { target, value }) = then_block.first() {
            if let Expr::BinaryOp {
                op: BinaryOpKind::Add,
                left,
                right,
            } = value
            {
                let self_add = matches!(left, Expr::Identifier(l) if *l == *target);
                if let (true, Expr::Literal(Literal::Number(k))) = (self_add, right) {
                    // X must be a plain integer accumulator (i64 add helper), not bignum/float/text —
                    // conservative: bail on any non-plain type so the add semantics match the normal path.
                    let plain_i64 = match ctx.get_variable_types().get(target) {
                        None => true,
                        Some(t) => {
                            !(t.contains("bigint")
                                || t.contains("LogosInt")
                                || t == "f64"
                                || t == "String"
                                || t == "Text"
                                || t.contains("__"))
                        }
                    };
                    if plain_i64 {
                        crate::optimize::mark_fired(crate::optimization::Opt::BranchlessAccumulate);
                        let cs = codegen_expr_with_async_oracle(
                            cond,
                            interner,
                            synced_vars,
                            async_functions,
                            ctx.get_variable_types(),
                            ctx.oracle(),
                        );
                        let cs = truthy_cond_wrap(cond, cs, interner, ctx.get_variable_types());
                        let xn = RustNames::new(interner).ident(*target);
                        // Opt::WideAccumulator: an i128 accumulator uses a PLAIN add (no checked branch,
                        // so the count vectorizes); a plain i64 accumulator keeps the checked helper.
                        if super::expr::is_wide_acc(*target)
                            && crate::optimize::active_config()
                                .is_on(crate::optimization::Opt::WideAccumulator)
                        {
                            crate::optimize::mark_fired(crate::optimization::Opt::WideAccumulator);
                            let delta = if *k == 1 {
                                format!("(({}) as i128)", cs)
                            } else {
                                format!("(({}) as i128 * {})", cs, k)
                            };
                            writeln!(output, "{}{} = {} + {};", indent_str, xn, xn, delta).unwrap();
                        } else {
                            let delta = if *k == 1 {
                                format!("(({}) as i64)", cs)
                            } else {
                                format!("(({}) as i64 * {})", cs, k)
                            };
                            writeln!(
                                output,
                                "{}{} = logos_add_i64({}, {});",
                                indent_str, xn, xn, delta
                            )
                            .unwrap();
                        }
                        return output;
                    }
                }
            }
        }
    }
    // Opt::BranchlessSelect: both arms ahead of the choice and `cond` selecting, while every arm's guard holds;
    // the branch itself, one level in, when one does not.
    if let Some(select) = branchless_select(
        then_block,
        else_block,
        interner,
        synced_vars,
        async_functions,
        ctx,
    ) {
        crate::optimize::mark_fired(crate::optimization::Opt::BranchlessSelect);
        let cond_str = codegen_expr_with_async_oracle(
            cond,
            interner,
            synced_vars,
            async_functions,
            ctx.get_variable_types(),
            ctx.oracle(),
        );
        let cond_str = truthy_cond_wrap(cond, cond_str, interner, ctx.get_variable_types());
        let target = RustNames::new(interner).ident(select.target);
        writeln!(output, "{}if {} {{", indent_str, select.guard).unwrap();
        writeln!(
            output,
            "{}    let __select_then: i64 = {};",
            indent_str, select.then_word
        )
        .unwrap();
        writeln!(
            output,
            "{}    let __select_else: i64 = {};",
            indent_str, select.else_word
        )
        .unwrap();
        writeln!(
            output,
            "{}    {} = if {} {{ __select_then }} else {{ __select_else }};",
            indent_str, target, cond_str
        )
        .unwrap();
        writeln!(output, "{}}} else {{", indent_str).unwrap();
        let output = codegen_if_branches(
            output,
            cond,
            then_block,
            else_block,
            interner,
            indent + 1,
            mutable_vars,
            ctx,
            lww_fields,
            mv_fields,
            synced_vars,
            var_caps,
            async_functions,
            pipe_vars,
            boxed_fields,
            registry,
            type_env,
            &"    ".repeat(indent + 1),
        );
        return output + &format!("{}}}\n", indent_str);
    }
    codegen_if_branches(
        output,
        cond,
        then_block,
        else_block,
        interner,
        indent,
        mutable_vars,
        ctx,
        lww_fields,
        mv_fields,
        synced_vars,
        var_caps,
        async_functions,
        pipe_vars,
        boxed_fields,
        registry,
        type_env,
        indent_str,
    )
}

/// `If m contains k:` as one lookup ([`probe_fusion`]): the map and key as Rust, the map's value type, and the probe
/// that answers the branch's reads of `item k of m`.
struct ProbeFusion {
    map: String,
    key: String,
    value_ty: String,
    probe: super::probe::Probe,
}

/// `If m contains k:` as `if let Some(p) = m.get(&k)`, its branch reading `item k of m` as `p`
/// ([`super::probe`]): `m` a map whose `get` answers an `Int`, `Real` or `Bool` by value, `k` a name or a literal of
/// the map's key type, and a branch that calls nothing and writes neither `m` nor `k`, so every read of `item k of m`
/// in it reads what the test found.
fn probe_fusion<'a>(
    cond: &Expr<'a>,
    then_block: &Block<'a>,
    interner: &Interner,
    synced_vars: &HashSet<Symbol>,
    async_functions: &HashSet<Symbol>,
    ctx: &RefinementContext<'a>,
) -> Option<ProbeFusion> {
    if !crate::optimize::active_config().is_on(crate::optimization::Opt::ProbeFuse) {
        return None;
    }
    let Expr::Contains {
        collection: map_expr @ Expr::Identifier(map),
        value: key,
    } = cond
    else {
        return None;
    };
    let probe_key = super::probe::ProbeKey::of(key)?;
    let writes = crate::optimize::writes::WriteSet::of_block(then_block);
    if writes.is_opaque()
        || writes.has_calls()
        || writes.may_write(*map)
        || matches!(probe_key, super::probe::ProbeKey::Name(k) if writes.may_write(k))
    {
        return None;
    }
    let types = ctx.get_variable_types();
    let map_ty = types
        .get(map)?
        .split("|__hl:")
        .next()?
        .trim_start_matches("&mut ")
        .trim_start_matches('&')
        .to_string();
    let (key_ty, value_ty) = if map_ty.starts_with("LogosI64Map")
        || map_ty.starts_with("LogosI32Map")
        || map_ty.starts_with("LogosDenseI64Map")
    {
        ("i64".to_string(), "i64".to_string())
    } else {
        let args = map_ty.strip_prefix("LogosMap<")?.strip_suffix('>')?;
        let mut depth = 0i32;
        let comma = args.char_indices().find(|&(_, c)| {
            match c {
                '<' | '(' | '[' => depth += 1,
                '>' | ')' | ']' => depth -= 1,
                _ => {}
            }
            c == ',' && depth == 0
        })?;
        (
            args[..comma.0].trim().to_string(),
            args[comma.0 + 1..].trim().to_string(),
        )
    };
    if !matches!(value_ty.as_str(), "i64" | "f64" | "bool") {
        return None;
    }
    let key_fits = match probe_key {
        super::probe::ProbeKey::Int(_) => key_ty == "i64",
        super::probe::ProbeKey::Text(_) => key_ty == "String",
        super::probe::ProbeKey::Name(_) if key_ty == "i64" => {
            super::types::infer_numeric_type(key, interner, types) == "i64"
        }
        super::probe::ProbeKey::Name(k) => types.get(&k).is_some_and(|t| *t == key_ty),
    };
    if !key_fits {
        return None;
    }
    let emit = |e: &Expr<'a>| {
        codegen_expr_with_async_oracle(
            e,
            interner,
            synced_vars,
            async_functions,
            types,
            ctx.oracle(),
        )
    };
    Some(ProbeFusion {
        map: emit(map_expr),
        key: emit(key),
        value_ty,
        probe: super::probe::Probe {
            map: *map,
            key: probe_key,
            name: super::probe::fresh_name(),
            reads: 0,
        },
    })
}

/// A two-armed assignment `Opt::BranchlessSelect` computes as a select: the guard under which both arms' machine
/// words are exact, the two words, and the name both arms assign.
struct BranchlessSelect {
    guard: String,
    then_word: String,
    else_word: String,
    target: Symbol,
}

/// `If c: Set x to a. Otherwise: Set x to b.` as a [`BranchlessSelect`], for an `x` held in a machine word and arms
/// that may be computed ahead of the choice ([`super::expr::select_arm`]). `None` as well when neither arm has a
/// guard: a branch between two raw arms is one the compiler already turns into a select.
fn branchless_select<'a>(
    then_block: &Block<'a>,
    else_block: &Option<Block<'a>>,
    interner: &Interner,
    synced_vars: &HashSet<Symbol>,
    async_functions: &HashSet<Symbol>,
    ctx: &RefinementContext<'a>,
) -> Option<BranchlessSelect> {
    if !crate::optimize::active_config().is_on(crate::optimization::Opt::BranchlessSelect) {
        return None;
    }
    let (
        [Stmt::Set {
            target,
            value: then_value,
        }],
        Some(
            [Stmt::Set {
                target: else_target,
                value: else_value,
            }],
        ),
    ) = (&then_block[..], else_block.as_deref())
    else {
        return None;
    };
    if target != else_target {
        return None;
    }
    let word = ctx
        .get_variable_types()
        .get(target)
        .map(|t| t.split("|__hl:").next().unwrap_or(t.as_str()));
    if word != Some("i64") {
        return None;
    }
    let arm = |value| {
        super::expr::select_arm(
            value,
            interner,
            synced_vars,
            async_functions,
            ctx.get_variable_types(),
            ctx.oracle(),
        )
    };
    let (then_arm, else_arm) = (arm(then_value)?, arm(else_value)?);
    let guard = match (then_arm.guard, else_arm.guard) {
        (None, None) => return None,
        (Some(g), None) | (None, Some(g)) => g,
        (Some(a), Some(b)) if a == b => a,
        (Some(a), Some(b)) => format!("({a}) && ({b})"),
    };
    Some(BranchlessSelect {
        guard,
        then_word: then_arm.fast,
        else_word: else_arm.fast,
        target: *target,
    })
}

/// The `If` statement as its branch: the condition, the then-block, and the else-block when there is one.
#[allow(clippy::too_many_arguments)]
fn codegen_if_branches<'a>(
    output: String,
    cond: &&'a Expr<'a>,
    then_block: &Block<'a>,
    else_block: &Option<Block<'a>>,
    interner: &Interner,
    indent: usize,
    mutable_vars: &HashSet<Symbol>,
    ctx: &mut RefinementContext<'a>,
    lww_fields: &HashSet<(String, String)>,
    mv_fields: &HashSet<(String, String)>,
    synced_vars: &mut HashSet<Symbol>,
    var_caps: &HashMap<Symbol, VariableCapabilities>,
    async_functions: &HashSet<Symbol>,
    pipe_vars: &HashSet<Symbol>,
    boxed_fields: &HashSet<(String, String, String)>,
    registry: &TypeRegistry,
    type_env: &crate::analysis::types::TypeEnv,
    indent_str: &str,
) -> String {
    let cond_str = codegen_expr_with_async_oracle(
        cond,
        interner,
        synced_vars,
        async_functions,
        ctx.get_variable_types(),
        ctx.oracle(),
    );
    let cond_str = truthy_cond_wrap(cond, cond_str, interner, ctx.get_variable_types());
    let head = format!("{}if {} {{", indent_str, cond_str);
    let (output, _) = codegen_if_arms(
        output,
        head,
        then_block,
        None,
        else_block,
        interner,
        indent,
        mutable_vars,
        ctx,
        lww_fields,
        mv_fields,
        synced_vars,
        var_caps,
        async_functions,
        pipe_vars,
        boxed_fields,
        registry,
        type_env,
        indent_str,
    );
    output
}

/// The `If` statement's two arms under `head`, its opening line: `then_stmts` and the else-block when there is one.
/// `probe` answers the then-arm's reads of its map for its key ([`super::probe`]); the count is the reads it served.
#[allow(clippy::too_many_arguments)]
fn codegen_if_arms<'a>(
    mut output: String,
    head: String,
    then_stmts: &[Stmt<'a>],
    probe: Option<super::probe::Probe>,
    else_block: &Option<Block<'a>>,
    interner: &Interner,
    indent: usize,
    mutable_vars: &HashSet<Symbol>,
    ctx: &mut RefinementContext<'a>,
    lww_fields: &HashSet<(String, String)>,
    mv_fields: &HashSet<(String, String)>,
    synced_vars: &mut HashSet<Symbol>,
    var_caps: &HashMap<Symbol, VariableCapabilities>,
    async_functions: &HashSet<Symbol>,
    pipe_vars: &HashSet<Symbol>,
    boxed_fields: &HashSet<(String, String, String)>,
    registry: &TypeRegistry,
    type_env: &crate::analysis::types::TypeEnv,
    indent_str: &str,
) -> (String, usize) {
    writeln!(output, "{}", head).unwrap();
    ctx.push_scope();
    let mut then_code = String::new();
    let mut emit_then = || {
        let block_refs: Vec<&Stmt> = then_stmts.iter().collect();
        let mut bi = 0;
        while bi < block_refs.len() {
            if let Some((code, skip)) = super::peephole::try_block_peepholes(
                &block_refs,
                bi,
                interner,
                indent + 1,
                mutable_vars,
                ctx,
                lww_fields,
                mv_fields,
                synced_vars,
                var_caps,
                async_functions,
                pipe_vars,
                boxed_fields,
                registry,
                type_env,
            ) {
                then_code.push_str(&code);
                bi += 1 + skip;
                continue;
            }
            then_code.push_str(&codegen_stmt(
                block_refs[bi],
                interner,
                indent + 1,
                mutable_vars,
                ctx,
                lww_fields,
                mv_fields,
                synced_vars,
                var_caps,
                async_functions,
                pipe_vars,
                boxed_fields,
                registry,
                type_env,
            ));
            bi += 1;
        }
    };
    let served = match probe {
        Some(probe) => super::probe::within(probe, emit_then).1,
        None => {
            emit_then();
            0
        }
    };
    output.push_str(&then_code);
    ctx.pop_scope();
    if let Some(else_stmts) = else_block {
        writeln!(output, "{}}} else {{", indent_str).unwrap();
        ctx.push_scope();
        {
            let block_refs: Vec<&Stmt> = else_stmts.iter().collect();
            let mut bi = 0;
            while bi < block_refs.len() {
                if let Some((code, skip)) = super::peephole::try_block_peepholes(
                    &block_refs,
                    bi,
                    interner,
                    indent + 1,
                    mutable_vars,
                    ctx,
                    lww_fields,
                    mv_fields,
                    synced_vars,
                    var_caps,
                    async_functions,
                    pipe_vars,
                    boxed_fields,
                    registry,
                    type_env,
                ) {
                    output.push_str(&code);
                    bi += 1 + skip;
                    continue;
                }
                output.push_str(&codegen_stmt(
                    block_refs[bi],
                    interner,
                    indent + 1,
                    mutable_vars,
                    ctx,
                    lww_fields,
                    mv_fields,
                    synced_vars,
                    var_caps,
                    async_functions,
                    pipe_vars,
                    boxed_fields,
                    registry,
                    type_env,
                ));
                bi += 1;
            }
        }
        ctx.pop_scope();
    }
    writeln!(output, "{}}}", indent_str).unwrap();
    (output, served)
}

/// The `Show` arm of [`codegen_stmt`]: an arm of [`codegen_stmt`], in a frame of its own — see [`codegen_let_stmt`].
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn codegen_show_stmt<'a>(
    mut output: String,
    object: &&'a Expr<'a>,
    recipient: &&'a Expr<'a>,
    interner: &Interner,
    ctx: &mut RefinementContext<'a>,
    synced_vars: &mut HashSet<Symbol>,
    async_functions: &HashSet<Symbol>,
    registry: &TypeRegistry,
    indent_str: &str,
    names: &RustNames,
) -> String {
    // OPT: Show single-char u8 variable → println!("{}", ch as char)
    if let Expr::Identifier(sym) = object {
        if ctx
            .get_variable_types()
            .get(sym)
            .map_or(false, |t| t == "__single_char_u8")
        {
            let var_name = names.ident(*sym);
            writeln!(
                output,
                "{}logicaffeine_system::io::println({} as char);",
                indent_str, var_name
            )
            .unwrap();
            return output;
        }
    }
    // Opt::WideAccumulator: `Show X` where X is an i128-widened accumulator — narrow `as i64` at the
    // sink (X ≤ trip ≤ i64::MAX, so exact) and print via the same recipient helper.
    if let Expr::Identifier(sym) = object {
        if crate::optimize::active_config().is_on(crate::optimization::Opt::WideAccumulator)
            && super::expr::is_wide_acc(*sym)
        {
            let recv = if let Expr::Identifier(r) = recipient {
                super::expr::called_function_path(*r, names.ident(*r), ctx.get_variable_types())
            } else {
                "show".to_string()
            };
            let var_name = names.ident(*sym);
            let narrowed = format!("(({}) as i64)", var_name);
            writeln!(
                output,
                "{}{}({});",
                indent_str,
                recv,
                super::expr::lend_to(recipient, &narrowed)
            )
            .unwrap();
            return output;
        }
    }
    // Optimization: Show with InterpolatedString → println! directly
    if let Expr::InterpolatedString(parts) = object {
        let recv_name = if let Expr::Identifier(sym) = recipient {
            interner.resolve(*sym).to_string()
        } else {
            String::new()
        };
        if recv_name == "show" {
            // Emit println! directly — no intermediate String allocation
            let mut fmt_str = String::new();
            let mut args = Vec::new();
            for part in parts {
                match part {
                    crate::ast::stmt::StringPart::Literal(sym) => {
                        let text = interner.resolve(*sym);
                        for ch in text.chars() {
                            match ch {
                                '{' => fmt_str.push_str("{{"),
                                '}' => fmt_str.push_str("}}"),
                                '\n' => fmt_str.push_str("\\n"),
                                '\t' => fmt_str.push_str("\\t"),
                                '\r' => fmt_str.push_str("\\r"),
                                // `"`/`\` sit inside the `println!("…")` literal — escape them or an
                                // embedded quote (`"key \"{k}\""`) closes the string early.
                                '"' => fmt_str.push_str("\\\""),
                                '\\' => fmt_str.push_str("\\\\"),
                                _ => fmt_str.push(ch),
                            }
                        }
                    }
                    crate::ast::stmt::StringPart::Expr {
                        value,
                        format_spec,
                        debug,
                    } => {
                        if *debug {
                            let debug_prefix = expr_debug_prefix(value, interner);
                            for ch in debug_prefix.chars() {
                                match ch {
                                    '{' => fmt_str.push_str("{{"),
                                    '}' => fmt_str.push_str("}}"),
                                    '"' => fmt_str.push_str("\\\""),
                                    '\\' => fmt_str.push_str("\\\\"),
                                    _ => fmt_str.push(ch),
                                }
                            }
                            fmt_str.push('=');
                        }
                        let arg_str = codegen_expr_with_async_and_strings(
                            value,
                            interner,
                            synced_vars,
                            async_functions,
                            ctx.get_string_vars(),
                            ctx.get_variable_types(),
                            ctx.get_fast_div(),
                            registry,
                        );
                        let (piece, arg) = super::expr::interpolation_hole(
                            format_spec.map(|s| interner.resolve(s)),
                            &arg_str,
                        );
                        fmt_str.push_str(&piece);
                        args.push(arg);
                    }
                }
            }
            writeln!(
                output,
                "{}logicaffeine_system::io::println(format_args!(\"{}\"{}));",
                indent_str,
                fmt_str,
                args.iter().map(|a| format!(", {}", a)).collect::<String>()
            )
            .unwrap();
        } else {
            let obj_str = codegen_expr_with_async_and_strings(
                object,
                interner,
                synced_vars,
                async_functions,
                ctx.get_string_vars(),
                ctx.get_variable_types(),
                ctx.get_fast_div(),
                registry,
            );
            let recv_str = recipient_path(
                recipient,
                interner,
                synced_vars,
                async_functions,
                ctx.get_variable_types(),
            );
            writeln!(
                output,
                "{}{}({});",
                indent_str,
                recv_str,
                super::expr::lend_to(recipient, &obj_str)
            )
            .unwrap();
        }
    } else {
        // Showing lends: the recipient reads the value and the caller keeps it (see `lend_to`).
        // Use string_vars for proper concatenation of string variables
        let obj_str = codegen_expr_with_async_and_strings(
            object,
            interner,
            synced_vars,
            async_functions,
            ctx.get_string_vars(),
            ctx.get_variable_types(),
            ctx.get_fast_div(),
            registry,
        );
        let recv_str = recipient_path(
            recipient,
            interner,
            synced_vars,
            async_functions,
            ctx.get_variable_types(),
        );
        writeln!(
            output,
            "{}{}({});",
            indent_str,
            recv_str,
            super::expr::lend_to(recipient, &obj_str)
        )
        .unwrap();
    }
    output
}

/// The function a `Show x to f` / `Give x to f` hands its value to: the program's function by its
/// call path (`crate::f` where a local named `f` would shadow it), or any other callee expression.
fn recipient_path(
    recipient: &Expr,
    interner: &Interner,
    synced_vars: &HashSet<Symbol>,
    async_functions: &HashSet<Symbol>,
    variable_types: &HashMap<Symbol, String>,
) -> String {
    match recipient {
        Expr::Identifier(f) => super::expr::called_function_path(
            *f,
            RustNames::new(interner).ident(*f),
            variable_types,
        ),
        other => codegen_expr_with_async(
            other,
            interner,
            synced_vars,
            async_functions,
            variable_types,
        ),
    }
}

/// The Rust type a `Let p be a PeerAgent at …` binding holds.
const PEER_AGENT_TYPE: &str = "logicaffeine_system::network::PeerAgent";

/// The `Call` arm of [`codegen_stmt`]: an arm of [`codegen_stmt`], in a frame of its own — see [`codegen_let_stmt`].
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn codegen_call_stmt<'a>(
    mut output: String,
    function: &Symbol,
    fish: &str,
    args: &[&'a Expr<'a>],
    interner: &Interner,
    ctx: &mut RefinementContext<'a>,
    synced_vars: &mut HashSet<Symbol>,
    async_functions: &HashSet<Symbol>,
    indent_str: &str,
    names: &RustNames,
) -> String {
    let func_name = super::expr::called_function_path(
        *function,
        names.ident(*function),
        ctx.get_variable_types(),
    );
    let ecx = super::expr::ExprCtx {
        async_functions,
        variable_types: ctx.get_variable_types(),
        ..super::expr::ExprCtx::bare(interner, synced_vars)
    };
    if let Some(code) = super::tower::extension_call(*function, args, &ecx) {
        writeln!(output, "{}{};", indent_str, code).unwrap();
        return output;
    }
    let args_str: Vec<String> = super::expr::call_arguments(*function, args, &ecx);
    let await_suffix = if async_functions.contains(function) {
        ".await"
    } else {
        ""
    };
    let path = format!("{func_name}{fish}");
    let call = super::expr::sequenced_call(
        &path,
        *function,
        args,
        args_str,
        await_suffix,
        ctx.get_variable_types(),
    );
    writeln!(output, "{}{};", indent_str, call).unwrap();
    output
}

/// The `Push` arm of [`codegen_stmt`]: an arm of [`codegen_stmt`], in a frame of its own — see [`codegen_let_stmt`].
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn codegen_push_stmt<'a>(
    mut output: String,
    value: &&'a Expr<'a>,
    collection: &&'a Expr<'a>,
    interner: &Interner,
    ctx: &mut RefinementContext<'a>,
    synced_vars: &mut HashSet<Symbol>,
    async_functions: &HashSet<Symbol>,
    boxed_fields: &HashSet<(String, String, String)>,
    registry: &TypeRegistry,
    indent_str: &str,
    names: &RustNames,
) -> String {
    if let Expr::Identifier(coll_sym) = collection {
        if ctx
            .get_variable_types()
            .get(coll_sym)
            .is_some_and(|t| t == super::tower::VALUE)
        {
            let pushed = super::tower::statement_value(
                value,
                interner,
                synced_vars,
                async_functions,
                boxed_fields,
                registry,
                ctx.get_variable_types(),
            );
            writeln!(
                output,
                "{}logicaffeine_data::failure::answered({}.push_value({}));",
                indent_str,
                names.ident(*coll_sym),
                pushed
            )
            .unwrap();
            return output;
        }
    }
    // O3: an init push into a scalarized `[T; N]` array becomes an
    // indexed store at the next fill position (detection proved
    // exactly N straight-line pushes precede any other use).
    if let Expr::Identifier(coll_sym) = collection {
        // Text write-side byte builder (Opt::TextFromBytesBuilder): `out` is a bare `Vec<u8>`, so
        // a push truncates the i64 byte value and appends it directly — no `LogosSeq`, no borrow,
        // matching the `v as u8` the runtime `text_from_bytes` used to do in a separate pass.
        if ctx
            .get_variable_types()
            .get(coll_sym)
            .map_or(false, |t| t == "ByteBuilder")
        {
            let val_str = codegen_expr_boxed_with_types_oracle(
                value,
                interner,
                synced_vars,
                boxed_fields,
                registry,
                async_functions,
                ctx.get_string_vars(),
                ctx.get_variable_types(),
                ctx.get_fast_div(),
                ctx.oracle(),
            );
            writeln!(
                output,
                "{}{}.push(({}) as u8);",
                indent_str,
                names.ident(*coll_sym),
                val_str
            )
            .unwrap();
            return output;
        }
        // Loop-built `[T; N]` return buffer (the digest): a push in the fill loop becomes a store
        // through the RUNTIME cursor — `out[cursor] = value; cursor += 1` — not the compile-time O3
        // fill (which would write slot 0 every iteration).
        if let Some(cursor) = ctx.loop_fill_cursor(*coll_sym).map(|s| s.to_string()) {
            let val_str = codegen_expr_boxed_with_types_oracle(
                value,
                interner,
                synced_vars,
                boxed_fields,
                registry,
                async_functions,
                ctx.get_string_vars(),
                ctx.get_variable_types(),
                ctx.get_fast_div(),
                ctx.oracle(),
            );
            let coll_name = names.ident(*coll_sym);
            writeln!(
                output,
                "{}{}[{}] = {}; {} += 1;",
                indent_str, coll_name, cursor, val_str, cursor
            )
            .unwrap();
            return output;
        }
        // Affine read-only array: the array is deleted, so its build push
        // emits nothing (reads substitute the closed form instead).
        if ctx.affine_array(*coll_sym).is_some() {
            return output;
        }
        // AoS interleaving: the k-th round-robin push of a member becomes
        // an indexed store into the backing's k-th row at this column.
        if let Some(tag) = parse_aos_tag(ctx.get_variable_types().get(coll_sym)) {
            let val_str = codegen_expr_boxed_with_types_oracle(
                value,
                interner,
                synced_vars,
                boxed_fields,
                registry,
                async_functions,
                ctx.get_string_vars(),
                ctx.get_variable_types(),
                ctx.get_fast_div(),
                ctx.oracle(),
            );
            let k = ctx.next_array_fill(*coll_sym);
            writeln!(
                output,
                "{}{}[{}][{}] = {};",
                indent_str, tag.backing, k, tag.col, val_str
            )
            .unwrap();
            return output;
        }
        if ctx.is_scalarized_array(*coll_sym) {
            let val_str = codegen_expr_boxed_with_types_oracle(
                value,
                interner,
                synced_vars,
                boxed_fields,
                registry,
                async_functions,
                ctx.get_string_vars(),
                ctx.get_variable_types(),
                ctx.get_fast_div(),
                ctx.oracle(),
            );
            let k = ctx.next_array_fill(*coll_sym);
            let coll_name = names.ident(*coll_sym);
            writeln!(output, "{}{}[{}] = {};", indent_str, coll_name, k, val_str).unwrap();
            return output;
        }
        // A1: inside a fill loop, the counted push of a sized
        // buffer-reuse buffer becomes an indexed write `buf[counter] =
        // val` (the buffer was `resize`d to the loop's trip count).
        if let Some((buf, counter)) = ctx.fill_loop().map(|(b, c)| (b, c.to_string())) {
            if buf == *coll_sym {
                let val_str = codegen_expr_boxed_with_types_oracle(
                    value,
                    interner,
                    synced_vars,
                    boxed_fields,
                    registry,
                    async_functions,
                    ctx.get_string_vars(),
                    ctx.get_variable_types(),
                    ctx.get_fast_div(),
                    ctx.oracle(),
                );
                let coll_name = names.ident(*coll_sym);
                // The buffer was `resize`d to this fill loop's trip count and
                // `counter` is its 0-based induction variable, so `counter < len`
                // holds by construction (a split-fill resizes to the full column
                // count, so every sub-loop counter stays under it). Hint it so LLVM
                // elides the bounds check on the DP-scan store — the "indexed
                // unchecked write" this transform is meant to be, matching the
                // append-only worklist path below. The `debug_assert` trips loudly
                // in tests if a future detection change ever breaks the invariant,
                // rather than the `assert_unchecked` going silently unsound.
                writeln!(output, "{}debug_assert!(({} as usize) < {}.len(), \"LOGOS fill-loop store out of range\");", indent_str, counter, coll_name).unwrap();
                writeln!(
                    output,
                    "{}unsafe {{ std::hint::assert_unchecked(({} as usize) < {}.len()); }}",
                    indent_str, counter, coll_name
                )
                .unwrap();
                writeln!(
                    output,
                    "{}{}[{} as usize] = {};",
                    indent_str, coll_name, counter, val_str
                )
                .unwrap();
                return output;
            }
        }
        // Append-only worklist: `q.push(x)` becomes a pre-sized buffer
        // write at a register tail — `q[tail] = x; tail += 1` — with NO
        // capacity check (the monotone visited-set bound proved
        // `tail < q.len()`). This is C's exact frontier code.
        if let Some(tail) = ctx.worklist(*coll_sym).map(|w| w.tail_var.clone()) {
            let val_str = codegen_expr_boxed_with_types_oracle(
                value,
                interner,
                synced_vars,
                boxed_fields,
                registry,
                async_functions,
                ctx.get_string_vars(),
                ctx.get_variable_types(),
                ctx.get_fast_div(),
                ctx.oracle(),
            );
            let coll_name = names.ident(*coll_sym);
            writeln!(
                output,
                "{}unsafe {{ std::hint::assert_unchecked({tail} < {coll_name}.len()); }}",
                indent_str
            )
            .unwrap();
            writeln!(output, "{}{coll_name}[{tail}] = {val_str};", indent_str).unwrap();
            writeln!(output, "{}{tail} += 1;", indent_str).unwrap();
            return output;
        }
    }
    let val_str = codegen_expr_boxed_with_types_oracle(
        value,
        interner,
        synced_vars,
        boxed_fields,
        registry,
        async_functions,
        ctx.get_string_vars(),
        ctx.get_variable_types(),
        ctx.get_fast_div(),
        ctx.oracle(),
    );
    // An item pushed onto a list of rationals is the rational it equals.
    let val_str = match super::rational::held_item(collection, super::rational::Typing { interner, variable_types: ctx.get_variable_types(), registry }) {
        Some(item) => rational_value(
            value,
            &item,
            interner,
            ctx,
            synced_vars,
            async_functions,
            boxed_fields,
            registry,
        )
        .unwrap_or(val_str),
        None => val_str,
    };
    let val_str = stored_value(value, val_str, ctx.get_variable_types());
    let val_str =
        value_taken_before_the_write(value, collection, val_str, ctx, indent_str, &mut output);
    let coll_str = codegen_expr_with_async(
        collection,
        interner,
        synced_vars,
        async_functions,
        ctx.get_variable_types(),
    );
    // MVS copy-on-write: a shared `LogosSeq` is deep-copied before this
    // in-place push so a value-semantics sibling is not mutated.
    emit_cow(collection, ctx, &names, &indent_str, &mut output);
    // A narrowed (`Vec<i32>`) sequence truncates on store — lossless,
    // since narrowing proved every pushed value fits i32.
    let narrowed_push = matches!(collection, Expr::Identifier(c) if ctx.is_narrowed(*c));
    if narrowed_push {
        // ★ A TYPED BINDING, NOT A CAST. Narrowing is lossless by proof, so the truncation is safe —
        // but a CAST CONSTRAINS NOTHING: `(expr) as i32` accepts any integer source type, and so does
        // `(expr) as i64`. When this store is the only use of a scalar, Rust has nothing left to infer
        // that scalar's type from and falls back to its default, `i32`. Only a typed BINDING
        // (`let n: i64 = expr;`) actually propagates the requirement back into the expression.
        //
        // MEASURED: `benchmarks/programs/prefix_sum/main.lg` seeds an LCG and pushes into `arr`. The
        // push was the one use pinning `seed` to `i64`; once `Opt::Narrow` proved `arr` fits `Vec<i32>`
        // and rewrote the push as `… as i32`, `seed` silently became `i32` and `seed * 1103515245`
        // overflowed on the first iteration — "attempt to multiply with overflow", failing
        // `counting_sort` and `prefix_sum` in `aot_native_corpus` (all 3 retries, and invisible in the
        // summary because a retried failure prints as a `TRY n FAIL` row).
        //
        // Anchoring to `i64` first says what is actually true — a LOGOS `Int` IS an i64, narrowed for
        // STORAGE — and keeps the value's width a property of the language rather than of whatever a
        // later pass leaves for inference to chew on. Fixing it here rather than at the `let` keeps the
        // blast radius at the pass that created the hazard.
        writeln!(
            output,
            "{}{}.push({{ let __nv: i64 = {}; __nv as i32 }});",
            indent_str, coll_str, val_str
        )
        .unwrap();
    } else {
        writeln!(output, "{}{}.push({});", indent_str, coll_str, val_str).unwrap();
    }
    output
}

/// `value_str`, the value a place-write stores (`Set`, a field write, an indexed store, a push, an
/// add), as a value of its own: a write stores a COPY, so a binding stored whole — a record, a
/// Text, a shared list or map, a `mutable` parameter lent as `&mut`, anything not `Copy` — is
/// cloned, and stays the program's to read after the write (a clone of a binding dead after the
/// write is one LLVM elides). Moved as they are: a borrowed slice, already a view of a copy, and
/// a buffer the de-Rc and scalarizing passes own outright (`Vec`, `HashMap`, `[T; N]`) — those
/// passes admit a buffer only when nothing reads it after it is stored.
pub(super) fn stored_value(
    value: &Expr,
    value_str: String,
    variable_types: &HashMap<Symbol, String>,
) -> String {
    let bare = match value {
        Expr::Copy { expr } => *expr,
        other => other,
    };
    if matches!(bare, Expr::FieldAccess { .. })
        && super::expr::copies_out_of_a_lend(bare, variable_types)
    {
        return format!("{value_str}.clone()");
    }
    let Expr::Identifier(sym) = bare else {
        return value_str;
    };
    if super::expr::lent_by_sharing(*sym, variable_types) {
        return format!("{value_str}.deep_clone()");
    }
    let copies = match variable_types.get(sym) {
        Some(ty) => {
            let ty = ty.split("|__hl:").next().unwrap_or(ty.as_str());
            let moved = is_copy_type(ty)
                || ty.contains("__bigint")
                || ty.starts_with('&')
                || ty.starts_with("Vec<")
                || ty.starts_with("HashMap")
                || ty.starts_with("FxHashMap")
                || ty.starts_with('[');
            !moved || super::expr::mentions_generic_param(ty)
        }
        None => true,
    };
    if copies {
        format!("{value_str}.clone()")
    } else {
        value_str
    }
}

/// `val_str`, the value a statement writes into `collection`. Every tier takes the value before it
/// touches the collection; when taking it lends the binding `collection` is rooted at, the value is
/// taken in a statement of its own (`__lent_value`), so the lend has ended by the time the write
/// borrows the collection.
fn value_taken_before_the_write(
    value: &Expr,
    collection: &Expr,
    val_str: String,
    ctx: &RefinementContext,
    indent_str: &str,
    output: &mut String,
) -> String {
    match get_root_identifier(collection) {
        Some(root) if super::expr::lends_binding(value, root, ctx.get_variable_types()) => {
            writeln!(output, "{}let __lent_value = {};", indent_str, val_str).unwrap();
            "__lent_value".to_string()
        }
        _ => val_str,
    }
}

/// The `Return` arm of [`codegen_stmt`]: an arm of [`codegen_stmt`], in a frame of its own — see [`codegen_let_stmt`].
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn codegen_return_stmt<'a>(
    mut output: String,
    value: &Option<&'a Expr<'a>>,
    interner: &Interner,
    ctx: &mut RefinementContext<'a>,
    synced_vars: &mut HashSet<Symbol>,
    async_functions: &HashSet<Symbol>,
    boxed_fields: &HashSet<(String, String, String)>,
    registry: &TypeRegistry,
    indent_str: &str,
) -> String {
    // A function whose result holds a value-or-nothing returns `nothing`, another whole, or a value present.
    if let (Some(v), Some(held)) = (value, super::absent::returning()) {
        let ecx = operand_ctx(interner, ctx, synced_vars, async_functions, boxed_fields, registry);
        if let Some(code) = super::absent::stored(v, &held, &ecx) {
            writeln!(output, "{}return {};", indent_str, code).unwrap();
            return output;
        }
    }
    if let (Some(v), true) = (value, super::tower::emitting_a_reached_function()) {
        let code = super::tower::statement_value(
            v,
            interner,
            synced_vars,
            async_functions,
            boxed_fields,
            registry,
            ctx.get_variable_types(),
        );
        writeln!(output, "{}return {};", indent_str, code).unwrap();
        return output;
    }
    // A function whose result holds rationals returns an integer as the rational it equals.
    if let (Some(v), Some(held)) = (value, ctx.returns_rationals().cloned()) {
        if let Some(code) = rational_value(
            v,
            &held,
            interner,
            ctx,
            synced_vars,
            async_functions,
            boxed_fields,
            registry,
        ) {
            writeln!(output, "{}return {};", indent_str, code).unwrap();
            return output;
        }
    }
    if let Some(v) = value {
        // A `-> LogosInt` function returns its value un-narrowed (`.into()` is identity for a
        // LogosInt, and promotes a plain `i64`), so an overflowing result stays a BigInt.
        if ctx.returns_bigint() {
            // `Return p` of a bignum local is p's last use — move it out, no clone.
            let movable_ret = matches!(v, Expr::Identifier(sym) if ctx.is_bigint_var(*sym));
            if let Expr::Identifier(sym) = v {
                if movable_ret {
                    set_movable_last_use(Some(*sym));
                }
            }
            let raw = codegen_expr_boxed_with_types_oracle_tolerant(
                v,
                interner,
                synced_vars,
                boxed_fields,
                registry,
                async_functions,
                ctx.get_string_vars(),
                ctx.get_variable_types(),
                ctx.get_fast_div(),
                ctx.oracle(),
            );
            if movable_ret {
                set_movable_last_use(None);
            }
            writeln!(
                output,
                "{}return logicaffeine_data::LogosInt::from({});",
                indent_str, raw
            )
            .unwrap();
            return output;
        }
        let value_str = codegen_expr_boxed_with_types_oracle(
            v,
            interner,
            synced_vars,
            boxed_fields,
            registry,
            async_functions,
            ctx.get_string_vars(),
            ctx.get_variable_types(),
            ctx.get_fast_div(),
            ctx.oracle(),
        );
        // If returning a borrowed slice param (or copy of one), wrap in LogosSeq
        let slice_sym = match v {
            Expr::Identifier(sym) => Some(*sym),
            Expr::Copy { expr: inner } => {
                if let Expr::Identifier(sym) = inner {
                    Some(*sym)
                } else {
                    None
                }
            }
            _ => None,
        };
        let needs_to_vec = slice_sym
            .and_then(|sym| ctx.get_variable_types().get(&sym))
            .map(|t| t.starts_with("&["))
            .unwrap_or(false);
        // A scalarized `[T; N]` stack buffer (scratch / indexed / cursor-fill) that
        // is `return`ed by a fn whose DECLARED result is a heap `Vec<T>` — the
        // local scalarization and the heap return type disagree, so materialize the
        // owned copy (`[T; N]` exposes `.to_vec()` just like `&[T]`). A PROPER
        // array-return fn has `returns_vec() == false` and stays a direct `return`.
        let ret_ty_is_scalar_array = slice_sym
            .and_then(|sym| ctx.get_variable_types().get(&sym))
            .map(|t| t.starts_with('['))
            .unwrap_or(false);
        // If returning a &mut borrow param, the mutation was in-place — just return
        let is_mut_borrow_return = if let Expr::Identifier(sym) = v {
            ctx.get_variable_types()
                .get(sym)
                .map(|t| t.starts_with("&mut ["))
                .unwrap_or(false)
        } else {
            false
        };
        if is_mut_borrow_return {
            writeln!(output, "{}return;", indent_str).unwrap();
        } else if (needs_to_vec || ret_ty_is_scalar_array) && ctx.returns_vec() {
            // Phase 4: the function returns owned `Vec<T>` — return the
            // owned copy directly, no `LogosSeq::from_vec(...)` wrapper.
            writeln!(output, "{}return {}.to_vec();", indent_str, value_str).unwrap();
        } else if needs_to_vec {
            writeln!(
                output,
                "{}return LogosSeq::from_vec({}.to_vec());",
                indent_str, value_str
            )
            .unwrap();
        } else if super::expr::copies_out_of_a_lend(v, ctx.get_variable_types()) {
            writeln!(output, "{}return {}.clone();", indent_str, value_str).unwrap();
        } else {
            writeln!(output, "{}return {};", indent_str, value_str).unwrap();
        }
    } else {
        writeln!(output, "{}return;", indent_str).unwrap();
    }
    output
}

/// What the `Serve` arm needs to know about the whole program: whether it declares a `## Session`,
/// and which of its functions take the program's bindings ([`crate::program_bindings`]) first.
#[derive(Default)]
struct ServeFacts {
    declares_session: bool,
    lends_program: HashSet<Symbol>,
}

thread_local! {
    /// [`ServeFacts`] for the program being emitted — installed once at codegen entry, where the
    /// whole program is visible, and read by the `Serve` arm, which reaches a statement with no
    /// program in hand.
    static SERVE_FACTS: std::cell::RefCell<ServeFacts> = std::cell::RefCell::new(ServeFacts::default());
}

pub(crate) fn set_serve_facts(stmts: &[Stmt], interner: &Interner) {
    let program = interner.lookup(crate::program_bindings::PROGRAM_BINDING);
    let lends_program = stmts
        .iter()
        .filter_map(|s| match s {
            Stmt::FunctionDef { name, params, .. }
                if program.is_some() && params.first().map(|(p, _)| *p) == program =>
            {
                Some(*name)
            }
            _ => None,
        })
        .collect();
    let declares_session =
        logicaffeine_language::parser::lgx_lower::declares_session(stmts, interner);
    SERVE_FACTS.with(|facts| {
        *facts.borrow_mut() = ServeFacts {
            declares_session,
            lends_program,
        }
    });
}

/// The emitted conversion of the handler's `Response` bound to `resp` into the wire's
/// `HttpResponse`, as the tail expression of the block it is written into. The response's headers
/// merge through the shared denylist and response-splitting filter, then the authoritative (and
/// wire-safe) Content-Type is set last; a non-empty `bytes` field is a raw binary body (a `.wasm`,
/// an image …) and wins over the text `body`, each Int one byte — the interpreter's
/// `value_to_http_response` rule, byte for byte.
fn emit_wire_response(output: &mut String, indent: &str, resp: &str) {
    writeln!(
        output,
        "{indent}let mut __headers = std::collections::BTreeMap::new();"
    )
    .unwrap();
    writeln!(
        output,
        "{indent}for (__k, __v) in {resp}.headers.borrow().iter() {{"
    )
    .unwrap();
    writeln!(output, "{indent}    if logicaffeine_system::http::wire::response_header_allowed(__k, __v) {{ __headers.insert(__k.clone(), __v.clone()); }}").unwrap();
    writeln!(output, "{indent}}}").unwrap();
    writeln!(output, "{indent}__headers.insert(\"Content-Type\".to_string(), logicaffeine_system::http::wire::safe_content_type(&{resp}.contentType).to_string());").unwrap();
    writeln!(
        output,
        "{indent}let __bytes: Vec<u8> = {resp}.bytes.iter().map(|__b| __b as u8).collect();"
    )
    .unwrap();
    writeln!(output, "{indent}let __body = if __bytes.is_empty() {{ {resp}.body.into_bytes() }} else {{ __bytes }};").unwrap();
    writeln!(output, "{indent}logicaffeine_system::http::wire::HttpResponse {{ status: {resp}.status as u16, headers: __headers, body: __body }}").unwrap();
}

/// The `Serve` arm of [`codegen_stmt`]: an arm of [`codegen_stmt`], in a frame of its own — see [`codegen_let_stmt`].
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn codegen_serve_stmt<'a>(
    mut output: String,
    port: &&'a Expr<'a>,
    handler: &&'a Expr<'a>,
    interner: &Interner,
    ctx: &mut RefinementContext<'a>,
    synced_vars: &mut HashSet<Symbol>,
    async_functions: &HashSet<Symbol>,
    indent_str: &str,
) -> String {
    let port_str = codegen_expr_with_async(
        port,
        interner,
        synced_vars,
        async_functions,
        ctx.get_variable_types(),
    );
    let handler_sym = match handler {
        Expr::Identifier(sym) => *sym,
        _ => {
            writeln!(
                output,
                "{}compile_error!(\"Serve: the handler must be a named function\");",
                indent_str
            )
            .unwrap();
            return output;
        }
    };
    let handler_name = interner.resolve(handler_sym).to_string();
    let (declares_session, lends_program) = SERVE_FACTS.with(|facts| {
        let facts = facts.borrow();
        (
            facts.declares_session,
            facts.lends_program.contains(&handler_sym),
        )
    });
    // ★ A HANDLER THAT TAKES THE PROGRAM'S BINDINGS is lent them on every call, and is served on
    // this thread (`serve_here`): the bindings hold collections that cannot cross to the engine's
    // threads. Calls stay one at a time, as `serve` serializes them. The record is the local
    // `## Main` holds, or the `mutable` parameter of a function that serves.
    let lend = if lends_program {
        let program = crate::program_bindings::PROGRAM_BINDING;
        let through_a_parameter = interner
            .lookup(program)
            .and_then(|sym| ctx.get_variable_types().get(&sym))
            .is_some_and(|ty| ty.starts_with('&'));
        if through_a_parameter {
            format!("&mut *{program}, ")
        } else {
            format!("&mut {program}, ")
        }
    } else {
        String::new()
    };
    // `LOGOS_HOST` names the interface (default loopback) and `LOGOS_PORT` (then
    // `PORT`) overrides the program's port expression — the same
    // `env_bind_address` seam the interpreter tier applies, so dev proxying and
    // PaaS deployment work identically on both tiers. The HOST matters here in
    // particular: this is the tier a site actually SHIPS as, and a binary that
    // binds loopback inside a container is unreachable from outside it.
    // An embedded site whose pages talk to the relay answers `/_relay` itself — a deployed page
    // can dial nothing else. Its pages decided that at build time (`logos_assets::USES_RELAY`);
    // a binary with no pages serves exactly as it always did.
    let upgrades = crate::codegen::embed_assets_enabled().then(|| {
        "if logos_assets::USES_RELAY { logicaffeine_system::relay::RelayHub::new().upgrades() } else { ::std::vec::Vec::new() }"
            .to_string()
    });
    let entry = match (lends_program, upgrades.is_some()) {
        (false, false) => "serve",
        (false, true) => "serve_with_upgrades",
        (true, false) => "serve_here",
        (true, true) => "serve_here_with_upgrades",
    };
    writeln!(output, "{indent_str}logicaffeine_system::http::server::{entry}(&logicaffeine_system::http::server::env_bind_address(logicaffeine_system::http::server::env_port_override(({port_str}) as u16)), |__raw: logicaffeine_system::http::wire::HttpRequest| {{").unwrap();
    // B4 — embedded full-stack site (a `largo build` web binary): consult the
    // bundled `logos_assets` table on the REQUEST PATH before the handler runs, so
    // a page/static asset shadows a backend route — exactly the dev router's
    // precedence (`serve_web::route_local`), making dev ≡ prod by construction.
    // A `GET` hits the table; anything else falls through to the handler, because a
    // POST to a path a file also occupies belongs to the handler. A QUERY STRING does
    // NOT disqualify a request: `logos_assets::lookup` matches by path (and splits a
    // query off itself), so `/style.css?v=2` — an ordinary cache-buster — serves here
    // exactly as it does in `largo serve`. Gating on an empty query made that 404 in
    // production while working all through development. The
    // scope is off by default, so a plain `Serve` program lowers byte-identically.
    if crate::codegen::embed_assets_enabled() {
        writeln!(output, "{indent_str}    if __raw.method == \"GET\" {{").unwrap();
        writeln!(output, "{indent_str}        if let Some((__ct, __cc, __et, __asset)) = logos_assets::lookup(&__raw.path) {{").unwrap();
        writeln!(
            output,
            "{indent_str}            let mut __ah = std::collections::BTreeMap::new();"
        )
        .unwrap();
        writeln!(
            output,
            "{indent_str}            __ah.insert(\"Content-Type\".to_string(), __ct.to_string());"
        )
        .unwrap();
        // The cache headers travel with the bytes: a content-addressed asset is cached for a year
        // and never revalidated (a different body would be a different URL), everything else is
        // revalidated cheaply against its ETag.
        writeln!(
            output,
            "{indent_str}            __ah.insert(\"Cache-Control\".to_string(), __cc.to_string());"
        )
        .unwrap();
        writeln!(
            output,
            "{indent_str}            __ah.insert(\"ETag\".to_string(), __et.to_string());"
        )
        .unwrap();
        // ★ A PAGE OF A SERVER WITH A `## Session` IS COMPOSED FOR ITS VISITOR, before any cache
        // validator is consulted: the handler is asked for the visitor's token at the router's
        // reserved path, with the visitor's own cookie, and the document goes out with that token
        // in its `<meta name="csrf-token">` — where the page's scripted requests read it — the
        // answer's `Set-Cookie`, and `no-store`, since the document now belongs to one visitor.
        // `largo serve` composes a page the same way (`serve_web::page_for_visitor`). A handler
        // that answers no token leaves the page as it was.
        if declares_session {
            let token_path = logicaffeine_language::parser::lgx_lower::CSRF_TOKEN_PATH;
            writeln!(
                output,
                "{indent_str}            if logos_assets::is_page(&__raw.path) {{"
            )
            .unwrap();
            writeln!(
                output,
                "{indent_str}                let __th = LogosMap::new();"
            )
            .unwrap();
            writeln!(output, "{indent_str}                if let Some((_, __c)) = __raw.headers.iter().find(|(__k, _)| __k.eq_ignore_ascii_case(\"cookie\")) {{ __th.insert(\"cookie\".to_string(), __c.clone()); }}").unwrap();
            writeln!(output, "{indent_str}                let __tr = {handler_name}({lend}Request {{ method: \"GET\".to_string(), path: {token_path:?}.to_string(), body: String::new(), headers: __th, query: LogosMap::new(), ..Default::default() }});").unwrap();
            writeln!(output, "{indent_str}                let __tok = {{").unwrap();
            emit_wire_response(
                &mut output,
                &format!("{indent_str}                    "),
                "__tr",
            );
            writeln!(output, "{indent_str}                }};").unwrap();
            writeln!(output, "{indent_str}                let __page = logicaffeine_system::http::wire::HttpResponse {{ status: 200, headers: __ah.clone(), body: __asset.to_vec() }};").unwrap();
            writeln!(output, "{indent_str}                if let Some(__composed) = logicaffeine_system::http::server::page_with_visitor_token(&__page, &__tok) {{ return __composed; }}").unwrap();
            writeln!(output, "{indent_str}            }}").unwrap();
        }
        // ★ NEGOTIATE, DECLARE, VARY — one contract in three headers. A compressed body
        // sent to a client that did not ask for one is unreadable; a compressed body sent
        // without `Vary: Accept-Encoding` is one a shared cache will hand to the next
        // client whether or not it asked. And the variant carries its OWN ETag, because
        // two bodies under one entity tag is exactly how that cache mix-up happens.
        writeln!(output, "{indent_str}            let __ae = __raw.headers.iter().find(|(__k, _)| __k.eq_ignore_ascii_case(\"accept-encoding\")).map(|(_, __v)| __v.as_str()).unwrap_or(\"\");").unwrap();
        writeln!(output, "{indent_str}            let __gz = if __ae.split(',').any(|__e| __e.trim().split(';').next().unwrap_or(\"\").eq_ignore_ascii_case(\"gzip\")) {{ logos_assets::encoded(&__raw.path) }} else {{ None }};").unwrap();
        writeln!(output, "{indent_str}            if __gz.is_some() {{ __ah.insert(\"Vary\".to_string(), \"Accept-Encoding\".to_string()); }}").unwrap();
        writeln!(output, "{indent_str}            let (__et, __body_src): (&str, &[u8]) = match __gz {{ Some((__e, __b)) => {{ __ah.insert(\"Content-Encoding\".to_string(), \"gzip\".to_string()); (__e, __b) }}, None => (__et, __asset) }};").unwrap();
        writeln!(output, "{indent_str}            __ah.insert(\"ETag\".to_string(), format!(\"\\\"{{}}\\\"\", __et.trim_matches('\\\"')));").unwrap();
        writeln!(output, "{indent_str}            let __inm = __raw.headers.iter().find(|(__k, _)| __k.eq_ignore_ascii_case(\"if-none-match\")).map(|(_, __v)| __v.as_str());").unwrap();
        writeln!(
            output,
            "{indent_str}            if logos_assets::fresh(__et, __inm) {{"
        )
        .unwrap();
        writeln!(output, "{indent_str}                return logicaffeine_system::http::wire::HttpResponse {{ status: 304, headers: __ah, body: Vec::new() }};").unwrap();
        writeln!(output, "{indent_str}            }}").unwrap();
        writeln!(output, "{indent_str}            return logicaffeine_system::http::wire::HttpResponse {{ status: 200, headers: __ah, body: __body_src.to_vec() }};").unwrap();
        writeln!(output, "{indent_str}        }}").unwrap();
        writeln!(output, "{indent_str}    }}").unwrap();
    }
    // Headers (wire-lowercased names) and query (percent-decoded) ride
    // into the Request as LogosMaps, inserted in the wire map's sorted
    // order — the interpreter's `http_request_to_value`, byte for byte.
    writeln!(output, "{indent_str}    let __hdrs = LogosMap::new();").unwrap();
    writeln!(
        output,
        "{indent_str}    for (__k, __v) in __raw.headers {{ __hdrs.insert(__k, __v); }}"
    )
    .unwrap();
    writeln!(output, "{indent_str}    let __qry = LogosMap::new();").unwrap();
    writeln!(
        output,
        "{indent_str}    for (__k, __v) in __raw.query {{ __qry.insert(__k, __v); }}"
    )
    .unwrap();
    writeln!(output, "{indent_str}    let __req = Request {{ method: __raw.method, path: __raw.path, body: String::from_utf8_lossy(&__raw.body).into_owned(), headers: __hdrs, query: __qry, ..Default::default() }};").unwrap();
    writeln!(
        output,
        "{indent_str}    let __resp = {handler_name}({lend}__req);"
    )
    .unwrap();
    emit_wire_response(&mut output, &format!("{indent_str}    "), "__resp");
    match upgrades {
        Some(table) => {
            writeln!(output, "{indent_str}}}, {table}).await.expect(\"serve\");").unwrap()
        }
        None => writeln!(output, "{indent_str}}}).await.expect(\"serve\");").unwrap(),
    }
    output
}

/// The `SetIndex` arm of [`codegen_stmt`], in a frame of its own — see [`codegen_let_stmt`].
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn codegen_set_index_stmt<'a>(
    mut output: String,
    collection: &&'a Expr<'a>,
    index: &&'a Expr<'a>,
    value: &&'a Expr<'a>,
    interner: &Interner,
    ctx: &mut RefinementContext<'a>,
    synced_vars: &mut HashSet<Symbol>,
    async_functions: &HashSet<Symbol>,
    boxed_fields: &HashSet<(String, String, String)>,
    registry: &TypeRegistry,
    indent_str: &str,
    names: &RustNames,
) -> String {
    if let Expr::Identifier(coll_sym) = collection {
        if ctx
            .get_variable_types()
            .get(coll_sym)
            .is_some_and(|t| t == super::tower::VALUE)
        {
            let value_of = |e: &Expr| {
                super::tower::statement_value(
                    e,
                    interner,
                    synced_vars,
                    async_functions,
                    boxed_fields,
                    registry,
                    ctx.get_variable_types(),
                )
            };
            let (at, stored) = (value_of(index), value_of(value));
            writeln!(
                output,
                "{}logicaffeine_data::failure::answered({}.set_item(&({}), {}));",
                indent_str,
                names.ident(*coll_sym),
                at,
                stored
            )
            .unwrap();
            return output;
        }
    }
    // Nested value-semantic THROUGH-WRITE `grid[k][i] = v` (a fused place-write, see
    // splice_fuse): cow the outer grid, then `set_nested` cow's the ROW only if it is shared
    // — O(1) when uniquely owned, versus the desugar's unconditional full-row clone. Scoped
    // to a `LogosSeq<LogosSeq<…>>` base (the nested-array case); any other nested shape falls
    // through to the general handling below, and the compiled-vs-interpreted differential
    // guards value-semantics soundness for every shape.
    if let Expr::Index {
        collection: base,
        index: outer_idx,
    } = collection
    {
        if let Expr::Identifier(base_sym) = base {
            let is_nested_seq = ctx.get_variable_types().get(base_sym).map_or(false, |t| {
                t.split("|__hl:")
                    .next()
                    .unwrap_or(t)
                    .starts_with("LogosSeq<LogosSeq")
            });
            if is_nested_seq {
                let value_str = codegen_expr_boxed_with_types_oracle(
                    value,
                    interner,
                    synced_vars,
                    boxed_fields,
                    registry,
                    async_functions,
                    ctx.get_string_vars(),
                    ctx.get_variable_types(),
                    ctx.get_fast_div(),
                    ctx.oracle(),
                );
                let mut k_idx =
                    super::peephole::logos_index(outer_idx, interner, ctx.get_variable_types());
                let mut i_idx = super::peephole::logos_index(index, interner, ctx.get_variable_types());
                let mut value_str = value_str;
                // Operands one of which can change what another reads are taken in the language's order — the
                // item's index, the value, then the row's — each in a statement of its own, before the store reads
                // any of them.
                if ordered(&[*index, *value, *outer_idx]) {
                    let full = |e: &Expr| {
                        codegen_expr_with_async(e, interner, synced_vars, async_functions, ctx.get_variable_types())
                    };
                    let (i_full, k_full) = (full(index), full(outer_idx));
                    writeln!(output, "{indent_str}let __place_index = {i_full};").unwrap();
                    writeln!(output, "{indent_str}let __place_value = {value_str};").unwrap();
                    writeln!(output, "{indent_str}let __place_key = {k_full};").unwrap();
                    (i_idx, value_str, k_idx) =
                        ("__place_index".to_string(), "__place_value".to_string(), "__place_key".to_string());
                }
                emit_cow(base, ctx, &names, &indent_str, &mut output);
                writeln!(
                    output,
                    "{}{}.set_nested({}, {}, {});",
                    indent_str,
                    names.ident(*base_sym),
                    k_idx,
                    i_idx,
                    value_str
                )
                .unwrap();
                return output;
            }
        }
    }
    // AoS interleaving: `Set item i of member to v` → `backing[(i-1)][col] = v`.
    if let Expr::Identifier(sym) = collection {
        if let Some(tag) = parse_aos_tag(ctx.get_variable_types().get(sym)) {
            let value_str = codegen_expr_boxed_with_types_oracle(
                value,
                interner,
                synced_vars,
                boxed_fields,
                registry,
                async_functions,
                ctx.get_string_vars(),
                ctx.get_variable_types(),
                ctx.get_fast_div(),
                ctx.oracle(),
            );
            let at = super::peephole::logos_index(index, interner, ctx.get_variable_types());
            let item = super::expr::item_accessor(index, ctx.oracle(), true);
            writeln!(
                output,
                "{}{}.{}({})[{}] = {};",
                indent_str, tag.backing, item, at, tag.col, value_str
            )
            .unwrap();
            return output;
        }
    }
    let coll_str = codegen_expr_with_async(
        collection,
        interner,
        synced_vars,
        async_functions,
        ctx.get_variable_types(),
    );
    let value_str = codegen_expr_boxed_with_types_oracle(
        value,
        interner,
        synced_vars,
        boxed_fields,
        registry,
        async_functions,
        ctx.get_string_vars(),
        ctx.get_variable_types(),
        ctx.get_fast_div(),
        ctx.oracle(),
    );
    // An item stored into a list of rationals is the rational it equals.
    let value_str = match super::rational::held_item(collection, super::rational::Typing { interner, variable_types: ctx.get_variable_types(), registry }) {
        Some(item) => rational_value(
            value,
            &item,
            interner,
            ctx,
            synced_vars,
            async_functions,
            boxed_fields,
            registry,
        )
        .unwrap_or(value_str),
        None => value_str,
    };
    let value_str = stored_value(value, value_str, ctx.get_variable_types());

    // A rational names the item it stores into as the integer it equals.
    let rational_index = {
        let ecx = super::expr::ExprCtx {
            async_functions,
            variable_types: ctx.get_variable_types(),
            ..super::expr::ExprCtx::bare(interner, synced_vars)
        };
        super::rational::item_index(
            collection,
            index,
            crate::semantics::collections::IntegerPlace::ListStore,
            &ecx,
        )
    };
    if let Some(at) = rational_index {
        writeln!(output, "{}let __rational_index: i64 = {};", indent_str, at).unwrap();
        writeln!(output, "{}let __rational_value = {};", indent_str, value_str).unwrap();
        emit_cow(collection, ctx, &names, &indent_str, &mut output);
        writeln!(
            output,
            "{}LogosIndexMut::logos_set(&mut {}, __rational_index, __rational_value);",
            indent_str, coll_str
        )
        .unwrap();
        return output;
    }

    // The index and then the value are taken before the collection is touched; when taking
    // either calls, or lends the collection's own binding, each is taken in a statement of its own,
    // so the index is read before the value can change it and every lend has ended by the time the
    // store borrows.
    if let Some(root) = get_root_identifier(collection) {
        let types = ctx.get_variable_types();
        if ordered(&[*index, *value])
            || super::expr::lends_binding(index, root, types)
            || super::expr::lends_binding(value, root, types)
        {
            let index_str = codegen_expr_with_async(
                index,
                interner,
                synced_vars,
                async_functions,
                ctx.get_variable_types(),
            );
            writeln!(output, "{}let __lent_index = {};", indent_str, index_str).unwrap();
            writeln!(output, "{}let __lent_value = {};", indent_str, value_str).unwrap();
            emit_cow(collection, ctx, &names, &indent_str, &mut output);
            writeln!(
                output,
                "{}LogosIndexMut::logos_set(&mut {}, __lent_index, __lent_value);",
                indent_str, coll_str
            )
            .unwrap();
            return output;
        }
    }

    // MVS copy-on-write before the in-place indexed store / map insert.
    // A no-op for the `Vec`/slice/array and `FxHashMap` arms below (they
    // are not shared `Rc`s); fires only for a real `LogosSeq`/`LogosMap`.
    emit_cow(collection, ctx, &names, &indent_str, &mut output);

    // Direct indexing for known collection types (avoids trait dispatch)
    // Strip |__hl: hoisting suffix so type matching works correctly.
    let known_type = if let Expr::Identifier(sym) = collection {
        ctx.get_variable_types()
            .get(sym)
            .map(|s| s.split("|__hl:").next().unwrap_or(s.as_str()))
    } else {
        None
    };

    match known_type {
        // A bit-packed `Seq of Bool` (`Opt::BitPack`): the store sets or clears the bit the index names, unchecked
        // where the oracle proves the index in range, where `true` is one `or` into its word.
        Some("LogosBits") => {
            let store_unchecked = crate::optimize::active_config()
                .is_on(crate::optimization::Opt::Unchecked)
                && ctx
                    .oracle()
                    .map_or(false, |o| o.index_provably_in_bounds(collection, index));
            if store_unchecked {
                crate::optimize::mark_fired(crate::optimization::Opt::Unchecked);
                let zb_index =
                    super::expr::zero_based_index_str(index, ctx.get_variable_types(), interner);
                let i0 = match &zb_index {
                    Some(s) => s.clone(),
                    None => simplify_1based_index(index, interner, false, ctx.get_variable_types()),
                };
                let offset = store_offset(index, zb_index.as_deref(), interner, ctx);
                writeln!(output, "{}debug_assert!(({}) >= 0 && ({}) < ({}.len() as i64), \"LOGOS oracle BCE: store out of range\");", indent_str, i0, i0, coll_str).unwrap();
                if matches!(value, Expr::Literal(Literal::Boolean(true))) {
                    writeln!(
                        output,
                        "{}unsafe {{ {}.set_true_unchecked({}); }}",
                        indent_str, coll_str, offset
                    )
                    .unwrap();
                } else {
                    writeln!(output, "{}let __set_val: bool = {};", indent_str, value_str).unwrap();
                    writeln!(
                        output,
                        "{}unsafe {{ {}.set_unchecked({}, __set_val); }}",
                        indent_str, coll_str, offset
                    )
                    .unwrap();
                }
            } else {
                let at = super::peephole::logos_index(index, interner, ctx.get_variable_types());
                writeln!(output, "{}let __set_val: bool = {};", indent_str, value_str).unwrap();
                writeln!(
                    output,
                    "{}{}.set_item({}, __set_val);",
                    indent_str, coll_str, at
                )
                .unwrap();
            }
        }
        Some(t) if t.starts_with("LogosSeq") => {
            // Use borrow_mut() for interior mutability — no &mut needed on the variable.
            let zb_index =
                super::expr::zero_based_index_str(index, ctx.get_variable_types(), interner);
            let at = super::peephole::logos_index(index, interner, ctx.get_variable_types());
            // BCE: when the oracle proves the store index in range, emit
            // an unchecked store through the borrow (no bounds branch)
            // with a `debug_assert!` net — the value-semantic analog of
            // the `Vec` path below. Soundness: the kernel-LIA proof + the
            // function entry precondition. `LOGOS_ORACLE_UNCHECKED=0` forces
            // checked. The `borrow_mut()` sees the owned buffer (a `cow()`
            // was emitted before this store for value bindings).
            let store_unchecked = crate::optimize::active_config()
                .is_on(crate::optimization::Opt::Unchecked)
                && ctx
                    .oracle()
                    .map_or(false, |o| o.index_provably_in_bounds(collection, index));
            // A store the range oracle proves in range is a plain indexed store, whose own check never fires.
            let place = if super::expr::range_proves_index(collection, index) {
                format!(
                    "{}.borrow_mut()[{}]",
                    coll_str,
                    store_offset(index, zb_index.as_deref(), interner, ctx)
                )
            } else {
                format!(
                    "*{}.borrow_mut().{}({})",
                    coll_str,
                    super::expr::item_accessor(index, ctx.oracle(), true),
                    at
                )
            };
            // If the RHS borrows ANY collection, evaluate it into a temp
            // first: under reference semantics that collection may alias
            // the target (e.g. `prev` after `Set prev to curr`), and a
            // live borrow() across the target's borrow_mut() panics.
            let needs_tmp = store_unchecked || expr_reads_any_collection(value);
            if store_unchecked {
                crate::optimize::mark_fired(crate::optimization::Opt::Unchecked);
                let i0 = match &zb_index {
                    Some(s) => s.clone(),
                    None => simplify_1based_index(index, interner, false, ctx.get_variable_types()),
                };
                let offset = store_offset(index, zb_index.as_deref(), interner, ctx);
                writeln!(output, "{}debug_assert!(({}) >= 0 && ({}) < ({}.len() as i64), \"LOGOS oracle BCE: store out of range\");", indent_str, i0, i0, coll_str).unwrap();
                writeln!(output, "{}let __set_val = {};", indent_str, value_str).unwrap();
                writeln!(
                    output,
                    "{}unsafe {{ *{}.borrow_mut().get_unchecked_mut({}) = __set_val; }}",
                    indent_str, coll_str, offset
                )
                .unwrap();
            } else if needs_tmp {
                writeln!(output, "{}let __set_val = {};", indent_str, value_str).unwrap();
                writeln!(output, "{}{} = __set_val;", indent_str, place).unwrap();
            } else {
                writeln!(output, "{}{} = {};", indent_str, place, value_str).unwrap();
            }
        }
        Some(t) if is_logos_map_type(t) => {
            let index_str = codegen_expr_with_async(
                index,
                interner,
                synced_vars,
                async_functions,
                ctx.get_variable_types(),
            );
            writeln!(
                output,
                "{}{}.insert({}, {});",
                indent_str, coll_str, index_str, value_str
            )
            .unwrap();
        }
        Some(t)
            if t.starts_with("Vec")
                || t.starts_with("&mut [")
                || t.starts_with("&[")
                || t.starts_with('[') =>
        {
            // Vec / slice / fixed array (O3) — a store through the item the index names.
            let zb_index =
                super::expr::zero_based_index_str(index, ctx.get_variable_types(), interner);
            // A narrowed (`Vec<i32>`) sequence truncates on store — lossless,
            // since narrowing proved every stored value fits i32.
            // Same anchor as the narrowed push above, and for the same reason: a typed binding,
            // because a cast would leave the stored expression's own type to inference.
            let narrowed = matches!(collection, Expr::Identifier(c) if ctx.is_narrowed(*c));
            let value_str = if narrowed {
                format!("{{ let __nv: i64 = {}; __nv as i32 }}", value_str)
            } else {
                value_str
            };
            // BCE: when the oracle proves the store index in range, emit an
            // unchecked store (no bounds branch) with a `debug_assert!` net.
            // Soundness: the kernel-LIA proof + the function's entry
            // precondition guard. `LOGOS_ORACLE_UNCHECKED=0` forces checked.
            let store_unchecked = crate::optimize::active_config()
                .is_on(crate::optimization::Opt::Unchecked)
                && ctx
                    .oracle()
                    .map_or(false, |o| o.index_provably_in_bounds(collection, index));
            if store_unchecked {
                crate::optimize::mark_fired(crate::optimization::Opt::Unchecked);
                let i0 = match &zb_index {
                    Some(s) => s.clone(),
                    None => simplify_1based_index(index, interner, false, ctx.get_variable_types()),
                };
                let offset = store_offset(index, zb_index.as_deref(), interner, ctx);
                writeln!(output, "{}debug_assert!(({}) >= 0 && ({}) < ({}.len() as i64), \"LOGOS oracle BCE: store out of range\");", indent_str, i0, i0, coll_str).unwrap();
                // Bind the value to a temp first: it may itself be a proven
                // `get_unchecked` read of the same slice (the partition swap),
                // so evaluating it before the `&mut` access avoids both a
                // nested `unsafe` block and an aliasing borrow.
                writeln!(output, "{}let __set_val = {};", indent_str, value_str).unwrap();
                writeln!(
                    output,
                    "{}unsafe {{ *{}.get_unchecked_mut({}) = __set_val; }}",
                    indent_str, coll_str, offset
                )
                .unwrap();
            } else if super::expr::range_proves_index(collection, index) {
                // A store the range oracle proves in range is a plain indexed store, whose own check never fires.
                let offset = store_offset(index, zb_index.as_deref(), interner, ctx);
                writeln!(
                    output,
                    "{}{}[{}] = {};",
                    indent_str, coll_str, offset, value_str
                )
                .unwrap();
            } else {
                let at = super::peephole::logos_index(index, interner, ctx.get_variable_types());
                writeln!(
                    output,
                    "{}*{}.{}({}) = {};",
                    indent_str,
                    coll_str,
                    super::expr::item_accessor(index, ctx.oracle(), true),
                    at,
                    value_str
                )
                .unwrap();
            }
        }
        Some(t)
            if t.starts_with("std::collections::HashMap")
                || t.starts_with("HashMap")
                || t.starts_with("rustc_hash::FxHashMap")
                || t.starts_with("FxHashMap") =>
        {
            let index_str = codegen_expr_with_async(
                index,
                interner,
                synced_vars,
                async_functions,
                ctx.get_variable_types(),
            );
            writeln!(
                output,
                "{}{}.insert({}, {});",
                indent_str, coll_str, index_str, value_str
            )
            .unwrap();
        }
        _ => {
            let index_str = codegen_expr_with_async(
                index,
                interner,
                synced_vars,
                async_functions,
                ctx.get_variable_types(),
            );
            // Fallback: polymorphic indexing via trait.
            // If the value expression reads ANY collection, bind it to a
            // temp first: that collection may alias the one being written
            // (reference semantics), and a live borrow across the mutable
            // access would conflict.
            let needs_tmp = expr_reads_any_collection(value);
            if needs_tmp {
                writeln!(output, "{}let __set_tmp = {};", indent_str, value_str).unwrap();
                writeln!(
                    output,
                    "{}LogosIndexMut::logos_set(&mut {}, {}, __set_tmp);",
                    indent_str, coll_str, index_str
                )
                .unwrap();
            } else {
                writeln!(
                    output,
                    "{}LogosIndexMut::logos_set(&mut {}, {}, {});",
                    indent_str, coll_str, index_str, value_str
                )
                .unwrap();
            }
        }
    }
    output
}

/// Whether evaluating `writer` can change what `reader` reads: it writes a name `reader` reads (a binding it lends,
/// a global a call it makes reaches), or does what the emitter cannot see into.
fn interferes(writer: &Expr, reader: &Expr) -> bool {
    let writes = crate::optimize::WriteSet::of_expr(writer);
    writes.is_opaque() || crate::optimize::Mentions::of_expr(reader).symbols().any(|s| writes.may_write(s))
}

/// Whether the operands of one statement must be taken one at a time, in order: one of them can change what another
/// reads.
fn ordered(operands: &[&Expr]) -> bool {
    operands.iter().enumerate().any(|(i, a)| operands.iter().enumerate().any(|(j, b)| i != j && interferes(a, b)))
}

/// The `Set` arm of [`codegen_stmt`], in a frame of its own — the second-widest arm, and like
/// [`codegen_let_stmt`] never on the recursive path. See that function for why the widest arms are
/// the ones that must not share the walker's frame.
#[inline(never)]
#[allow(clippy::too_many_arguments)]
/// Collect the leaves of a left-leaning string concatenation (`X + a + b`) into `out`,
/// left to right. Only `Concat`/`Add` binary nodes are split; everything else is a leaf.
fn collect_concat_leaves<'a>(e: &'a Expr<'a>, out: &mut Vec<&'a Expr<'a>>) {
    match e {
        Expr::BinaryOp {
            op: BinaryOpKind::Concat | BinaryOpKind::Add,
            left,
            right,
        } => {
            collect_concat_leaves(left, out);
            out.push(right);
        }
        _ => out.push(e),
    }
}

/// A concat operand PROVABLY safe to feed `push_str` — i.e. string-typed. `String + <number>`
/// is legal LOGOS (it stringifies the number), so a leaf whose type we cannot prove to be a
/// string must be DECLINED: assuming "string" here emitted `push_str(&i64)` (ill-typed Rust that
/// fails to compile). Declining just falls back to the correct `format!` path for that concat.
/// Recognised as string: a Text literal, a variable in the string set or of `String`/`&str`
/// type, and `item i of xs` where `xs`'s element type is a string. Every other shape (a number
/// literal, a call, a field, an index into a non-string seq) is rejected.
fn concat_leaf_is_string(e: &Expr, ctx: &RefinementContext) -> bool {
    match e {
        Expr::Literal(Literal::Text(_)) => true,
        Expr::Identifier(s) => {
            ctx.get_string_vars().contains(s)
                || matches!(
                    ctx.get_variable_types().get(s).map(String::as_str),
                    Some("String") | Some("&str")
                )
        }
        Expr::Index { collection, .. } => matches!(
            crate::codegen::expr::seq_element_rust_type(collection, ctx.get_variable_types()),
            Some("String") | Some("&str")
        ),
        _ => false,
    }
}

/// `Set X to X combined with Y...` where X is a mutable owned `String` and no appended operand
/// mentions X → append each operand in place via `push_str` (amortized O(1) growth) instead of
/// `X = format!(...)` — which reallocates and copies the whole accumulator every call, making
/// `repeatText` and any concat-accumulator loop O(N^2). Shape-based (`Opt::StringAccumReuse`).
fn try_string_accum_push_str<'a>(
    target: &Symbol,
    value: &'a Expr<'a>,
    target_name: &str,
    interner: &Interner,
    ctx: &RefinementContext<'a>,
    synced_vars: &mut HashSet<Symbol>,
    async_functions: &HashSet<Symbol>,
    indent_str: &str,
) -> Option<String> {
    // Target must be a mutable owned String (push_str valid, in-place mutation sound).
    if ctx.get_variable_types().get(target).map(String::as_str) != Some("String") {
        return None;
    }
    // Value must be a concat whose LEFTMOST leaf is the target: `X + a + b`.
    let mut leaves: Vec<&Expr> = Vec::new();
    collect_concat_leaves(value, &mut leaves);
    if leaves.len() < 2 {
        return None;
    }
    match leaves[0] {
        Expr::Identifier(s) if *s == *target => {}
        _ => return None,
    }
    // Each appended leaf must be string-typed and must NOT mention the target (a self-append
    // like `X + X` aliases the `&mut` borrow → E0502; only the leftmost occurrence accumulates).
    let vt = ctx.get_variable_types();
    for leaf in &leaves[1..] {
        if !concat_leaf_is_string(leaf, ctx) {
            return None;
        }
        let mut syms: Vec<Symbol> = Vec::new();
        collect_expr_symbols(leaf, &mut syms);
        if syms.contains(target) {
            return None;
        }
    }
    let mut code = String::new();
    for leaf in &leaves[1..] {
        push_concat_leaf(
            &mut code,
            target_name,
            leaf,
            interner,
            synced_vars,
            async_functions,
            vt,
            indent_str,
        );
    }
    Some(code)
}

/// Append one string-valued concat leaf to the `String` named `target_name`, in place: a text literal as a
/// `push_str` of its escaped bytes, a one-character text narrowed to a byte (`__single_char_u8`) as a character —
/// its value is a `u8`, which `push_str` does not take — and any other string-valued expression as a `push_str` of
/// a borrow. The one append emitter of the in-place concat passes ([`try_string_accum_push_str`],
/// [`try_string_concat_let`]).
fn push_concat_leaf<'a>(
    code: &mut String,
    target_name: &str,
    leaf: &Expr<'a>,
    interner: &Interner,
    synced_vars: &mut HashSet<Symbol>,
    async_functions: &HashSet<Symbol>,
    vt: &HashMap<Symbol, String>,
    indent_str: &str,
) {
    match leaf {
        Expr::Literal(Literal::Text(sym)) => {
            writeln!(
                code,
                "{}{}.push_str(\"{}\");",
                indent_str,
                target_name,
                rust_string_escape(interner.resolve(*sym))
            )
            .unwrap();
        }
        Expr::Identifier(sym) if vt.get(sym).map(String::as_str) == Some("__single_char_u8") => {
            let ch = codegen_expr_with_async(leaf, interner, synced_vars, async_functions, vt);
            writeln!(code, "{}{}.push({} as char);", indent_str, target_name, ch).unwrap();
        }
        _ => {
            let s = codegen_expr_with_async(leaf, interner, synced_vars, async_functions, vt);
            writeln!(code, "{}{}.push_str(&({}));", indent_str, target_name, s).unwrap();
        }
    }
}

/// `Let X be A + B...` where A (leftmost leaf) is a fresh OWNED `String` — a call whose return type
/// is `String`, i.e. a movable temporary with no aliasing — reuses A's buffer: `let mut X = A;
/// X.push_str(&B)...` instead of `X = format!("{}{}", A, B)` (a fresh alloc + a copy of EVERY operand
/// including A). The `Let`-form twin of [`try_string_accum_push_str`]. A bare identifier / `&str` /
/// literal is NOT taken as A (moving a local risks a later use; a `&str` is not owned). Shape-based
/// (`Opt::StringConcatReuse`) — any user `Let s be build(..) + tail` benefits identically.
fn try_string_concat_let<'a>(
    var: &Symbol,
    value: &'a Expr<'a>,
    var_name: &str,
    interner: &Interner,
    ctx: &RefinementContext<'a>,
    synced_vars: &mut HashSet<Symbol>,
    async_functions: &HashSet<Symbol>,
    indent_str: &str,
) -> Option<String> {
    let mut leaves: Vec<&Expr> = Vec::new();
    collect_concat_leaves(value, &mut leaves);
    if leaves.len() < 2 {
        return None;
    }
    // A (leftmost) must be a fresh OWNED String: a call returning `String`.
    let a_owned_string = match leaves[0] {
        Expr::Call { function, .. } => {
            ctx.get_fn_return(function).map(String::as_str) == Some("String")
        }
        _ => false,
    };
    if !a_owned_string {
        return None;
    }
    // Every appended leaf must be push_str-able (string-typed) and must not mention X (X is new, so
    // this holds trivially, but guard against a self-referential concat all the same).
    for leaf in &leaves[1..] {
        if !concat_leaf_is_string(leaf, ctx) {
            return None;
        }
        let mut syms: Vec<Symbol> = Vec::new();
        collect_expr_symbols(leaf, &mut syms);
        if syms.contains(var) {
            return None;
        }
    }
    let vt = ctx.get_variable_types();
    let a_code = codegen_expr_with_async(leaves[0], interner, synced_vars, async_functions, vt);
    let mut code = format!("{}let mut {} = {};\n", indent_str, var_name, a_code);
    for leaf in &leaves[1..] {
        push_concat_leaf(
            &mut code,
            var_name,
            leaf,
            interner,
            synced_vars,
            async_functions,
            vt,
            indent_str,
        );
    }
    Some(code)
}

fn codegen_set_stmt<'a>(
    mut output: String,
    target: &Symbol,
    value: &&'a Expr<'a>,
    interner: &Interner,
    ctx: &mut RefinementContext<'a>,
    synced_vars: &mut HashSet<Symbol>,
    async_functions: &HashSet<Symbol>,
    boxed_fields: &HashSet<(String, String, String)>,
    registry: &TypeRegistry,
    indent_str: &str,
    names: &RustNames,
    live_vars_after: Option<HashSet<Symbol>>,
) -> String {
    // A `mutable` parameter lent as `&mut` is set through the reference: the caller's value.
    let target_name = if super::expr::lent_in_place(*target, ctx.get_variable_types()) {
        format!("(*{})", names.ident(*target))
    } else {
        names.ident(*target)
    };

    // A `mutable` list or map parameter lent by sharing is the caller's own buffer: setting it replaces what that
    // buffer holds. The new value is taken first, so it may read the parameter.
    if super::expr::lent_by_sharing(*target, ctx.get_variable_types()) {
        let held = ctx.get_variable_types().get(target).cloned().unwrap_or_default();
        let value_str = codegen_expr_with_async(value, interner, synced_vars, async_functions, ctx.get_variable_types());
        let contents = if held.starts_with("LogosSeq") { "to_vec()" } else { "borrow().clone()" };
        writeln!(
            output,
            "{}{{ let __replacement = {}; let __contents = __replacement.{}; *{}.borrow_mut() = __contents; }}",
            indent_str, value_str, contents, target_name
        )
        .unwrap();
        return output;
    }

    // A binding that holds a value-or-nothing is set to `nothing`, to another whole, or to a value present.
    if let Some(held) = ctx.get_variable_types().get(target).cloned() {
        let ecx = operand_ctx(interner, ctx, synced_vars, async_functions, boxed_fields, registry);
        if let Some(code) = super::absent::stored(value, &held, &ecx) {
            writeln!(output, "{}{} = {};", indent_str, target_name, code).unwrap();
            return output;
        }
    }

    if ctx
        .get_variable_types()
        .get(target)
        .is_some_and(|t| t == super::tower::VALUE)
    {
        let code = super::tower::statement_value(
            value,
            interner,
            synced_vars,
            async_functions,
            boxed_fields,
            registry,
            ctx.get_variable_types(),
        );
        writeln!(output, "{}{} = {};", indent_str, target_name, code).unwrap();
        return output;
    }

    // A binding that holds rationals holds every integer it is set to as the rational it equals.
    if let Some(held) = ctx
        .get_variable_types()
        .get(target)
        .filter(|t| t.contains(super::rational::RATIONAL))
        .cloned()
    {
        if let Some(code) = rational_value(
            value,
            &held,
            interner,
            ctx,
            synced_vars,
            async_functions,
            boxed_fields,
            registry,
        ) {
            writeln!(output, "{}{} = {};", indent_str, target_name, code).unwrap();
            return output;
        }
    }

    // Opt::WideAccumulator: `Set X to X + k` where X is an i128-widened counting accumulator — emit a
    // plain i128 add (no checked-add branch), so the count auto-vectorizes. Byte-identical: X ≤ trip ≤
    // i64::MAX, so the value is exact.
    if crate::optimize::active_config().is_on(crate::optimization::Opt::WideAccumulator)
        && super::expr::is_wide_acc(*target)
    {
        let val: &Expr = value;
        if let Expr::BinaryOp {
            op: BinaryOpKind::Add,
            left,
            right,
        } = val
        {
            if matches!(left, Expr::Identifier(l) if *l == *target) {
                if let Expr::Literal(Literal::Number(k)) = right {
                    crate::optimize::mark_fired(crate::optimization::Opt::WideAccumulator);
                    writeln!(
                        output,
                        "{}{} = {} + {}i128;",
                        indent_str, target_name, target_name, k
                    )
                    .unwrap();
                    return output;
                }
            }
        }
    }

    // A self-assignment `Set X to X` is a semantic no-op on every tier (the
    // binding rebinds to itself; value semantics make it observably nothing).
    // Emitting `x = x;` is not only pointless but *invalid* Rust when X is an
    // immutable binding — e.g. a `for`-loop variable — so elide it entirely.
    if let Expr::Identifier(sym) = value {
        if *sym == *target {
            return output;
        }
    }

    // String accumulator reuse: `Set X to X combined with Y...` — X a mutable owned String,
    // no appended operand mentions X — mutates X in place via `push_str` (amortized O(1) growth)
    // instead of `X = format!("{}{}", X, Y)` (a full realloc+copy each call → O(N^2) accumulator,
    // the repeatText cost). Yields to the bignum and cursor-indexed-buffer specializations below.
    if crate::optimize::active_config().is_on(crate::optimization::Opt::StringAccumReuse)
        && !ctx.is_bigint_var(*target)
        && ctx.indexed_string_build(*target).is_none()
    {
        if let Some(code) = try_string_accum_push_str(
            target,
            *value,
            &target_name,
            interner,
            ctx,
            synced_vars,
            async_functions,
            indent_str,
        ) {
            crate::optimize::mark_fired(crate::optimization::Opt::StringAccumReuse);
            output.push_str(&code);
            return output;
        }
    }

    // Overflow-promoting Int target: assign an un-narrowed `LogosInt` (`.into()` is
    // identity when the RHS already is one, and promotes a plain `i64` otherwise), so a
    // running product/sum that exceeds i64 promotes to BigInt instead of panicking. A
    // bigint target is always a scalar integer, so none of the string/char/collection
    // special cases below apply.
    if ctx.is_bigint_var(*target) {
        // In-place scalar multiply. `Set acc to acc * <i64>` (or `<i64> * acc`) — a promoted
        // `Int` accumulator, dead the instant it is reassigned, times a compile-time-known
        // machine integer (a non-bignum variable or a numeric literal) — mutates `acc`'s heap
        // magnitude IN PLACE via `mul_assign_i64`, reusing the buffer instead of allocating a
        // fresh product every iteration (the factorial hot loop). Byte-identical value; the
        // accumulator must appear EXACTLY once (a true dead last-use) so nothing else reads the
        // pre-mutation value. A Big × Big multiply (`acc * acc`) has no scalar form and falls
        // through to the by-ref path below.
        if let Expr::BinaryOp {
            op: BinaryOpKind::Multiply,
            left,
            right,
        } = value
        {
            if crate::optimize::active_config().is_on(crate::optimization::Opt::BignumInPlace) {
                let left_is_acc = matches!(left, Expr::Identifier(s) if *s == *target);
                let right_is_acc = matches!(right, Expr::Identifier(s) if *s == *target);
                let scalar = match (left_is_acc, right_is_acc) {
                    (true, false) => Some(right),
                    (false, true) => Some(left),
                    _ => None,
                };
                if let Some(scalar) = scalar {
                    let scalar_str = match scalar {
                        Expr::Identifier(s) if !ctx.is_bigint_var(*s) => Some(names.ident(*s)),
                        Expr::Literal(Literal::Number(n)) => Some(format!("{}i64", n)),
                        _ => None,
                    };
                    if let Some(scalar_str) = scalar_str {
                        if count_target_reads(value, *target) == 1 {
                            crate::optimize::mark_fired(crate::optimization::Opt::BignumInPlace);
                            writeln!(
                                output,
                                "{}{}.mul_assign_i64({});",
                                indent_str, target_name, scalar_str
                            )
                            .unwrap();
                            if let Some((bound_var, predicate)) = ctx.get_constraint(*target) {
                                emit_refinement_check(
                                    &target_name,
                                    bound_var,
                                    predicate,
                                    interner,
                                    &indent_str,
                                    &mut output,
                                );
                            }
                            return output;
                        }
                    }
                }
            }
        }
        // The accumulator is dead the instant this statement reassigns it, so if
        // it appears exactly once in the RHS its read may MOVE instead of clone.
        let movable = count_target_reads(value, *target) == 1;
        if movable {
            set_movable_last_use(Some(*target));
        }
        let raw = codegen_expr_boxed_with_types_oracle_tolerant(
            value,
            interner,
            synced_vars,
            boxed_fields,
            registry,
            async_functions,
            ctx.get_string_vars(),
            ctx.get_variable_types(),
            ctx.get_fast_div(),
            ctx.oracle(),
        );
        if movable {
            set_movable_last_use(None);
        }
        writeln!(
            output,
            "{}{} = logicaffeine_data::LogosInt::from({});",
            indent_str, target_name, raw
        )
        .unwrap();
        if let Some((bound_var, predicate)) = ctx.get_constraint(*target) {
            emit_refinement_check(
                &target_name,
                bound_var,
                predicate,
                interner,
                &indent_str,
                &mut output,
            );
        }
        return output;
    }

    // Cursor-indexed string build: `Set text to text + X` writes X at the
    // cursor in the pre-sized buffer (no push). The paired `Set c to c +
    // len(X)` that follows advances the cursor as usual. See
    // peephole::try_emit_indexed_string_build.
    if let Some(cursor) = ctx.indexed_string_build(*target).map(|c| c.to_string()) {
        if let Expr::BinaryOp {
            op: BinaryOpKind::Add,
            right,
            ..
        } = value
        {
            match right {
                Expr::Literal(Literal::Text(sym)) => {
                    let lit = interner.resolve(*sym);
                    let bytes: Vec<String> = lit.bytes().map(|b| format!("{}u8", b)).collect();
                    writeln!(
                        output,
                        "{ind}unsafe {{ std::ptr::copy_nonoverlapping([{b}].as_ptr(), {t}.as_mut_vec().as_mut_ptr().add(({c}) as usize), {k}); }}",
                        ind = indent_str, t = target_name, c = cursor, k = lit.len(), b = bytes.join(", ")
                    )
                    .unwrap();
                    return output;
                }
                Expr::Identifier(v)
                    if ctx.get_variable_types().get(v).map(String::as_str)
                        == Some("__single_char_u8") =>
                {
                    let vn = names.ident(*v);
                    writeln!(
                        output,
                        "{ind}unsafe {{ *{t}.as_mut_vec().as_mut_ptr().add(({c}) as usize) = {vn}; }}",
                        ind = indent_str, t = target_name, c = cursor
                    )
                    .unwrap();
                    return output;
                }
                _ => {}
            }
        }
    }

    // O2 de-Rc: reassigning a de-Rc'd Seq to a FRESH allocation keeps
    // it a plain `Vec<T>` (`xs = Vec::new()`), not `Seq::default()`.
    if ctx.is_de_rc(*target) {
        if let Some((_, init)) = derc_vec_decl(
            value,
            interner,
            synced_vars,
            boxed_fields,
            registry,
            async_functions,
            ctx,
        ) {
            writeln!(output, "{}{} = {};", indent_str, target_name, init).unwrap();
            return output;
        }
    }

    // OPT: Single-char u8 variable → emit byte literal assignment.
    let is_target_single_char_u8 = ctx
        .get_variable_types()
        .get(target)
        .map_or(false, |t| t == "__single_char_u8");
    if is_target_single_char_u8 {
        if let Expr::Literal(Literal::Text(sym)) = value {
            let text = interner.resolve(*sym);
            if text.len() == 1 {
                let ch = text.as_bytes()[0];
                writeln!(output, "{}{} = b'{}';", indent_str, target_name, ch as char).unwrap();
                return output;
            }
        }
    }

    let string_vars = ctx.get_string_vars();
    let var_types = ctx.get_variable_types();

    // Optimization: detect self-append pattern (result = result + x + y)
    // and emit write!(result, "{}{}", x, y) instead of result = format!(...).
    // This is O(n) amortized (in-place append) vs O(n²) (full copy each iteration).
    let used_write = if ctx.is_string_var(*target)
        && is_definitely_string_expr_with_vars(value, string_vars)
    {
        let mut operands = Vec::new();
        collect_string_concat_operands(value, string_vars, &mut operands);

        // Need at least 2 operands, leftmost must be the target variable
        if operands.len() >= 2 && matches!(operands[0], Expr::Identifier(sym) if *sym == *target) {
            // Check no other operand references target (would cause borrow conflict)
            let tail = &operands[1..];
            let mut tail_ids = HashSet::new();
            for op in tail {
                collect_expr_identifiers(op, &mut tail_ids);
            }

            if !tail_ids.contains(target) {
                if tail.len() == 1 {
                    // Single operand: use push/push_str for zero-format-overhead append
                    let operand = tail[0];
                    if let Expr::Literal(Literal::Text(sym)) = operand {
                        let text = interner.resolve(*sym);
                        if text.len() == 1 {
                            let ch = text.chars().next().unwrap();
                            writeln!(
                                output,
                                "{}{}.push('{}');",
                                indent_str,
                                target_name,
                                rust_char_escape(ch)
                            )
                            .unwrap();
                        } else {
                            writeln!(
                                output,
                                "{}{}.push_str(\"{}\");",
                                indent_str,
                                target_name,
                                rust_string_escape(text)
                            )
                            .unwrap();
                        }
                    } else if let Expr::Identifier(sym) = operand {
                        if var_types
                            .get(sym)
                            .map_or(false, |t| t == "__single_char_u8")
                        {
                            // OPT: u8 single-char var → push(ch as char)
                            let val_str = codegen_expr_boxed_with_types(
                                operand,
                                interner,
                                synced_vars,
                                boxed_fields,
                                registry,
                                async_functions,
                                string_vars,
                                var_types,
                                ctx.get_fast_div(),
                            );
                            writeln!(
                                output,
                                "{}{}.push({} as char);",
                                indent_str, target_name, val_str
                            )
                            .unwrap();
                        } else if string_vars.contains(sym) {
                            let val_str = codegen_expr_boxed_with_types(
                                operand,
                                interner,
                                synced_vars,
                                boxed_fields,
                                registry,
                                async_functions,
                                string_vars,
                                var_types,
                                ctx.get_fast_div(),
                            );
                            writeln!(
                                output,
                                "{}{}.push_str(&{});",
                                indent_str, target_name, val_str
                            )
                            .unwrap();
                        } else {
                            // Non-string single operand: use write!
                            let val_str = codegen_expr_boxed_with_types(
                                operand,
                                interner,
                                synced_vars,
                                boxed_fields,
                                registry,
                                async_functions,
                                string_vars,
                                var_types,
                                ctx.get_fast_div(),
                            );
                            writeln!(
                                output,
                                "{}write!({}, \"{{}}\", {}).unwrap();",
                                indent_str, target_name, val_str
                            )
                            .unwrap();
                        }
                    } else {
                        // Non-string single operand: use write!
                        let val_str = codegen_expr_boxed_with_types(
                            operand,
                            interner,
                            synced_vars,
                            boxed_fields,
                            registry,
                            async_functions,
                            string_vars,
                            var_types,
                            ctx.get_fast_div(),
                        );
                        writeln!(
                            output,
                            "{}write!({}, \"{{}}\", {}).unwrap();",
                            indent_str, target_name, val_str
                        )
                        .unwrap();
                    }
                } else if tail.iter().any(|op| {
                    crate::codegen::types::infer_numeric_type(op, interner, var_types) == "i64"
                }) {
                    // Multiple operands, at least one integer: append each with the
                    // fastest path for its type — `.push`/`.push_str` for char/string
                    // literals and string vars, `.push_i64` (itoa fast path, no
                    // `fmt::Display` vtable) for integers, `write!` only as the
                    // fallback. Sequential appends produce the identical string a
                    // single `write!("{}{}…")` would, without routing each integer
                    // through the formatter machinery.
                    for op in tail {
                        if let Expr::Literal(Literal::Text(sym)) = op {
                            let text = interner.resolve(*sym);
                            if text.chars().count() == 1 {
                                writeln!(
                                    output,
                                    "{}{}.push('{}');",
                                    indent_str,
                                    target_name,
                                    rust_char_escape(text.chars().next().unwrap())
                                )
                                .unwrap();
                            } else {
                                writeln!(
                                    output,
                                    "{}{}.push_str(\"{}\");",
                                    indent_str,
                                    target_name,
                                    rust_string_escape(text)
                                )
                                .unwrap();
                            }
                            continue;
                        }
                        let val_str = codegen_expr_boxed_with_types(
                            op,
                            interner,
                            synced_vars,
                            boxed_fields,
                            registry,
                            async_functions,
                            string_vars,
                            var_types,
                            ctx.get_fast_div(),
                        );
                        if let Expr::Identifier(sym) = op {
                            if string_vars.contains(sym) {
                                writeln!(
                                    output,
                                    "{}{}.push_str(&{});",
                                    indent_str, target_name, val_str
                                )
                                .unwrap();
                                continue;
                            }
                            if var_types
                                .get(sym)
                                .map_or(false, |t| t == "__single_char_u8")
                            {
                                writeln!(
                                    output,
                                    "{}{}.push({} as char);",
                                    indent_str, target_name, val_str
                                )
                                .unwrap();
                                continue;
                            }
                        }
                        if crate::codegen::types::infer_numeric_type(op, interner, var_types)
                            == "i64"
                        {
                            writeln!(
                                output,
                                "{}{}.push_i64({});",
                                indent_str, target_name, val_str
                            )
                            .unwrap();
                        } else {
                            writeln!(
                                output,
                                "{}write!({}, \"{{}}\", {}).unwrap();",
                                indent_str, target_name, val_str
                            )
                            .unwrap();
                        }
                    }
                } else {
                    // Multiple operands, all string/char: no integer to accelerate,
                    // so keep the single `write!` — the itoa fast path would gain
                    // nothing here (locked by e2e_opt_push_str_multi_operand_keeps_write).
                    // `write!`'s `{}` formats through a shared `&` reference, so feed it
                    // borrowed operands: string literals as `&str` (`" "`, not
                    // `String::from(" ")`), string vars by `&`-borrow — no throwaway
                    // heap `String` per append.
                    let mut fmt = String::new();
                    let mut args = Vec::new();
                    for op in tail {
                        fmt.push_str("{}");
                        if let Expr::Literal(Literal::Text(sym)) = op {
                            args.push(format!(
                                "\"{}\"",
                                rust_string_escape(interner.resolve(*sym))
                            ));
                            continue;
                        }
                        if let Expr::Identifier(sym) = op {
                            if string_vars.contains(sym) {
                                args.push(format!("&{}", names.ident(*sym)));
                                continue;
                            }
                        }
                        args.push(codegen_expr_boxed_with_types(
                            op,
                            interner,
                            synced_vars,
                            boxed_fields,
                            registry,
                            async_functions,
                            string_vars,
                            var_types,
                            ctx.get_fast_div(),
                        ));
                    }
                    writeln!(
                        output,
                        "{}write!({}, \"{}\", {}).unwrap();",
                        indent_str,
                        target_name,
                        fmt,
                        args.join(", ")
                    )
                    .unwrap();
                }
                true
            } else {
                false
            }
        } else {
            false
        }
    } else {
        false
    };

    if !used_write {
        // &mut borrow call-site transformation:
        // `Set x to f(x, ...)` where f has fn_mut_borrow at the target position
        // → `f(&mut x, ...)` (no assignment, mutation is in-place)
        let mut used_mut_borrow = false;
        if let Expr::Call { function, args } = value {
            let callee_mut_borrow_indices: HashSet<usize> =
                super::fn_role_indices(var_types.get(function), super::FnRole::MutBorrow);
            if !callee_mut_borrow_indices.is_empty() {
                // Check if target is at a mut_borrow position
                let target_at_mut_borrow = args.iter().enumerate().any(|(i, a)| {
                    callee_mut_borrow_indices.contains(&i)
                        && matches!(a, Expr::Identifier(sym) if *sym == *target)
                });
                if target_at_mut_borrow {
                    let callee_borrow_indices: HashSet<usize> =
                        super::fn_role_indices(var_types.get(function), super::FnRole::Borrow);
                    let func_name = names.ident(*function);
                    let args_str: Vec<String> = args
                        .iter()
                        .enumerate()
                        .map(|(i, a)| {
                            let s = codegen_expr_boxed_with_types(
                                a,
                                interner,
                                synced_vars,
                                boxed_fields,
                                registry,
                                async_functions,
                                string_vars,
                                var_types,
                                ctx.get_fast_div(),
                            );
                            if callee_mut_borrow_indices.contains(&i) {
                                // Mut borrow param: pass &mut [T] reference
                                if let Expr::Identifier(sym) = a {
                                    if let Some(ty) = var_types.get(sym) {
                                        let ty = ty.split("|__hl:").next().unwrap_or(ty.as_str());
                                        if ty.starts_with("&mut [") {
                                            return s; // Already &mut slice
                                        }
                                        // Phase 3b: a de-Rc'd `Vec` lends `&mut x`
                                        // directly — no RefCell to borrow_mut.
                                        if ty.starts_with("Vec<") {
                                            return format!("&mut {}", s);
                                        }
                                        if ty.starts_with("LogosSeq") {
                                            return format!("&mut *{}.borrow_mut()", s);
                                        }
                                    }
                                    // Unknown type — default to LogosSeq
                                    return format!("&mut *{}.borrow_mut()", s);
                                }
                                format!("&mut {}", s)
                            } else if callee_borrow_indices.contains(&i) {
                                if let Expr::Identifier(sym) = a {
                                    if let Some(ty) = var_types.get(sym) {
                                        let ty = ty.split("|__hl:").next().unwrap_or(ty.as_str());
                                        if ty.starts_with("&[") || ty.starts_with("&mut [") {
                                            return s;
                                        }
                                        if ty.starts_with("Vec<") {
                                            return format!("&{}", s);
                                        }
                                        if ty.starts_with('[') {
                                            // Const-table stack array [T; N] coerces to &[T].
                                            return format!("&{}", s);
                                        }
                                        if ty.starts_with("LogosSeq") {
                                            return format!("&*{}.borrow()", s);
                                        }
                                    }
                                    // Unknown type — default to LogosSeq
                                    return format!("&*{}.borrow()", s);
                                }
                                format!("&*{}.borrow()", s)
                            } else {
                                s
                            }
                        })
                        .collect();
                    // Value semantics: make each lent handle's buffer
                    // UNIQUE before the in-place call — an alias keeps
                    // the pre-call value, the callee's writes land on
                    // ours alone. Amortized free: a refcount-1 cow is
                    // a branch, only a genuinely shared buffer copies.
                    if crate::semantics::collections::value_semantics_enabled() {
                        for (i, a) in args.iter().enumerate() {
                            if !callee_mut_borrow_indices.contains(&i) {
                                continue;
                            }
                            if let Expr::Identifier(sym) = a {
                                let lends_refcell = match var_types.get(sym) {
                                    Some(t) => t
                                        .split("|__hl:")
                                        .next()
                                        .unwrap_or(t.as_str())
                                        .starts_with("LogosSeq"),
                                    None => true, // unknown type lowers via borrow_mut above
                                };
                                if lends_refcell {
                                    writeln!(output, "{}{}.cow();", indent_str, names.ident(*sym))
                                        .unwrap();
                                }
                            }
                        }
                    }
                    let await_suffix = if async_functions.contains(function) {
                        ".await"
                    } else {
                        ""
                    };
                    writeln!(
                        output,
                        "{}{}({}){};",
                        indent_str,
                        func_name,
                        args_str.join(", "),
                        await_suffix
                    )
                    .unwrap();
                    used_mut_borrow = true;
                }
            }
        }

        // OPT-1B: Last-use clone elimination for `Set x to f(x, ...)`.
        // When the target variable appears as a direct non-borrow argument
        // and doesn't appear in other arg sub-expressions, skip cloning it
        // since the old value is immediately overwritten by the result.
        let value_str = if used_mut_borrow {
            String::new() // Already emitted above
        } else if let Expr::Call { function, args } = value {
            if crate::semantics::builtins::builtin_from_name(interner.resolve(*function)).is_some()
            {
                // A math/value builtin (`sqrt`/`abs`/`pow`/…) needs its dedicated lowering
                // (`(x as f64).sqrt()`, `LogosComplexF::…`, …), NOT the bare `name(args)` the
                // last-use-move fast path below emits — `Set x to sqrt(x)` must round-trip like
                // `Let`/`Return` do. Route it through the full expression codegen.
                codegen_expr_boxed_with_types(
                    value,
                    interner,
                    synced_vars,
                    boxed_fields,
                    registry,
                    async_functions,
                    string_vars,
                    var_types,
                    ctx.get_fast_div(),
                )
            } else {
                // The arguments as every call writes them — borrows, lends, exact promotions, from
                // the one emitter (`call_arguments`) — with the one thing a `Set` of a call adds:
                // an identifier this call is the last reader of is MOVED rather than cloned. The
                // target is overwritten the moment the call returns, and a name liveness says is
                // dead after this statement is too, as long as no other argument reads it. A target
                // borrowed in place is borrowed from a copy, since it is assigned while borrowed.
                let ecx = super::expr::ExprCtx {
                    boxed_fields,
                    registry,
                    async_functions,
                    string_vars,
                    variable_types: var_types,
                    fast_div: ctx.get_fast_div(),
                    ..super::expr::ExprCtx::bare(interner, synced_vars)
                };
                let mut args_str = super::expr::call_arguments(*function, args, &ecx);
                for (i, a) in args.iter().enumerate() {
                    let Expr::Identifier(sym) = a else { continue };
                    let read = codegen_expr_boxed_with_types(
                        a,
                        interner,
                        synced_vars,
                        boxed_fields,
                        registry,
                        async_functions,
                        string_vars,
                        var_types,
                        ctx.get_fast_div(),
                    );
                    let elsewhere = args.iter().enumerate().any(|(j, b)| {
                        j != i && {
                            let mut ids = HashSet::new();
                            collect_expr_identifiers(b, &mut ids);
                            ids.contains(sym)
                        }
                    });
                    if *sym == *target && args_str[i] == format!("&*{read}.borrow()") {
                        args_str[i] = format!("&*{read}.clone().borrow()");
                        continue;
                    }
                    let last_read = *sym == *target
                        || live_vars_after
                            .as_ref()
                            .is_some_and(|live| !live.contains(sym));
                    if last_read && !elsewhere && args_str[i] == format!("{read}.clone()") {
                        args_str[i] = read;
                    }
                }
                let func_name =
                    super::expr::called_function_path(*function, names.ident(*function), var_types);
                let await_suffix = if async_functions.contains(function) {
                    ".await"
                } else {
                    ""
                };
                super::expr::sequenced_call(
                    &func_name,
                    *function,
                    args,
                    args_str,
                    await_suffix,
                    var_types,
                )
            }
        } else {
            let vs = codegen_expr_boxed_with_types(
                value,
                interner,
                synced_vars,
                boxed_fields,
                registry,
                async_functions,
                string_vars,
                var_types,
                ctx.get_fast_div(),
            );
            stored_value(value, vs, var_types)
        };
        if !used_mut_borrow {
            writeln!(output, "{}{} = {};", indent_str, target_name, value_str).unwrap();
        }
    }

    // Phase 43C: Check if this variable has a refinement constraint
    if let Some((bound_var, predicate)) = ctx.get_constraint(*target) {
        emit_refinement_check(
            &target_name,
            bound_var,
            predicate,
            interner,
            &indent_str,
            &mut output,
        );
    }
    output
}

/// The machine word a `Let` declares when its initializer is [`type_silent_int`](super::types::type_silent_int)
/// and the binding is a plain `Int`: without it, a binding no use pins is an `i32` to rustc.
/// The emission of `value` as a value of the Rust type `held`, which holds rationals, where it is not one already
/// ([`super::rational::value_as`]).
#[allow(clippy::too_many_arguments)]
fn rational_value(
    value: &Expr,
    held: &str,
    interner: &Interner,
    ctx: &RefinementContext,
    synced_vars: &HashSet<Symbol>,
    async_functions: &HashSet<Symbol>,
    boxed_fields: &HashSet<(String, String, String)>,
    registry: &TypeRegistry,
) -> Option<String> {
    let ecx = super::expr::ExprCtx {
        boxed_fields,
        registry,
        async_functions,
        string_vars: ctx.get_string_vars(),
        variable_types: ctx.get_variable_types(),
        fast_div: ctx.get_fast_div(),
        oracle: ctx.oracle(),
        ..super::expr::ExprCtx::bare(interner, synced_vars)
    };
    super::rational::value_as(value, &super::rational::named(held), &ecx)
}

/// The context a statement's own expressions are emitted in.
fn operand_ctx<'c>(
    interner: &'c Interner,
    ctx: &'c RefinementContext,
    synced_vars: &'c HashSet<Symbol>,
    async_functions: &'c HashSet<Symbol>,
    boxed_fields: &'c HashSet<(String, String, String)>,
    registry: &'c TypeRegistry,
) -> super::expr::ExprCtx<'c> {
    super::expr::ExprCtx {
        boxed_fields,
        registry,
        async_functions,
        string_vars: ctx.get_string_vars(),
        variable_types: ctx.get_variable_types(),
        fast_div: ctx.get_fast_div(),
        oracle: ctx.oracle(),
        ..super::expr::ExprCtx::bare(interner, synced_vars)
    }
}

fn type_silent_let_annotation(value: &Expr, registered: Option<&String>) -> Option<String> {
    (super::types::type_silent_int(value).is_some() && registered.is_some_and(|t| t == "i64"))
        .then(|| "i64".to_string())
}

/// The `Let` arm of [`codegen_stmt`], in a frame of its own.
///
/// A compile recurses once per nesting level of the program, and in a debug build every arm's
/// locals share the enclosing function's frame — so the widest arms set the stack cost of EVERY
/// level, including levels that never run them. `Let` is the widest and is never on the recursive
/// path; giving it its own frame keeps it out of the nesting cost entirely.
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn codegen_let_stmt<'a>(
    mut output: String,
    stmt: &Stmt<'a>,
    var: &Symbol,
    ty: &Option<&'a TypeExpr<'a>>,
    value: &&'a Expr<'a>,
    mutable: &bool,
    interner: &Interner,
    mutable_vars: &HashSet<Symbol>,
    ctx: &mut RefinementContext<'a>,
    synced_vars: &mut HashSet<Symbol>,
    async_functions: &HashSet<Symbol>,
    boxed_fields: &HashSet<(String, String, String)>,
    registry: &TypeRegistry,
    indent_str: &str,
    names: &RustNames,
) -> String {
    let var_name = names.ident(*var);

    // A binding a stored `nothing` reaches holds a value-or-nothing from its first assignment on.
    let held = {
        let ecx = operand_ctx(interner, ctx, synced_vars, async_functions, boxed_fields, registry);
        super::absent::binding(stmt, *ty, value, &ecx).map(|held| {
            let code = super::absent::stored(value, &held, &ecx)
                .unwrap_or_else(|| super::absent::whole(value, &ecx));
            (held, code)
        })
    };
    if let Some((held, code)) = held {
        let decl_mut = if *mutable || mutable_vars.contains(var) { "mut " } else { "" };
        writeln!(output, "{}let {}{}: {} = {};", indent_str, decl_mut, var_name, held, code).unwrap();
        ctx.register_variable_type(*var, held);
        return output;
    }

    // A binding the exact extension reaches holds the compiled exact value from its first
    // assignment on, so every later write and read means what the interpreter's does.
    if super::tower::binding(*var) {
        let code = super::tower::statement_value(
            value,
            interner,
            synced_vars,
            async_functions,
            boxed_fields,
            registry,
            ctx.get_variable_types(),
        );
        let decl_mut = if *mutable || mutable_vars.contains(var) { "mut " } else { "" };
        writeln!(
            output,
            "{}let {}{}: {} = {};",
            indent_str,
            decl_mut,
            var_name,
            super::tower::VALUE,
            code
        )
        .unwrap();
        ctx.register_variable_type(*var, super::tower::VALUE.to_string());
        return output;
    }

    // Opt::WideAccumulator: `Let X be 0` where X is an i128-widened counting accumulator — declare it
    // `i128` (the analysis guarantees the init is the literal 0), so the loop's adds stay plain i128
    // (no checked-add branch → the count vectorizes). Narrowed `as i64` at the `Show` sink.
    if crate::optimize::active_config().is_on(crate::optimization::Opt::WideAccumulator)
        && super::expr::is_wide_acc(*var)
    {
        writeln!(output, "{}let mut {}: i128 = 0i128;", indent_str, var_name).unwrap();
        return output;
    }

    // AoS interleaving: a co-indexed group member. Column 0 emits the
    // single fused `[[T; W]; N]` backing (reusing the first member's
    // name); the other columns' declarations are dropped — their storage
    // lives in the backing. Runs before the scalarization branch below.
    if matches!(value, Expr::New { .. }) {
        if let Some(tag) = parse_aos_tag(ctx.get_variable_types().get(var)) {
            if tag.col == 0 {
                let default = match tag.elem.as_str() {
                    "f64" => "0f64",
                    "bool" => "false",
                    _ => "0i64",
                };
                writeln!(
                    output,
                    "{}let mut {}: [[{}; {}]; {}] = [[{}; {}]; {}];",
                    indent_str,
                    tag.backing,
                    tag.elem,
                    tag.width,
                    tag.len,
                    default,
                    tag.width,
                    tag.len
                )
                .unwrap();
            }
            return output;
        }
    }

    // Opt::StringConcatReuse: `Let X be A + B...` where A is a fresh owned String (a call returning
    // `String`) → `let mut X = A; X.push_str(&B)...`, reusing A's buffer instead of `format!` (which
    // allocates a new String and copies every operand). The `Let`-form twin of StringAccumReuse.
    if crate::optimize::active_config().is_on(crate::optimization::Opt::StringConcatReuse) {
        if let Some(code) = try_string_concat_let(
            var,
            *value,
            &var_name,
            interner,
            ctx,
            synced_vars,
            async_functions,
            indent_str,
        ) {
            crate::optimize::mark_fired(crate::optimization::Opt::StringConcatReuse);
            ctx.register_variable_type(*var, "String".to_string());
            output.push_str(&code);
            return output;
        }
    }

    // O3: a fixed-size Seq pre-registered as `[T; N]` (scalarization)
    // — emit a stack array and let the init pushes fill it via indexed
    // writes. Must run before the `new Seq` type registration below,
    // which would otherwise overwrite the array type with LogosSeq.
    if ctx.is_scalarized_array(*var) && matches!(value, Expr::New { .. }) {
        if let Some(arr_ty) = ctx.get_variable_types().get(var).cloned() {
            let inner = arr_ty.trim_start_matches('[').trim_end_matches(']');
            let mut parts = inner.split("; ");
            let elem = parts.next().unwrap_or("i64");
            let n = parts.next().unwrap_or("0");
            let default = match elem {
                "f64" => "0f64",
                "bool" => "false",
                "Word8" => "word8(0)",
                "Word16" => "word16(0)",
                "Word32" => "word32(0)",
                "Word64" => "word64(0)",
                _ => "0i64",
            };
            writeln!(
                output,
                "{}let mut {}: {} = [{}; {}];",
                indent_str, var_name, arr_ty, default, n
            )
            .unwrap();
            // A loop-built return buffer (the digest) fills through a RUNTIME cursor (the pushes
            // live in a loop): declare it here; the `Push` handler emits `out[cursor]=v; cursor+=1`.
            if let Some(cursor) = ctx.loop_fill_cursor(*var).map(|s| s.to_string()) {
                writeln!(output, "{}let mut {}: usize = 0;", indent_str, cursor).unwrap();
            }
            ctx.init_array_fill(*var);
            return output;
        }
    }

    // Affine read-only array (CSR offset array): deleted entirely. The
    // build push is suppressed and every `item _ of`/`length of` read
    // substitutes the closed form, so the declaration emits nothing. The
    // `__affine_array:...` type was registered in `codegen_program`.
    if matches!(value, Expr::New { .. }) && ctx.affine_array(*var).is_some() {
        return output;
    }

    // Append-only worklist: a de-Rc'd Seq frontier proven bounded by a
    // monotone visited-set guard. Emit a PRE-SIZED buffer + a register
    // tail counter; the pushes lower to `q[tail]=x; tail+=1` and
    // `length of q` to `tail`. This is C's frontier representation.
    if matches!(value, Expr::New { .. }) {
        if let Some((cap, default, tail)) = ctx.worklist(*var).map(|w| {
            (
                w.capacity.clone(),
                w.elem_default.clone(),
                w.tail_var.clone(),
            )
        }) {
            writeln!(
                output,
                "{}let mut {}: Vec<i64> = vec![{}; {}];",
                indent_str, var_name, default, cap
            )
            .unwrap();
            writeln!(output, "{}let mut {}: usize = 0;", indent_str, tail).unwrap();
            // Encode the LOGICAL length (the tail) as a hoisted length so
            // `length of q` reads `tail`, not the buffer size, while every
            // other site strips the suffix and sees a plain `Vec<i64>`.
            ctx.register_variable_type(*var, format!("Vec<i64>|__hl:({} as i64)", tail));
            return output;
        }
    }

    // Opt::BitPack: a local `Seq of Bool` used only through item reads and stores, pushes and its
    // length is held one bit per item (`codegen::bits`).
    if ctx.is_bitpacked(*var) {
        let kw = if *mutable || mutable_vars.contains(var) {
            "let mut"
        } else {
            "let"
        };
        let init = match ctx.get_vec_presize(*var) {
            Some(cap) => format!(
                "logicaffeine_data::LogosBits::with_capacity(({}) as usize)",
                cap
            ),
            None => "logicaffeine_data::LogosBits::new()".to_string(),
        };
        writeln!(
            output,
            "{}{} {}: logicaffeine_data::LogosBits = {};",
            indent_str, kw, var_name, init
        )
        .unwrap();
        ctx.register_variable_type(*var, "LogosBits".to_string());
        return output;
    }

    // O2 de-Rc: a Seq proven to never need reference semantics is a
    // plain `Vec<T>` (no Rc/RefCell). Emit the owned-Vec declaration;
    // access sites dispatch on the `Vec<T>` type exactly as they do for
    // borrow-hoisted slices and materialized vecs.
    if ctx.is_de_rc(*var) {
        if let Some((vec_ty, init)) = derc_vec_decl(
            value,
            interner,
            synced_vars,
            boxed_fields,
            registry,
            async_functions,
            ctx,
        ) {
            let is_mut = *mutable || mutable_vars.contains(var);
            let kw = if is_mut { "let mut" } else { "let" };
            // Pre-size a push-built Vec the program later index-reads up to
            // a bound: reserve once instead of reallocating as it grows
            // (C sizes the buffer exactly). Capacity is a hint — sound.
            let init = match (init.as_str(), ctx.get_vec_presize(*var)) {
                ("Vec::new()", Some(cap)) => {
                    format!("Vec::with_capacity(({}) as usize)", cap)
                }
                _ => init,
            };
            // i64→i32 narrowing: a Seq of Int proven to hold only
            // i32-range values is `Vec<i32>` (access sites convert).
            let vec_ty = if ctx.is_narrowed(*var) && vec_ty == "Vec<i64>" {
                "Vec<i32>".to_string()
            } else {
                vec_ty
            };
            writeln!(
                output,
                "{}{} {}: {} = {};",
                indent_str, kw, var_name, vec_ty, init
            )
            .unwrap();
            for g in ctx.narrow_guards(*var).to_vec() {
                writeln!(
                    output,
                    "{}assert!({}, \"LOGOS i32-narrowing guard: element value must fit i32\");",
                    indent_str, g
                )
                .unwrap();
            }
            ctx.register_variable_type(*var, vec_ty);
            return output;
        }
    }

    // Register collection type for direct indexing optimization.
    // Check explicit type annotation first, then infer from Expr::New.
    if let Some(TypeExpr::Generic { base, params }) = ty {
        let base_name = interner.resolve(*base);
        match base_name {
            "Seq" | "List" | "Vec" => {
                let rust_type = if !params.is_empty() {
                    format!("LogosSeq<{}>", codegen_type_expr(&params[0], interner))
                } else {
                    "LogosSeq<()>".to_string()
                };
                ctx.register_variable_type(*var, rust_type);
            }
            "Map" | "HashMap" => {
                let rust_type = if params.len() >= 2 {
                    map_rust_type(
                        &codegen_type_expr(&params[0], interner),
                        &codegen_type_expr(&params[1], interner),
                        *var,
                        ctx,
                    )
                } else {
                    "LogosMap<String, String>".to_string()
                };
                ctx.register_variable_type(*var, rust_type);
            }
            _ => {}
        }
    } else if let Expr::New {
        type_name,
        type_args,
        ..
    } = value
    {
        let type_str = interner.resolve(*type_name);
        match type_str {
            "Seq" | "List" | "Vec" => {
                let rust_type = if !type_args.is_empty() {
                    format!("LogosSeq<{}>", codegen_type_expr(&type_args[0], interner))
                } else {
                    "LogosSeq<()>".to_string()
                };
                ctx.register_variable_type(*var, rust_type);
            }
            "Map" | "HashMap" => {
                let rust_type = if type_args.len() >= 2 {
                    map_rust_type(
                        &codegen_type_expr(&type_args[0], interner),
                        &codegen_type_expr(&type_args[1], interner),
                        *var,
                        ctx,
                    )
                } else {
                    "LogosMap<String, String>".to_string()
                };
                ctx.register_variable_type(*var, rust_type);
            }
            // A plain user-defined record (`Let p be a new Parcel ...`): register its own Rust
            // struct name, exactly as a typed function PARAMETER already is
            // (`codegen_type_expr`'s `Named` arm) — so `record_field_rust_type` can resolve
            // `p's field` for a LOCAL the same way it already does for a parameter. Without this,
            // `p's label + p's label` (no call, no literal on either side of `+` to carry the
            // "this is Text" signal past the concat-fold decision) fell through to a raw `+`.
            // Confirmed against the REGISTRY (not just "not Seq/Map") so a `New` of some other
            // generic container this match does not yet name (`Set`/`Option`/…, whose OWN Rust
            // type is never the bare type name) can never be mistaken for a struct here.
            other => {
                let is_struct = registry.iter_types().any(|(sym, def)| {
                    matches!(def, TypeDef::Struct { .. })
                        && super::types::rust_type_ident(interner.resolve(*sym)) == other
                });
                if is_struct {
                    ctx.register_variable_type(*var, super::types::rust_type_ident(type_str));
                }
            }
        }
    } else if let Expr::List(items) = value {
        // Infer element type from the first element. Every VALUE TYPE (`Seq of Money` /
        // `Seq of Quantity` / `Seq of Decimal` / `Seq of Modular` / `Seq of Complex[F]`) is
        // recognized richly via its canonical predicate so `item i of xs` and — crucially —
        // a value-type loop variable (`Repeat for m in ms`) resolve to the right Rust type;
        // otherwise the literal-type inference (which sees only `Expr::Literal`) would flatten
        // a `money(..)`/`quantity(..)` constructor call to `_`, and a `total + m` reduce would
        // lower to a raw Rust `+` on `LogosMoney` (E0369). Order: most specific first.
        let held = super::rational::type_of(value, super::rational::Typing { interner, variable_types: ctx.get_variable_types(), registry });
        if held.holds_rational() {
            ctx.register_variable_type(*var, held.to_rust_type());
        } else {
            let elem_type = items
                .first()
                .and_then(|e| {
                    crate::codegen::expr::value_rust_type(
                        e,
                        ctx.get_variable_types(),
                        registry,
                        interner,
                    )
                })
                .unwrap_or_else(|| "_".to_string());
            ctx.register_variable_type(*var, format!("LogosSeq<{}>", elem_type));
        }
    } else if matches!(value, Expr::Tuple(_) | Expr::NewVariant { .. }) {
        // A tuple literal records each element's type in its marker, so `item N of t` gives an
        // element back as itself; a constructed variant records its enum, as a record does above.
        if let Some(rust_ty) = crate::codegen::expr::value_rust_type(
            value,
            ctx.get_variable_types(),
            registry,
            interner,
        ) {
            ctx.register_variable_type(*var, rust_ty);
        }
    } else if let Expr::Identifier(src_sym) = value {
        // Propagate type from source variable: `Let result be arr` inherits arr's type.
        // For borrow params (&[T]), the copy becomes Vec<T> (owned).
        if let Some(src_type) = ctx.get_variable_types().get(src_sym).cloned() {
            if src_type.starts_with("&[") {
                // Propagate borrow type — rebinding the slice, not copying
                ctx.register_variable_type(*var, src_type);
            } else if src_type.starts_with("&mut [") {
                // Mutable borrow param consumed into alias → propagate &mut [T]
                // so the alias continues to be treated as the in-place mutable borrow.
                ctx.register_variable_type(*var, src_type);
            } else if !super::is_fn_role_slot(&src_type) {
                ctx.register_variable_type(*var, src_type);
            }
        }
    } else if let Expr::Copy { expr: inner } = value {
        // `Copy of items ... of arr` or `Copy of arr` produces same collection type
        let src_sym = match inner {
            Expr::Slice { collection, .. } => {
                if let Expr::Identifier(s) = collection {
                    Some(*s)
                } else {
                    None
                }
            }
            Expr::Identifier(s) => Some(*s),
            _ => None,
        };
        if let Some(s) = src_sym {
            if let Some(src_type) = ctx.get_variable_types().get(&s).cloned() {
                if src_type.starts_with("LogosSeq")
                    || src_type.starts_with("Vec")
                    || src_type.starts_with("&[")
                {
                    let elem = if src_type.starts_with("LogosSeq<") {
                        src_type
                            .strip_prefix("LogosSeq<")
                            .and_then(|t| t.strip_suffix('>'))
                            .unwrap_or("_")
                    } else if src_type.starts_with("Vec<") {
                        src_type
                            .strip_prefix("Vec<")
                            .and_then(|t| t.strip_suffix('>'))
                            .unwrap_or("_")
                    } else {
                        src_type
                            .strip_prefix("&[")
                            .and_then(|t| t.strip_suffix(']'))
                            .unwrap_or("_")
                    };
                    ctx.register_variable_type(*var, format!("LogosSeq<{}>", elem));
                } else if src_type.starts_with("LogosMap") {
                    ctx.register_variable_type(*var, src_type);
                }
            }
        }
    }

    // An exact-value-producing RHS binds that value type, OVERRIDING any earlier inference:
    // `sqrt` normally types `f64`, but `sqrt` of a `ComplexF` is a `ComplexF` — so complex
    // Cholesky's `Let d be sqrt(..)` then reuse of `d` takes the ComplexF codegen path — and a
    // decimal, modular, quantity or money result (`Let subtotal be latte + muffin`) binds its own
    // type, so later arithmetic on the binding (`subtotal * 2`) stays exact.
    if let Some(exact) =
        crate::codegen::expr::exact_value_type_of(value, ctx.get_variable_types(), interner)
    {
        ctx.register_variable_type(*var, exact.to_string());
    }

    // `Let fac be f's lu` — a struct field-access binding takes the field's DECLARED type, so a
    // `Seq of ComplexF` field's elements route through the ComplexF codegen path later
    // (`item i of fac` → `.mul`/`.add`, not the type-mismatched `as f64` float path).
    if let Expr::FieldAccess { object, field } = value {
        if let Expr::Identifier(obj_sym) = object {
            if let Some(struct_name) = ctx.get_variable_types().get(obj_sym).cloned() {
                let field_name = interner.resolve(*field);
                for (tname, tdef) in registry.iter_types() {
                    if interner.resolve(*tname) == struct_name {
                        if let crate::analysis::registry::TypeDef::Struct { fields, .. } = tdef {
                            if let Some(fd) = fields
                                .iter()
                                .find(|fd| interner.resolve(fd.name) == field_name)
                            {
                                let rust_ty =
                                    crate::codegen::types::codegen_field_type(&fd.ty, interner);
                                ctx.register_variable_type(*var, rust_ty);
                            }
                        }
                        break;
                    }
                }
            }
        }
    }

    // Infer result type from function call return types
    if !ctx.get_variable_types().contains_key(var) {
        if let Expr::Call { function, .. } = value {
            // The `decimal(..)` builtin yields an exact `LogosDecimal` — record it so
            // later `+ − ×` over this binding take the exact decimal codegen path.
            if interner.resolve(*function) == "decimal" {
                ctx.register_variable_type(*var, "LogosDecimal".to_string());
            } else if interner.resolve(*function) == "complex" {
                // Two exact reals (a float part was caught above as `ComplexF`).
                ctx.register_variable_type(*var, "LogosComplex".to_string());
            } else if interner.resolve(*function) == "modular" {
                ctx.register_variable_type(*var, "LogosModular".to_string());
            } else if matches!(interner.resolve(*function), "quantity" | "convert") {
                // Both quantity builtins yield a `LogosQuantity`; record it so later
                // `+ − × ÷` / comparison over this binding take the quantity codegen path.
                ctx.register_variable_type(*var, "LogosQuantity".to_string());
            } else if interner.resolve(*function) == "money" {
                // `money(..)` yields a `LogosMoney`; record it so later `+ − × ÷` /
                // comparison over this binding take the money codegen path.
                ctx.register_variable_type(*var, "LogosMoney".to_string());
            } else if matches!(
                interner.resolve(*function),
                "uuid"
                    | "uuid_nil"
                    | "uuid_max"
                    | "uuid_v3"
                    | "uuid_v5"
                    | "uuid_dns"
                    | "uuid_url"
                    | "uuid_oid"
                    | "uuid_x500"
            ) {
                // Every UUID constructor yields a `LogosUuid`; record it so a later
                // comparison over this binding takes the (Ord) UUID codegen path.
                ctx.register_variable_type(*var, "LogosUuid".to_string());
            } else if interner.resolve(*function) == "text_from_bytes" {
                // The byte→Text builtin yields a `String`; record it so a subsequent
                // `Let userVal be creds` (this binding read by value) clones instead of
                // moving — a bare move would strand a later read as a use-after-move.
                ctx.register_variable_type(*var, "String".to_string());
            } else if let Some(rt) = ctx.get_fn_return(function).cloned() {
                ctx.register_variable_type(*var, rt);
            }
        }
    }

    // Register scalar types for mixed Float*Int arithmetic coercion
    if !ctx.get_variable_types().contains_key(var) {
        let inferred = infer_rust_type_from_expr(value, interner);
        if inferred != "_" {
            ctx.register_variable_type(*var, inferred);
        } else {
            // Try deeper numeric type inference for expressions like `4.0 * pi * pi`
            let numeric = infer_numeric_type(value, interner, ctx.get_variable_types());
            if numeric != "unknown" {
                ctx.register_variable_type(*var, numeric.to_string());
            } else if let crate::analysis::types::LogosType::Map(k, v) =
                super::types::infer_logos_type(value, interner, ctx.get_variable_types())
            {
                // A `{k: v, …}` map-literal binding: record its `Map<K, V>` type so a
                // numeric-unified key lookup (`m contains 1.0` on an Int-keyed map) can
                // coerce. Only Map is registered here — the narrowest change that closes
                // the compiled key-coercion gap.
                ctx.register_variable_type(
                    *var,
                    crate::analysis::types::LogosType::Map(k, v).to_rust_type(),
                );
            }
        }
    }

    // A destructure binder `Let fv be item N of tmp` over a MARKED tuple whose element unboxes
    // to a concrete type: register that element type. The desugared binder's value is an
    // `item N of tmp` Index, so `infer_rust_type_from_expr` cannot see the element type (it
    // lives in the tuple's marker), leaving `fv` untyped — and an untyped `Set sev to fv` then
    // MOVES `fv`, stranding a sibling branch's `Set sqlstate to fv` as a use-after-move. Reading
    // the marker here types the binder so the value-copy clone fires.
    if !ctx.get_variable_types().contains_key(var) {
        if let Expr::Index {
            collection,
            index: Expr::Literal(Literal::Number(n)),
        } = value
        {
            if *n >= 1 {
                let tuple_type = match collection {
                    Expr::Identifier(tuple_sym) => ctx.get_variable_types().get(tuple_sym).cloned(),
                    other => crate::codegen::expr::record_field_rust_type(
                        other,
                        interner,
                        ctx.get_variable_types(),
                        registry,
                    ),
                };
                if let Some(ty) = tuple_type
                    .as_deref()
                    .and_then(|t| super::types::logos_tuple_elem_type(t, *n).map(str::to_string))
                {
                    ctx.register_variable_type(*var, ty);
                }
            }
        }
    }

    // A nested list literal (`[[…], …]`) whose whole-program type only resolved to the
    // imprecise `LogosSeq<_>`: re-register the precise `LogosSeq<LogosSeq<…>>` so the
    // nested-write through-write fast path can specialise on the inner Seq. Narrowly gated
    // on the imprecise inner, so a de-Rc'd `Vec` or a specialised representation is untouched.
    if matches!(value, Expr::List(_))
        && ctx
            .get_variable_types()
            .get(var)
            .map_or(false, |t| t == "LogosSeq<_>")
    {
        if let crate::analysis::types::LogosType::Seq(inner) =
            super::types::infer_logos_type(value, interner, ctx.get_variable_types())
        {
            if matches!(*inner, crate::analysis::types::LogosType::Seq(_)) {
                ctx.register_variable_type(
                    *var,
                    crate::analysis::types::LogosType::Seq(inner).to_rust_type(),
                );
            }
        }
    }

    // OPT: Single-char text variable → u8 byte.
    // If this variable is pre-registered as __single_char_u8, emit u8 instead of String.
    let is_single_char_u8 = ctx
        .get_variable_types()
        .get(var)
        .map_or(false, |t| t == "__single_char_u8");

    if is_single_char_u8 {
        if let Expr::Literal(Literal::Text(sym)) = value {
            let text = interner.resolve(*sym);
            if text.len() == 1 {
                let ch = text.as_bytes()[0];
                let is_mutable = *mutable || mutable_vars.contains(var);
                if is_mutable {
                    writeln!(
                        output,
                        "{}let mut {}: u8 = b'{}';",
                        indent_str, var_name, ch as char
                    )
                    .unwrap();
                } else {
                    writeln!(
                        output,
                        "{}let {}: u8 = b'{}';",
                        indent_str, var_name, ch as char
                    )
                    .unwrap();
                }
                // Don't register as string var — it's a u8 now
                return output;
            }
        }
    }

    // Materialize Expr::Slice on LogosSeq as Vec<T> to avoid dangling
    // &[T] borrows from temporary Ref guards. Stored as Vec (not LogosSeq)
    // so that downstream `copy of` can wrap with LogosSeq::from_vec()
    // for zero-copy ownership transfer instead of deep_clone.
    if let Expr::Slice {
        collection,
        start,
        end,
    } = value
    {
        if let Expr::Identifier(coll_sym) = collection {
            if let Some(coll_type) = ctx.get_variable_types().get(coll_sym).cloned() {
                if coll_type.starts_with("LogosSeq<") {
                    let elem_type = coll_type
                        .strip_prefix("LogosSeq<")
                        .and_then(|t| t.strip_suffix('>'))
                        .unwrap_or("_");
                    let coll_name = names.ident(*coll_sym);
                    let start_str = codegen_expr_boxed_with_types(
                        start,
                        interner,
                        synced_vars,
                        boxed_fields,
                        registry,
                        async_functions,
                        ctx.get_string_vars(),
                        ctx.get_variable_types(),
                        ctx.get_fast_div(),
                    );
                    let end_str = codegen_expr_boxed_with_types(
                        end,
                        interner,
                        synced_vars,
                        boxed_fields,
                        registry,
                        async_functions,
                        ctx.get_string_vars(),
                        ctx.get_variable_types(),
                        ctx.get_fast_div(),
                    );
                    let is_mutable = *mutable || mutable_vars.contains(var);
                    let mutk = if is_mutable { "mut " } else { "" };
                    writeln!(
                        output,
                        "{}let {}{}: Vec<{}> = {}.borrow().logos_slice({}, {}).to_vec();",
                        indent_str, mutk, var_name, elem_type, coll_name, start_str, end_str
                    )
                    .unwrap();
                    // Register as Vec<T> — indexing works directly, and `copy of`
                    // wraps with LogosSeq::from_vec(x.clone()) for one-copy semantics.
                    ctx.register_variable_type(*var, format!("Vec<{}>", elem_type));
                    if is_definitely_string_expr_with_vars(value, ctx.get_string_vars()) {
                        ctx.register_string_var(*var);
                    }
                    return output;
                } else if coll_type.starts_with("&[") {
                    let elem_type = coll_type
                        .strip_prefix("&[")
                        .and_then(|t| t.strip_suffix(']'))
                        .unwrap_or("_");
                    let coll_name = names.ident(*coll_sym);
                    let start_str = codegen_expr_boxed_with_types(
                        start,
                        interner,
                        synced_vars,
                        boxed_fields,
                        registry,
                        async_functions,
                        ctx.get_string_vars(),
                        ctx.get_variable_types(),
                        ctx.get_fast_div(),
                    );
                    let end_str = codegen_expr_boxed_with_types(
                        end,
                        interner,
                        synced_vars,
                        boxed_fields,
                        registry,
                        async_functions,
                        ctx.get_string_vars(),
                        ctx.get_variable_types(),
                        ctx.get_fast_div(),
                    );
                    let is_mutable = *mutable || mutable_vars.contains(var);
                    let mutk = if is_mutable { "mut " } else { "" };
                    // Direct slice of borrow param — no .borrow() needed
                    writeln!(
                        output,
                        "{}let {}{}: Vec<{}> = {}.logos_slice({}, {}).to_vec();",
                        indent_str, mutk, var_name, elem_type, coll_name, start_str, end_str
                    )
                    .unwrap();
                    ctx.register_variable_type(*var, format!("Vec<{}>", elem_type));
                    if is_definitely_string_expr_with_vars(value, ctx.get_string_vars()) {
                        ctx.register_string_var(*var);
                    }
                    return output;
                }
            }
        }
    }

    // Overflow-promoting Int binding: a promotion candidate whose initializer is an
    // integer expression is stored as `LogosInt` (not a bare `i64`), so a value that
    // exceeds i64 promotes instead of panicking. Register the `|__bigint` sentinel BEFORE
    // codegen so any self-reference in the RHS already reads as a LogosInt.
    let is_bigint_binding = ty.is_none()
        && ctx.is_promotable_candidate(*var)
        && crate::codegen::types::infer_numeric_type(value, interner, ctx.get_variable_types())
            == "i64";
    if is_bigint_binding {
        ctx.register_variable_type(*var, "i64|__bigint".to_string());
    }

    // Phase 54+: the initializer through `codegen_expr_boxed_with_types_oracle`, with string + type tracking.
    let mut value_str = if is_bigint_binding {
        // Tolerant (un-narrowed) RHS, coerced to LogosInt (`.into()` is identity when the
        // RHS is already a LogosInt, and promotes a plain `i64` otherwise).
        let raw = codegen_expr_boxed_with_types_oracle_tolerant(
            value,
            interner,
            synced_vars,
            boxed_fields,
            registry,
            async_functions,
            ctx.get_string_vars(),
            ctx.get_variable_types(),
            ctx.get_fast_div(),
            ctx.oracle(),
        );
        format!("logicaffeine_data::LogosInt::from({})", raw)
    } else {
        codegen_expr_boxed_with_types_oracle(
            value,
            interner,
            synced_vars,
            boxed_fields,
            registry,
            async_functions,
            ctx.get_string_vars(),
            ctx.get_variable_types(),
            ctx.get_fast_div(),
            ctx.oracle(),
        )
    };

    // Phase D: lower a non-aliased `Map of Int to Int` to the specialized
    // open-addressing `LogosI64Map`, or the keys-only `LogosI64Set` when
    // the value is never read (contains + insert only). Override the
    // constructor AND re-register the type so the two never disagree,
    // regardless of which `Let` initializer shape registered it above.
    if ctx.is_i64_map(*var) {
        // Densest tier first: a proven bounded key domain → a
        // direct-addressed array sized to the capacity hint, offset by the
        // proven window `lo`. Otherwise the open-addressing map/set.
        let dense = ctx.dense_info(*var);
        let i64_ty = if let Some(info) = dense {
            dense_type_name(info.kind)
        } else if ctx.is_i32_set(*var) {
            "LogosI32Set"
        } else if ctx.is_i32_map(*var) {
            "LogosI32Map"
        } else if ctx.is_i64_set(*var) {
            "LogosI64Set"
        } else {
            "LogosI64Map"
        };
        if let Some(ctor) = i64_map_ctor(
            value,
            i64_ty,
            dense.map(|i| i.lo),
            interner,
            synced_vars,
            boxed_fields,
            registry,
            async_functions,
            ctx,
        ) {
            value_str = ctor;
            ctx.register_variable_type(*var, i64_ty.to_string());
        }
    }

    // Grand Challenge: Variable is mutable if explicitly marked OR if it's a Set target.
    // A `LogosI64Map` is mutated in place via `&mut self` (the analysis
    // requires ≥1 insert), so it must bind as `let mut`.
    let is_mutable = *mutable || mutable_vars.contains(var) || ctx.is_i64_map(*var);

    // Reference semantics: clone LogosSeq/LogosMap on assignment to share the Rc.
    // Borrow param (&[T]) assigned to mutable local → convert to owned.
    // A non-Copy SCALAR source (a `String` — `Let userVal be creds`) is copied by VALUE on
    // every tier (the interpreter/VM bind a fresh value), so it must clone too: the source
    // variable is frequently read again after the binding (a following `indexOf(creds)` or a
    // sibling-branch `cutBefore(creds, …)`), and a bare move would leave those a use-after-move
    // (E0382). Mirrors the field-read clone just below and the defensive arg clone in the call
    // codegen; a redundant clone of a truly-dead value is elided by LLVM.
    if let Expr::Identifier(src_sym) = value {
        if let Some(src_type) = ctx.get_variable_types().get(src_sym) {
            if super::expr::lent_by_sharing(*src_sym, ctx.get_variable_types()) {
                value_str = format!("{}.deep_clone()", value_str);
            } else if src_type.starts_with("LogosSeq") || src_type.starts_with("LogosMap") {
                value_str = format!("{}.clone()", value_str);
            } else if is_mutable && src_type.starts_with("&[") {
                value_str = format!("{}.to_vec()", value_str);
            } else if !is_copy_type(src_type)
                && !src_type.starts_with("&[")
                && !src_type.starts_with("&mut [")
            {
                value_str = format!("{}.clone()", value_str);
            }
        }
    }

    // Value semantics for a field READ bound into an owned local — the analog of
    // the identifier clone above. `Let rows be result's rows` MOVES the field out
    // of the object; if the object is used again afterward (`… pgIntAt(result, …)`)
    // that partial move fails to compile ("borrow of partially moved value"). A
    // `Let` owns its binding, so clone the field — keeping the object whole, as the
    // interpreter/VM already do. Only the owning `Let` needs this; a borrowed read
    // (`Show`/`length of`/indexing) never moves. A Copy SCALAR field (Int/Real/Bool)
    // is copied by a bare read already, so leave it a free move (no redundant clone).
    if let Expr::FieldAccess { object, field } = value {
        let field_name = interner.resolve(*field);
        let is_copy_scalar = if let Expr::Identifier(obj_sym) = object {
            ctx.get_variable_types()
                .get(obj_sym)
                .and_then(|ty_name| interner.lookup(ty_name))
                .and_then(|ty_sym| registry.get(ty_sym))
                .and_then(|def| match def {
                    logicaffeine_language::analysis::registry::TypeDef::Struct {
                        fields, ..
                    } => fields
                        .iter()
                        .find(|f| interner.resolve(f.name) == field_name),
                    _ => None,
                })
                .map(|fd| {
                    matches!(
                        super::types::codegen_field_type(&fd.ty, interner).as_str(),
                        "i64" | "f64" | "bool"
                    )
                })
                .unwrap_or(false)
        } else {
            false
        };
        if !is_copy_scalar {
            value_str = format!("{}.clone()", value_str);
        }
    }

    // Phase 103: Get explicit type annotation or infer for multi-param generic enums
    let type_annotation = if is_bigint_binding {
        Some("logicaffeine_data::LogosInt".to_string())
    } else {
        ty.map(|t| codegen_type_expr(t, interner))
            .or_else(|| ctx.rational_join(*var).cloned())
            .or_else(|| super::rational::rational_type_of(value, super::rational::Typing { interner, variable_types: ctx.get_variable_types(), registry }))
            .or_else(|| infer_variant_type_annotation(value, registry, interner))
            .or_else(|| type_silent_let_annotation(value, ctx.get_variable_types().get(var)))
    };

    // A binding that holds rationals — declared one, a list of them, or exactly joined from integers and rationals —
    // holds every integer it is given as the rational it equals. A value no type is known for is an integer.
    if let Some(held) = type_annotation.clone().filter(|t| t.contains(super::rational::RATIONAL)) {
        let to = super::rational::named(&held);
        if let Some(code) = rational_value(
            value,
            &held,
            interner,
            ctx,
            synced_vars,
            async_functions,
            boxed_fields,
            registry,
        ) {
            value_str = code;
        } else if to == crate::analysis::types::LogosType::Rational
            && super::rational::type_of(value, super::rational::Typing { interner, variable_types: ctx.get_variable_types(), registry })
                == crate::analysis::types::LogosType::Unknown
            && !is_rational_expr(value, ctx.get_variable_types())
        {
            value_str = format!("LogosRational::from_i64(({}) as i64)", value_str);
        }
        ctx.register_variable_type(*var, held);
    }

    match (is_mutable, type_annotation) {
        (true, Some(t)) => writeln!(
            output,
            "{}let mut {}: {} = {};",
            indent_str, var_name, t, value_str
        )
        .unwrap(),
        (true, None) => writeln!(
            output,
            "{}let mut {} = {};",
            indent_str, var_name, value_str
        )
        .unwrap(),
        (false, Some(t)) => writeln!(
            output,
            "{}let {}: {} = {};",
            indent_str, var_name, t, value_str
        )
        .unwrap(),
        (false, None) => {
            writeln!(output, "{}let {} = {};", indent_str, var_name, value_str).unwrap()
        }
    }

    // O9 libdivide: if this binding is a loop-invariant positive divisor,
    // precompute its magic multiply ONCE here so every later `% n` / `/ n`
    // lowers to `helper.rem(..)` / `helper.div(..)`. Harmless if `n <= 0`
    // (the helper is only ever read inside loops that don't run there).
    if let Some(helper) = ctx.get_fast_div().get(var) {
        writeln!(
            output,
            "{}let {} = logicaffeine_data::LogosDivU64::new(({}) as u64);",
            indent_str, helper, var_name
        )
        .unwrap();
    }

    // Track string variables for proper concatenation in subsequent expressions
    if is_definitely_string_expr_with_vars(value, ctx.get_string_vars()) {
        ctx.register_string_var(*var);
    }

    // Phase 43C: Handle refinement type
    if let Some(TypeExpr::Refinement {
        base: _,
        var: bound_var,
        predicate,
    }) = ty
    {
        emit_refinement_check(
            &var_name,
            *bound_var,
            predicate,
            interner,
            &indent_str,
            &mut output,
        );
        ctx.register(*var, *bound_var, predicate);
    }
    output
}

pub fn codegen_stmt<'a>(
    stmt: &Stmt<'a>,
    interner: &Interner,
    indent: usize,
    mutable_vars: &HashSet<Symbol>,
    ctx: &mut RefinementContext<'a>,
    lww_fields: &HashSet<(String, String)>,
    mv_fields: &HashSet<(String, String)>, // Phase 49b: MVRegister fields (no timestamp)
    synced_vars: &mut HashSet<Symbol>,     // Phase 52: Track synced variables
    var_caps: &HashMap<Symbol, VariableCapabilities>, // Phase 56: Mount+Sync detection
    async_functions: &HashSet<Symbol>,     // Phase 54: Functions that are async
    pipe_vars: &HashSet<Symbol>,           // Phase 54: Pipe declarations (have _tx/_rx suffixes)
    boxed_fields: &HashSet<(String, String, String)>, // Phase 102: Recursive enum fields
    registry: &TypeRegistry,               // Phase 103: For type annotations on polymorphic enums
    type_env: &crate::analysis::types::TypeEnv,
) -> String {
    let indent_str = "    ".repeat(indent);
    let mut output = String::new();
    let names = RustNames::new(interner);

    // OPT-1C: the variables live after this statement (`None` for a statement the liveness analysis never saw →
    // conservative clone), published for the expressions it emits.
    let live_vars_after: Option<HashSet<Symbol>> = super::flow::live_after(stmt);
    let _flow = super::flow::enter(stmt);
    let _control = super::control::enter(stmt);

    // Mode B: a scheduler operation lowers onto the deterministic replay executor.
    if super::deterministic_replay_enabled() {
        let mut scope = super::replay::Scope {
            interner,
            indent,
            mutable_vars,
            ctx: &mut *ctx,
            lww_fields,
            mv_fields,
            synced_vars: &mut *synced_vars,
            var_caps,
            async_functions,
            pipe_vars,
            boxed_fields,
            registry,
            type_env,
        };
        if let Some(code) = super::replay::replay_stmt(stmt, &mut scope) {
            output.push_str(&code);
            return output;
        }
    }

    // Drop a scratch buffer's `new Seq` declaration at any nesting: its fill loop is rewritten in place
    // to `let w: [T; N] = from_fn(…)` (the `Stmt::Repeat` handler), which becomes the binding, so this
    // `Seq::default()` declaration would be a dead duplicate.
    if let Stmt::Let { var, .. } = stmt {
        if ctx.scratch_buffer(*var).is_some() {
            return output;
        }
    }

    // O5: prepend oracle-proven bounds-elision hints for this statement's
    // indexed accesses (the relational `i <= length(arr)` proofs O5a's for-range
    // counter hints cannot reach — e.g. a growing FIFO read by a cursor).
    output.push_str(&emit_oracle_index_hints(stmt, ctx, interner, registry, &indent_str));

    // A change to a value-or-nothing, or to a collection that holds them.
    let held = {
        let ecx = operand_ctx(interner, ctx, synced_vars, async_functions, boxed_fields, registry);
        super::absent::statement(stmt, &ecx, &indent_str)
    };
    if let Some((code, bound)) = held {
        output.push_str(&code);
        if let Some((var, ty)) = bound {
            ctx.register_variable_type(var, ty);
        }
        return output;
    }

    match stmt {
        // ★ A LOWERED `## Route` IS A DECLARATION HERE, exactly as it is on the VM. It became an
        // ordinary function plus the synthesized `route`/`routeStatus` walk before this codegen
        // ever ran, so emitting anything for the declaration itself would be emitting the route
        // twice — and emitting a `compile_error!` meant NO route program had ever built to
        // AOT-Rust, which is the tier a server actually ships as. The refusal is asked the SAME
        // question the VM asks (`route_lowerable`) so the two tiers cannot disagree about which
        // routes run, and a route that genuinely did not lower still refuses BY NAME rather than
        // vanishing into a server that answers nothing.
        Stmt::RouteDef { method, path, .. } => {
            if let Some(why) = crate::codegen::expr::route_refusal(*method, *path) {
                output.push_str(&format!(
                    "{indent_str}compile_error!(\"this `## Route` cannot run on this tier yet: {}.\");\n",
                    why.replace('"', "'")
                ));
            }
        }
        // Declarations the router carries — the lowering already emitted their functions.
        Stmt::MiddlewareDef { .. } | Stmt::RedirectDef { .. } => {}
        // ★ A `## Component` IS A DECLARATION HERE, exactly as a `## Route` is. Its functions were
        // synthesized by `lower_lgx_components` before this codegen ever ran, so the declaration
        // itself emits nothing — and the browser-only half of those functions is skipped by
        // `is_browser_only_lowering`, leaving `__html_<Name>` to render markup on a tier with no
        // document.
        //
        // It used to emit a blanket `compile_error!`, which meant a server declaring ONE component
        // did not build, and therefore that PER-REQUEST SERVER RENDERING DID NOT EXIST IN
        // PRODUCTION: markup could only be produced at build time by pre-rendering, so a dashboard
        // or a search result had no server-rendered form at all. A `## Directive` is the same
        // shape — behaviour attached to an element, which no server template can reference
        // (`view_html_lowerable` refuses a directive attribute by name).
        Stmt::ComponentDef(_) | Stmt::DirectiveDef { .. } => {}
        // These still need a browser and say so. `Mount` renders into a document; `Fetch` and
        // `Serve fetches` are the page's side of the wire; `Replay` drives a recorded session.
        // A server that silently dropped one would answer with a page that never appears.
        Stmt::Ui(_)
        | Stmt::Fetch { .. }
        | Stmt::GivenFetch { .. }
        | Stmt::ServeFetches
        | Stmt::Replay { .. } => {
            output.push_str(&format!("{indent_str}compile_error!(\"this needs a browser: mounting, fetching and replaying run in a page, not in a server binary\");\n"));
        }
        Stmt::TestDef { .. } => {
            // `## Test` blocks never enter a compiled program: the runner
            // executes them on the interpreter tier (constitution Wave 3;
            // compiled tiers join in Wave 13).
        }
        Stmt::Expect { .. }
        | Stmt::ExpectFail { .. }
        | Stmt::ExpectOutput { .. }
        | Stmt::AdvanceTime { .. }
        | Stmt::GivenFile { .. }
        | Stmt::ForAll { .. } => {
            // Parser-gated to `## Test` bodies, which are never code-generated.
            unreachable!("test-only statement reached codegen outside a ## Test block")
        }
        Stmt::Observe {
            category,
            message,
            payloads,
            sample: _,
        } => {
            // Wave 13 — the OBSERVED profile emits the observation into the
            // compiled binary's channel; the ELIDED (release) profile strips
            // `Observe` before codegen (`elide_observation`), so this arm is only
            // reached when observation is wanted.
            use logicaffeine_language::ast::stmt::ObsCategory;
            let kind = match category {
                ObsCategory::Note => "note",
                ObsCategory::Warn => "warn",
                ObsCategory::Record => "record",
            };
            let msg = interner.resolve(*message);
            let parts: Vec<String> = payloads
                .iter()
                .map(|(name, expr)| {
                    let name_s = interner.resolve(*name);
                    let expr_rust = codegen_expr_with_async_and_strings(
                        expr,
                        interner,
                        synced_vars,
                        async_functions,
                        ctx.get_string_vars(),
                        ctx.get_variable_types(),
                        ctx.get_fast_div(),
                        registry,
                    );
                    format!("({name_s:?}, format!(\"{{}}\", {expr_rust}))")
                })
                .collect();
            writeln!(
                output,
                "{indent_str}logicaffeine_system::obs::record({kind:?}, {msg:?}, &[{}]);",
                parts.join(", ")
            )
            .unwrap();
        }
        Stmt::During { label, body } => {
            // Wave 13 — a balanced trace span in the OBSERVED profile (the ELIDED
            // profile unwraps `During` to its body before codegen).
            let name = interner.resolve(*label);
            writeln!(
                output,
                "{indent_str}logicaffeine_system::obs::span_enter({name:?});"
            )
            .unwrap();
            for inner in body.iter() {
                output.push_str(&codegen_stmt(
                    inner,
                    interner,
                    indent,
                    mutable_vars,
                    ctx,
                    lww_fields,
                    mv_fields,
                    synced_vars,
                    var_caps,
                    async_functions,
                    pipe_vars,
                    boxed_fields,
                    registry,
                    type_env,
                ));
            }
            writeln!(
                output,
                "{indent_str}logicaffeine_system::obs::span_exit({name:?});"
            )
            .unwrap();
        }
        Stmt::Swap { a, b } => {
            // Reversible register exchange → a plain Rust swap of the two same-typed locals; a name swapped with
            // itself changes nothing.
            let na = names.ident(*a);
            let nb = names.ident(*b);
            if na != nb {
                writeln!(output, "{indent_str}std::mem::swap(&mut {na}, &mut {nb});").unwrap();
            }
        }
        Stmt::Splice { body } => {
            // Scope-transparent desugar output: emit the statements FLAT —
            // no braces, same indent — so a multi-push lowers byte-identically
            // to the consecutive pushes it desugars from.
            for inner in body.iter() {
                output.push_str(&codegen_stmt(
                    inner,
                    interner,
                    indent,
                    mutable_vars,
                    ctx,
                    lww_fields,
                    mv_fields,
                    synced_vars,
                    var_caps,
                    async_functions,
                    pipe_vars,
                    boxed_fields,
                    registry,
                    type_env,
                ));
            }
        }
        Stmt::Reversibly { body } => {
            // A reversible/uncompute block is output-TRANSPARENT: the AOT tier has no history-tape
            // notion, so it emits the body FLAT (same as `Splice`), producing byte-identical output
            // to the same statements outside the block.
            for inner in body.iter() {
                output.push_str(&codegen_stmt(
                    inner,
                    interner,
                    indent,
                    mutable_vars,
                    ctx,
                    lww_fields,
                    mv_fields,
                    synced_vars,
                    var_caps,
                    async_functions,
                    pipe_vars,
                    boxed_fields,
                    registry,
                    type_env,
                ));
            }
        }
        Stmt::Let {
            var,
            ty,
            value,
            mutable,
        } => {
            return codegen_let_stmt(
                output,
                stmt,
                var,
                ty,
                value,
                mutable,
                interner,
                mutable_vars,
                ctx,
                synced_vars,
                async_functions,
                boxed_fields,
                registry,
                &indent_str,
                &names,
            );
        }

        Stmt::Set { target, value } => {
            return codegen_set_stmt(
                output,
                target,
                value,
                interner,
                ctx,
                synced_vars,
                async_functions,
                boxed_fields,
                registry,
                &indent_str,
                &names,
                live_vars_after,
            );
        }

        Stmt::Call { function, args } => {
            let fish = super::instantiation::for_stmt(stmt);
            return codegen_call_stmt(
                output,
                function,
                &fish,
                args,
                interner,
                ctx,
                synced_vars,
                async_functions,
                &indent_str,
                &names,
            );
        }

        Stmt::If {
            cond,
            then_block,
            else_block,
        } => {
            return codegen_if_stmt(
                output,
                cond,
                then_block,
                else_block,
                interner,
                indent,
                mutable_vars,
                ctx,
                lww_fields,
                mv_fields,
                synced_vars,
                var_caps,
                async_functions,
                pipe_vars,
                boxed_fields,
                registry,
                type_env,
                &indent_str,
            );
        }

        Stmt::While {
            cond,
            body,
            decreasing: _,
        } => {
            return codegen_while_stmt(
                output,
                stmt,
                cond,
                body,
                interner,
                indent,
                mutable_vars,
                ctx,
                lww_fields,
                mv_fields,
                synced_vars,
                var_caps,
                async_functions,
                pipe_vars,
                boxed_fields,
                registry,
                type_env,
                &indent_str,
            );
        }

        Stmt::Repeat {
            pattern,
            iterable,
            body,
        } => {
            return codegen_repeat_stmt(
                output,
                stmt,
                pattern,
                iterable,
                body,
                interner,
                indent,
                mutable_vars,
                ctx,
                lww_fields,
                mv_fields,
                synced_vars,
                var_caps,
                async_functions,
                pipe_vars,
                boxed_fields,
                registry,
                type_env,
                &indent_str,
                &names,
            );
        }

        Stmt::Return { value } => {
            // A `Return` inside a zone leaves the zone, its value computed and set aside.
            if let Some(exit) = super::control::return_label() {
                if let Some(value) = value {
                    let value = codegen_expr_with_async(
                        value,
                        interner,
                        synced_vars,
                        async_functions,
                        ctx.get_variable_types(),
                    );
                    writeln!(output, "{indent_str}let _ = {value};").unwrap();
                }
                writeln!(output, "{indent_str}break '{exit};").unwrap();
                return output;
            }
            return codegen_return_stmt(
                output,
                value,
                interner,
                ctx,
                synced_vars,
                async_functions,
                boxed_fields,
                registry,
                &indent_str,
            );
        }

        Stmt::Break => match super::control::break_label() {
            Some(exit) => writeln!(output, "{indent_str}break '{exit};").unwrap(),
            None => writeln!(output, "{}break;", indent_str).unwrap(),
        },

        // A proposition the program asserts or trusts is a proof obligation, discharged (or taken on
        // trust) before the program runs; no tier evaluates it at run time.
        Stmt::Assert { .. } => {}

        // Phase 35: Trust with documented justification
        Stmt::Trust { justification, .. } => {
            let reason = interner.resolve(*justification);
            // Strip quotes if present (string literals include their quotes)
            let reason_clean = reason.trim_matches('"');
            writeln!(output, "{}// TRUST: {}", indent_str, reason_clean).unwrap();
        }

        // `Require` and `Assert that` are both checked, in every build, as every tier checks them. A
        // reason is evaluated only when the contract breaks, and is the whole report — the panic hook
        // prints the payload verbatim.
        Stmt::RuntimeAssert {
            condition,
            hard: _,
            dim: _,
            reason,
        } => {
            let cond_str = codegen_expr_with_async_oracle(
                condition,
                interner,
                synced_vars,
                async_functions,
                ctx.get_variable_types(),
                ctx.oracle(),
            );
            match reason {
                None => {
                    // The same words every tier reports — never Rust's `assertion failed: <expr>`.
                    writeln!(
                        output,
                        "{}if !({}) {{ panic!(\"Assertion failed\"); }}",
                        indent_str, cond_str
                    )
                    .unwrap();
                }
                Some(r) => {
                    let reason_str = codegen_expr_with_async_oracle(
                        r,
                        interner,
                        synced_vars,
                        async_functions,
                        ctx.get_variable_types(),
                        ctx.oracle(),
                    );
                    writeln!(output, "{}if !({}) {{ panic!(\"{{}}\", logicaffeine_data::contract_reason(&({}))); }}", indent_str, cond_str, reason_str).unwrap();
                }
            }
        }

        Stmt::Fail { reason } => {
            let reason_str = codegen_expr_with_async_oracle(
                reason,
                interner,
                synced_vars,
                async_functions,
                ctx.get_variable_types(),
                ctx.oracle(),
            );
            // The report is the raised value exactly as `Show` prints it, whatever its type.
            writeln!(
                output,
                "{}panic!(\"{{}}\", logicaffeine_system::io::show_string(&({})));",
                indent_str, reason_str
            )
            .unwrap();
        }

        // Phase 50: Security Check - mandatory runtime guard (NEVER optimized out)
        Stmt::Check {
            subject,
            predicate,
            is_capability,
            object,
            source_text,
            ..
        } => {
            let subj_name = interner.resolve(*subject);
            let pred_name = interner.resolve(*predicate).to_lowercase();

            let call = if *is_capability {
                let obj_sym = object.expect("capability must have object");
                let obj_word = interner.resolve(obj_sym);

                // Phase 50: Type-based resolution
                // "Check that user can publish the document" -> find variable of type Document
                // First try to find a variable whose type matches the object word
                let obj_name = ctx
                    .find_variable_by_type(obj_word, interner)
                    .unwrap_or_else(|| obj_word.to_string());

                format!("{}.can_{}(&{})", subj_name, pred_name, obj_name)
            } else {
                format!("{}.is_{}()", subj_name, pred_name)
            };

            writeln!(output, "{}if !({}) {{", indent_str, call).unwrap();
            writeln!(
                output,
                "{}    logicaffeine_system::panic_with({:?});",
                indent_str,
                crate::semantics::policy::check_failed(source_text)
            )
            .unwrap();
            writeln!(output, "{}}}", indent_str).unwrap();
        }

        // Phase 51: P2P Networking - Listen on network address
        Stmt::Listen { address, secure } => {
            // The PNP one-time pad is wired only into the interpreter's channel seam; the native-AOT
            // transpile backend does not carry that seam yet, so fail loud (never silently drop crypto).
            if secure.is_some() {
                writeln!(output, "{}compile_error!(\"the PNP one-time pad (`with pad`) is only supported by the interpreter tier, not the native-AOT/transpile backend\");", indent_str).unwrap();
                return output;
            }
            let addr_str = codegen_expr_with_async(
                address,
                interner,
                synced_vars,
                async_functions,
                ctx.get_variable_types(),
            );
            // Pass &str instead of String
            writeln!(
                output,
                "{}logicaffeine_system::network::listen(&{}).await.expect(\"Failed to listen\");",
                indent_str, addr_str
            )
            .unwrap();
        }

        // HTTP server: bind the port, then per request marshal the parsed
        // `HttpRequest` into a LOGOS `Request`, call the handler, and marshal the
        // returned `Response` back to an `HttpResponse`. Byte-identical to the
        // interpreter tier: both go through `http::server::serve` →
        // `serialize_response`, and neither emits `Date`/volatile headers.
        Stmt::Serve { port, handler } => {
            return codegen_serve_stmt(
                output,
                port,
                handler,
                interner,
                ctx,
                synced_vars,
                async_functions,
                &indent_str,
            );
        }

        // Phase 51: P2P Networking - Connect to remote peer
        Stmt::ConnectTo { address, secure } => {
            if secure.is_some() {
                writeln!(output, "{}compile_error!(\"the PNP one-time pad (`with pad`) is only supported by the interpreter tier, not the native-AOT/transpile backend\");", indent_str).unwrap();
                return output;
            }
            let addr_str = codegen_expr_with_async(
                address,
                interner,
                synced_vars,
                async_functions,
                ctx.get_variable_types(),
            );
            // Pass &str instead of String
            writeln!(
                output,
                "{}logicaffeine_system::network::connect(&{}).await.expect(\"Failed to connect\");",
                indent_str, addr_str
            )
            .unwrap();
        }

        // Phase 51: P2P Networking - Create PeerAgent remote handle
        Stmt::LetPeerAgent { var, address } => {
            let var_name = interner.resolve(*var);
            let addr_str = codegen_expr_with_async(
                address,
                interner,
                synced_vars,
                async_functions,
                ctx.get_variable_types(),
            );
            // Pass &str instead of String
            writeln!(output, "{}let {} = logicaffeine_system::network::PeerAgent::new(&{}).expect(\"Invalid address\");",
                     indent_str, var_name, addr_str).unwrap();
            ctx.register_variable_type(*var, PEER_AGENT_TYPE.to_string());
        }

        // Phase 51: Sleep - supports Duration literals or milliseconds
        Stmt::Sleep { milliseconds } => {
            let expr_str = codegen_expr_with_async(
                milliseconds,
                interner,
                synced_vars,
                async_functions,
                ctx.get_variable_types(),
            );
            let inferred_type = infer_rust_type_from_expr(milliseconds, interner);

            if inferred_type == "std::time::Duration" {
                // Duration type: use directly (already a std::time::Duration)
                writeln!(
                    output,
                    "{}tokio::time::sleep({}).await;",
                    indent_str, expr_str
                )
                .unwrap();
            } else {
                // Assume milliseconds (integer) - legacy behavior
                writeln!(
                    output,
                    "{}tokio::time::sleep(std::time::Duration::from_millis({} as u64)).await;",
                    indent_str, expr_str
                )
                .unwrap();
            }
        }

        // Phase 52/56: Sync CRDT variable on topic
        Stmt::Sync { var, topic } => {
            let var_name = interner.resolve(*var);
            let topic_str = codegen_expr_with_async(
                topic,
                interner,
                synced_vars,
                async_functions,
                ctx.get_variable_types(),
            );

            // Phase 56: Check if this variable is also mounted
            if let Some(caps) = var_caps.get(var) {
                if caps.mounted {
                    // Both Mount and Sync: use Distributed<T>
                    // Mount statement will handle the Distributed::mount call
                    // Here we just track it as synced
                    synced_vars.insert(*var);
                    return output; // Skip - Mount will emit Distributed<T>
                }
            }

            // Sync-only: use Synced<T>
            writeln!(
                output,
                "{}let {} = logicaffeine_system::crdt::Synced::new({}, &{}).await;",
                indent_str, var_name, var_name, topic_str
            )
            .unwrap();
            synced_vars.insert(*var);
        }

        // Phase 53/56: Mount persistent CRDT from journal
        Stmt::Mount { var, path } => {
            let var_name = interner.resolve(*var);
            let path_str = codegen_expr_with_async(
                path,
                interner,
                synced_vars,
                async_functions,
                ctx.get_variable_types(),
            );

            // Phase 56: Check if this variable is also synced
            if let Some(caps) = var_caps.get(var) {
                if caps.synced {
                    // Both Mount and Sync: use Distributed<T>
                    let topic_str = caps
                        .sync_topic
                        .as_ref()
                        .map(|s| s.as_str())
                        .unwrap_or("\"default\"");
                    writeln!(
                        output,
                        "{}let {} = logicaffeine_system::distributed::Distributed::mount(vfs.clone(), &{}, Some({}.to_string())).await.expect(\"Failed to mount\");",
                        indent_str, var_name, path_str, topic_str
                    ).unwrap();
                    synced_vars.insert(*var);
                    return output;
                }
            }

            // Mount-only: use Persistent<T>
            writeln!(
                output,
                "{}let {} = logicaffeine_system::storage::Persistent::mount(vfs.clone(), &{}).await.expect(\"Failed to mount\");",
                indent_str, var_name, path_str
            ).unwrap();
            synced_vars.insert(*var);
        }

        // =====================================================================
        // Phase 54: Go-like Concurrency Codegen
        // =====================================================================
        Stmt::LaunchTask { function, args } => {
            let fn_name = names.ident(*function);
            // Phase 54: When passing a pipe variable, pass the sender (_tx)
            let args_str: Vec<String> = args
                .iter()
                .map(|a| {
                    if let Expr::Identifier(sym) = a {
                        if pipe_vars.contains(sym) {
                            return format!("{}_tx.clone()", interner.resolve(*sym));
                        }
                    }
                    let arg = codegen_expr_with_async(
                        a,
                        interner,
                        synced_vars,
                        async_functions,
                        ctx.get_variable_types(),
                    );
                    super::tower::crossing_to_a_task(a, arg, ctx.get_variable_types(), interner)
                })
                .collect();
            let args_str: Vec<(String, bool)> = args_str
                .into_iter()
                .zip(args.iter())
                .map(|(s, a)| (s, matches!(a, Expr::Literal(_))))
                .collect();
            // Phase 54: Add .await only if the function is async
            let await_suffix = if async_functions.contains(function) {
                ".await"
            } else {
                ""
            };
            writeln!(
                output,
                "{}tokio::spawn({});",
                indent_str,
                launched_future(&fn_name, &args_str, &format!("{await_suffix};"))
            )
            .unwrap();
        }

        Stmt::LaunchTaskWithHandle {
            handle,
            function,
            args,
        } => {
            let handle_name = interner.resolve(*handle);
            let fn_name = names.ident(*function);
            // Phase 54: When passing a pipe variable, pass the sender (_tx)
            let args_str: Vec<String> = args
                .iter()
                .map(|a| {
                    if let Expr::Identifier(sym) = a {
                        if pipe_vars.contains(sym) {
                            return format!("{}_tx.clone()", interner.resolve(*sym));
                        }
                    }
                    let arg = codegen_expr_with_async(
                        a,
                        interner,
                        synced_vars,
                        async_functions,
                        ctx.get_variable_types(),
                    );
                    super::tower::crossing_to_a_task(a, arg, ctx.get_variable_types(), interner)
                })
                .collect();
            let args_str: Vec<(String, bool)> = args_str
                .into_iter()
                .zip(args.iter())
                .map(|(s, a)| (s, matches!(a, Expr::Literal(_))))
                .collect();
            // Phase 54: Add .await only if the function is async
            let await_suffix = if async_functions.contains(function) {
                ".await"
            } else {
                ""
            };
            writeln!(
                output,
                "{}let {} = tokio::spawn({});",
                indent_str,
                handle_name,
                launched_future(&fn_name, &args_str, await_suffix)
            )
            .unwrap();
        }

        Stmt::CreatePipe {
            var,
            element_type,
            capacity,
        } => {
            let var_name = interner.resolve(*var);
            let type_name = interner.resolve(*element_type);
            let cap = capacity.unwrap_or(32);
            // Map LOGOS types to Rust types
            let rust_type = match type_name {
                "Int" => "i64",
                "Nat" => "u64",
                "Text" => "String",
                "Bool" => "bool",
                _ => type_name,
            };
            writeln!(
                output,
                "{}let ({}_tx, mut {}_rx) = tokio::sync::mpsc::channel::<{}>({});",
                indent_str, var_name, var_name, rust_type, cap
            )
            .unwrap();
        }

        Stmt::SendPipe { value, pipe } => {
            let val_str = codegen_expr_boxed_with_types_oracle(
                value,
                interner,
                synced_vars,
                boxed_fields,
                registry,
                async_functions,
                ctx.get_string_vars(),
                ctx.get_variable_types(),
                ctx.get_fast_div(),
                ctx.oracle(),
            );
            let pipe_str = codegen_expr_with_async(
                pipe,
                interner,
                synced_vars,
                async_functions,
                ctx.get_variable_types(),
            );
            // Phase 54: Check if pipe is a local declaration (has _tx suffix) or parameter (no suffix)
            let is_local_pipe = if let Expr::Identifier(sym) = pipe {
                pipe_vars.contains(sym)
            } else {
                false
            };
            if is_local_pipe {
                writeln!(
                    output,
                    "{}{}_tx.send({}).await.expect(\"pipe send failed\");",
                    indent_str, pipe_str, val_str
                )
                .unwrap();
            } else {
                writeln!(
                    output,
                    "{}{}.send({}).await.expect(\"pipe send failed\");",
                    indent_str, pipe_str, val_str
                )
                .unwrap();
            }
        }

        Stmt::ReceivePipe { var, pipe } => {
            let var_name = interner.resolve(*var);
            let pipe_str = codegen_expr_with_async(
                pipe,
                interner,
                synced_vars,
                async_functions,
                ctx.get_variable_types(),
            );
            // Phase 54: Check if pipe is a local declaration (has _rx suffix) or parameter (no suffix)
            let is_local_pipe = if let Expr::Identifier(sym) = pipe {
                pipe_vars.contains(sym)
            } else {
                false
            };
            if is_local_pipe {
                writeln!(
                    output,
                    "{}let {} = {}_rx.recv().await.expect(\"pipe closed\");",
                    indent_str, var_name, pipe_str
                )
                .unwrap();
            } else {
                writeln!(
                    output,
                    "{}let {} = {}.recv().await.expect(\"pipe closed\");",
                    indent_str, var_name, pipe_str
                )
                .unwrap();
            }
        }

        Stmt::TrySendPipe {
            value,
            pipe,
            result,
        } => {
            let val_str = codegen_expr_boxed_with_types_oracle(
                value,
                interner,
                synced_vars,
                boxed_fields,
                registry,
                async_functions,
                ctx.get_string_vars(),
                ctx.get_variable_types(),
                ctx.get_fast_div(),
                ctx.oracle(),
            );
            let pipe_str = codegen_expr_with_async(
                pipe,
                interner,
                synced_vars,
                async_functions,
                ctx.get_variable_types(),
            );
            // Phase 54: Check if pipe is a local declaration
            let is_local_pipe = if let Expr::Identifier(sym) = pipe {
                pipe_vars.contains(sym)
            } else {
                false
            };
            let suffix = if is_local_pipe { "_tx" } else { "" };
            if let Some(res) = result {
                let res_name = interner.resolve(*res);
                writeln!(
                    output,
                    "{}let {} = {}{}.try_send({}).is_ok();",
                    indent_str, res_name, pipe_str, suffix, val_str
                )
                .unwrap();
            } else {
                writeln!(
                    output,
                    "{}let _ = {}{}.try_send({});",
                    indent_str, pipe_str, suffix, val_str
                )
                .unwrap();
            }
        }

        Stmt::TryReceivePipe { var, pipe } => {
            let var_name = interner.resolve(*var);
            let pipe_str = codegen_expr_with_async(
                pipe,
                interner,
                synced_vars,
                async_functions,
                ctx.get_variable_types(),
            );
            // Phase 54: Check if pipe is a local declaration
            let is_local_pipe = if let Expr::Identifier(sym) = pipe {
                pipe_vars.contains(sym)
            } else {
                false
            };
            let suffix = if is_local_pipe { "_rx" } else { "" };
            writeln!(
                output,
                "{}let {} = {}{}.try_recv().ok();",
                indent_str, var_name, pipe_str, suffix
            )
            .unwrap();
        }

        Stmt::StopTask { handle } => {
            let handle_str = codegen_expr_with_async(
                handle,
                interner,
                synced_vars,
                async_functions,
                ctx.get_variable_types(),
            );
            writeln!(output, "{}{}.abort();", indent_str, handle_str).unwrap();
        }

        Stmt::CloseChannel { channel } => {
            let chan_str = codegen_expr_with_async(
                channel,
                interner,
                synced_vars,
                async_functions,
                ctx.get_variable_types(),
            );
            // A local pipe splits into `_tx`/`_rx`; closing is the sender's `close()`. A parameter channel
            // is the sender itself.
            let is_local_pipe = if let Expr::Identifier(sym) = channel {
                pipe_vars.contains(sym)
            } else {
                false
            };
            let suffix = if is_local_pipe { "_tx" } else { "" };
            writeln!(output, "{}{}{}.close();", indent_str, chan_str, suffix).unwrap();
        }

        Stmt::AwaitTask { handle, result } => {
            let handle_str = codegen_expr_with_async(
                handle,
                interner,
                synced_vars,
                async_functions,
                ctx.get_variable_types(),
            );
            // A tokio `JoinHandle` awaits to a `Result<T, JoinError>`; the join panics on a task panic
            // (the deterministic scheduler joins cleanly). `… into r` binds the value; a bare await discards.
            match result {
                Some(r) => {
                    let r_name = interner.resolve(*r);
                    writeln!(
                        output,
                        "{}let {} = {}.await.expect(\"task join failed\");",
                        indent_str, r_name, handle_str
                    )
                    .unwrap();
                }
                None => {
                    writeln!(output, "{}let _ = {}.await;", indent_str, handle_str).unwrap();
                }
            }
        }

        Stmt::Select { branches } => {
            write_tokio_select(
                &mut output,
                branches,
                interner,
                indent,
                mutable_vars,
                ctx,
                lww_fields,
                mv_fields,
                synced_vars,
                var_caps,
                async_functions,
                pipe_vars,
                boxed_fields,
                registry,
                type_env,
            );
        }

        Stmt::Give { object, recipient } => {
            // Move semantics: pass ownership without borrowing
            let obj_str = codegen_expr_with_async(
                object,
                interner,
                synced_vars,
                async_functions,
                ctx.get_variable_types(),
            );
            let recv_str = recipient_path(
                recipient,
                interner,
                synced_vars,
                async_functions,
                ctx.get_variable_types(),
            );
            writeln!(output, "{}{}({});", indent_str, recv_str, obj_str).unwrap();
        }

        Stmt::Show { object, recipient } => {
            return codegen_show_stmt(
                output,
                object,
                recipient,
                interner,
                ctx,
                synced_vars,
                async_functions,
                registry,
                &indent_str,
                &names,
            );
        }

        Stmt::SetField {
            object,
            field,
            value,
        } => {
            let obj_str = codegen_expr_with_async(
                object,
                interner,
                synced_vars,
                async_functions,
                ctx.get_variable_types(),
            );
            let field_name = interner.resolve(*field);
            let value_str = codegen_expr_boxed_with_types_oracle(
                value,
                interner,
                synced_vars,
                boxed_fields,
                registry,
                async_functions,
                ctx.get_string_vars(),
                ctx.get_variable_types(),
                ctx.get_fast_div(),
                ctx.oracle(),
            );
            let value_str = stored_value(value, value_str, ctx.get_variable_types());

            // Phase 49: Check if this field is an LWWRegister or MVRegister
            // LWW needs .set(value, timestamp), MV needs .set(value)
            let is_lww = lww_fields.iter().any(|(_, f)| f == field_name);
            let is_mv = mv_fields.iter().any(|(_, f)| f == field_name);
            let field_name = escape_rust_ident(field_name);
            if is_lww {
                // LWWRegister needs a timestamp - use current system time in microseconds
                writeln!(output, "{}{}.{}.set({}, std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_micros() as u64);", indent_str, obj_str, field_name, value_str).unwrap();
            } else if is_mv {
                // MVRegister just needs the value
                writeln!(
                    output,
                    "{}{}.{}.set({});",
                    indent_str, obj_str, field_name, value_str
                )
                .unwrap();
            } else {
                writeln!(
                    output,
                    "{}{}.{} = {};",
                    indent_str, obj_str, field_name, value_str
                )
                .unwrap();
            }
        }

        Stmt::StructDef { .. } => {
            // Struct definitions are handled in codegen_program, not here
        }

        Stmt::FunctionDef { .. } => {
            // Function definitions are handled in codegen_program, not here
        }

        Stmt::Inspect { target, arms, .. } => {
            return codegen_inspect_stmt(
                output,
                target,
                arms,
                interner,
                indent,
                mutable_vars,
                ctx,
                lww_fields,
                mv_fields,
                synced_vars,
                var_caps,
                async_functions,
                pipe_vars,
                boxed_fields,
                registry,
                type_env,
                &indent_str,
            );
        }

        Stmt::Push { value, collection } => {
            return codegen_push_stmt(
                output,
                value,
                collection,
                interner,
                ctx,
                synced_vars,
                async_functions,
                boxed_fields,
                registry,
                &indent_str,
                &names,
            );
        }

        Stmt::Pop { collection, into } => {
            if let Expr::Identifier(coll_sym) = collection {
                if ctx
                    .get_variable_types()
                    .get(coll_sym)
                    .is_some_and(|t| t == super::tower::VALUE)
                {
                    let popped = format!(
                        "logicaffeine_data::failure::answered({}.pop_value())",
                        names.ident(*coll_sym)
                    );
                    match into {
                        Some(var) => {
                            writeln!(
                                output,
                                "{}let {}: {} = {};",
                                indent_str,
                                names.ident(*var),
                                super::tower::VALUE,
                                popped
                            )
                            .unwrap();
                            ctx.register_variable_type(*var, super::tower::VALUE.to_string());
                        }
                        None => writeln!(output, "{}{};", indent_str, popped).unwrap(),
                    }
                    return output;
                }
            }
            let coll_str = codegen_expr_with_async(
                collection,
                interner,
                synced_vars,
                async_functions,
                ctx.get_variable_types(),
            );
            // MVS copy-on-write before the in-place pop (mutates the buffer).
            emit_cow(collection, ctx, &names, &indent_str, &mut output);
            match into {
                Some(var) => {
                    let var_name = names.ident(*var);
                    // An item popped from a list of rationals is the rational it is.
                    let rational = super::rational::held_item(collection, super::rational::Typing { interner, variable_types: ctx.get_variable_types(), registry });
                    let typed = rational.as_ref().map(|t| format!(": {t}")).unwrap_or_default();
                    // Unwrap the Option returned by pop() - panics if empty
                    writeln!(
                        output,
                        "{}let {}{} = {}.pop().expect(\"Pop from empty collection\");",
                        indent_str, var_name, typed, coll_str
                    )
                    .unwrap();
                    if let Some(item) = rational {
                        ctx.register_variable_type(*var, item);
                    }
                }
                None => {
                    writeln!(output, "{}{}.pop();", indent_str, coll_str).unwrap();
                }
            }
        }

        Stmt::Add { value, collection } => {
            let val_str = codegen_expr_boxed_with_types_oracle(
                value,
                interner,
                synced_vars,
                boxed_fields,
                registry,
                async_functions,
                ctx.get_string_vars(),
                ctx.get_variable_types(),
                ctx.get_fast_div(),
                ctx.oracle(),
            );
            let val_str = stored_value(value, val_str, ctx.get_variable_types());
            let val_str = value_taken_before_the_write(
                value,
                collection,
                val_str,
                ctx,
                &indent_str,
                &mut output,
            );
            let coll_str = codegen_expr_with_async(
                collection,
                interner,
                synced_vars,
                async_functions,
                ctx.get_variable_types(),
            );
            // MVS copy-on-write before the in-place insert (a `LogosMap`; a plain
            // `FxHashSet`/`Set` is already value-semantic and yields no `cow`).
            emit_cow(collection, ctx, &names, &indent_str, &mut output);
            writeln!(output, "{}{}.insert({});", indent_str, coll_str, val_str).unwrap();
        }

        Stmt::Remove { value, collection } => {
            let val_str = codegen_expr_boxed_with_types_oracle(
                value,
                interner,
                synced_vars,
                boxed_fields,
                registry,
                async_functions,
                ctx.get_string_vars(),
                ctx.get_variable_types(),
                ctx.get_fast_div(),
                ctx.oracle(),
            );
            let val_str = value_taken_before_the_write(
                value,
                collection,
                val_str,
                ctx,
                &indent_str,
                &mut output,
            );
            let coll_str = codegen_expr_with_async(
                collection,
                interner,
                synced_vars,
                async_functions,
                ctx.get_variable_types(),
            );
            // MVS copy-on-write before the in-place remove.
            emit_cow(collection, ctx, &names, &indent_str, &mut output);
            // Order-preserving removal: a `Set` is an IndexSet, whose plain
            // `remove` is a swap_remove — `shift_remove` keeps insertion
            // order (`LogosMap::remove` already shifts internally).
            let is_set = matches!(collection, Expr::Identifier(sym)
                if ctx.get_variable_types().get(sym).is_some_and(|t| t.starts_with("Set<") || t.starts_with("FxHashSet<")));
            let method = if is_set { "shift_remove" } else { "remove" };
            writeln!(
                output,
                "{}{}.{}(&{});",
                indent_str, coll_str, method, val_str
            )
            .unwrap();
        }

        Stmt::SetIndex {
            collection,
            index,
            value,
        } => {
            return codegen_set_index_stmt(
                output,
                collection,
                index,
                value,
                interner,
                ctx,
                synced_vars,
                async_functions,
                boxed_fields,
                registry,
                &indent_str,
                &names,
            );
        }

        // Phase 8.5: Zone (memory arena) block
        Stmt::Zone {
            name,
            capacity,
            source_file,
            body,
        } => {
            return codegen_zone_stmt(
                output,
                name,
                capacity,
                source_file,
                body,
                interner,
                indent,
                mutable_vars,
                ctx,
                lww_fields,
                mv_fields,
                synced_vars,
                var_caps,
                async_functions,
                pipe_vars,
                boxed_fields,
                registry,
                type_env,
                &indent_str,
            );
        }

        // Phase 9: Concurrent execution block (async, I/O-bound)
        // Generates tokio::join! for concurrent task execution
        // Phase 51: Variables used across multiple tasks are cloned to avoid move issues
        Stmt::Concurrent { tasks } => {
            return codegen_concurrent_stmt(
                output,
                tasks,
                interner,
                indent,
                mutable_vars,
                ctx,
                lww_fields,
                mv_fields,
                synced_vars,
                var_caps,
                async_functions,
                pipe_vars,
                boxed_fields,
                registry,
                type_env,
                &indent_str,
            );
        }

        // Parallel execution block (CPU-bound): one balanced tree of `rayon::join`s over every task
        Stmt::Parallel { tasks } => {
            return codegen_parallel_stmt(
                output,
                tasks,
                interner,
                indent,
                mutable_vars,
                ctx,
                lww_fields,
                mv_fields,
                synced_vars,
                var_caps,
                async_functions,
                pipe_vars,
                boxed_fields,
                registry,
                type_env,
                &indent_str,
            );
        }

        // Phase 10: Read from console or file
        // Phase 53: File reads now use async VFS
        Stmt::ReadFrom { var, source } => {
            let var_name = interner.resolve(*var);
            match source {
                ReadSource::Console => {
                    writeln!(
                        output,
                        "{}let {} = logicaffeine_system::io::read_line();",
                        indent_str, var_name
                    )
                    .unwrap();
                }
                ReadSource::File(path_expr) => {
                    let path_str = codegen_expr_with_async(
                        path_expr,
                        interner,
                        synced_vars,
                        async_functions,
                        ctx.get_variable_types(),
                    );
                    // Phase 53: Use VFS with async
                    writeln!(
                        output,
                        "{}let {} = vfs.read_to_string(&{}).await.expect(\"Failed to read file\");",
                        indent_str, var_name, path_str
                    )
                    .unwrap();
                }
            }
        }

        // Phase 10: Write to file
        // Phase 53: File writes now use async VFS
        Stmt::WriteFile { content, path } => {
            let content_str = codegen_expr_with_async(
                content,
                interner,
                synced_vars,
                async_functions,
                ctx.get_variable_types(),
            );
            let path_str = codegen_expr_with_async(
                path,
                interner,
                synced_vars,
                async_functions,
                ctx.get_variable_types(),
            );
            // Phase 53: Use VFS with async
            writeln!(
                output,
                "{}vfs.write(&{}, {}.as_bytes()).await.expect(\"Failed to write file\");",
                indent_str, path_str, content_str
            )
            .unwrap();
        }

        // Phase 46: Spawn an agent
        Stmt::Spawn { agent_type, name } => {
            let type_name = interner.resolve(*agent_type);
            let agent_name = interner.resolve(*name);
            // Generate agent spawn with tokio channel
            writeln!(
                output,
                "{}let {} = tokio::spawn(async move {{ /* {} agent loop */ }});",
                indent_str, agent_name, type_name
            )
            .unwrap();
        }

        // Phase 46: Send message to agent
        Stmt::SendMessage {
            message,
            destination,
            ..
        } => {
            let msg_str = codegen_expr_with_async(
                message,
                interner,
                synced_vars,
                async_functions,
                ctx.get_variable_types(),
            );
            let dest_str = codegen_expr_with_async(
                destination,
                interner,
                synced_vars,
                async_functions,
                ctx.get_variable_types(),
            );
            // A remote peer is reached through the mesh (`network::send`); an agent is a channel.
            let to_peer = matches!(destination, Expr::Identifier(sym)
                if ctx.get_variable_types().get(sym).is_some_and(|t| t == PEER_AGENT_TYPE));
            if to_peer {
                writeln!(
                    output,
                    "{}logicaffeine_system::network::send(&{}, &{}).await.expect(\"Failed to send message\");",
                    indent_str, dest_str, msg_str
                ).unwrap();
            } else {
                writeln!(
                    output,
                    "{}{}.send({}).await.expect(\"Failed to send message\");",
                    indent_str, dest_str, msg_str
                )
                .unwrap();
            }
        }

        // Streaming is a tree-walker feature (the relay receive runs in the interpreter); the AOT
        // path does not lower it yet — emit a marker rather than silently dropping it.
        Stmt::StreamMessage { .. } => {
            writeln!(
                output,
                "{}// Stream: interpreter-only (not yet lowered to AOT)",
                indent_str
            )
            .unwrap();
        }

        // Phase 46: Await response from agent. (`view` is a tree-walker zero-copy receive hint;
        // the compiled path decodes eagerly to the same values.)
        Stmt::AwaitMessage {
            source,
            into,
            view: _,
            stream: _,
        } => {
            let src_str = codegen_expr_with_async(
                source,
                interner,
                synced_vars,
                async_functions,
                ctx.get_variable_types(),
            );
            let var_name = interner.resolve(*into);
            writeln!(
                output,
                "{}let {} = {}.recv().await.expect(\"Failed to receive message\");",
                indent_str, var_name, src_str
            )
            .unwrap();
        }

        // Phase 49: Merge CRDT state
        Stmt::MergeCrdt { source, target } => {
            let src_str = codegen_expr_with_async(
                source,
                interner,
                synced_vars,
                async_functions,
                ctx.get_variable_types(),
            );
            let tgt_str = codegen_expr_with_async(
                target,
                interner,
                synced_vars,
                async_functions,
                ctx.get_variable_types(),
            );
            write_crdt_fold(
                &mut output,
                &indent_str,
                "merge",
                &tgt_str,
                &src_str,
                source,
                target,
            );
        }

        // The IDEMPOTENT sibling of `MergeCrdt` — the lattice join. It differs from `Merge` on bare-`Int`
        // fields (per-field MAX, not ADD), so it calls the struct's inherent `join` (which folds EVERY field
        // ≡ the VM's `crdt_join_field`), NOT `merge` (whose Int fields ADD).
        Stmt::JoinCrdt { source, target } => {
            let src_str = codegen_expr_with_async(
                source,
                interner,
                synced_vars,
                async_functions,
                ctx.get_variable_types(),
            );
            let tgt_str = codegen_expr_with_async(
                target,
                interner,
                synced_vars,
                async_functions,
                ctx.get_variable_types(),
            );
            write_crdt_fold(
                &mut output,
                &indent_str,
                "join",
                &tgt_str,
                &src_str,
                source,
                target,
            );
        }

        // Phase 49: Increment GCounter
        // Phase 52: If object is synced, wrap in .mutate() for auto-publish
        Stmt::IncreaseCrdt {
            object,
            field,
            amount,
        } => {
            let field_name = escape_rust_ident(interner.resolve(*field));
            let amount_str = codegen_expr_with_async(
                amount,
                interner,
                synced_vars,
                async_functions,
                ctx.get_variable_types(),
            );

            // Check if the root object is synced
            let root_sym = get_root_identifier(object);
            if let Some(sym) = root_sym {
                if synced_vars.contains(&sym) {
                    // Synced: use .mutate() for auto-publish
                    let obj_name = interner.resolve(sym);
                    writeln!(
                        output,
                        "{}{}.mutate(|inner| inner.{}.increment({} as u64)).await;",
                        indent_str, obj_name, field_name, amount_str
                    )
                    .unwrap();
                    return output;
                }
            }

            // Not synced: direct access
            let obj_str = codegen_expr_with_async(
                object,
                interner,
                synced_vars,
                async_functions,
                ctx.get_variable_types(),
            );
            writeln!(
                output,
                "{}{}.{}.increment({} as u64);",
                indent_str, obj_str, field_name, amount_str
            )
            .unwrap();
        }

        // Phase 49b: Decrement PNCounter
        Stmt::DecreaseCrdt {
            object,
            field,
            amount,
        } => {
            let field_name = escape_rust_ident(interner.resolve(*field));
            let amount_str = codegen_expr_with_async(
                amount,
                interner,
                synced_vars,
                async_functions,
                ctx.get_variable_types(),
            );

            // Check if the root object is synced
            let root_sym = get_root_identifier(object);
            if let Some(sym) = root_sym {
                if synced_vars.contains(&sym) {
                    // Synced: use .mutate() for auto-publish
                    let obj_name = interner.resolve(sym);
                    writeln!(
                        output,
                        "{}{}.mutate(|inner| inner.{}.decrement({} as u64)).await;",
                        indent_str, obj_name, field_name, amount_str
                    )
                    .unwrap();
                    return output;
                }
            }

            // Not synced: direct access
            let obj_str = codegen_expr_with_async(
                object,
                interner,
                synced_vars,
                async_functions,
                ctx.get_variable_types(),
            );
            writeln!(
                output,
                "{}{}.{}.decrement({} as u64);",
                indent_str, obj_str, field_name, amount_str
            )
            .unwrap();
        }

        // Phase 49b: Append to SharedSequence (RGA)
        Stmt::AppendToSequence { sequence, value } => {
            let seq_str = codegen_expr_with_async(
                sequence,
                interner,
                synced_vars,
                async_functions,
                ctx.get_variable_types(),
            );
            let val_str = codegen_expr_boxed_with_types_oracle(
                value,
                interner,
                synced_vars,
                boxed_fields,
                registry,
                async_functions,
                ctx.get_string_vars(),
                ctx.get_variable_types(),
                ctx.get_fast_div(),
                ctx.oracle(),
            );
            writeln!(output, "{}{}.append({});", indent_str, seq_str, val_str).unwrap();
        }

        // Phase 49b: Resolve MVRegister conflicts
        Stmt::ResolveConflict {
            object,
            field,
            value,
        } => {
            let field_name = escape_rust_ident(interner.resolve(*field));
            let val_str = codegen_expr_boxed_with_types_oracle(
                value,
                interner,
                synced_vars,
                boxed_fields,
                registry,
                async_functions,
                ctx.get_string_vars(),
                ctx.get_variable_types(),
                ctx.get_fast_div(),
                ctx.oracle(),
            );
            let obj_str = codegen_expr_with_async(
                object,
                interner,
                synced_vars,
                async_functions,
                ctx.get_variable_types(),
            );
            writeln!(
                output,
                "{}{}.{}.resolve({});",
                indent_str, obj_str, field_name, val_str
            )
            .unwrap();
        }

        // Escape hatch: emit raw foreign code wrapped in braces for scope isolation
        Stmt::Escape { code, .. } => {
            let raw_code = interner.resolve(*code);
            // The block's own Rust writes to stdout directly, so everything shown before it goes out first.
            writeln!(
                output,
                "{}logicaffeine_system::io::flush_stdout();",
                indent_str
            )
            .unwrap();
            write!(output, "{}{{\n", indent_str).unwrap();
            for line in raw_code.lines() {
                write!(output, "{}    {}\n", indent_str, line).unwrap();
            }
            write!(output, "{}}}\n", indent_str).unwrap();
        }

        // Dependencies are metadata; no Rust code emitted.
        Stmt::Require { .. } => {}

        // Theorems and definitions are proof-layer declarations verified at
        // compile-time; they generate no runtime code (handled separately by the
        // theorem pipeline at the meta-level).
        Stmt::Theorem(_) | Stmt::Definition(_) | Stmt::Axiom(_) | Stmt::Theory(_) => {}

        // ★ A FAILURE-SITE MARKER (Wave 3). AOT-Rust does not yet track "current site" the way
        // the tree-walker does — accepted and ignored here, explicitly, rather than by a wildcard,
        // so the day this backend DOES need to emit something for it, a new variant elsewhere
        // cannot silently land in this same arm unnoticed. Emits nothing: a marker this tier
        // does not read costs it zero runtime code, which is the whole point of a marker over a
        // span field every `Stmt` variant would otherwise carry.
        Stmt::Site(_) => {}
    }

    output
}

/// OPT-5: Sentinel → break transformation.
///
/// Detects `If cond then: ...; Set counter to limit.` inside a `While counter < limit:` loop
/// and replaces the sentinel set with `break;`. This gives LLVM a clean early-exit edge
/// instead of an opaque counter assignment that forces one more condition check.
///
/// Returns the generated code if the pattern matches, or None.
fn try_emit_sentinel_break<'a>(
    stmt: &Stmt<'a>,
    counter_sym: Symbol,
    limit_expr: &Expr<'a>,
    interner: &Interner,
    indent: usize,
    mutable_vars: &HashSet<Symbol>,
    ctx: &mut RefinementContext<'a>,
    lww_fields: &HashSet<(String, String)>,
    mv_fields: &HashSet<(String, String)>,
    synced_vars: &mut HashSet<Symbol>,
    var_caps: &HashMap<Symbol, VariableCapabilities>,
    async_functions: &HashSet<Symbol>,
    pipe_vars: &HashSet<Symbol>,
    boxed_fields: &HashSet<(String, String, String)>,
    registry: &TypeRegistry,
    type_env: &crate::analysis::types::TypeEnv,
) -> Option<String> {
    // Match: If cond then: ...; Set counter to limit. (no else block)
    let (if_cond, then_block, else_block) = match stmt {
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => (cond, then_block, else_block),
        _ => return None,
    };

    if else_block.is_some() {
        return None;
    }

    if then_block.is_empty() {
        return None;
    }

    // Last statement in then_block must be `Set counter to limit`
    let last = &then_block[then_block.len() - 1];
    match last {
        Stmt::Set { target, value, .. } => {
            if *target != counter_sym || !exprs_equal(value, limit_expr) {
                return None;
            }
        }
        _ => return None,
    }

    // Pattern matched! Emit if block with break instead of sentinel set.
    let indent_str = "    ".repeat(indent);
    let var_types = ctx.get_variable_types();
    let cond_str =
        codegen_expr_with_async(if_cond, interner, synced_vars, async_functions, var_types);
    let mut output = String::new();
    writeln!(output, "{}if {} {{", indent_str, cond_str).unwrap();

    // Emit all then_block statements except the last (the sentinel set)
    for s in &then_block[..then_block.len() - 1] {
        output.push_str(&codegen_stmt(
            s,
            interner,
            indent + 1,
            mutable_vars,
            ctx,
            lww_fields,
            mv_fields,
            synced_vars,
            var_caps,
            async_functions,
            pipe_vars,
            boxed_fields,
            registry,
            type_env,
        ));
    }

    // Emit break instead of the sentinel set
    writeln!(output, "{}    break;", indent_str).unwrap();
    writeln!(output, "{}}}", indent_str).unwrap();

    Some(output)
}

/// `Merge <source> into <target>` / `Converge …`: fold the source's state into the target. The
/// source is read as it stood BEFORE the fold — when both name the same variable (`Merge a into a`)
/// it is snapshotted first, exactly as the interpreter reads it (and a borrow of `a` while `a` is
/// mutated is not Rust).
fn write_crdt_fold(
    output: &mut String,
    indent: &str,
    method: &str,
    tgt: &str,
    src: &str,
    source: &Expr,
    target: &Expr,
) {
    let shares_root = matches!(
        (get_root_identifier(source), get_root_identifier(target)),
        (Some(a), Some(b)) if a == b
    );
    if shares_root {
        writeln!(
            output,
            "{indent}{{ let __fold_src = ({src}).clone(); {tgt}.{method}(&__fold_src); }}"
        )
        .unwrap();
    } else {
        writeln!(output, "{indent}{tgt}.{method}(&{src});").unwrap();
    }
}

/// Phase 52: Extract the root identifier from an expression.
/// For `x.field.subfield`, returns `x`.
pub(crate) fn get_root_identifier(expr: &Expr) -> Option<Symbol> {
    match expr {
        Expr::Identifier(sym) => Some(*sym),
        Expr::FieldAccess { object, .. } => get_root_identifier(object),
        _ => None,
    }
}

/// Extract all symbols from `Expr::Length { collection: Expr::Identifier(sym) }` nodes
/// in an expression tree. Used for loop bounds hoisting.
pub(crate) fn extract_length_expr_syms(expr: &Expr) -> Vec<Symbol> {
    let mut out = Vec::new();
    collect_length_syms_from_expr(expr, &mut out);
    out
}

fn collect_length_syms_from_expr(expr: &Expr, out: &mut Vec<Symbol>) {
    match expr {
        Expr::Length { collection } => {
            if let Expr::Identifier(sym) = collection {
                out.push(*sym);
            }
        }
        Expr::BinaryOp { left, right, .. } => {
            collect_length_syms_from_expr(left, out);
            collect_length_syms_from_expr(right, out);
        }
        Expr::Not { operand } => collect_length_syms_from_expr(operand, out),
        Expr::Index { collection, index } => {
            collect_length_syms_from_expr(collection, out);
            collect_length_syms_from_expr(index, out);
        }
        Expr::Call { args, .. } => {
            for arg in args.iter() {
                collect_length_syms_from_expr(arg, out);
            }
        }
        _ => {}
    }
}

/// Collect all `Expr::Length { Identifier(sym) }` symbols from a statement list (recursively).
pub(crate) fn collect_length_syms_from_stmts(stmts: &[Stmt], out: &mut Vec<Symbol>) {
    for stmt in stmts {
        collect_length_syms_from_stmt(stmt, out);
    }
}

fn collect_length_syms_from_stmt(stmt: &Stmt, out: &mut Vec<Symbol>) {
    match stmt {
        Stmt::Let { value, .. } => collect_length_syms_from_expr(value, out),
        Stmt::Set { value, .. } => collect_length_syms_from_expr(value, out),
        Stmt::Show { object, .. } => collect_length_syms_from_expr(object, out),
        Stmt::Push { value, collection } => {
            collect_length_syms_from_expr(value, out);
            collect_length_syms_from_expr(collection, out);
        }
        Stmt::SetIndex {
            collection,
            index,
            value,
        } => {
            collect_length_syms_from_expr(collection, out);
            collect_length_syms_from_expr(index, out);
            collect_length_syms_from_expr(value, out);
        }
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => {
            collect_length_syms_from_expr(cond, out);
            collect_length_syms_from_stmts(then_block, out);
            if let Some(else_stmts) = else_block {
                collect_length_syms_from_stmts(else_stmts, out);
            }
        }
        Stmt::While { cond, body, .. } => {
            collect_length_syms_from_expr(cond, out);
            collect_length_syms_from_stmts(body, out);
        }
        Stmt::Repeat { body, .. } => {
            collect_length_syms_from_stmts(body, out);
        }
        Stmt::Return { value } => {
            if let Some(v) = value {
                collect_length_syms_from_expr(v, out);
            }
        }
        Stmt::Call { args, .. } => {
            for arg in args.iter() {
                collect_length_syms_from_expr(arg, out);
            }
        }
        _ => {}
    }
}

/// Check if a type string represents a Copy type (no .clone() needed).
/// Delegates to `LogosType::is_copy()` — single source of truth.
pub(crate) fn is_copy_type(ty: &str) -> bool {
    crate::analysis::types::LogosType::from_rust_type_str(ty).is_copy()
}

/// Wrap an `If`/`While` condition in `logos_truthy` unless it's statically
/// Bool — `If xs:` on a Seq tests emptiness, `If 0.0:` is false, exactly the
/// interpreter's truthiness. Bool conditions emit untouched (hot paths pay
/// nothing); an imprecisely-inferred Bool still works (`Truthy` covers bool).
pub(super) fn truthy_cond_wrap(
    cond: &Expr,
    cond_str: String,
    interner: &Interner,
    variable_types: &HashMap<Symbol, String>,
) -> String {
    if super::tower::is_tower_expr(cond, variable_types, interner) {
        return format!("logos_truthy(&({}))", cond_str);
    }
    if matches!(
        super::types::infer_logos_type(cond, interner, variable_types),
        crate::analysis::types::LogosType::Bool
    ) {
        cond_str
    } else {
        format!("logos_truthy(&({}))", cond_str)
    }
}

/// Check if a Vec<T> type has a Copy element type.
/// Delegates to `LogosType::element_type().is_copy()` — single source of truth.
pub(crate) fn has_copy_element_type(vec_type: &str) -> bool {
    crate::analysis::types::LogosType::from_rust_type_str(vec_type)
        .element_type()
        .map_or(false, |e| e.is_copy())
}

/// Check if a HashMap<K, V> type has a Copy value type.
/// Delegates to `LogosType::value_type().is_copy()` — single source of truth.
pub(crate) fn has_copy_value_type(map_type: &str) -> bool {
    crate::analysis::types::LogosType::from_rust_type_str(map_type)
        .value_type()
        .map_or(false, |v| v.is_copy())
}

/// Detect commutative reduction patterns in Repeat loops and emit par_iter() code.
///
/// Pattern: `Repeat for x in coll: Set acc = acc + expr(x)`
/// Emits:
/// ```text
/// if coll.len() >= 10000 {
///     acc += coll.par_iter().copied().map(|x| expr).sum::<i64>();
/// } else {
///     for x in coll.iter().copied() { acc = acc + expr; }
/// }
/// ```
fn try_emit_parallel_reduction<'a>(
    var_sym: Symbol,
    pattern_str: &str,
    iter_str: &str,
    iterable: &'a Expr<'a>,
    body: &'a [Stmt<'a>],
    interner: &Interner,
    indent: usize,
    ctx: &RefinementContext<'a>,
    synced_vars: &HashSet<Symbol>,
    async_functions: &HashSet<Symbol>,
) -> Option<String> {
    // Body must be exactly 1 statement: Set acc = acc OP expr
    if body.len() != 1 {
        return None;
    }

    let (acc_sym, op, increment_expr) = match &body[0] {
        Stmt::Set { target, value } => {
            if let Expr::BinaryOp { op, left, right } = value {
                match op {
                    BinaryOpKind::Add => {
                        // acc = acc + expr  OR  acc = expr + acc
                        if let Expr::Identifier(lhs) = left {
                            if *lhs == *target {
                                Some((*target, BinaryOpKind::Add, *right))
                            } else {
                                None
                            }
                        } else if let Expr::Identifier(rhs) = right {
                            if *rhs == *target {
                                Some((*target, BinaryOpKind::Add, *left))
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    }
                    _ => None,
                }
            } else {
                None
            }
        }
        _ => None,
    }?;

    // The increment expression must only involve the loop variable and literals
    if !expr_is_pure_of(increment_expr, var_sym) {
        return None;
    }

    // The collection must be a typed Vec (for .par_iter().copied())
    let is_vec_i64 = if let Expr::Identifier(coll_sym) = iterable {
        ctx.get_variable_types()
            .get(coll_sym)
            .map(|t| {
                let t = t.split("|__hl:").next().unwrap_or(t.as_str());
                (t.starts_with("LogosSeq<") || t.starts_with("Vec<") || t.starts_with("&["))
                    && has_copy_element_type(t)
            })
            .unwrap_or(false)
    } else {
        false
    };

    if !is_vec_i64 {
        return None;
    }

    let indent_str = "    ".repeat(indent);
    let acc_name = interner.resolve(acc_sym);

    // Parallel reduction reorders the additions, so it is sound ONLY for an
    // associative accumulator. Integer addition is associative (even under i64
    // wrapping), so the parallel sum equals the sequential one bit-for-bit.
    // IEEE-754 float addition is NOT associative — a parallel `f64` sum diverges
    // from the sequential sum every other tier computes (tree-walker, VM, AOT-C),
    // breaking the cross-tier byte-identity that is the whole product. So a float
    // accumulator declines the optimization and stays a sequential loop. The
    // `.sum::<T>()` type must still match the accumulator (not a hardcoded i64).
    let acc_ty = ctx
        .get_variable_types()
        .get(&acc_sym)
        .map(|s| s.split("|__hl:").next().unwrap_or(s.as_str()))
        .unwrap_or("i64");
    let sum_ty = match acc_ty {
        "i64" | "u64" | "i32" | "u32" | "usize" | "isize" => acc_ty,
        _ => return None,
    };

    // Generate the increment expression as Rust code
    let incr_code = codegen_expr_with_async(
        increment_expr,
        interner,
        synced_vars,
        async_functions,
        ctx.get_variable_types(),
    );

    // Determine if we need a .map() or can use .sum() directly
    let needs_map = !matches!(increment_expr, Expr::Identifier(s) if *s == var_sym);

    let mut out = String::new();

    // Emit parallel reduction with runtime threshold
    // LogosSeq needs .borrow() to access the inner Vec for .par_iter()/.iter()
    // &[T] and Vec<T> can be used directly without .borrow()
    let (borrow_expr, len_expr) = if let Expr::Identifier(coll_sym) = iterable {
        let ty = ctx
            .get_variable_types()
            .get(coll_sym)
            .map(|s| s.split("|__hl:").next().unwrap_or(s.as_str()));
        if matches!(ty, Some(t) if t.starts_with("LogosSeq")) {
            (
                format!("{}.borrow()", iter_str),
                format!("{}.len()", iter_str),
            )
        } else {
            (iter_str.to_string(), format!("{}.len()", iter_str))
        }
    } else {
        (
            format!("{}.borrow()", iter_str),
            format!("{}.len()", iter_str),
        )
    };
    writeln!(out, "{}if {} >= 10000 {{", indent_str, len_expr).unwrap();
    writeln!(out, "{}    use rayon::prelude::*;", indent_str).unwrap();
    writeln!(out, "{}    let __par_ref = {};", indent_str, borrow_expr).unwrap();
    if needs_map {
        writeln!(
            out,
            "{}    {} += __par_ref.par_iter().copied().map(|{}| {}).sum::<{}>();",
            indent_str, acc_name, pattern_str, incr_code, sum_ty
        )
        .unwrap();
    } else {
        writeln!(
            out,
            "{}    {} += __par_ref.par_iter().copied().sum::<{}>();",
            indent_str, acc_name, sum_ty
        )
        .unwrap();
    }
    writeln!(out, "{}}} else {{", indent_str).unwrap();
    writeln!(
        out,
        "{}    for {} in {}.to_vec() {{",
        indent_str, pattern_str, iter_str
    )
    .unwrap();

    match op {
        BinaryOpKind::Add => {
            writeln!(out, "{}        {} += {};", indent_str, acc_name, incr_code).unwrap();
        }
        _ => {
            writeln!(
                out,
                "{}        {} = {} {:?} {};",
                indent_str, acc_name, acc_name, op, incr_code
            )
            .unwrap();
        }
    }
    writeln!(out, "{}    }}", indent_str).unwrap();
    writeln!(out, "{}}}", indent_str).unwrap();

    Some(out)
}

/// Check if an expression only involves the given symbol and literals (no side effects).
fn expr_is_pure_of(expr: &Expr, allowed_sym: Symbol) -> bool {
    match expr {
        Expr::Identifier(s) => *s == allowed_sym,
        Expr::Literal(_) => true,
        Expr::BinaryOp { left, right, .. } => {
            expr_is_pure_of(left, allowed_sym) && expr_is_pure_of(right, allowed_sym)
        }
        Expr::Not { operand } => expr_is_pure_of(operand, allowed_sym),
        _ => false,
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen/stmt/tests.rs"]
mod tests;
