use std::collections::{HashMap, HashSet};
use std::fmt::Write;

use super::context::{RefinementContext, VariableCapabilities};
use super::detection::{symbol_appears_in_expr, symbol_appears_in_stmts, value_read_after};
use super::i64_map::map_rust_type;
use super::types::codegen_type_expr;
use crate::analysis::registry::TypeRegistry;
use crate::analysis::types::RustNames;
use crate::ast::stmt::{BinaryOpKind, Expr, Literal, Pattern, Stmt, TypeExpr};
use crate::intern::{Interner, Symbol};

/// Collection type information for with_capacity pattern detection.
enum CollInfo {
    Vec(String),
    Map(String, String),
}

/// A1: the default-element literal for a `Vec<T>` (the value `resize` pads with;
/// every slot is overwritten by the fill loop, so it only has to type-check).
fn default_literal_for_vec_elem(vec_type: &str) -> &'static str {
    let elem = vec_type
        .strip_prefix("Vec<")
        .and_then(|t| t.strip_suffix('>'))
        .unwrap_or(vec_type);
    match elem {
        "i64" | "i32" | "i16" | "i8" | "u64" | "u32" | "u16" | "u8" | "usize" | "isize" => "0",
        "f64" | "f32" => "0.0",
        "bool" => "false",
        "char" => "'\\0'",
        _ => "Default::default()",
    }
}

/// A1: count the structural mutations (`Push`/`Pop`/`Add`/`Remove`/`SetIndex`)
/// of `buf` anywhere in `stmts` (including nested blocks). A fill conversion is
/// only safe when this is exactly 1 — the single counted top-level push.
fn count_buffer_structural_mutations(stmts: &[Stmt], buf: Symbol) -> usize {
    fn is_buf(e: &Expr, buf: Symbol) -> bool {
        matches!(e, Expr::Identifier(s) if *s == buf)
    }
    let mut n = 0;
    for s in stmts {
        match s {
            Stmt::Push { collection, .. }
            | Stmt::Pop { collection, .. }
            | Stmt::Add { collection, .. }
            | Stmt::Remove { collection, .. }
            | Stmt::SetIndex { collection, .. } => {
                if is_buf(collection, buf) {
                    n += 1;
                }
            }
            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
                n += count_buffer_structural_mutations(then_block, buf);
                if let Some(eb) = else_block {
                    n += count_buffer_structural_mutations(eb, buf);
                }
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } | Stmt::Zone { body, .. } => {
                n += count_buffer_structural_mutations(body, buf);
            }
            _ => {}
        }
    }
    n
}

/// A1: count `Push v to buf` at the TOP LEVEL of `stmts` only (not nested).
fn top_level_push_count(stmts: &[Stmt], buf: Symbol) -> usize {
    stmts
        .iter()
        .filter(|s| matches!(s, Stmt::Push { collection: Expr::Identifier(b), .. } if *b == buf))
        .count()
}

/// A1: count `Push v to buf` anywhere in `stmts`, including nested blocks/loops.
fn total_push_count(stmts: &[Stmt], buf: Symbol) -> usize {
    let mut n = 0;
    for s in stmts {
        match s {
            Stmt::Push {
                collection: Expr::Identifier(b),
                ..
            } if *b == buf => n += 1,
            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
                n += total_push_count(then_block, buf);
                if let Some(eb) = else_block {
                    n += total_push_count(eb, buf);
                }
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } | Stmt::Zone { body, .. } => {
                n += total_push_count(body, buf);
            }
            _ => {}
        }
    }
    n
}

/// A1: a buffer-reuse buffer is fill-convertible only when its sole structural
/// mutation is a single push located in a NESTED loop (knapsack's inner w-loop
/// builds `curr` to length `cols`, then the outer loop swaps it). A push at the
/// TOP LEVEL of the swap loop means the buffer is push-once-then-swapped each
/// iteration (a fresh length-1 Seq rebound into a handle) — it must keep its
/// `clear()` + `push()` and must never be sized to the trip count.
fn buffer_filled_by_nested_loop(body: &[Stmt], buf: Symbol) -> bool {
    top_level_push_count(body, buf) == 0
        && total_push_count(body, buf) == 1
        && count_buffer_structural_mutations(body, buf) == 1
}

/// A1: if `body` refills a registered buffer-reuse buffer via exactly one
/// TOP-LEVEL `Push v to buf` and no other structural mutation of `buf`, return
/// `buf` — the loop can write it by index instead of pushing.
fn detect_fill_push_target(body: &[Stmt], ctx: &RefinementContext) -> Option<Symbol> {
    let mut target: Option<Symbol> = None;
    for stmt in body {
        if let Stmt::Push {
            collection: Expr::Identifier(buf),
            ..
        } = stmt
        {
            if ctx.is_buffer_reuse_fill(*buf) {
                if target.is_some() {
                    return None; // more than one fill push at top level
                }
                target = Some(*buf);
            }
        }
    }
    let buf = target?;
    if count_buffer_structural_mutations(body, buf) == 1 {
        Some(buf)
    } else {
        None
    }
}

/// Peephole optimization: detect `Let counter = start. While counter <= limit: body; Set counter to counter + 1`
/// and emit `for counter in start..=limit { body } let mut counter = limit + 1;` instead.
/// The for-range form enables LLVM trip count analysis, unrolling, and vectorization.
/// Returns (generated_code, number_of_extra_statements_consumed) or None if pattern doesn't match.
pub(crate) fn try_emit_for_range_pattern<'a>(
    stmts: &[&Stmt<'a>],
    idx: usize,
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
) -> Option<(String, usize)> {
    if idx + 1 >= stmts.len() {
        return None;
    }

    // Recognize the counted loop — an init (`Let`/`Set` counter = simple start),
    // a `While counter </<= limit`, and a trailing `counter + 1` increment — plus
    // its validity (counter not re-modified mid-body, limit loop-invariant) via
    // the SHARED recognizer in `loop_shape`. That is the single source of truth
    // for this shape; the AOT loop-unroller consumes the same function. The
    // codegen-specific extras below are recomputed from the same two statements.
    let (cl, _consumed) = crate::loop_shape::extract_counted_while(stmts, idx)?;
    let counter_sym = cl.var;
    let counter_start_expr = cl.start;
    let limit_expr = cl.limit;
    let is_exclusive = !cl.inclusive;
    let body_without_increment = cl.body_without_increment;
    // New binding (`Let`) vs reused counter (`Set`) — drives whether emission
    // re-declares the loop variable.
    let is_new_binding = matches!(stmts[idx], Stmt::Let { .. });
    // Literal start enables the zero-basing / tiling / fill specializations.
    let counter_start_literal = match counter_start_expr {
        Expr::Literal(Literal::Number(n)) => Some(*n),
        _ => None,
    };
    // The `While` arena address is the oracle key for borrow-hoist alias snapshots.
    let loop_stmt: &Stmt = stmts[idx + 1];

    // A nest the dependence test proved may run in tile order (`optimize::version` records it on the loop's
    // condition) emits as a 6-level tiled nest with step_by for L1 locality; any other runs as written.
    let tile_legal = matches!(loop_stmt, Stmt::While { cond, .. }
        if crate::optimize::facts::holds_expr(crate::optimize::facts::Fact::TileLegal, *cond));
    if counter_start_literal == Some(0) && is_exclusive && tile_legal {
        if let Some(tiled) = try_emit_tiled_inner(
            counter_sym,
            limit_expr,
            body_without_increment,
            stmts,
            idx,
            is_new_binding,
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
        ) {
            return Some(tiled);
        }
    }

    // Detect buffer reuse: inner buffer allocated FRESH each iteration (via
    // `Let mutable inner = new Seq`), filled, then transferred to an outer var.
    // This is the only sound buffer-swap optimization under LOGOS reference
    // semantics: the per-iteration `new Seq` guarantees `inner` is a distinct
    // buffer from `outer` during the fill, so reusing the old buffer (swap +
    // clear) is observably equivalent to fresh allocation. The unsound
    // `detect_double_buffer_swap` — which turned a bare `Set X to Y` into a swap
    // even when X aliases Y after the assignment (e.g. knapsack's `prev[w-wi]`
    // cross-index read) — was removed: `Set X to Y` always aliases.
    let buffer_reuse = detect_buffer_reuse_in_body(body_without_increment, interner, ctx);

    // Pattern matched! Emit for-range loop.
    let indent_str = "    ".repeat(indent);
    let names = RustNames::new(interner);
    let counter_name = names.ident(counter_sym);
    // A bound that reads a PROMOTED (`LogosInt`) binding narrows at the range — a Rust `Range`
    // needs i64 ends, and the i64-sink ruling is a loud canonical error (a loop can't run 2^63
    // times anyway; the interpreter's behavior past that bound is unreachable in either form). A
    // bare binding narrows itself; an expression over one is the typed emitter's exact arithmetic,
    // which narrows its result.
    let bound = |e: &Expr| -> String {
        let mut read = HashSet::new();
        super::detection::collect_expr_identifiers(e, &mut read);
        let promoted = read.iter().any(|sym| {
            ctx.get_variable_types()
                .get(sym)
                .is_some_and(|t| t.contains("__bigint"))
        });
        match e {
            _ if !promoted => codegen_expr_simple(e, interner),
            Expr::Identifier(_) => {
                format!("{}.expect_i64(\"Int\")", codegen_expr_simple(e, interner))
            }
            _ => super::expr::codegen_expr_boxed_with_types(
                e,
                interner,
                synced_vars,
                boxed_fields,
                registry,
                async_functions,
                ctx.get_string_vars(),
                ctx.get_variable_types(),
                ctx.get_fast_div(),
            ),
        }
    };
    let limit_str = bound(limit_expr);
    // The loop reads its start once, before the first iteration. A start whose variables the body changes
    // is taken into a local of its own, and the value after the loop reads it from there.
    let start_read: Vec<Symbol> = {
        let mut read = Vec::new();
        collect_expr_symbols(counter_start_expr, &mut read);
        read
    };
    let start_moves = start_read.iter().any(|sym| {
        body_modifies_var(body_without_increment, *sym)
            || body_mutates_collection(body_without_increment, *sym)
    });
    let (start_str, start_snapshot) = if start_moves {
        let local = format!("__{}_from", counter_name.trim_start_matches("r#"));
        let line = format!(
            "{}let {} = {};\n",
            indent_str,
            local,
            bound(counter_start_expr)
        );
        (local, Some(line))
    } else {
        (bound(counter_start_expr), None)
    };

    // OPT-8: Zero-based counter normalization (general, any start).
    // When the counter is ONLY used for direct array indexing in the body, shift
    // the whole loop 0-based so the generated Rust indexes `arr[i]` rather than
    // `arr[i - 1]`. This is a pure variable substitution: the counter is rebased
    // `i' = i - 1` (range and every index use shifted by the same -1, comparison
    // uses compensated `+1` in codegen), so behavior — including any out-of-range
    // panic — is identical for ANY start value, not just the literal `1`. A
    // literal start `<= 0` is excluded (not a 1-based index); a runtime start
    // (`None`) is allowed (e.g. a recursive partition's `lo`). The counter is
    // registered as "__zero_based_i64" so index codegen skips the -1.
    let use_zero_based = counter_start_literal.map_or(true, |s| s >= 1)
        && !is_exclusive
        && counter_has_index_uses(body_without_increment, counter_sym)
        && counter_only_used_for_indexing(body_without_increment, counter_sym)
        && counter_indexes_only_vec_types(body_without_increment, counter_sym, ctx);

    // Always use exclusive ranges (Range) instead of inclusive (RangeInclusive).
    // RangeInclusive has a known performance overhead in Rust due to internal
    // bookkeeping for edge cases, which compounds in hot inner loops.
    // Convert `i <= limit` to `i < (limit + 1)`.
    //
    // Two type-silent bounds leave the counter's width to rustc's `i32` default, so the limit states `i64`.
    let typed = super::types::type_silent_int(counter_start_expr).is_some()
        && super::types::type_silent_int(limit_expr).is_some();
    let typed_limit = |n: i64| {
        if typed {
            format!("{n}_i64")
        } else {
            format!("{n}")
        }
    };
    let limit_str = match typed
        .then(|| super::types::type_silent_int_i64(limit_expr))
        .flatten()
    {
        Some(t) if !matches!(limit_expr, Expr::Literal(Literal::Number(_))) => t,
        _ => limit_str,
    };
    let range_str = if use_zero_based {
        // 0-based: rebase inclusive `start..=limit` (1-based, indexing arr[i-1])
        // to `(start-1)..limit` indexing arr[i]. For start == 1 this is `0..limit`.
        let lo = match counter_start_expr {
            Expr::Literal(Literal::Number(n)) => format!("{}", n - 1),
            _ => format!("({} - 1)", start_str),
        };
        if let Expr::Literal(Literal::Number(n)) = limit_expr {
            format!("{}..{}", lo, typed_limit(*n))
        } else {
            format!("{}..{}", lo, limit_str)
        }
    } else if is_exclusive {
        match limit_expr {
            Expr::Literal(Literal::Number(n)) if typed => {
                format!("{}..{}", start_str, typed_limit(*n))
            }
            _ => format!("{}..{}", start_str, limit_str),
        }
    } else {
        // For literal limits, compute limit+1 at compile time
        if let Expr::Literal(Literal::Number(n)) = limit_expr {
            format!("{}..{}", start_str, typed_limit(n + 1))
        } else {
            format!("{}..({} + 1)", start_str, limit_str)
        }
    };

    let mut output = start_snapshot.unwrap_or_default();

    // OPT-4: bounds hints for arrays indexed by the counter, so LLVM can
    // elide bounds checks and vectorize. Strength, qualification, and the
    // soundness contract live in plan_bounds_hints. The length snapshot is
    // hoisted here; the assert pair goes at the top of the loop body so it
    // never executes when the loop doesn't run.
    let bounds_hints = plan_bounds_hints(
        body_without_increment,
        counter_sym,
        is_exclusive,
        use_zero_based,
        limit_expr,
        &limit_str,
        ctx.get_variable_types(),
    );
    emit_bounds_hint_preheader(&bounds_hints, interner, &indent_str, &mut output);

    // Row-major affine-offset accesses (`c[i*n+j]`, `i*n` fixed over this loop) the loop's versioning guard
    // proved in range get a per-iteration hint in the body. The offset/index are loop-invariant arithmetic, so
    // empty sync/async sets suffice for codegen.
    let affine_empty: HashSet<Symbol> = HashSet::new();
    let affine_guards = super::stmt::plan_affine_offset_guards(
        body_without_increment,
        counter_sym,
        is_exclusive,
        ctx.get_variable_types(),
    );

    // Hoist inner buffer if buffer reuse detected (stays outside any borrow
    // scope; the swap rebinds it across iterations).
    // O2 de-Rc Phase 2: both partners of a buffer-reuse swap de-Rc'd → the
    // hoisted inner buffer is a plain `Vec<T>`, cleared directly, swapped by
    // value. Both must agree (the analysis drops the pair otherwise).
    let de_rc_reuse = buffer_reuse.as_ref().map_or(false, |r| {
        ctx.is_de_rc(r.inner_sym) && ctx.is_de_rc(r.outer_sym)
    });
    // Loop-split fill: the reused buffer is filled across the prefix/suffix
    // sub-loops of a version-guard `If`, so the single-fill path (one 0-based
    // resize) cannot size it. Resize once to the full column count and drive
    // every sub-loop's push through `fill_loop` (indexed unchecked writes).
    let split_fill = buffer_reuse.as_ref().filter(|_| de_rc_reuse).and_then(|r| {
        detect_split_fill(
            body_without_increment,
            r.inner_sym,
            &r.inner_elem_type,
            interner,
        )
    });
    if let Some(ref reuse) = buffer_reuse {
        let reuse_inner = names.ident(reuse.inner_sym);
        if de_rc_reuse {
            writeln!(
                output,
                "{}let mut {}: Vec<{}> = Vec::new();",
                indent_str, reuse_inner, reuse.inner_elem_type
            )
            .unwrap();
            // A1: a counted push-refill of this reused buffer by a NESTED loop
            // (knapsack's inner w-loop fills `curr` to length `cols`) becomes a
            // SIZED `resize` + INDEXED WRITES, removing the per-iteration `len`
            // mutation that blocks vectorization. Gated on the nested-fill shape
            // so a top-level push-once-then-swap buffer keeps its `clear()` +
            // `push()` (it never accumulates trip-count elements).
            if buffer_filled_by_nested_loop(body_without_increment, reuse.inner_sym) {
                ctx.register_buffer_reuse_fill(reuse.inner_sym);
            }
        } else {
            writeln!(
                output,
                "{}let mut {}: LogosSeq<{}> = LogosSeq::new();",
                indent_str, reuse_inner, reuse.inner_elem_type
            )
            .unwrap();
        }
    }

    // O1: scope-extract per-handle borrows around the loop when the alias
    // oracle proves it sound. Disabled under buffer-reuse (its swap rebinds
    // a handle every iteration, which a long-lived borrow would forbid).
    let hoist_plan = if buffer_reuse.is_none() {
        super::hoist::plan_borrow_hoist(loop_stmt, None, body_without_increment, ctx, interner)
    } else {
        Vec::new()
    };
    super::hoist::emit_hoist_open(&hoist_plan, interner, &indent_str, ctx, &mut output);

    // A1 buffer-fill conversion: a 0-based loop that refills a registered
    // buffer-reuse buffer via one counted push → size it once (`resize`) and
    // write by index in the body, dropping the per-iteration `len` mutation
    // that blocks vectorization of the DP scan. Sound: the buffer ends with the
    // same N elements as the push form, so the swap partner is unchanged.
    let fill_target: Option<Symbol> = if counter_start_literal == Some(0) {
        detect_fill_push_target(body_without_increment, ctx)
    } else {
        None
    };
    if let Some(buf) = fill_target {
        let buf_name = names.ident(buf);
        let count_str = if is_exclusive {
            limit_str.clone()
        } else if let Expr::Literal(Literal::Number(n)) = limit_expr {
            format!("{}", n + 1)
        } else {
            format!("({} + 1)", limit_str)
        };
        let default = ctx
            .get_variable_types()
            .get(&buf)
            .map(|t| default_literal_for_vec_elem(t))
            .unwrap_or("Default::default()");
        writeln!(
            output,
            "{}{}.resize(({}) as usize, {});",
            indent_str, buf_name, count_str, default
        )
        .unwrap();
        ctx.set_fill_loop(buf, counter_name.clone());
    }

    // Scratch-buffer allocation hoisting (counted-loop path; same transform as
    // the `Stmt::While` handler). A loop-local buffer fully overwritten by a
    // copy each iteration, used in place, and never escaping is hoisted to ONE
    // allocation before the loop and reused (`clear()` + `extend_from_slice`).
    let scratch_hoisted: Vec<Symbol> =
        detect_scratch_hoist_in_body(body_without_increment, interner, ctx)
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

    let loop_start = output.len();
    writeln!(
        output,
        "{}for {} in {} {{",
        indent_str, counter_name, range_str
    )
    .unwrap();
    emit_bounds_hint_header(
        &bounds_hints,
        interner,
        &"    ".repeat(indent + 1),
        &mut output,
    );
    super::stmt::emit_affine_offset_header(
        &affine_guards,
        interner,
        &affine_empty,
        &affine_empty,
        ctx.get_variable_types(),
        &"    ".repeat(indent + 1),
        &mut output,
    );

    // Emit body statements (excluding the final counter increment)
    // Apply full peephole suite to enable nested loop optimization.
    ctx.push_scope();

    // Register zero-based counter so index codegen skips the (i - 1) subtraction
    if use_zero_based {
        ctx.register_variable_type(counter_sym, "__zero_based_i64".to_string());
    }
    let body_refs: Vec<&Stmt> = body_without_increment.iter().collect();
    let mut bi = 0;
    while bi < body_refs.len() {
        // Buffer reuse interception: replace allocation with clear, transfer with swap.
        if let Some(ref reuse) = buffer_reuse {
            if bi == reuse.inner_let_idx {
                let reuse_inner = names.ident(reuse.inner_sym);
                if de_rc_reuse {
                    // Loop-split fill: size the buffer once to the full column
                    // count (the suffix loop's end). A `resize` to the same
                    // length each round (after the swap) is a no-op; only the
                    // first round allocates. The sub-loops then overwrite every
                    // slot by index — no `clear()` (which would re-init defaults).
                    if let Some(ref sf) = split_fill {
                        writeln!(
                            output,
                            "{}{}.resize(({}) as usize, {});",
                            "    ".repeat(indent + 1),
                            reuse_inner,
                            sf.cols_str,
                            sf.default
                        )
                        .unwrap();
                    } else if !ctx.is_buffer_reuse_fill(reuse.inner_sym) {
                        // A1: a fill-converted buffer is sized by the inner loop's
                        // `resize` and fully overwritten by index — no `clear()`
                        // (which would force the resize to re-init N defaults every
                        // outer round). Plain reused buffers still clear.
                        writeln!(
                            output,
                            "{}{}.clear();",
                            "    ".repeat(indent + 1),
                            reuse_inner
                        )
                        .unwrap();
                    }
                    ctx.register_variable_type(
                        reuse.inner_sym,
                        format!("Vec<{}>", reuse.inner_elem_type),
                    );
                } else {
                    writeln!(
                        output,
                        "{}{}.borrow_mut().clear();",
                        "    ".repeat(indent + 1),
                        reuse_inner
                    )
                    .unwrap();
                    ctx.register_variable_type(
                        reuse.inner_sym,
                        format!("LogosSeq<{}>", reuse.inner_elem_type),
                    );
                }
                bi += 1;
                continue;
            }
            if bi == reuse.set_idx {
                let reuse_inner = names.ident(reuse.inner_sym);
                let reuse_outer = names.ident(reuse.outer_sym);
                writeln!(
                    output,
                    "{}std::mem::swap(&mut {}, &mut {});",
                    "    ".repeat(indent + 1),
                    reuse_outer,
                    reuse_inner
                )
                .unwrap();
                bi += 1;
                continue;
            }
            // Loop-split version-guard `If`: drive every push to the reused
            // buffer through `fill_loop` so the prefix/suffix/fallback sub-loops
            // emit indexed unchecked writes `curr[w] = v` into the once-sized
            // buffer (the vectorizable DP store).
            if let Some(ref sf) = split_fill {
                if bi == sf.if_idx {
                    ctx.set_fill_loop(reuse.inner_sym, sf.iv_name.clone());
                    output.push_str(&super::codegen_stmt(
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
                    ctx.clear_fill_loop();
                    bi += 1;
                    continue;
                }
            }
        }
        if let Some((code, skip)) =
            try_emit_vec_fill_pattern(&body_refs, bi, interner, indent + 1, ctx)
        {
            output.push_str(&code);
            bi += 1 + skip;
            continue;
        }
        if let Some((code, skip)) = try_emit_for_range_pattern(
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
        if let Some((code, skip)) = try_emit_prefix_reverse(
            &body_refs,
            bi,
            interner,
            indent + 1,
            ctx.get_variable_types(),
        ) {
            output.push_str(&code);
            bi += 1 + skip;
            continue;
        }
        if let Some((code, skip)) = try_emit_swap_pattern(
            &body_refs,
            bi,
            interner,
            indent + 1,
            ctx.get_variable_types(),
            ctx.oracle(),
        ) {
            output.push_str(&code);
            bi += 1 + skip;
            continue;
        }
        if let Some((code, skip)) = try_emit_seq_from_slice_pattern(
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
        if let Some((code, skip)) = try_emit_rotate_left_pattern(
            &body_refs,
            bi,
            interner,
            indent + 1,
            ctx.get_variable_types(),
        ) {
            output.push_str(&code);
            bi += 1 + skip;
            continue;
        }
        output.push_str(&super::codegen_stmt(
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
    if fill_target.is_some() {
        ctx.clear_fill_loop();
    }
    ctx.pop_scope();
    // Stop treating the hoisted scratch buffers as reused once the loop closes.
    for dst in scratch_hoisted {
        ctx.clear_scratch_hoist(dst);
    }
    // Clear the zero-based marker since the counter reverts to 1-based after the loop.
    // variable_types is a flat HashMap (not scoped), so push/pop_scope doesn't clean it.
    if use_zero_based {
        ctx.register_variable_type(counter_sym, "i64".to_string());
    }
    writeln!(output, "{}}}", indent_str).unwrap();
    version_on_bounds_hints(
        &mut output,
        loop_start,
        &bounds_hints,
        body_without_increment,
        ctx.oracle(),
        interner,
        &indent_str,
    );
    // Close the borrow-hoist scope and restore the handles' tracked types.
    super::hoist::emit_hoist_close(&hoist_plan, &indent_str, ctx, &mut output);

    // Emit the post-loop counter value wherever it can be read after the loop.
    let remaining_stmts = &stmts[idx + 2..];
    if value_read_after(counter_sym, stmts, idx + 2) {
        // OPT-7: If the next statement immediately overwrites the counter,
        // skip the max() computation and just declare the variable (if needed).
        let next_stmt_overwrites_counter = remaining_stmts.first().map_or(
            false,
            |s| matches!(s, Stmt::Set { target, .. } if *target == counter_sym),
        );

        if next_stmt_overwrites_counter {
            // For Let-based counters, we still need to declare the variable
            // so it exists in scope for the overwriting Set statement.
            if is_new_binding {
                writeln!(output, "{}let mut {}: i64 = 0;", indent_str, counter_name).unwrap();
            }
        } else {
            // After `while (i <= limit) { ...; i++ }`, i == limit + 1.
            // After `while (i < limit) { ...; i++ }`, i == limit.
            // If the loop never executes (start >= limit), counter stays at start.
            let post_value = if is_exclusive {
                match (counter_start_literal, limit_expr) {
                    (Some(s), Expr::Literal(Literal::Number(n))) => {
                        format!("{}", std::cmp::max(s, *n))
                    }
                    (Some(s), _) => {
                        format!("({}_i64).max({})", s, limit_str)
                    }
                    (None, _) => {
                        format!("({}).max({})", start_str, limit_str)
                    }
                }
            } else {
                match (counter_start_literal, limit_expr) {
                    (Some(s), Expr::Literal(Literal::Number(n))) => {
                        format!("{}", std::cmp::max(s, n + 1))
                    }
                    (Some(s), _) => {
                        format!("({}_i64).max({} + 1)", s, limit_str)
                    }
                    (None, _) => {
                        format!("({}).max({} + 1)", start_str, limit_str)
                    }
                }
            };
            if is_new_binding {
                writeln!(
                    output,
                    "{}let mut {}: i64 = {};",
                    indent_str, counter_name, post_value
                )
                .unwrap();
            } else {
                writeln!(output, "{}{} = {};", indent_str, counter_name, post_value).unwrap();
            }
        }
    }

    Some((output, 1)) // consumed 1 extra statement (the While)
}

/// Collect all identifier symbols referenced in an expression.
/// Used to check whether a loop bound depends on variables modified in the body.
pub(crate) fn collect_expr_symbols(expr: &Expr, out: &mut Vec<Symbol>) {
    match expr {
        Expr::Identifier(sym) => out.push(*sym),
        Expr::Length { collection } => collect_expr_symbols(collection, out),
        Expr::BinaryOp { left, right, .. } => {
            collect_expr_symbols(left, out);
            collect_expr_symbols(right, out);
        }
        _ => {}
    }
}

/// Check if a slice of statements modifies a specific variable (used for for-range validity).
/// Recursively walks into nested If/While/Repeat blocks.
/// True when `sym` is declared by a `Let` anywhere in `stmts` (including nested blocks). A loop-bound
/// length hoist must NOT lift `sym.len()` above such a declaration: `sym` is local to the loop body
/// (e.g. `Let eb be … ` inside a branch), so a hoisted `let sym_len = sym.len()` at the pre-loop
/// header would reference a name not yet in scope (a Rust E0425).
pub(crate) fn body_declares_var(stmts: &[Stmt], sym: Symbol) -> bool {
    for stmt in stmts {
        match stmt {
            Stmt::Let { var, .. } if *var == sym => return true,
            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
                if body_declares_var(then_block, sym) {
                    return true;
                }
                if let Some(else_stmts) = else_block {
                    if body_declares_var(else_stmts, sym) {
                        return true;
                    }
                }
            }
            Stmt::While { body, .. } => {
                if body_declares_var(body, sym) {
                    return true;
                }
            }
            Stmt::Repeat { body, .. } => {
                if body_declares_var(body, sym) {
                    return true;
                }
            }
            Stmt::Zone { body, .. } => {
                if body_declares_var(body, sym) {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

/// True if `stmts` may give the binding `sym` a new value — `Set`, `Swap`, a lend to a `mutable` parameter — nested
/// blocks included: the optimizer's one exhaustive write walk. An element changed in place is not a new value of the
/// binding. Foreign code may reassign anything.
pub(crate) fn body_modifies_var(stmts: &[Stmt], sym: Symbol) -> bool {
    reassigns(&crate::optimize::WriteSet::of_block(stmts), sym)
}

/// [`body_modifies_var`] for one statement.
fn stmt_modifies_var(stmt: &Stmt, sym: Symbol) -> bool {
    reassigns(&crate::optimize::WriteSet::of_stmt(stmt), sym)
}

fn reassigns(writes: &crate::optimize::WriteSet, sym: Symbol) -> bool {
    writes.is_opaque() || writes.assigned().contains(&sym)
}

/// Conservatively resolve `expr` to a compile-time `i64` using the bindings
/// preceding position `upto` in `stmts`. SOUND by construction: a variable
/// resolves only when it has exactly one top-level `Let … be <literal>` before
/// `upto` AND is never `Set` anywhere in the block, so its value is stable at
/// every use. Returns None on anything it cannot prove — callers must treat
/// None as "no match", never as a value.
fn resolve_const_i64(expr: &Expr, stmts: &[&Stmt], upto: usize) -> Option<i64> {
    match expr {
        Expr::Literal(Literal::Number(n)) => Some(*n),
        Expr::Identifier(sym) => {
            if stmts.iter().any(|s| stmt_modifies_var(s, *sym)) {
                return None;
            }
            let mut bound = None;
            let mut count = 0usize;
            for s in &stmts[..upto.min(stmts.len())] {
                if let Stmt::Let { var, value, .. } = s {
                    if *var == *sym {
                        count += 1;
                        bound = match value {
                            Expr::Literal(Literal::Number(n)) => Some(*n),
                            _ => None,
                        };
                    }
                }
            }
            if count == 1 {
                bound
            } else {
                None
            }
        }
        Expr::BinaryOp {
            op: BinaryOpKind::Add,
            left,
            right,
        } => resolve_const_i64(left, stmts, upto)?
            .checked_add(resolve_const_i64(right, stmts, upto)?),
        Expr::BinaryOp {
            op: BinaryOpKind::Subtract,
            left,
            right,
        } => resolve_const_i64(left, stmts, upto)?
            .checked_sub(resolve_const_i64(right, stmts, upto)?),
        Expr::BinaryOp {
            op: BinaryOpKind::Multiply,
            left,
            right,
        } => resolve_const_i64(left, stmts, upto)?
            .checked_mul(resolve_const_i64(right, stmts, upto)?),
        _ => None,
    }
}

/// Whether a loop body may change the collection `coll_sym` — reassign it or change it in place (`Push`, `SetIndex`,
/// `Remove`, …) — nested blocks included (used for iterator optimization).
pub(crate) fn body_mutates_collection(stmts: &[Stmt], coll_sym: Symbol) -> bool {
    let writes = crate::optimize::WriteSet::of_block(stmts);
    reassigns(&writes, coll_sym) || writes.mutated().contains(&coll_sym)
}

/// A loop pattern that binds the name `xs` (shadowing the outer Seq).
pub(crate) fn pattern_binds(p: &Pattern, xs: Symbol) -> bool {
    match p {
        Pattern::Identifier(s) => *s == xs,
        Pattern::Tuple(syms) => syms.iter().any(|s| *s == xs),
    }
}

/// Conservatively COMPLETE "does `xs` appear anywhere in `e`" — any unhandled
/// expression form returns `true` (assume it might mention `xs`).
fn seq_expr_mentions(e: &Expr, xs: Symbol) -> bool {
    match e {
        Expr::Identifier(s) => *s == xs,
        Expr::Literal(_) | Expr::OptionNone | Expr::Escape { .. } => false,
        Expr::BinaryOp { left, right, .. }
        | Expr::Union { left, right }
        | Expr::Intersection { left, right }
        | Expr::Range { start: left, end: right } => {
            seq_expr_mentions(left, xs) || seq_expr_mentions(right, xs)
        }
        Expr::Not { operand } => seq_expr_mentions(operand, xs),
        Expr::Call { args, .. } => args.iter().any(|a| seq_expr_mentions(a, xs)),
        Expr::CallExpr { callee, args } => {
            seq_expr_mentions(callee, xs) || args.iter().any(|a| seq_expr_mentions(a, xs))
        }
        Expr::Index { collection, index } => {
            seq_expr_mentions(collection, xs) || seq_expr_mentions(index, xs)
        }
        Expr::Slice { collection, start, end } => {
            seq_expr_mentions(collection, xs)
                || seq_expr_mentions(start, xs)
                || seq_expr_mentions(end, xs)
        }
        Expr::Copy { expr }
        | Expr::Give { value: expr }
        | Expr::OptionSome { value: expr }
        | Expr::Length { collection: expr } => seq_expr_mentions(expr, xs),
        Expr::FieldAccess { object, .. } => seq_expr_mentions(object, xs),
        Expr::Contains { collection, value } => {
            seq_expr_mentions(collection, xs) || seq_expr_mentions(value, xs)
        }
        Expr::ManifestOf { zone } => seq_expr_mentions(zone, xs),
        Expr::ChunkAt { index, zone } => {
            seq_expr_mentions(index, xs) || seq_expr_mentions(zone, xs)
        }
        Expr::List(items) | Expr::Tuple(items) => items.iter().any(|i| seq_expr_mentions(i, xs)),
        Expr::New { init_fields, .. } => init_fields.iter().any(|(_, e)| seq_expr_mentions(e, xs)),
        Expr::NewVariant { fields, .. } => fields.iter().any(|(_, e)| seq_expr_mentions(e, xs)),
        Expr::WithCapacity { value, capacity } => {
            seq_expr_mentions(value, xs) || seq_expr_mentions(capacity, xs)
        }
        Expr::InterpolatedString(parts) => parts.iter().any(|p| {
            matches!(p, crate::ast::stmt::StringPart::Expr { value, .. } if seq_expr_mentions(value, xs))
        }),
        // Closure/View/Dom or any future variant: conservatively assume it mentions `xs`.
        _ => true,
    }
}

/// The recognizer for the `text_bytes` ZERO-COPY VIEW: every occurrence of `sb` in
/// `stmts` is a VIEW-COMPATIBLE READ — `item _ of sb` (an `Index` whose collection is
/// `sb`), `length of sb` (a `Length` of `sb`), or the iterable of `Repeat for _ in sb`.
///
/// When `sb = text_bytes(s)` is used ONLY through these read forms, the `Vec<i64>`
/// byte-list (8x memory blow-up + `Rc<RefCell>` box + a `.borrow()` and bounds check per
/// byte) can be replaced by a borrowed `s.as_bytes()` view (a `&[u8]`, zero-copy). Any
/// OTHER use — mutation (`Push`/`Set item`), passing `sb` itself to a call, `Give`ing it,
/// a bare read, or reassignment — needs the real `LogosSeq<i64>` (the view is a `&[u8]`,
/// a different type), so this analysis is conservatively COMPLETE: any construct it cannot
/// prove is a view-compatible read forces the `LogosSeq` fallback (returns `false`).
pub(crate) fn text_bytes_view_safe(stmts: &[Stmt], sb: Symbol) -> bool {
    stmts.iter().all(|s| stmt_view_read_only(s, sb))
}

fn stmt_view_read_only(s: &Stmt, sb: Symbol) -> bool {
    match s {
        // `Repeat for _ in sb` iterates the view (OK); a pattern binding `sb` shadows it (bail).
        Stmt::Repeat {
            pattern,
            iterable,
            body,
        } => {
            !pattern_binds(pattern, sb)
                && match iterable {
                    Expr::Identifier(s2) if *s2 == sb => true,
                    other => expr_view_read_only(other, sb),
                }
                && text_bytes_view_safe(body, sb)
        }
        Stmt::Let { var, value, .. } => *var != sb && expr_view_read_only(value, sb),
        Stmt::Set { target, value } => *target != sb && expr_view_read_only(value, sb),
        Stmt::SetField { object, value, .. } => {
            expr_view_read_only(object, sb) && expr_view_read_only(value, sb)
        }
        // A collection-building op MUST NOT carry `sb` in its value/index: a loop that
        // copies `item i of sb` into another collection is fused by a seq-copy/slice peephole
        // into `from_vec(sb.borrow()[range].to_vec())`, and `.borrow()` does not exist on the
        // `&[u8]` view (E0599). Reading `item i of sb` into a SCALAR `Let`/`Set` and pushing a
        // TRANSFORMED value (the upper/lower/title/swapcase shape) stays safe.
        Stmt::SetIndex {
            collection,
            index,
            value,
        } => {
            !seq_expr_mentions(collection, sb)
                && !seq_expr_mentions(index, sb)
                && !seq_expr_mentions(value, sb)
        }
        Stmt::Push { value, collection }
        | Stmt::Add { value, collection }
        | Stmt::Remove { value, collection } => {
            !seq_expr_mentions(collection, sb) && !seq_expr_mentions(value, sb)
        }
        Stmt::Pop { collection, .. } => !seq_expr_mentions(collection, sb),
        Stmt::Swap { a, b } => *a != sb && *b != sb,
        Stmt::Return { value } => value.map_or(true, |v| expr_view_read_only(v, sb)),
        Stmt::Break => true,
        Stmt::RuntimeAssert { condition, .. } => expr_view_read_only(condition, sb),
        // `Show` is an immutable read (allow `Show item i of sb` / `length of sb`);
        // `Give` MOVES `sb` out, which the view cannot support.
        Stmt::Show { object, recipient } => {
            expr_view_read_only(object, sb) && expr_view_read_only(recipient, sb)
        }
        Stmt::Give { object, recipient } => {
            !seq_expr_mentions(object, sb) && !seq_expr_mentions(recipient, sb)
        }
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => {
            expr_view_read_only(cond, sb)
                && text_bytes_view_safe(then_block, sb)
                && else_block
                    .as_ref()
                    .map_or(true, |e| text_bytes_view_safe(e, sb))
        }
        Stmt::While {
            cond,
            body,
            decreasing,
        } => {
            expr_view_read_only(cond, sb)
                && decreasing.map_or(true, |d| expr_view_read_only(d, sb))
                && text_bytes_view_safe(body, sb)
        }
        Stmt::Inspect { target, arms, .. } => {
            expr_view_read_only(target, sb)
                && arms.iter().all(|arm| {
                    !arm.bindings.iter().any(|(_, b)| *b == sb)
                        && text_bytes_view_safe(arm.body, sb)
                })
        }
        Stmt::Zone { body, .. }
        | Stmt::Splice { body }
        | Stmt::Reversibly { body }
        | Stmt::During { body, .. } => text_bytes_view_safe(body, sb),
        // A statement-position call: unsafe if `sb` itself is an argument (view is `&[u8]`,
        // the callee expects `Seq of Int`); passing `item i of sb` (a byte) is fine.
        Stmt::Call { args, .. } => args.iter().all(|a| expr_view_read_only(a, sb)),
        _ => false,
    }
}

/// Every occurrence of `sb` in `e` is a view-compatible READ (the collection of an
/// `Index` or `Length`). A BARE `sb`, or `sb` anywhere else, is NOT view-compatible.
fn expr_view_read_only(e: &Expr, sb: Symbol) -> bool {
    match e {
        Expr::Index { collection, index } => {
            let coll_ok = match collection {
                Expr::Identifier(s) if *s == sb => true,
                other => expr_view_read_only(other, sb),
            };
            coll_ok && expr_view_read_only(index, sb)
        }
        Expr::Length { collection } => match collection {
            Expr::Identifier(s) if *s == sb => true,
            other => expr_view_read_only(other, sb),
        },
        Expr::Identifier(s) => *s != sb,
        Expr::Literal(_) | Expr::OptionNone | Expr::Escape { .. } => true,
        Expr::BinaryOp { left, right, .. }
        | Expr::Union { left, right }
        | Expr::Intersection { left, right }
        | Expr::Range {
            start: left,
            end: right,
        } => expr_view_read_only(left, sb) && expr_view_read_only(right, sb),
        Expr::Not { operand } => expr_view_read_only(operand, sb),
        Expr::Slice {
            collection,
            start,
            end,
        } => {
            expr_view_read_only(collection, sb)
                && expr_view_read_only(start, sb)
                && expr_view_read_only(end, sb)
        }
        Expr::Copy { expr } | Expr::Give { value: expr } | Expr::OptionSome { value: expr } => {
            expr_view_read_only(expr, sb)
        }
        Expr::FieldAccess { object, .. } => expr_view_read_only(object, sb),
        Expr::Contains { collection, value } => {
            expr_view_read_only(collection, sb) && expr_view_read_only(value, sb)
        }
        Expr::ManifestOf { zone } => expr_view_read_only(zone, sb),
        Expr::ChunkAt { index, zone } => {
            expr_view_read_only(index, sb) && expr_view_read_only(zone, sb)
        }
        Expr::Call { args, .. } => args.iter().all(|a| expr_view_read_only(a, sb)),
        Expr::CallExpr { callee, args } => {
            expr_view_read_only(callee, sb) && args.iter().all(|a| expr_view_read_only(a, sb))
        }
        Expr::List(items) | Expr::Tuple(items) => items.iter().all(|i| expr_view_read_only(i, sb)),
        Expr::New { init_fields, .. } => {
            init_fields.iter().all(|(_, e)| expr_view_read_only(e, sb))
        }
        Expr::NewVariant { fields, .. } => fields.iter().all(|(_, e)| expr_view_read_only(e, sb)),
        Expr::WithCapacity { value, capacity } => {
            expr_view_read_only(value, sb) && expr_view_read_only(capacity, sb)
        }
        Expr::InterpolatedString(parts) => parts.iter().all(|p| match p {
            crate::ast::stmt::StringPart::Expr { value, .. } => expr_view_read_only(value, sb),
            _ => true,
        }),
        // Closure/View/Dom or any future variant: safe only if `sb` is absent.
        _ => !seq_expr_mentions(e, sb),
    }
}

/// Check if a loop body resizes a specific collection via Push, Pop, Add, or Remove.
/// Unlike `body_mutates_collection`, this does NOT flag SetIndex (element-level writes),
/// making it suitable for detecting double-buffer patterns where SetIndex is expected.
/// Recursively walks into nested If/While/Repeat/Zone blocks.
pub(crate) fn body_resizes_collection(stmts: &[Stmt], coll_sym: Symbol) -> bool {
    for stmt in stmts {
        match stmt {
            Stmt::Push { collection, .. }
            | Stmt::Pop { collection, .. }
            | Stmt::Add { collection, .. }
            | Stmt::Remove { collection, .. } => {
                if let Expr::Identifier(sym) = collection {
                    if *sym == coll_sym {
                        return true;
                    }
                }
            }
            Stmt::Set { target, .. } if *target == coll_sym => return true,
            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
                if body_resizes_collection(then_block, coll_sym) {
                    return true;
                }
                if let Some(else_stmts) = else_block {
                    if body_resizes_collection(else_stmts, coll_sym) {
                        return true;
                    }
                }
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } => {
                if body_resizes_collection(body, coll_sym) {
                    return true;
                }
            }
            Stmt::Zone { body, .. } => {
                if body_resizes_collection(body, coll_sym) {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

/// Check if every execution path through `stmts` includes at least one `Push` targeting
/// `coll_sym`. Returns true only when the push count is deterministic (every branch pushes),
/// which makes `with_capacity(loop_count)` a valid pre-allocation.
/// For filter patterns (push inside If without Otherwise), returns false.
fn all_paths_push_to(stmts: &[Stmt], coll_sym: Symbol) -> bool {
    stmts.iter().any(|s| match s {
        Stmt::Push { collection, .. } => {
            matches!(collection, Expr::Identifier(sym) if *sym == coll_sym)
        }
        Stmt::If {
            then_block,
            else_block,
            ..
        } => {
            if let Some(else_stmts) = else_block {
                all_paths_push_to(then_block, coll_sym) && all_paths_push_to(else_stmts, coll_sym)
            } else {
                false
            }
        }
        _ => false,
    })
}

/// Check if every execution path through `stmts` includes at least one `SetIndex` targeting
/// `coll_sym`. Same logic as `all_paths_push_to` but for map insertion.
fn all_paths_set_index_to(stmts: &[Stmt], coll_sym: Symbol) -> bool {
    stmts.iter().any(|s| match s {
        Stmt::SetIndex { collection, .. } => {
            matches!(collection, Expr::Identifier(sym) if *sym == coll_sym)
        }
        Stmt::If {
            then_block,
            else_block,
            ..
        } => {
            if let Some(else_stmts) = else_block {
                all_paths_set_index_to(then_block, coll_sym)
                    && all_paths_set_index_to(else_stmts, coll_sym)
            } else {
                false
            }
        }
        _ => false,
    })
}

/// Check that the counter is used in at least one Expr::Index or Stmt::SetIndex
/// within the body. If there are zero index uses, zero-based normalization
/// provides no benefit (no -1 subtracts to eliminate).
fn counter_has_index_uses(stmts: &[Stmt], counter_sym: Symbol) -> bool {
    stmts
        .iter()
        .any(|s| stmt_has_counter_index_use(s, counter_sym))
}

fn stmt_has_counter_index_use(stmt: &Stmt, counter_sym: Symbol) -> bool {
    match stmt {
        Stmt::Let { value, .. } => expr_has_counter_index_use(value, counter_sym),
        Stmt::Set { value, .. } => expr_has_counter_index_use(value, counter_sym),
        Stmt::Show { object, .. } => expr_has_counter_index_use(object, counter_sym),
        Stmt::Push { value, .. } => expr_has_counter_index_use(value, counter_sym),
        Stmt::SetIndex { index, value, .. } => {
            is_coeff1_affine_index(index, counter_sym)
                || expr_has_counter_index_use(value, counter_sym)
        }
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => {
            expr_has_counter_index_use(cond, counter_sym)
                || counter_has_index_uses(then_block, counter_sym)
                || else_block
                    .as_ref()
                    .map_or(false, |eb| counter_has_index_uses(eb, counter_sym))
        }
        Stmt::While { body, .. } => counter_has_index_uses(body, counter_sym),
        Stmt::Return { value } => {
            value.map_or(false, |v| expr_has_counter_index_use(v, counter_sym))
        }
        Stmt::Call { args, .. } => args
            .iter()
            .any(|a| expr_has_counter_index_use(a, counter_sym)),
        Stmt::Repeat { body, .. } => counter_has_index_uses(body, counter_sym),
        _ => false,
    }
}

fn expr_has_counter_index_use(expr: &Expr, counter_sym: Symbol) -> bool {
    match expr {
        Expr::Index { collection, index } => {
            is_coeff1_affine_index(index, counter_sym)
                || expr_has_counter_index_use(collection, counter_sym)
                || expr_has_counter_index_use(index, counter_sym)
        }
        Expr::BinaryOp { left, right, .. } => {
            expr_has_counter_index_use(left, counter_sym)
                || expr_has_counter_index_use(right, counter_sym)
        }
        Expr::Call { args, .. } => args
            .iter()
            .any(|a| expr_has_counter_index_use(a, counter_sym)),
        Expr::Length { collection } => expr_has_counter_index_use(collection, counter_sym),
        Expr::List(items) => items
            .iter()
            .any(|e| expr_has_counter_index_use(e, counter_sym)),
        Expr::Not { operand } => expr_has_counter_index_use(operand, counter_sym),
        Expr::Copy { expr: inner } => expr_has_counter_index_use(inner, counter_sym),
        Expr::Slice {
            collection,
            start,
            end,
        } => {
            expr_has_counter_index_use(collection, counter_sym)
                || expr_has_counter_index_use(start, counter_sym)
                || expr_has_counter_index_use(end, counter_sym)
        }
        _ => false,
    }
}

/// Verify that every collection indexed by the counter is a Vec or slice type.
/// Zero-based normalization only works for direct `arr[i as usize]` codegen —
/// string indexing goes through `LogosIndex::logos_get()` which does internal
/// 1-based conversion, so passing a 0-based counter would be incorrect.
fn counter_indexes_only_vec_types(
    stmts: &[Stmt],
    counter_sym: Symbol,
    ctx: &RefinementContext,
) -> bool {
    for stmt in stmts {
        if !stmt_counter_indexes_vec_types(stmt, counter_sym, ctx) {
            return false;
        }
    }
    true
}

fn stmt_counter_indexes_vec_types(
    stmt: &Stmt,
    counter_sym: Symbol,
    ctx: &RefinementContext,
) -> bool {
    match stmt {
        Stmt::Let { value, .. } => expr_counter_indexes_vec_types(value, counter_sym, ctx),
        Stmt::Set { value, .. } => expr_counter_indexes_vec_types(value, counter_sym, ctx),
        Stmt::Show { object, .. } => expr_counter_indexes_vec_types(object, counter_sym, ctx),
        Stmt::Push { value, .. } => expr_counter_indexes_vec_types(value, counter_sym, ctx),
        Stmt::SetIndex {
            collection,
            index,
            value,
        } => {
            let idx_uses_counter = is_coeff1_affine_index(index, counter_sym);
            if idx_uses_counter {
                if let Expr::Identifier(coll_sym) = collection {
                    let is_vec = ctx.get_variable_types().get(coll_sym).map_or(false, |t| {
                        t.starts_with("LogosSeq")
                            || t.starts_with("Vec")
                            || t.starts_with("&[")
                            || t.starts_with("&mut [")
                            || t.starts_with("[")
                    });
                    if !is_vec {
                        return false;
                    }
                } else {
                    return false;
                }
            }
            expr_counter_indexes_vec_types(value, counter_sym, ctx)
        }
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => {
            expr_counter_indexes_vec_types(cond, counter_sym, ctx)
                && counter_indexes_only_vec_types(then_block, counter_sym, ctx)
                && else_block.as_ref().map_or(true, |eb| {
                    counter_indexes_only_vec_types(eb, counter_sym, ctx)
                })
        }
        Stmt::While { body, .. } => counter_indexes_only_vec_types(body, counter_sym, ctx),
        Stmt::Repeat { body, .. } => counter_indexes_only_vec_types(body, counter_sym, ctx),
        Stmt::Return { value } => value.map_or(true, |v| {
            expr_counter_indexes_vec_types(v, counter_sym, ctx)
        }),
        Stmt::Call { args, .. } => args
            .iter()
            .all(|a| expr_counter_indexes_vec_types(a, counter_sym, ctx)),
        _ => true,
    }
}

fn expr_counter_indexes_vec_types(
    expr: &Expr,
    counter_sym: Symbol,
    ctx: &RefinementContext,
) -> bool {
    match expr {
        Expr::Index { collection, index } => {
            let idx_uses_counter = is_coeff1_affine_index(index, counter_sym);
            if idx_uses_counter {
                if let Expr::Identifier(coll_sym) = collection {
                    let is_vec = ctx.get_variable_types().get(coll_sym).map_or(false, |t| {
                        t.starts_with("LogosSeq")
                            || t.starts_with("Vec")
                            || t.starts_with("&[")
                            || t.starts_with("&mut [")
                            || t.starts_with("[")
                    });
                    if !is_vec {
                        return false;
                    }
                } else {
                    return false;
                }
            }
            expr_counter_indexes_vec_types(collection, counter_sym, ctx)
                && expr_counter_indexes_vec_types(index, counter_sym, ctx)
        }
        Expr::BinaryOp { left, right, .. } => {
            expr_counter_indexes_vec_types(left, counter_sym, ctx)
                && expr_counter_indexes_vec_types(right, counter_sym, ctx)
        }
        Expr::Call { args, .. } => args
            .iter()
            .all(|a| expr_counter_indexes_vec_types(a, counter_sym, ctx)),
        Expr::Not { operand } => expr_counter_indexes_vec_types(operand, counter_sym, ctx),
        Expr::Copy { expr: inner } => expr_counter_indexes_vec_types(inner, counter_sym, ctx),
        Expr::Length { collection } => expr_counter_indexes_vec_types(collection, counter_sym, ctx),
        Expr::List(items) => items
            .iter()
            .all(|e| expr_counter_indexes_vec_types(e, counter_sym, ctx)),
        Expr::Slice {
            collection,
            start,
            end,
        } => {
            expr_counter_indexes_vec_types(collection, counter_sym, ctx)
                && expr_counter_indexes_vec_types(start, counter_sym, ctx)
                && expr_counter_indexes_vec_types(end, counter_sym, ctx)
        }
        _ => true,
    }
}

/// Check if a counter symbol is used ONLY as a direct array index in the body.
/// Returns true when every occurrence of `counter_sym` in the body is either:
/// - The index in `Expr::Index { collection, index: Identifier(counter) }`
/// - The index in `Stmt::SetIndex { collection, index: Identifier(counter), .. }`
///
/// Returns false if the counter appears in any other context (arithmetic,
/// comparison, Show, function arguments, etc.), since shifting the counter
/// to 0-based would change the computation.
fn counter_only_used_for_indexing(stmts: &[Stmt], counter_sym: Symbol) -> bool {
    for stmt in stmts {
        if !check_counter_stmt_indexing_only(stmt, counter_sym) {
            return false;
        }
    }
    true
}

/// Check a single statement for non-index uses of counter.
/// Returns false if the counter is used in a non-index context.
fn check_counter_stmt_indexing_only(stmt: &Stmt, counter_sym: Symbol) -> bool {
    match stmt {
        Stmt::Let { value, .. } => expr_uses_counter_only_in_index(value, counter_sym),
        Stmt::Set { value, .. } => expr_uses_counter_only_in_index(value, counter_sym),
        Stmt::Show { object, .. } => expr_uses_counter_only_in_index(object, counter_sym),
        Stmt::Push { value, .. } => expr_uses_counter_only_in_index(value, counter_sym),
        Stmt::SetIndex {
            collection: _,
            index,
            value,
        } => {
            // The index position of SetIndex IS a valid index use: a coefficient-1
            // affine index in the counter (`counter`, `counter ± k`) is rebasable,
            // or the index may not reference the counter at all.
            let index_ok = is_coeff1_affine_index(index, counter_sym)
                || !expr_contains_symbol(index, counter_sym);
            let value_ok = expr_uses_counter_only_in_index(value, counter_sym);
            index_ok && value_ok
        }
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => {
            expr_uses_counter_only_in_index(cond, counter_sym)
                && counter_only_used_for_indexing(then_block, counter_sym)
                && else_block
                    .as_ref()
                    .map_or(true, |eb| counter_only_used_for_indexing(eb, counter_sym))
        }
        Stmt::While { cond, body, .. } => {
            expr_uses_counter_only_in_index(cond, counter_sym)
                && counter_only_used_for_indexing(body, counter_sym)
        }
        Stmt::Repeat { body, .. } => counter_only_used_for_indexing(body, counter_sym),
        Stmt::Call { args, .. } => args
            .iter()
            .all(|a| expr_uses_counter_only_in_index(a, counter_sym)),
        Stmt::Return { value } => {
            value.map_or(true, |v| expr_uses_counter_only_in_index(v, counter_sym))
        }
        _ => {
            // For other statements, conservatively check they don't reference counter
            true
        }
    }
}

/// Check if an expression uses the counter symbol ONLY inside Expr::Index positions.
/// Returns false if the counter appears in any non-index context.
fn expr_uses_counter_only_in_index(expr: &Expr, counter_sym: Symbol) -> bool {
    match expr {
        Expr::Identifier(sym) => {
            // Bare identifier use of counter = non-index use
            *sym != counter_sym
        }
        Expr::Index { collection, index } => {
            // The index position IS a valid use for the counter
            let collection_ok = expr_uses_counter_only_in_index(collection, counter_sym);
            // A coefficient-1 affine index (`counter`, `counter ± k`) is a valid,
            // rebasable use — the 0-based codegen (`zero_based_index_str`) shifts it
            // exactly. Fall back to the recursive check for nested-index forms.
            let index_ok = is_coeff1_affine_index(index, counter_sym)
                || expr_uses_counter_only_in_index(index, counter_sym);
            collection_ok && index_ok
        }
        Expr::BinaryOp { op, left, right } => {
            // For comparison operators, also allow the counter as a bare operand.
            // This enables zero-based normalization for loops like:
            //   While i is at most n: If i is greater than k: ... item i of arr ...
            // The counter `i` appears in both an index and a comparison. In codegen,
            // the comparison operand gets `(i + 1)` to compensate for the 0-based shift.
            match op {
                BinaryOpKind::Lt
                | BinaryOpKind::LtEq
                | BinaryOpKind::Gt
                | BinaryOpKind::GtEq
                | BinaryOpKind::Eq
                | BinaryOpKind::NotEq => {
                    let left_is_counter = matches!(left, Expr::Identifier(s) if *s == counter_sym);
                    let right_is_counter =
                        matches!(right, Expr::Identifier(s) if *s == counter_sym);
                    if left_is_counter && !expr_contains_symbol(right, counter_sym) {
                        return true;
                    }
                    if right_is_counter && !expr_contains_symbol(left, counter_sym) {
                        return true;
                    }
                    // Counter in both sides or in sub-expressions — fall through
                    expr_uses_counter_only_in_index(left, counter_sym)
                        && expr_uses_counter_only_in_index(right, counter_sym)
                }
                _ => {
                    expr_uses_counter_only_in_index(left, counter_sym)
                        && expr_uses_counter_only_in_index(right, counter_sym)
                }
            }
        }
        Expr::Not { operand } => expr_uses_counter_only_in_index(operand, counter_sym),
        Expr::Call { args, .. } => args
            .iter()
            .all(|a| expr_uses_counter_only_in_index(a, counter_sym)),
        Expr::Length { collection } => expr_uses_counter_only_in_index(collection, counter_sym),
        Expr::Literal(_) => true,
        Expr::List(items) => items
            .iter()
            .all(|e| expr_uses_counter_only_in_index(e, counter_sym)),
        Expr::Slice {
            collection,
            start,
            end,
        } => {
            expr_uses_counter_only_in_index(collection, counter_sym)
                && expr_uses_counter_only_in_index(start, counter_sym)
                && expr_uses_counter_only_in_index(end, counter_sym)
        }
        Expr::Copy { expr: inner } => expr_uses_counter_only_in_index(inner, counter_sym),
        // A tuple's elements are VALUE positions: a `Return (s, i + 1)` reads the counter directly, so
        // rebasing the loop 0-based would silently change what the tuple returns. Each element must be
        // index-only for the tuple as a whole to be.
        Expr::Tuple(items) => items
            .iter()
            .all(|e| expr_uses_counter_only_in_index(e, counter_sym)),
        _ => !expr_contains_symbol(expr, counter_sym),
    }
}

/// Check if an expression contains a specific symbol anywhere.
fn expr_contains_symbol(expr: &Expr, sym: Symbol) -> bool {
    match expr {
        Expr::Identifier(s) => *s == sym,
        Expr::BinaryOp { left, right, .. } => {
            expr_contains_symbol(left, sym) || expr_contains_symbol(right, sym)
        }
        Expr::Not { operand } => expr_contains_symbol(operand, sym),
        Expr::Call { args, .. } => args.iter().any(|a| expr_contains_symbol(a, sym)),
        Expr::Index { collection, index } => {
            expr_contains_symbol(collection, sym) || expr_contains_symbol(index, sym)
        }
        Expr::Length { collection } => expr_contains_symbol(collection, sym),
        Expr::List(items) => items.iter().any(|e| expr_contains_symbol(e, sym)),
        Expr::Slice {
            collection,
            start,
            end,
        } => {
            expr_contains_symbol(collection, sym)
                || expr_contains_symbol(start, sym)
                || expr_contains_symbol(end, sym)
        }
        Expr::Copy { expr: inner } => expr_contains_symbol(inner, sym),
        // A TUPLE (a multi-value `Return (s, i + 1)`) can carry the counter in a non-index element,
        // so it MUST recurse — otherwise the zero-based-counter guard wrongly reads `i` as unused and
        // rebases the loop to 0-based, corrupting the `i + 1` the tuple returns (an off-by-one on the
        // parsed offset, e.g. `pgReadCStringAt`'s `afterName`).
        Expr::Tuple(items) => items.iter().any(|e| expr_contains_symbol(e, sym)),
        Expr::Literal(_) => false,
        _ => false,
    }
}

/// Is `index` a **coefficient-1 affine** expression in `counter` — `counter`,
/// `counter ± k`, or `k + counter` (literal `k`)? These are exactly the index
/// shapes the 0-based rebase codegen ([`super::expr::zero_based_index_str`]) can
/// shift; the two predicates MUST stay in lock-step.
fn is_coeff1_affine_index(index: &Expr, counter: Symbol) -> bool {
    match index {
        Expr::Identifier(s) => *s == counter,
        Expr::BinaryOp {
            op: BinaryOpKind::Add,
            left,
            right,
        } => {
            (matches!(left, Expr::Identifier(s) if *s == counter)
                && matches!(right, Expr::Literal(Literal::Number(_))))
                || (matches!(right, Expr::Identifier(s) if *s == counter)
                    && matches!(left, Expr::Literal(Literal::Number(_))))
        }
        Expr::BinaryOp {
            op: BinaryOpKind::Subtract,
            left,
            right,
        } => {
            matches!(left, Expr::Identifier(s) if *s == counter)
                && matches!(right, Expr::Literal(Literal::Number(_)))
        }
        _ => false,
    }
}

/// Check if a statement is `Set counter to counter + 1`.
pub(crate) fn is_counter_increment(stmt: &Stmt, counter: Symbol) -> bool {
    match stmt {
        Stmt::Set { target, value } if *target == counter => match value {
            Expr::BinaryOp {
                op: BinaryOpKind::Add,
                left,
                right,
            } => match (left, right) {
                (Expr::Identifier(s), Expr::Literal(Literal::Number(1))) => *s == counter,
                (Expr::Literal(Literal::Number(1)), Expr::Identifier(s)) => *s == counter,
                _ => false,
            },
            _ => false,
        },
        _ => false,
    }
}

/// Match a 1-based row-major `(R*S + C) + 1` index where `R` and `C` are tile
/// counters and `S` is the (loop-invariant) stride. Returns the emitted index
/// `R*S + C` so a bounds-elision hint can name it.
fn match_tiled_bilinear<'a>(
    index: &'a Expr<'a>,
    counters: &[Symbol],
    stride: &Expr,
) -> Option<&'a Expr<'a>> {
    let is_counter = |e: &Expr| matches!(e, Expr::Identifier(s) if counters.contains(s));
    let is_rs = |e: &Expr| {
        matches!(
            e,
            Expr::BinaryOp { op: BinaryOpKind::Multiply, left: r, right: s }
                if is_counter(r) && exprs_equal(s, stride)
        )
    };
    if let Expr::BinaryOp {
        op: BinaryOpKind::Add,
        left,
        right,
    } = index
    {
        if matches!(right, Expr::Literal(Literal::Number(1))) {
            if let Expr::BinaryOp {
                op: BinaryOpKind::Add,
                left: la,
                right: lb,
            } = left
            {
                if (is_rs(la) && is_counter(lb)) || (is_counter(la) && is_rs(lb)) {
                    return Some(left);
                }
            }
        }
    }
    None
}

/// The `assert_unchecked` hints a tiled nest's innermost body carries: for each access the versioning guard
/// proved in range ([`crate::optimize::facts::Fact::InBounds`]) to a `Vec`/slice indexed by a row-major
/// bilinear form (`a[i*n+k]`, or `c[idx-1]` where `idx = i*n+j+1`), the array and the EMITTED 0-based index
/// `R*S+C`, in body order and once each. An access without the fact gets no hint.
fn tiled_hints<'a>(
    body: &'a [Stmt<'a>],
    counters: &[Symbol],
    stride: &Expr,
    interner: &Interner,
    var_types: &HashMap<Symbol, String>,
) -> Vec<(Symbol, String)> {
    // Vars bound to a bilinear 1-based index (`Let idx be i*n+j+1`), mapped to the RESOLVED emitted index
    // `R*S+C` (`i*n+j`). Resolving avoids referencing the binding (which is emitted INSIDE the body, after the
    // hint) — the counters are loop variables always in scope, and LLVM GVNs `idx-1` to it.
    let mut bilinear_vars: HashMap<Symbol, &Expr> = HashMap::new();
    for stmt in body {
        if let Stmt::Let { var, value, .. } = stmt {
            if let Some(a) = match_tiled_bilinear(value, counters, stride) {
                bilinear_vars.insert(*var, a);
            }
        }
    }
    let mut accesses: Vec<(&Expr, &Expr)> = Vec::new();
    for stmt in body {
        collect_index_pairs_in_stmt(stmt, &mut accesses);
    }
    let mut hints: Vec<(Symbol, String)> = Vec::new();
    for (coll, index) in accesses {
        let Expr::Identifier(arr) = coll else {
            continue;
        };
        if !crate::optimize::facts::holds_expr(crate::optimize::facts::Fact::InBounds, index) {
            continue;
        }
        // Only `Vec`/slice arrays (`.len()` is valid and stable). A LogosSeq would need `.borrow()`; a resized
        // array's length would go stale.
        let qualifies = var_types.get(arr).map_or(false, |t| {
            let t = t.split("|__hl:").next().unwrap_or(t.as_str());
            t.starts_with("Vec<") || t.starts_with("&[") || t.starts_with("&mut [")
        });
        if !qualifies || body_resizes_collection(body, *arr) {
            continue;
        }
        let resolved = match_tiled_bilinear(index, counters, stride).or_else(|| match index {
            Expr::Identifier(v) => bilinear_vars.get(v).copied(),
            _ => None,
        });
        if let Some(a) = resolved {
            let hint = (*arr, codegen_expr_simple(a, interner));
            if !hints.contains(&hint) {
                hints.push(hint);
            }
        }
    }
    hints
}

/// Collect `(collection, index)` pairs from every index access in a statement
/// (SetIndex and `item _ of _` reads anywhere in its expressions).
fn collect_index_pairs_in_stmt<'a>(
    stmt: &'a Stmt<'a>,
    out: &mut Vec<(&'a Expr<'a>, &'a Expr<'a>)>,
) {
    fn in_expr<'a>(e: &'a Expr<'a>, out: &mut Vec<(&'a Expr<'a>, &'a Expr<'a>)>) {
        match e {
            Expr::Index { collection, index } => {
                out.push((collection, index));
                in_expr(collection, out);
                in_expr(index, out);
            }
            Expr::BinaryOp { left, right, .. } => {
                in_expr(left, out);
                in_expr(right, out);
            }
            Expr::Not { operand } => in_expr(operand, out),
            Expr::Call { args, .. } => {
                for a in args.iter() {
                    in_expr(a, out);
                }
            }
            Expr::Length { collection } => in_expr(collection, out),
            _ => {}
        }
    }
    match stmt {
        Stmt::Let { value, .. } | Stmt::Set { value, .. } => in_expr(value, out),
        Stmt::SetIndex {
            collection,
            index,
            value,
        } => {
            out.push((collection, index));
            in_expr(index, out);
            in_expr(value, out);
        }
        _ => {}
    }
}

/// Detect triple-nested for-range loop with same bound and emit tiled loop nest.
/// Pattern: for i in 0..N { for k in 0..N { for j in 0..N { body } } }
/// This is the matrix multiplication pattern where tiling dramatically improves
/// L1 cache locality. Emits 6-level nest: outer tiles (step_by) + inner iteration.
#[allow(clippy::too_many_arguments)]
fn try_emit_tiled_inner<'a>(
    outer_sym: Symbol,
    outer_bound: &Expr<'a>,
    outer_body: &[Stmt<'a>],
    stmts: &[&Stmt<'a>],
    idx: usize,
    is_new_binding: bool,
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
) -> Option<(String, usize)> {
    const TILE_SIZE: i64 = 32;

    // Outer body must be exactly: Let mid = 0, While mid < bound: [mid_body], Set mid = mid+1
    // After removing the outer increment, we have body_without_increment = outer_body.
    // This should be exactly 2 statements: Let mid = 0, While mid < bound.
    if outer_body.len() != 2 {
        return None;
    }

    // Match middle loop init: Let mid = 0
    let mid_sym = match &outer_body[0] {
        Stmt::Let { var, value, .. } => {
            if matches!(value, Expr::Literal(Literal::Number(0))) {
                *var
            } else {
                return None;
            }
        }
        _ => return None,
    };

    // Match middle loop: While mid < bound
    let mid_body = match &outer_body[1] {
        Stmt::While { cond, body, .. } => match cond {
            Expr::BinaryOp {
                op: BinaryOpKind::Lt,
                left,
                right,
            } => {
                if let Expr::Identifier(s) = left {
                    if *s == mid_sym && exprs_equal(right, outer_bound) {
                        *body
                    } else {
                        return None;
                    }
                } else {
                    return None;
                }
            }
            _ => return None,
        },
        _ => return None,
    };

    // Check mid body has increment as last statement
    if mid_body.is_empty() {
        return None;
    }
    if !is_counter_increment(&mid_body[mid_body.len() - 1], mid_sym) {
        return None;
    }
    let mid_body_sans_inc = &mid_body[..mid_body.len() - 1];

    // Middle body (sans increment) must be exactly 2 statements: Let inner = 0, While inner < bound
    if mid_body_sans_inc.len() != 2 {
        return None;
    }

    // Match inner loop init: Let inner = 0
    let inner_sym = match &mid_body_sans_inc[0] {
        Stmt::Let { var, value, .. } => {
            if matches!(value, Expr::Literal(Literal::Number(0))) {
                *var
            } else {
                return None;
            }
        }
        _ => return None,
    };

    // Match inner loop: While inner < bound
    // Keep the inner While reference — its arena address keys the oracle
    // alias snapshot used to hoist the tile body's handles.
    let inner_loop_stmt: &Stmt = &mid_body_sans_inc[1];
    let inner_body = match &mid_body_sans_inc[1] {
        Stmt::While { cond, body, .. } => match cond {
            Expr::BinaryOp {
                op: BinaryOpKind::Lt,
                left,
                right,
            } => {
                if let Expr::Identifier(s) = left {
                    if *s == inner_sym && exprs_equal(right, outer_bound) {
                        *body
                    } else {
                        return None;
                    }
                } else {
                    return None;
                }
            }
            _ => return None,
        },
        _ => return None,
    };

    // Check inner body has increment as last statement
    if inner_body.is_empty() {
        return None;
    }
    if !is_counter_increment(&inner_body[inner_body.len() - 1], inner_sym) {
        return None;
    }
    let inner_body_sans_inc = &inner_body[..inner_body.len() - 1];

    // All three levels match with the same bound. Emit tiled 6-level loop nest.
    let names = RustNames::new(interner);
    let outer_name = names.ident(outer_sym);
    let mid_name = names.ident(mid_sym);
    let inner_name = names.ident(inner_sym);
    let bound_str = codegen_expr_simple(outer_bound, interner);

    let pad = "    ".repeat(indent);
    let mut out = String::new();

    // No counter-bound hints here: tiled bodies index with flattened affine
    // forms (i*n+j+1) whose maximum is n*n-shaped, not counter-shaped — a
    // per-counter bound says nothing about them. Interval-derived hints for
    // affine indices are O5 territory.

    writeln!(out, "{}{{", pad).unwrap();
    writeln!(out, "{}    let __tile: i64 = {};", pad, TILE_SIZE).unwrap();

    // O1-D: hoist the tile body's Seq borrows around the whole nest. The
    // matrix kernel reads a/b and read-writes c, all distinct fresh
    // allocations — one borrow each instead of per-FMA.
    let tile_indent = format!("{}    ", pad);
    let hoist_plan =
        super::hoist::plan_borrow_hoist(inner_loop_stmt, None, inner_body_sans_inc, ctx, interner);
    super::hoist::emit_hoist_open(&hoist_plan, interner, &tile_indent, ctx, &mut out);

    // Every access the nest's versioning guard proved in range gets an `assert_unchecked` hint in the
    // innermost body, after which LLVM drops its check; the guard also proved the index arithmetic stays in
    // i64 (`optimize::version` recorded both).
    let hints = tiled_hints(
        inner_body_sans_inc,
        &[outer_sym, mid_sym, inner_sym],
        outer_bound,
        interner,
        ctx.get_variable_types(),
    );
    crate::optimize::mark_fired(crate::optimization::Opt::Tile);

    writeln!(
        out,
        "{}    for __{}_t in (0i64..{}).step_by(__tile as usize) {{",
        pad, outer_name, bound_str
    )
    .unwrap();
    writeln!(
        out,
        "{}        for __{}_t in (0i64..{}).step_by(__tile as usize) {{",
        pad, mid_name, bound_str
    )
    .unwrap();
    writeln!(
        out,
        "{}            for __{}_t in (0i64..{}).step_by(__tile as usize) {{",
        pad, inner_name, bound_str
    )
    .unwrap();
    writeln!(
        out,
        "{}                for {} in __{}_t..(__{}_t + __tile).min({}) {{",
        pad, outer_name, outer_name, outer_name, bound_str
    )
    .unwrap();
    writeln!(
        out,
        "{}                    for {} in __{}_t..(__{}_t + __tile).min({}) {{",
        pad, mid_name, mid_name, mid_name, bound_str
    )
    .unwrap();
    writeln!(
        out,
        "{}                        for {} in __{}_t..(__{}_t + __tile).min({}) {{",
        pad, inner_name, inner_name, inner_name, bound_str
    )
    .unwrap();

    // Emit inner body using regular codegen
    let body_indent = indent + 7;
    {
        let bi = "    ".repeat(body_indent);
        for (arr, idx) in &hints {
            writeln!(
                out,
                "{}unsafe {{ std::hint::assert_unchecked(({idx}) >= 0 && ({idx}) < ({arr}.len() as i64)); }}",
                bi, idx = idx, arr = names.ident(*arr)
            ).unwrap();
        }
    }
    for stmt in inner_body_sans_inc {
        out.push_str(&super::codegen_stmt(
            stmt,
            interner,
            body_indent,
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

    writeln!(out, "{}                        }}", pad).unwrap();
    writeln!(out, "{}                    }}", pad).unwrap();
    writeln!(out, "{}                }}", pad).unwrap();
    writeln!(out, "{}            }}", pad).unwrap();
    writeln!(out, "{}        }}", pad).unwrap();
    writeln!(out, "{}    }}", pad).unwrap();
    // Close the borrow-hoist scope and restore the handles' tracked types.
    super::hoist::emit_hoist_close(&hoist_plan, &tile_indent, ctx, &mut out);
    writeln!(out, "{}}}", pad).unwrap();

    // Post-loop counter value for the outer counter.
    // Middle and inner counters are scoped inside the outer body and don't escape.
    let remaining = &stmts[idx + 2..];
    if value_read_after(outer_sym, stmts, idx + 2) {
        let next_overwrites = remaining.first().map_or(
            false,
            |s| matches!(s, Stmt::Set { target, .. } if *target == outer_sym),
        );
        if next_overwrites {
            if is_new_binding {
                writeln!(out, "{}let mut {}: i64 = 0;", pad, outer_name).unwrap();
            }
        } else {
            // A tiled nest counts from 0 below its bound, so its counter leaves at the bound, or at 0 when it ran
            // no pass; a counter the nest reuses is assigned, one it declares is declared.
            let exit = format!("(0_i64).max({})", bound_str);
            if is_new_binding {
                writeln!(out, "{}let mut {}: i64 = {};", pad, outer_name, exit).unwrap();
            } else {
                writeln!(out, "{}{} = {};", pad, outer_name, exit).unwrap();
            }
        }
    }

    Some((out, 1))
}

/// Peephole optimization: detect `Let vec = new Seq. [Push val to vec.]* Let i = 0. While i <= limit: push const to vec, i = i+1`
/// and emit `let mut vec: Vec<T> = vec![const; total_count]` with prefix overrides.
///
/// Handles two patterns:
/// - Basic: `new Seq` → counter init → push-loop (existing)
/// - Extended: `new Seq` → 1+ prefix pushes → counter init → push-loop (coins DP pattern)
///
/// Returns (generated_code, number_of_extra_statements_consumed) or None if pattern doesn't match.
pub(crate) fn try_emit_vec_fill_pattern<'a>(
    stmts: &[&Stmt<'a>],
    idx: usize,
    interner: &Interner,
    indent: usize,
    ctx: &mut RefinementContext<'a>,
) -> Option<(String, usize)> {
    if idx + 2 >= stmts.len() {
        return None;
    }

    // Statement 1: Let [mutable] vec_var be a new Seq of T.
    // Note: mutable keyword is optional — mutability is inferred from Push in the loop body.
    let (vec_sym, elem_type, vec_is_mutable) = match stmts[idx] {
        Stmt::Let {
            var,
            value,
            ty,
            mutable,
        } => {
            // Check for explicit type annotation like `: Seq of Bool`
            let type_from_annotation = if let Some(TypeExpr::Generic { base, params }) = ty {
                let base_name = interner.resolve(*base);
                if matches!(base_name, "Seq" | "List" | "Vec") && !params.is_empty() {
                    Some(codegen_type_expr(&params[0], interner))
                } else {
                    None
                }
            } else {
                None
            };

            // Check for `a new Seq of T`
            let type_from_new = if let Expr::New {
                type_name,
                type_args,
                init_fields,
            } = value
            {
                let tn = interner.resolve(*type_name);
                if matches!(tn, "Seq" | "List" | "Vec") && init_fields.is_empty() {
                    if !type_args.is_empty() {
                        Some(codegen_type_expr(&type_args[0], interner))
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            };

            match type_from_annotation.or(type_from_new) {
                Some(t) => (*var, t, *mutable),
                None => return None,
            }
        }
        _ => return None,
    };

    // An affine read-only array is deleted (reads substitute the closed form), so
    // it must not be re-materialized as a `vec![..]` fill here.
    if ctx.affine_array(vec_sym).is_some() {
        return None;
    }

    // Scan for optional prefix Push statements: `Push const to vec`
    // These are elements pushed before the fill loop (e.g., `Push 1 to dp` in coins).
    let mut prefix_values: Vec<String> = Vec::new();
    let mut cursor = idx + 1;
    while cursor < stmts.len() {
        if let Stmt::Push { value, collection } = stmts[cursor] {
            if let Expr::Identifier(sym) = collection {
                if *sym == vec_sym {
                    let val_str = match value {
                        Expr::Literal(
                            lit @ (Literal::Number(_)
                            | Literal::Float(_)
                            | Literal::Boolean(_)
                            | Literal::Char(_)
                            | Literal::Text(_)),
                        ) => Some(super::expr::codegen_literal(lit, interner)),
                        _ => None,
                    };
                    if let Some(vs) = val_str {
                        prefix_values.push(vs);
                        cursor += 1;
                        continue;
                    }
                }
            }
        }
        break;
    }

    // Need at least 2 more statements: counter init + while loop
    if cursor + 1 >= stmts.len() {
        return None;
    }

    // Counter init: Let [mutable] counter = start_literal  OR  Set counter to start_literal
    let counter_is_new_binding = matches!(stmts[cursor], Stmt::Let { .. });
    let (counter_sym, counter_start) = match stmts[cursor] {
        Stmt::Let {
            var,
            value: Expr::Literal(Literal::Number(n)),
            ..
        } => (*var, *n),
        Stmt::Set {
            target,
            value: Expr::Literal(Literal::Number(n)),
        } => (*target, *n),
        _ => return None,
    };

    // While loop: counter <= limit (or counter < limit): Push const_val to vec_var. Set counter to counter + 1.
    match stmts[cursor + 1] {
        Stmt::While { cond, body, .. } => {
            // Check condition: counter <= limit OR counter < limit
            let (limit_expr, is_exclusive) = match cond {
                Expr::BinaryOp {
                    op: BinaryOpKind::LtEq,
                    left,
                    right,
                } => {
                    if let Expr::Identifier(sym) = left {
                        if *sym == counter_sym {
                            (Some(*right), false)
                        } else {
                            (None, false)
                        }
                    } else {
                        (None, false)
                    }
                }
                Expr::BinaryOp {
                    op: BinaryOpKind::Lt,
                    left,
                    right,
                } => {
                    if let Expr::Identifier(sym) = left {
                        if *sym == counter_sym {
                            (Some(*right), true)
                        } else {
                            (None, false)
                        }
                    } else {
                        (None, false)
                    }
                }
                _ => (None, false),
            };
            let limit_expr = limit_expr?;
            // The loop reads its limit every iteration; the fill reads it once, which is the same only
            // when the loop changes nothing the limit reads (neither the sequence it fills nor the counter).
            if !is_simple_expr(limit_expr)
                || symbol_appears_in_expr(vec_sym, limit_expr)
                || symbol_appears_in_expr(counter_sym, limit_expr)
            {
                return None;
            }

            // Body must have exactly 2 statements: Push and Set
            if body.len() != 2 {
                return None;
            }

            // First body stmt: Push const_val to vec_var
            let push_val = match &body[0] {
                Stmt::Push { value, collection } => {
                    if let Expr::Identifier(sym) = collection {
                        if *sym == vec_sym {
                            Some(*value)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
                _ => None,
            }?;

            // Push value must be a constant literal
            let fill_val_str = match push_val {
                Expr::Literal(
                    lit @ (Literal::Number(_)
                    | Literal::Float(_)
                    | Literal::Boolean(_)
                    | Literal::Char(_)
                    | Literal::Text(_)),
                ) => super::expr::codegen_literal(lit, interner),
                _ => return None,
            };

            // Second body stmt: Set counter to counter + 1
            match &body[1] {
                Stmt::Set { target, value, .. } => {
                    if *target != counter_sym {
                        return None;
                    }
                    // Value must be counter + 1
                    match value {
                        Expr::BinaryOp {
                            op: BinaryOpKind::Add,
                            left,
                            right,
                        } => {
                            let is_counter_plus_1 = match (left, right) {
                                (Expr::Identifier(s), Expr::Literal(Literal::Number(1)))
                                    if *s == counter_sym =>
                                {
                                    true
                                }
                                (Expr::Literal(Literal::Number(1)), Expr::Identifier(s))
                                    if *s == counter_sym =>
                                {
                                    true
                                }
                                _ => false,
                            };
                            if !is_counter_plus_1 {
                                return None;
                            }
                        }
                        _ => return None,
                    }
                }
                _ => return None,
            }

            // Pattern matched! Emit optimized code.
            let indent_str = "    ".repeat(indent);
            let vec_name = interner.resolve(vec_sym);
            let limit_str = codegen_expr_simple(limit_expr, interner);
            let prefix_count = prefix_values.len();

            // Calculate loop iteration count (without prefix)
            // Inclusive (<=): loop_count = limit - start + 1
            // Exclusive (<):  loop_count = limit - start
            let raw_loop_count = if is_exclusive {
                if counter_start == 0 {
                    limit_str.clone()
                } else {
                    format!("({} - {})", limit_str, counter_start)
                }
            } else {
                if counter_start == 0 {
                    format!("({} + 1)", limit_str)
                } else if counter_start == 1 {
                    limit_str.clone()
                } else {
                    format!("({} - {} + 1)", limit_str, counter_start)
                }
            };

            // Total count = prefix elements + loop iterations; a loop whose limit is below its start runs
            // no iterations.
            let count_expr = if prefix_count == 0 {
                format!("({}).max(0) as usize", raw_loop_count)
            } else {
                format!("({} + ({}).max(0)) as usize", prefix_count, raw_loop_count)
            };

            let mut output = String::new();
            let mut_kw = if vec_is_mutable { "mut " } else { "" };
            // O2 de-Rc: a Seq proven not to need reference semantics is a plain
            // `Vec<T>` (no Rc/RefCell); prefix overrides become direct stores.
            let de_rc = ctx.is_de_rc(vec_sym);
            // i64→i32 narrowing: a de-Rc'd Seq proven to hold only i32-range
            // values is `Vec<i32>` (the fill literal coerces; access sites convert).
            let narrow = de_rc && ctx.is_narrowed(vec_sym);
            let elem = if narrow { "i32" } else { elem_type.as_str() };
            let bits = ctx.is_bitpacked(vec_sym);
            if bits {
                writeln!(
                    output,
                    "{}let {}{}: logicaffeine_data::LogosBits = logicaffeine_data::LogosBits::filled({}, {});",
                    indent_str, mut_kw, vec_name, count_expr, fill_val_str
                )
                .unwrap();
                ctx.register_variable_type(vec_sym, "LogosBits".to_string());
            } else if de_rc {
                writeln!(
                    output,
                    "{}let {}{}: Vec<{}> = vec![{}; {}];",
                    indent_str, mut_kw, vec_name, elem, fill_val_str, count_expr
                )
                .unwrap();
                emit_narrow_guards(&mut output, vec_sym, ctx, &indent_str);
                ctx.register_variable_type(vec_sym, format!("Vec<{}>", elem));
            } else {
                writeln!(
                    output,
                    "{}let {}{}: LogosSeq<{}> = LogosSeq::from_vec(vec![{}; {}]);",
                    indent_str, mut_kw, vec_name, elem_type, fill_val_str, count_expr
                )
                .unwrap();
                ctx.register_variable_type(vec_sym, format!("LogosSeq<{}>", elem_type));
            }
            let narrow_prefix = narrow;

            // Emit prefix element overrides (only for values different from fill)
            for (i, prefix_val) in prefix_values.iter().enumerate() {
                if *prefix_val != fill_val_str {
                    // A narrowed buffer truncates the override (lossless by proof).
                    let pv = if narrow_prefix {
                        format!("({}) as i32", prefix_val)
                    } else {
                        prefix_val.clone()
                    };
                    if bits {
                        writeln!(
                            output,
                            "{}{}.set_item({}, {});",
                            indent_str,
                            vec_name,
                            i + 1,
                            pv
                        )
                        .unwrap();
                    } else if de_rc {
                        writeln!(output, "{}{}[{}] = {};", indent_str, vec_name, i, pv).unwrap();
                    } else {
                        writeln!(
                            output,
                            "{}{}.borrow_mut()[{}] = {};",
                            indent_str, vec_name, i, pv
                        )
                        .unwrap();
                    }
                }
            }

            // The counter as the loop leaves it: one past the limit, or its start when the loop never ran.
            let names = RustNames::new(interner);
            let counter_name = names.ident(counter_sym);
            let after = counter_after(&format!("{}_i64", counter_start), &limit_str, is_exclusive);
            if counter_is_new_binding {
                writeln!(
                    output,
                    "{}let mut {}: i64 = {};",
                    indent_str, counter_name, after
                )
                .unwrap();
            } else {
                writeln!(output, "{}{} = {};", indent_str, counter_name, after).unwrap();
            }

            // Extra consumed: prefix pushes + counter init + while loop
            let extra_consumed = (cursor - idx) + 1;
            Some((output, extra_consumed))
        }
        _ => None,
    }
}

/// Check if an expression can be handled by codegen_expr_simple without fallback.
pub(crate) fn is_simple_expr(expr: &Expr) -> bool {
    match expr {
        Expr::Literal(Literal::Number(_))
        | Expr::Literal(Literal::Float(_))
        | Expr::Literal(Literal::Boolean(_))
        | Expr::Literal(Literal::Text(_))
        | Expr::Identifier(_) => true,
        Expr::BinaryOp { op, left, right } => {
            matches!(
                op,
                BinaryOpKind::Add
                    | BinaryOpKind::Subtract
                    | BinaryOpKind::Multiply
                    | BinaryOpKind::Divide
                    | BinaryOpKind::Modulo
            ) && is_simple_expr(left)
                && is_simple_expr(right)
        }
        Expr::Length { collection } => {
            matches!(collection, Expr::Identifier(_))
        }
        _ => false,
    }
}

/// Simple expression codegen for peephole patterns (no async/context needed).
fn codegen_expr_simple(expr: &Expr, interner: &Interner) -> String {
    match expr {
        Expr::Literal(Literal::Number(n)) => format!("{}", n),
        Expr::Literal(Literal::Float(f)) => format!("{:?}", f),
        Expr::Literal(Literal::Boolean(b)) => format!("{}", b),
        // `{:?}` on a &str emits a fully-escaped Rust string literal token, so a Text-literal
        // chunk (e.g. center's `" "`) becomes `" "`, never the `_` sentinel. is_simple_expr agrees.
        Expr::Literal(Literal::Text(sym)) => format!("{:?}", interner.resolve(*sym)),
        Expr::Identifier(sym) => interner.resolve(*sym).to_string(),
        Expr::BinaryOp { op, left, right } => {
            let l = codegen_expr_simple(left, interner);
            let r = codegen_expr_simple(right, interner);
            let op_str = match op {
                BinaryOpKind::Add => "+",
                BinaryOpKind::Subtract => "-",
                BinaryOpKind::Multiply => "*",
                BinaryOpKind::Divide => "/",
                BinaryOpKind::Modulo => "%",
                _ => return format!("({})", l),
            };
            format!("({} {} {})", l, op_str, r)
        }
        Expr::Length { collection } => {
            if let Expr::Identifier(sym) = collection {
                format!("({}.len() as i64)", interner.resolve(*sym))
            } else {
                "_".to_string()
            }
        }
        _ => "_".to_string(),
    }
}

/// Peephole optimization: detect `Let mutable text be ""` followed by a counted
/// loop that self-appends to `text`, and emit `String::with_capacity(n as usize)`
/// instead of `String::from("")`.
///
/// Pattern:
///   Let mutable text be "".
///   Let counter be start.
///   While counter < limit: ... Set text to text + ...; counter++
///
/// The limit expression gives us the capacity hint. For exclusive (<), capacity = limit - start.
/// For inclusive (<=), capacity = limit - start + 1.
///
/// Returns (generated_code, number_of_extra_statements_consumed) or None.
pub(crate) fn try_emit_string_with_capacity_pattern<'a>(
    stmts: &[&Stmt<'a>],
    idx: usize,
    interner: &Interner,
    indent: usize,
    mutable_vars: &HashSet<Symbol>,
    ctx: &mut RefinementContext<'a>,
) -> Option<(String, usize)> {
    if idx + 2 >= stmts.len() {
        return None;
    }

    // Statement 1: Let mutable text be "".
    let str_sym = match stmts[idx] {
        Stmt::Let {
            var,
            value,
            mutable,
            ..
        } => {
            if !*mutable && !mutable_vars.contains(var) {
                return None;
            }
            if let Expr::Literal(Literal::Text(sym)) = value {
                if interner.resolve(*sym).is_empty() {
                    *var
                } else {
                    return None;
                }
            } else {
                return None;
            }
        }
        _ => return None,
    };

    // Scan forward for a counter-init + While pair where the loop body appends to str_sym.
    // Skip intervening statements that don't reference str_sym.
    for scan in (idx + 1)..stmts.len() {
        let stmt = stmts[scan];

        // Try: is this a counter init?
        let is_counter_init = match stmt {
            Stmt::Let { value, .. } if is_simple_expr(value) => true,
            Stmt::Set { value, .. } if is_simple_expr(value) => true,
            _ => false,
        };

        if is_counter_init && scan + 1 < stmts.len() {
            let (counter_sym, start_expr) = match stmt {
                Stmt::Let { var, value, .. } => (*var, *value),
                Stmt::Set { target, value } => (*target, *value),
                _ => unreachable!(),
            };

            if let Stmt::While { cond, body, .. } = stmts[scan + 1] {
                // Check condition shape: counter < limit or counter <= limit
                let (limit_expr, is_exclusive) = match cond {
                    Expr::BinaryOp {
                        op: BinaryOpKind::Lt,
                        left,
                        right,
                    } => {
                        if matches!(left, Expr::Identifier(sym) if *sym == counter_sym) {
                            (Some(*right), true)
                        } else {
                            (None, false)
                        }
                    }
                    Expr::BinaryOp {
                        op: BinaryOpKind::LtEq,
                        left,
                        right,
                    } => {
                        if matches!(left, Expr::Identifier(sym) if *sym == counter_sym) {
                            (Some(*right), false)
                        } else {
                            (None, false)
                        }
                    }
                    _ => (None, false),
                };

                if let Some(limit_expr) = limit_expr {
                    if !is_simple_expr(limit_expr) {
                        continue;
                    }

                    // Check body ends with counter increment
                    if body.len() >= 2 {
                        let last_is_increment = match &body[body.len() - 1] {
                            Stmt::Set { target, value } if *target == counter_sym => {
                                matches!(value, Expr::BinaryOp { op: BinaryOpKind::Add, left, right }
                                    if (matches!(left, Expr::Identifier(s) if *s == counter_sym) && matches!(right, Expr::Literal(Literal::Number(1))))
                                    || (matches!(left, Expr::Literal(Literal::Number(1))) && matches!(right, Expr::Identifier(s) if *s == counter_sym))
                                )
                            }
                            _ => false,
                        };

                        if last_is_increment {
                            // Check that loop body appends to str_sym (via Set self-append)
                            let body_appends = body_has_string_self_append(body, str_sym);
                            if body_appends {
                                // Pattern matched! Emit with_capacity.
                                let indent_str = "    ".repeat(indent);
                                let var_name = interner.resolve(str_sym);
                                let limit_str = codegen_expr_simple(limit_expr, interner);
                                let start_str = codegen_expr_simple(start_expr, interner);

                                let capacity_expr = if is_exclusive {
                                    match start_expr {
                                        Expr::Literal(Literal::Number(0)) => limit_str.clone(),
                                        Expr::Literal(Literal::Number(s)) => {
                                            if let Expr::Literal(Literal::Number(n)) = limit_expr {
                                                format!("{}", n - s)
                                            } else {
                                                format!("({} - {})", limit_str, s)
                                            }
                                        }
                                        _ => format!("({} - {})", limit_str, start_str),
                                    }
                                } else {
                                    match start_expr {
                                        Expr::Literal(Literal::Number(0)) => {
                                            if let Expr::Literal(Literal::Number(n)) = limit_expr {
                                                format!("{}", n + 1)
                                            } else {
                                                format!("({} + 1)", limit_str)
                                            }
                                        }
                                        Expr::Literal(Literal::Number(1)) => limit_str.clone(),
                                        Expr::Literal(Literal::Number(s)) => {
                                            if let Expr::Literal(Literal::Number(n)) = limit_expr {
                                                format!("{}", n - s + 1)
                                            } else {
                                                format!("({} - {} + 1)", limit_str, s)
                                            }
                                        }
                                        _ => format!("({} - {} + 1)", limit_str, start_str),
                                    }
                                };

                                let mut output = String::new();
                                writeln!(
                                    output,
                                    "{}let mut {} = String::with_capacity({} as usize);",
                                    indent_str, var_name, capacity_expr
                                )
                                .unwrap();

                                // Register as string var
                                ctx.register_string_var(str_sym);

                                // Now emit the remaining statements normally (counter init + while loop)
                                // via for-range pattern or fallback.
                                // We consumed 0 extra statements — only replaced the Let.
                                // The counter init + while will be processed by subsequent peephole passes.
                                return Some((output, 0));
                            }
                        }
                    }
                }
            }
        }

        // If this statement references str_sym, bail out.
        if symbol_appears_in_stmts(str_sym, &[stmt]) {
            return None;
        }
    }

    None
}

/// Check if a loop body contains a self-append to the string variable.
/// Looks for `Set str_sym to str_sym + ...` pattern anywhere in the body.
fn body_has_string_self_append(stmts: &[Stmt], str_sym: Symbol) -> bool {
    for stmt in stmts {
        match stmt {
            Stmt::Set { target, value } if *target == str_sym => {
                // Check for self-append: target + something
                if let Expr::BinaryOp {
                    op: BinaryOpKind::Add,
                    left,
                    ..
                } = value
                {
                    if matches!(left, Expr::Identifier(sym) if *sym == str_sym) {
                        return true;
                    }
                }
            }
            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
                if body_has_string_self_append(then_block, str_sym) {
                    return true;
                }
                if let Some(eb) = else_block {
                    if body_has_string_self_append(eb, str_sym) {
                        return true;
                    }
                }
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } => {
                if body_has_string_self_append(body, str_sym) {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

/// `codegen_expr_simple`, but a promoted (`LogosInt`) variable narrows to i64 via
/// `.expect_i64("Int")` — the write/swap-path mirror of the read path's index-context
/// narrow (`ExprCtx::int_index_context`). An index must fit i64 (collections are
/// memory-bounded), so the narrow is the same fail-loud contract the reads use.
fn codegen_index_operand(
    expr: &Expr,
    interner: &Interner,
    variable_types: &HashMap<Symbol, String>,
) -> String {
    match expr {
        Expr::Identifier(sym)
            if variable_types
                .get(sym)
                .map_or(false, |t| t.contains("__bigint")) =>
        {
            format!("{}.expect_i64(\"Int\")", interner.resolve(*sym))
        }
        Expr::BinaryOp { op, left, right } => {
            let l = codegen_index_operand(left, interner, variable_types);
            let r = codegen_index_operand(right, interner, variable_types);
            let op_str = match op {
                BinaryOpKind::Add => "+",
                BinaryOpKind::Subtract => "-",
                BinaryOpKind::Multiply => "*",
                BinaryOpKind::Divide => "/",
                BinaryOpKind::Modulo => "%",
                _ => return format!("({})", l),
            };
            format!("({} {} {})", l, op_str, r)
        }
        _ => codegen_expr_simple(expr, interner),
    }
}

/// Convert a LOGOS 1-based index expression to a Rust 0-based index string.
///
/// Algebraic simplifications:
///   Literal(1)      → "0"
///   Literal(N)      → "N-1"  (compile-time constant)
///   (X + 1)         → "X as usize"   (or just "X" if raw=true)
///   (1 + X)         → "X as usize"
///   (X + K)         → "(X + K-1) as usize"  where K is literal > 1
///   fallback        → "(expr - 1) as usize"
///
/// When `include_as_usize` is false, the result omits " as usize" (for use in `.swap()` calls
/// where the caller adds it). `variable_types` narrows promoted (`LogosInt`) variables inside
/// the index expression, matching the read path.
pub(crate) fn simplify_1based_index(
    expr: &Expr,
    interner: &Interner,
    include_as_usize: bool,
    variable_types: &HashMap<Symbol, String>,
) -> String {
    let cast = if include_as_usize { " as usize" } else { "" };

    // A rebased 0-based counter (`j`, `j ± k`) indexes directly: its rebase already
    // absorbed the 1-based `-1`, so skip the subtraction. Centralized here so every
    // peephole that formats an index (compare-swap, swaps, slice-copy, …) inherits
    // it consistently with the read/write sites in expr.rs / stmt.rs.
    if let Some(zb) = super::expr::zero_based_index_str(expr, variable_types, interner) {
        return format!("{}{}", zb, cast);
    }

    match expr {
        // Literal(1) → 0
        Expr::Literal(Literal::Number(1)) => "0".to_string(),
        // Literal(N) → N-1 (compile-time constant, no cast needed)
        Expr::Literal(Literal::Number(n)) => format!("{}", n - 1),
        // (X + K) where K is a literal
        Expr::BinaryOp {
            op: BinaryOpKind::Add,
            left,
            right,
        } => {
            match (left, right) {
                // (X + 1) → X
                (_, Expr::Literal(Literal::Number(1))) => {
                    let inner = codegen_index_operand(left, interner, variable_types);
                    if include_as_usize {
                        format!("({}){}", inner, cast)
                    } else {
                        inner
                    }
                }
                // (1 + X) → X
                (Expr::Literal(Literal::Number(1)), _) => {
                    let inner = codegen_index_operand(right, interner, variable_types);
                    if include_as_usize {
                        format!("({}){}", inner, cast)
                    } else {
                        inner
                    }
                }
                // (X + K) where K > 1 → (X + K-1)
                (_, Expr::Literal(Literal::Number(k))) if *k > 1 => {
                    let inner = codegen_index_operand(left, interner, variable_types);
                    format!("({} + {}){}", inner, k - 1, cast)
                }
                // (K + X) where K > 1 → (X + K-1)
                (Expr::Literal(Literal::Number(k)), _) if *k > 1 => {
                    let inner = codegen_index_operand(right, interner, variable_types);
                    format!("({} + {}){}", inner, k - 1, cast)
                }
                // Fallback: (expr - 1)
                _ => {
                    let full = codegen_index_operand(expr, interner, variable_types);
                    format!("({} - 1){}", full, cast)
                }
            }
        }
        // Fallback: (expr - 1)
        _ => {
            let full = codegen_index_operand(expr, interner, variable_types);
            format!("({} - 1){}", full, cast)
        }
    }
}

/// The 1-based index `expr` names, as the i64 `LogosItems::logos_item` takes: a rebased 0-based counter
/// with its 1 given back, anything else the index as written. [`simplify_1based_index`] folds an index
/// into an offset, which is sound only where the index is proved to name an item; every other access
/// formats its index here, so a negative index counts from the back and a miss fails in the words every
/// tier uses.
pub(crate) fn logos_index(
    expr: &Expr,
    interner: &Interner,
    variable_types: &HashMap<Symbol, String>,
) -> String {
    super::expr::rebased_logos_index(expr, variable_types, interner)
        .unwrap_or_else(|| codegen_index_operand(expr, interner, variable_types))
}

/// Comparison strength of a bounds hint: `bound < len` vs `bound <= len`.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum HintStrength {
    Lt,
    Le,
}

/// A planned bounds hint for one array in one loop. Emitted as a hoisted
/// length snapshot (`let __{arr}_bhl = arr.len() as i64;`) in the preheader
/// plus a `debug_assert!` + `assert_unchecked` pair at the top of the loop
/// body, so the hint never executes for a loop that never runs.
pub(crate) struct BoundsHintPlan {
    pub arr_sym: Symbol,
    pub bound_str: String,
    pub strength: HintStrength,
}

/// How a loop counter reaches an array, per the index-emission convention
/// (`item E of a` emits `a[E - 1]`, folding `item (c + 1)` to `a[c]`).
#[derive(Default, Clone, Copy)]
struct CounterAccessKinds {
    /// AST index `counter + 1` → emitted index = counter.
    direct: bool,
    /// AST index `counter` → emitted index = counter - 1.
    minus_one: bool,
}

fn record_index_shape(
    arr_sym: Symbol,
    index: &Expr,
    counter_sym: Symbol,
    order: &mut Vec<Symbol>,
    kinds: &mut HashMap<Symbol, CounterAccessKinds>,
) {
    let shape = match index {
        Expr::Identifier(s) if *s == counter_sym => Some(false),
        Expr::BinaryOp {
            op: BinaryOpKind::Add,
            left,
            right,
        } => match (left, right) {
            (Expr::Identifier(s), Expr::Literal(Literal::Number(1))) if *s == counter_sym => {
                Some(true)
            }
            (Expr::Literal(Literal::Number(1)), Expr::Identifier(s)) if *s == counter_sym => {
                Some(true)
            }
            _ => None,
        },
        _ => None,
    };
    if let Some(is_direct) = shape {
        let entry = kinds.entry(arr_sym).or_insert_with(|| {
            order.push(arr_sym);
            CounterAccessKinds::default()
        });
        if is_direct {
            entry.direct = true;
        } else {
            entry.minus_one = true;
        }
    }
}

fn classify_expr_accesses(
    expr: &Expr,
    counter_sym: Symbol,
    order: &mut Vec<Symbol>,
    kinds: &mut HashMap<Symbol, CounterAccessKinds>,
) {
    match expr {
        Expr::Index { collection, index } => {
            if let Expr::Identifier(sym) = collection {
                record_index_shape(*sym, index, counter_sym, order, kinds);
            }
            classify_expr_accesses(collection, counter_sym, order, kinds);
            classify_expr_accesses(index, counter_sym, order, kinds);
        }
        // The right operand of And/Or short-circuits — only the left side is
        // guaranteed to evaluate.
        Expr::BinaryOp {
            op: BinaryOpKind::And | BinaryOpKind::Or,
            left,
            ..
        } => {
            classify_expr_accesses(left, counter_sym, order, kinds);
        }
        Expr::BinaryOp { left, right, .. } => {
            classify_expr_accesses(left, counter_sym, order, kinds);
            classify_expr_accesses(right, counter_sym, order, kinds);
        }
        Expr::Not { operand } => classify_expr_accesses(operand, counter_sym, order, kinds),
        Expr::Length { collection } => {
            classify_expr_accesses(collection, counter_sym, order, kinds)
        }
        _ => {}
    }
}

/// Collect, per array, the counter-index shapes that are UNCONDITIONALLY
/// evaluated on every loop iteration. Accesses under `If` branches, nested
/// loops, or short-circuit right operands contribute nothing: loop
/// correctness does not force them to execute at the counter's maximum, so
/// no hint may be derived from them.
fn classify_counter_accesses(
    stmts: &[Stmt],
    counter_sym: Symbol,
) -> Vec<(Symbol, CounterAccessKinds)> {
    let mut order: Vec<Symbol> = Vec::new();
    let mut kinds: HashMap<Symbol, CounterAccessKinds> = HashMap::new();
    for stmt in stmts {
        match stmt {
            Stmt::Set { value, .. } | Stmt::Let { value, .. } => {
                classify_expr_accesses(value, counter_sym, &mut order, &mut kinds);
            }
            Stmt::Show { object, recipient } => {
                classify_expr_accesses(object, counter_sym, &mut order, &mut kinds);
                classify_expr_accesses(recipient, counter_sym, &mut order, &mut kinds);
            }
            Stmt::Push { value, .. } => {
                classify_expr_accesses(value, counter_sym, &mut order, &mut kinds);
            }
            Stmt::SetIndex {
                collection,
                index,
                value,
            } => {
                // The write target is itself an access at `index`.
                if let Expr::Identifier(sym) = collection {
                    record_index_shape(*sym, index, counter_sym, &mut order, &mut kinds);
                }
                classify_expr_accesses(index, counter_sym, &mut order, &mut kinds);
                classify_expr_accesses(value, counter_sym, &mut order, &mut kinds);
            }
            // An If condition evaluates on every iteration; its branches do not.
            Stmt::If { cond, .. } => {
                classify_expr_accesses(cond, counter_sym, &mut order, &mut kinds);
            }
            Stmt::RuntimeAssert { condition, .. } => {
                classify_expr_accesses(condition, counter_sym, &mut order, &mut kinds);
            }
            _ => {}
        }
    }
    order
        .into_iter()
        .map(|s| {
            let k = kinds[&s];
            (s, k)
        })
        .collect()
}

/// True if the block can exit a loop before the counter reaches its bound.
pub(crate) fn body_has_early_exit(stmts: &[Stmt]) -> bool {
    for stmt in stmts {
        match stmt {
            Stmt::Break | Stmt::Return { .. } => return true,
            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
                if body_has_early_exit(then_block) {
                    return true;
                }
                if let Some(el) = else_block {
                    if body_has_early_exit(el) {
                        return true;
                    }
                }
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } | Stmt::Zone { body, .. } => {
                if body_has_early_exit(body) {
                    return true;
                }
            }
            Stmt::Inspect { arms, .. } => {
                for arm in arms {
                    if body_has_early_exit(&arm.body) {
                        return true;
                    }
                }
            }
            _ => {}
        }
    }
    false
}

/// OPT-4/OPT-9: plan bounds hints for arrays indexed by a unit-stepping loop
/// counter.
///
/// Soundness contract: every emitted `assert_unchecked` predicate must be
/// TRUE in any program that does not panic. A hint for array `a` is derived
/// only from accesses that loop correctness forces to execute at the
/// counter's maximum value:
///   - the access is unconditional in the body (or in the loop condition),
///   - the counter takes every value up to its bound (unit increment, no
///     other counter writes — callers guarantee or check this),
///   - the bound is loop-invariant (callers guarantee or check this),
///   - nothing exits the loop early (checked here),
///   - the array is not resized or rebound in the body (the preheader
///     length snapshot would go stale).
///
/// Strength: with max counter value M and emitted index `counter` (direct)
/// or `counter - 1` (minus-one), the max index is M or M-1. For inclusive
/// loops M = limit; for exclusive and zero-based loops M = limit - 1:
///
///   inclusive + direct     → `limit <  len`   (the knapsack off-by-one fix)
///   inclusive + minus-one  → `limit <= len`
///   exclusive + direct     → `limit <= len`
///   exclusive + minus-one  → `limit - 1 <= len`
///   zero-based (raw index) → `limit <= len`
pub(crate) fn plan_bounds_hints(
    body: &[Stmt],
    counter_sym: Symbol,
    is_exclusive: bool,
    use_zero_based: bool,
    limit_expr: &Expr,
    limit_str: &str,
    variable_types: &HashMap<Symbol, String>,
) -> Vec<BoundsHintPlan> {
    if body_has_early_exit(body) {
        return Vec::new();
    }
    let mut plans = Vec::new();
    for (arr_sym, kinds) in classify_counter_accesses(body, counter_sym) {
        let base_ty = variable_types
            .get(&arr_sym)
            .map(|t| t.split("|__hl:").next().unwrap_or(t.as_str()));
        let qualifies = matches!(
            base_ty,
            Some(t) if t.starts_with("LogosSeq") || t.starts_with("Vec<")
                || t.starts_with("&[") || t.starts_with("&mut [")
        );
        if !qualifies {
            continue;
        }
        if body_resizes_collection(body, arr_sym) || body_modifies_var(body, arr_sym) {
            continue;
        }
        let (bound_str, strength) = if use_zero_based {
            // Range is 0..limit with raw indexing; max index = limit - 1.
            // A direct (`counter + 1`) access should not survive zero-basing;
            // refuse defensively if one does.
            if kinds.direct || !kinds.minus_one {
                continue;
            }
            (limit_str.to_string(), HintStrength::Le)
        } else if !is_exclusive {
            if kinds.direct {
                (limit_str.to_string(), HintStrength::Lt)
            } else {
                (limit_str.to_string(), HintStrength::Le)
            }
        } else if kinds.direct {
            (limit_str.to_string(), HintStrength::Le)
        } else {
            let folded = match limit_expr {
                Expr::Literal(Literal::Number(n)) => format!("{}", n - 1),
                _ => format!("({} - 1)", limit_str),
            };
            (folded, HintStrength::Le)
        };
        plans.push(BoundsHintPlan {
            arr_sym,
            bound_str,
            strength,
        });
    }
    plans
}

/// Emit the preheader length snapshots for planned bounds hints.
pub(crate) fn emit_bounds_hint_preheader(
    plans: &[BoundsHintPlan],
    interner: &Interner,
    indent_str: &str,
    output: &mut String,
) {
    let names = RustNames::new(interner);
    for p in plans {
        let arr = names.ident(p.arr_sym);
        let op = match p.strength {
            HintStrength::Lt => "<",
            HintStrength::Le => "<=",
        };
        writeln!(
            output,
            "{}let __{}_bhl = {}.len() as i64;",
            indent_str, arr, arr
        )
        .unwrap();
        writeln!(
            output,
            "{}let __{}_bh_ok = {} {} __{}_bhl;",
            indent_str, arr, p.bound_str, op, arr
        )
        .unwrap();
    }
}

/// Emit the in-loop assumption for each planned bounds hint, made only when the preheader found it true
/// (`__{arr}_bh_ok`). A program whose loop does index past the end therefore assumes nothing: every access
/// is checked and it fails at the iteration, and with the reason, it fails with on every other backend. The
/// flag is loop-invariant, so LLVM can unswitch the loop on it and keep the unchecked loop where it holds.
pub(crate) fn emit_bounds_hint_header(
    plans: &[BoundsHintPlan],
    interner: &Interner,
    indent_str: &str,
    output: &mut String,
) {
    let names = RustNames::new(interner);
    for p in plans {
        let arr = names.ident(p.arr_sym);
        let op = match p.strength {
            HintStrength::Lt => "<",
            HintStrength::Le => "<=",
        };
        writeln!(
            output,
            "{}if __{}_bh_ok {{ unsafe {{ std::hint::assert_unchecked({} {} __{}_bhl); }} }}",
            indent_str, arr, p.bound_str, op, arr
        )
        .unwrap();
    }
}

/// Version a loop on its bounds hints. `output[loop_start..]` is the loop as emitted, each hint's in-loop assumption
/// conditional on its `__{arr}_bh_ok` flag; it becomes `if <every flag> { <the loop, assuming each hint> } else { <the
/// loop, assuming nothing> }`, so the loop that runs where every hint holds is one LLVM reads unchecked without having
/// to unswitch it first (it did not, for counting_sort's checksum loop). Only a loop holding no hinted loop of its own,
/// and of bounded size, is versioned: nested versioning would double the code at every level. And only a loop with a
/// check for a hint to discharge ([`hints_discharge_a_check`]): where the Oracle proves every access of the hinted
/// sequences, the copy that assumes the hints is the copy that assumes nothing.
/// Whether some item access in `body` of a sequence `plans` hints is one the Oracle does not prove in range, and so
/// keeps a check a hint can discharge. Without the Oracle, or with its proofs not taken (`unchecked` or `oraclehints`
/// off), every access keeps its check.
pub(crate) fn hints_discharge_a_check(
    body: &[Stmt],
    plans: &[BoundsHintPlan],
    oracle: Option<&crate::optimize::OracleFacts>,
) -> bool {
    use logicaffeine_language::ast_depth::{walk_nodes, Visited, Walk};
    let config = crate::optimize::active_config();
    let Some(oracle) = oracle.filter(|_| {
        config.is_on(crate::optimization::Opt::Unchecked)
            && config.is_on(crate::optimization::Opt::OracleHints)
    }) else {
        return true;
    };
    let hinted: HashSet<Symbol> = plans.iter().map(|p| p.arr_sym).collect();
    let mut checked = false;
    walk_nodes(body, |node| {
        let access = match node {
            Visited::Expr(Expr::Index { collection, index }) => Some((*collection, *index)),
            Visited::Stmt(Stmt::SetIndex {
                collection, index, ..
            }) => Some((*collection, *index)),
            _ => None,
        };
        if let Some((collection, index)) = access {
            if matches!(collection, Expr::Identifier(s) if hinted.contains(s))
                && !oracle.index_provably_in_bounds(collection, index)
            {
                checked = true;
            }
        }
        Walk::Into
    });
    checked
}

pub(crate) fn version_on_bounds_hints(
    output: &mut String,
    loop_start: usize,
    plans: &[BoundsHintPlan],
    body: &[Stmt],
    oracle: Option<&crate::optimize::OracleFacts>,
    interner: &Interner,
    indent_str: &str,
) {
    const MAX_VERSIONED_LOOP: usize = 16 * 1024;
    if plans.is_empty()
        || !crate::optimize::active_config().is_on(crate::optimization::Opt::HintVersion)
        || !hints_discharge_a_check(body, plans, oracle)
    {
        return;
    }
    let names = RustNames::new(interner);
    let conditional: Vec<(String, String)> = plans
        .iter()
        .map(|p| {
            let arr = names.ident(p.arr_sym);
            let op = match p.strength {
                HintStrength::Lt => "<",
                HintStrength::Le => "<=",
            };
            let assume = format!(
                "unsafe {{ std::hint::assert_unchecked({} {} __{}_bhl); }}",
                p.bound_str, op, arr
            );
            (format!("if __{arr}_bh_ok {{ {assume} }}"), assume)
        })
        .collect();
    let loop_text = output[loop_start..].to_string();
    if loop_text.len() > MAX_VERSIONED_LOOP || loop_text.matches("_bh_ok").count() != plans.len() {
        return;
    }
    let mut assuming = loop_text.clone();
    let mut checked = String::new();
    for (conditional_line, assume) in &conditional {
        assuming = assuming.replacen(conditional_line, assume, 1);
    }
    for line in loop_text.lines() {
        if !conditional.iter().any(|(c, _)| line.trim() == c) {
            checked.push_str(line);
            checked.push('\n');
        }
    }
    let flags = plans
        .iter()
        .map(|p| format!("__{}_bh_ok", names.ident(p.arr_sym)))
        .collect::<Vec<_>>()
        .join(" && ");
    crate::optimize::mark_fired(crate::optimization::Opt::HintVersion);
    output.truncate(loop_start);
    output.push_str(&format!(
        "{indent_str}if {flags} {{\n{assuming}{indent_str}}} else {{\n{checked}{indent_str}}}\n"
    ));
}

/// Check if two expressions are structurally equal (for swap pattern detection, sentinel detection).
pub(crate) fn exprs_equal(a: &Expr, b: &Expr) -> bool {
    match (a, b) {
        (Expr::Identifier(s1), Expr::Identifier(s2)) => s1 == s2,
        (Expr::Literal(Literal::Number(n1)), Expr::Literal(Literal::Number(n2))) => n1 == n2,
        (
            Expr::BinaryOp {
                op: op1,
                left: l1,
                right: r1,
            },
            Expr::BinaryOp {
                op: op2,
                left: l2,
                right: r2,
            },
        ) => op1 == op2 && exprs_equal(l1, l2) && exprs_equal(r1, r2),
        _ => false,
    }
}

/// Peephole optimization: detect swap patterns and emit `arr.swap()` instead.
///
/// Pattern A (conditional, adjacent indices — bubble sort):
///   Let a be item j of arr. Let b be item (j+1) of arr.
///   If a > b then: Set item j of arr to b. Set item (j+1) of arr to a.
/// → `if arr[j-1] > arr[j] { arr.swap(j-1, j); }`
///
/// Pattern B (unconditional, any indices — quicksort/heapsort):
///   Let tmp be item I of arr.
///   Set item I of arr to item J of arr.
///   Set item J of arr to tmp.
/// → `arr.swap((I-1) as usize, (J-1) as usize);`
///
/// Returns (generated_code, number_of_extra_statements_consumed) or None.
/// The `bcmp` idiom: a fixed-window byte-compare loop
/// ```text
/// While j < LEN:
///     If item (a + j) of TEXT is not item (b + j) of NEEDLE:
///         Set FLAG to V.
///         Set j to LEN.        # break
///     Set j to j + 1.
/// ```
/// is exactly `FLAG := V unless TEXT[a-1 .. a-1+LEN] == NEEDLE[b-1 .. b-1+LEN]`.
/// Emit the single slice inequality `if &TEXT.as_bytes()[..] != &NEEDLE.as_bytes()[..]
/// { FLAG = V }` — LLVM lowers it to `bcmp`, replacing the scalar byte loop +
/// branch. Semantics-preserving: the flag is left untouched when the windows are
/// equal (the loop never enters the `if`), set to `V` on the first mismatch.
pub(crate) struct ByteCompareWindow<'a> {
    pub text: Symbol,
    pub needle: Symbol,
    pub flag: Symbol,
    pub text_collection: &'a Expr<'a>,
    pub needle_collection: &'a Expr<'a>,
    pub text_index: &'a Expr<'a>,
    pub needle_index: &'a Expr<'a>,
    pub text_base: &'a Expr<'a>,
    pub needle_base: &'a Expr<'a>,
    pub len: &'a Expr<'a>,
    pub flag_value: &'a Expr<'a>,
    pub const_len: Option<i64>,
}

/// Target-neutral recognition for [`try_emit_byte_compare_window`].  Rust and
/// C consume this same proof object; a backend may choose different syntax,
/// but it cannot silently recognize a weaker/stronger source shape.
pub(crate) fn match_byte_compare_window<'a>(
    stmt: &'a Stmt<'a>,
    interner: &Interner,
    oracle: Option<&crate::optimize::OracleFacts>,
) -> Option<ByteCompareWindow<'a>> {
    let Stmt::While { cond, body, .. } = stmt else {
        return None;
    };
    let Expr::BinaryOp {
        op: BinaryOpKind::Lt,
        left: j_e,
        right: len_e,
    } = cond
    else {
        return None;
    };
    let Expr::Identifier(j) = &**j_e else {
        return None;
    };
    let j = *j;
    if body.len() != 2 || !is_increment_by_one_of(&body[1], j) {
        return None;
    }
    let Stmt::If {
        cond: mism,
        then_block,
        else_block: None,
    } = &body[0]
    else {
        return None;
    };
    let Expr::BinaryOp {
        op: BinaryOpKind::NotEq,
        left: t_e,
        right: n_e,
    } = mism
    else {
        return None;
    };
    let (text, t_coll, t_idx) = index_into(t_e)?;
    let (needle, n_coll, n_idx) = index_into(n_e)?;
    let base_t = affine_base_in_j(t_idx, j)?;
    let base_n = affine_base_in_j(n_idx, j)?;
    let const_len = oracle
        .and_then(|o| o.expr_int_range(len_e))
        .and_then(|(lo, hi)| (lo == hi && lo > 0 && lo <= 16).then_some(lo));
    let len_src = const_len
        .map(|k| k.to_string())
        .unwrap_or_else(|| codegen_expr_simple(len_e, interner));
    let mut flag = None;
    let mut saw_break = false;
    for s in then_block.iter() {
        match s {
            Stmt::Break => saw_break = true,
            Stmt::Set { target, value } if *target == j => {
                if codegen_expr_simple(value, interner) != len_src {
                    return None;
                }
                saw_break = true;
            }
            Stmt::Set { target, value } if is_simple_expr(value) => {
                if flag.replace((*target, *value)).is_some() {
                    return None;
                }
            }
            _ => return None,
        }
    }
    let (flag, flag_value) = flag?;
    saw_break.then_some(ByteCompareWindow {
        text,
        needle,
        flag,
        text_collection: t_coll,
        needle_collection: n_coll,
        text_index: t_idx,
        needle_index: n_idx,
        text_base: base_t,
        needle_base: base_n,
        len: len_e,
        flag_value,
        const_len,
    })
}

pub(crate) fn try_emit_byte_compare_window(
    stmt: &Stmt,
    interner: &Interner,
    indent: usize,
    variable_types: &HashMap<Symbol, String>,
    oracle: Option<&crate::optimize::OracleFacts>,
) -> Option<String> {
    let m = match_byte_compare_window(stmt, interner, oracle)?;
    if !is_byte_indexable(variable_types.get(&m.text))
        || !is_byte_indexable(variable_types.get(&m.needle))
    {
        return None;
    }
    let len_str = match m.const_len {
        Some(k) => k.to_string(),
        None => codegen_expr_simple(m.len, interner),
    };
    let names = RustNames::new(interner);
    let text_n = names.ident(m.text);
    let needle_n = names.ident(m.needle);
    let flag_n = names.ident(m.flag);
    let bt = codegen_expr_simple(m.text_base, interner);
    let bn = codegen_expr_simple(m.needle_base, interner);
    let fv = codegen_expr_simple(m.flag_value, interner);
    let ind = "    ".repeat(indent);
    let ind1 = "    ".repeat(indent + 1);
    // Byte index = `item` index - 1 (1-based to 0-based). Window length = LEN.
    let mut out = String::new();
    // Carry the window's bounds proof forward: when the oracle proved the
    // PER-BYTE accesses in range (the BCE it would have hinted on the scalar
    // loop), the whole window `[base-1 .. base-1+LEN)` is in bounds, so the
    // slice indexing is too — `assert_unchecked` it so LLVM elides the slice
    // bound check (and the access stays as cheap as the elided byte loop was).
    let proven =
        |o: &crate::optimize::OracleFacts, coll, idx| o.index_provably_in_bounds(coll, idx);
    if oracle.map_or(false, |o| proven(o, m.text_collection, m.text_index)) {
        writeln!(
            out,
            "{ind}unsafe {{ std::hint::assert_unchecked((({bt}) - 1) >= 0 && ((({bt}) - 1) + ({len_str})) <= ({text_n}.len() as i64)); }}"
        )
        .unwrap();
    }
    if oracle.map_or(false, |o| proven(o, m.needle_collection, m.needle_index)) {
        writeln!(
            out,
            "{ind}unsafe {{ std::hint::assert_unchecked((({bn}) - 1) >= 0 && ((({bn}) - 1) + ({len_str})) <= ({needle_n}.len() as i64)); }}"
        )
        .unwrap();
    }
    if let Some(k) = m.const_len {
        // Constant window: emit C's UNROLLED element-wise byte compare — K byte
        // loads ANDed, short-circuiting. No `bcmp` call, no slice-eq machinery;
        // the `assert_unchecked` hints above make each index a bare load, so this
        // is exactly C's `cmpb`/`jne` chain.
        let terms: Vec<String> = (0..k)
            .map(|off| {
                format!(
                    "{text_n}.as_bytes()[(({bt}) - 1 + {off}) as usize] == {needle_n}.as_bytes()[(({bn}) - 1 + {off}) as usize]"
                )
            })
            .collect();
        writeln!(out, "{ind}if !({}) {{", terms.join(" && ")).unwrap();
    } else {
        writeln!(
            out,
            "{ind}if &{text_n}.as_bytes()[(({bt}) - 1) as usize..((({bt}) - 1) + ({len_str})) as usize] != &{needle_n}.as_bytes()[(({bn}) - 1) as usize..((({bn}) - 1) + ({len_str})) as usize] {{"
        )
        .unwrap();
    }
    writeln!(out, "{ind1}{flag_n} = {fv};").unwrap();
    writeln!(out, "{ind}}}").unwrap();
    Some(out)
}

/// The naive substring-search idiom, recognized as an OVERLAPPING occurrence
/// count and lowered to one call into the SIMD kernel `__logos_count_window_
/// matches` (emitted into the program by `program.rs` via [`RUNTIME_SRC`]):
///
/// ```text
/// Let count be 0.
/// Let i be 1.
/// While i <= TEXTLEN - M + 1:          (or `< …`)
///     Let match be 1.
///     Let j be 0.
///     While j < M:                      (fixed-window byte compare; the bcmp shape)
///         If item (i + j) of text is not item (j + 1) of needle:
///             Set match to 0.
///             Set j to M.               (break)
///         Set j to j + 1.
///     If match equals 1:
///         Set count to count + 1.
///     Set i to i + 1.
/// ```
///
/// The match is captured as owned data so both the prelude pre-scan
/// ([`stmts_contain_naive_search`]) and the call-site emitter
/// ([`try_emit_naive_search`]) share one matcher; the pre-scan is therefore a
/// superset of the emitter, so the kernel is never called without being emitted.
struct NaiveSearch {
    count: Symbol,
    i: Symbol,
    text: Symbol,
    needle: Symbol,
    i_init: i64,
    inclusive: bool,
    hi_src: String,
    m: i64,
}

/// Most recent constant bound to `e` before `before`: a literal, or an
/// identifier whose last `Let`/`Set` before the loop is a number literal.
fn resolve_const_before(e: &Expr, stmts: &[&Stmt], before: usize) -> Option<i64> {
    if let Expr::Literal(Literal::Number(n)) = e {
        return Some(*n);
    }
    let Expr::Identifier(sym) = e else {
        return None;
    };
    let mut found = None;
    for s in &stmts[..before] {
        match s {
            Stmt::Let { var, value, .. } if var == sym => {
                if let Expr::Literal(Literal::Number(n)) = value {
                    found = Some(*n);
                } else {
                    found = None; // reassigned to a non-constant
                }
            }
            Stmt::Set { target, value } if target == sym => {
                if let Expr::Literal(Literal::Number(n)) = value {
                    found = Some(*n);
                } else {
                    found = None;
                }
            }
            _ => {}
        }
    }
    found
}

/// Byte length of `needle` when it is bound to a string literal before `before`
/// and never reassigned (so the literal's bytes are exactly what the loop
/// compares against). `None` if non-literal or reassigned.
fn needle_literal_len(
    needle: Symbol,
    stmts: &[&Stmt],
    before: usize,
    interner: &Interner,
) -> Option<usize> {
    let mut found = None;
    for s in &stmts[..before] {
        match s {
            Stmt::Let { var, value, .. } if *var == needle => {
                if let Expr::Literal(Literal::Text(sym)) = value {
                    found = Some(interner.resolve(*sym).len());
                } else {
                    return None;
                }
            }
            Stmt::Set { target, .. } if *target == needle => return None,
            _ => {}
        }
    }
    found
}

/// Match the naive-search nest anchored at `stmts[idx]` (the `count` init).
/// Pure structure + constant resolution — no codegen `ctx` needed, so the
/// prelude pre-scan can reuse it.
fn match_naive_search(stmts: &[&Stmt], idx: usize, interner: &Interner) -> Option<NaiveSearch> {
    if idx + 2 >= stmts.len() {
        return None;
    }
    // s0: `Let count be 0`.
    let Stmt::Let {
        var: count,
        value: c0,
        ..
    } = stmts[idx]
    else {
        return None;
    };
    if !matches!(c0, Expr::Literal(Literal::Number(0))) {
        return None;
    }
    let count = *count;
    // s1: `Let i be <literal>`.
    let Stmt::Let {
        var: i, value: iv, ..
    } = stmts[idx + 1]
    else {
        return None;
    };
    let Expr::Literal(Literal::Number(i_init)) = iv else {
        return None;
    };
    let (i, i_init) = (*i, *i_init);
    // s2: `While i </<= HI { body }`.
    let Stmt::While { cond, body, .. } = stmts[idx + 2] else {
        return None;
    };
    let (inclusive, hi) = match cond {
        Expr::BinaryOp {
            op: BinaryOpKind::LtEq,
            left,
            right,
        } if matches!(left, Expr::Identifier(s) if *s == i) => (true, *right),
        Expr::BinaryOp {
            op: BinaryOpKind::Lt,
            left,
            right,
        } if matches!(left, Expr::Identifier(s) if *s == i) => (false, *right),
        _ => return None,
    };
    if body.len() != 5 {
        return None;
    }
    // body[0]: `Let match be 1`.
    let Stmt::Let {
        var: match_var,
        value: m1,
        ..
    } = &body[0]
    else {
        return None;
    };
    if !matches!(m1, Expr::Literal(Literal::Number(1))) {
        return None;
    }
    let match_var = *match_var;
    // body[1]: `Let j be 0`.
    let Stmt::Let {
        var: j, value: j0, ..
    } = &body[1]
    else {
        return None;
    };
    if !matches!(j0, Expr::Literal(Literal::Number(0))) {
        return None;
    }
    let j = *j;
    // body[2]: the inner fixed-window byte compare (bcmp shape).
    let Stmt::While {
        cond: inner,
        body: inner_body,
        ..
    } = &body[2]
    else {
        return None;
    };
    let Expr::BinaryOp {
        op: BinaryOpKind::Lt,
        left: j_e,
        right: m_expr,
    } = inner
    else {
        return None;
    };
    if !matches!(j_e, Expr::Identifier(s) if *s == j) {
        return None;
    }
    if inner_body.len() != 2 || !is_increment_by_one_of(&inner_body[1], j) {
        return None;
    }
    let Stmt::If {
        cond: mism,
        then_block,
        else_block: None,
    } = &inner_body[0]
    else {
        return None;
    };
    let Expr::BinaryOp {
        op: BinaryOpKind::NotEq,
        left: t_e,
        right: n_e,
    } = mism
    else {
        return None;
    };
    let (text, _tc, t_idx) = index_into(t_e)?;
    let (needle, _nc, n_idx) = index_into(n_e)?;
    let base_t = affine_base_in_j(t_idx, j)?;
    let base_n = affine_base_in_j(n_idx, j)?;
    // Window start tracks the outer counter (`text[i-1+k]`) and the needle is
    // compared from its start (`needle[0+k]`).
    if !matches!(base_t, Expr::Identifier(s) if *s == i) {
        return None;
    }
    if !matches!(base_n, Expr::Literal(Literal::Number(1))) {
        return None;
    }
    // then-block: exactly a flag write (`match = …`) and a break (`Break`, or
    // `Set j to M` with the SAME bound expr so the loop provably exits).
    let m_src = codegen_expr_simple(m_expr, interner);
    let (mut saw_flag, mut saw_break) = (false, false);
    for s in then_block.iter() {
        match s {
            Stmt::Break => saw_break = true,
            Stmt::Set { target, value } if *target == j => {
                if codegen_expr_simple(value, interner) != m_src {
                    return None;
                }
                saw_break = true;
            }
            Stmt::Set { target, .. } if *target == match_var => saw_flag = true,
            _ => return None,
        }
    }
    if !saw_flag || !saw_break {
        return None;
    }
    // body[3]: `If match equals 1 { count = count + 1 }`.
    let Stmt::If {
        cond: mc,
        then_block: inc,
        else_block: None,
    } = &body[3]
    else {
        return None;
    };
    match mc {
        Expr::BinaryOp {
            op: BinaryOpKind::Eq,
            left,
            right,
        } if matches!(left, Expr::Identifier(s) if *s == match_var)
            && matches!(right, Expr::Literal(Literal::Number(1))) => {}
        _ => return None,
    }
    if inc.len() != 1 || !is_increment_by_one_of(&inc[0], count) {
        return None;
    }
    // body[4]: `Set i to i + 1`.
    if !is_increment_by_one_of(&body[4], i) {
        return None;
    }
    // The window length `M` must resolve to a constant within the needle's known
    // length, so the emitted `needle.as_bytes()[..M]` slice can never panic
    // where the original short-circuiting loop would not.
    let m = resolve_const_before(m_expr, stmts, idx)?;
    if m <= 0 {
        return None;
    }
    let needle_len = needle_literal_len(needle, stmts, idx, interner)?;
    if (m as usize) > needle_len {
        return None;
    }
    if text == needle || i_init < 1 {
        return None;
    }
    Some(NaiveSearch {
        count,
        i,
        text,
        needle,
        i_init,
        inclusive,
        hi_src: codegen_expr_simple(hi, interner),
        m,
    })
}

/// Emit the kernel call for a recognized naive-search nest, consuming the three
/// statements `[count=0, i=init, While{…}]`. Returns `(code, extra_consumed)`.
pub(crate) fn try_emit_naive_search(
    stmts: &[&Stmt],
    idx: usize,
    interner: &Interner,
    indent: usize,
    ctx: &mut RefinementContext,
) -> Option<(String, usize)> {
    if !crate::optimize::active_config().is_on(crate::optimization::Opt::Simd) {
        return None;
    }
    let m = match_naive_search(stmts, idx, interner)?;
    // text/needle must be byte-indexable (String/&str) — `.as_bytes()` is valid.
    let vt = ctx.get_variable_types();
    if !is_byte_indexable(vt.get(&m.text)) || !is_byte_indexable(vt.get(&m.needle)) {
        return None;
    }
    // The loop counter `i` (and body-locals) must not be read after the nest.
    if value_read_after(m.i, stmts, idx + 3) {
        return None;
    }
    let names = RustNames::new(interner);
    let text_n = names.ident(m.text);
    let needle_n = names.ident(m.needle);
    let count_n = names.ident(m.count);
    // 0-based start positions: `p = i - 1` for `i` in [i_init, HI] (inclusive)
    // or [i_init, HI) (exclusive). Half-open `[start, end)`:
    let start = m.i_init - 1;
    let end_src = if m.inclusive {
        m.hi_src.clone()
    } else {
        format!("({}) - 1", m.hi_src)
    };
    let ind = "    ".repeat(indent);
    let mut out = String::new();
    writeln!(out, "{ind}let mut {count_n} = 0i64;").unwrap();
    writeln!(
        out,
        "{ind}{count_n} += __logos_count_window_matches({text_n}.as_bytes(), &{needle_n}.as_bytes()[..{m_len}], ({start}) as usize, ({end_src}) as usize);",
        m_len = m.m
    )
    .unwrap();
    ctx.register_variable_type(m.count, "i64".to_string());
    crate::optimize::mark_fired(crate::optimization::Opt::Simd);
    Some((out, 2))
}

/// True if any statement list reachable from `stmts` contains the naive-search
/// nest. Drives the one-time emission of [`RUNTIME_SRC`] into the program's
/// prelude (mirrors `program_uses_count_ones`). A superset of the call-site
/// emitter's matches, so the kernel is always present when a call is emitted.
pub(crate) fn stmts_contain_naive_search(stmts: &[Stmt], interner: &Interner) -> bool {
    let refs: Vec<&Stmt> = stmts.iter().collect();
    for idx in 0..refs.len() {
        if match_naive_search(&refs, idx, interner).is_some() {
            return true;
        }
    }
    stmts.iter().any(|s| match s {
        Stmt::While { body, .. } | Stmt::Repeat { body, .. } | Stmt::FunctionDef { body, .. } => {
            stmts_contain_naive_search(body, interner)
        }
        Stmt::If {
            then_block,
            else_block,
            ..
        } => {
            stmts_contain_naive_search(then_block, interner)
                || matches!(else_block, Some(e) if stmts_contain_naive_search(e, interner))
        }
        _ => false,
    })
}

/// The canonical statement-sequence peephole chain, shared by every block
/// context (main, function bodies, loop bodies, if/else arms). Tries each
/// recognizer in priority order and returns the first `(code, extra_consumed)`,
/// or `None` to fall through to ordinary statement codegen. Centralizing the
/// chain here means a new idiom is added in exactly one place and fires
/// uniformly everywhere, instead of being pasted into each block site.
#[allow(clippy::too_many_arguments)]
/// Opt::TextBytesView — a `Let sb be text_bytes(s)` binding used ONLY for reads lowers to a
/// zero-copy `s.as_bytes()` (`&[u8]`) instead of a `Vec<i64>` byte-list. Fires when:
///   - `sb` is an immutable `Let` bound to `text_bytes(<Identifier s>)`;
///   - every later use of `sb` is a view-compatible read (`text_bytes_view_safe`);
///   - `s` is NOT used again after the binding — so borrowing it (`&[u8]` of the owned `String`)
///     for the rest of the function is lifetime-safe (the source is never moved/mutated under the
///     view). The whole-function correctness lock catches any residual borrow error loudly.
/// `sb`'s type becomes the `"TextBytes"` marker so `item`/`length`/iteration read the slice and
/// widen each byte back to `i64`.
fn try_emit_text_bytes_view<'a>(
    stmts: &[&Stmt<'a>],
    idx: usize,
    interner: &Interner,
    indent: usize,
    ctx: &mut RefinementContext<'a>,
) -> Option<(String, usize)> {
    if !crate::optimize::active_config().is_on(crate::optimization::Opt::TextBytesView) {
        return None;
    }
    let Stmt::Let {
        var: sb,
        value,
        mutable,
        ..
    } = stmts[idx]
    else {
        return None;
    };
    if *mutable {
        return None;
    }
    let sb = *sb;
    let Expr::Call { function, args } = value else {
        return None;
    };
    if interner.resolve(*function) != "text_bytes" || args.len() != 1 {
        return None;
    }
    let Expr::Identifier(s) = args[0] else {
        return None;
    };
    let s = *s;
    if s == sb {
        return None;
    }
    let remaining = &stmts[idx + 1..];
    // `s` is a borrowed `&str` param (e.g. under Opt::TextSliceReturn) → a shared re-borrow
    // (`s.as_bytes()`) coexists with any later shared use, so the dead-after guard (which protects
    // moving an OWNED String) does not apply.
    let s_is_borrowed_str = ctx
        .get_variable_types()
        .get(&s)
        .map_or(false, |t| t.starts_with("&str") || t.starts_with("&'a str"));
    // For a slice-return fn, the build-loop tail (`Let out … Return text_from_bytes(out)`) is
    // ELIMINATED by TextSliceReturn (which dispatches before any seq-copy peephole), so only the lo/hi
    // SCANS remain — check view-read-only on the scans, not the doomed build tail (whose
    // `Push item i of sb to out` would otherwise (over-conservatively) block the view).
    let check_end = if ctx.str_slice_source().is_some() && s_is_borrowed_str {
        (0..remaining.len().saturating_sub(3))
            .find(|&j| {
                match_slice_builder_at(
                    remaining[j],
                    remaining[j + 1],
                    remaining[j + 2],
                    remaining[j + 3],
                    sb,
                    interner,
                )
                .is_some()
            })
            .unwrap_or(remaining.len())
    } else {
        remaining.len()
    };
    if !remaining[..check_end]
        .iter()
        .all(|st| stmt_view_read_only(st, sb))
    {
        return None;
    }
    if !s_is_borrowed_str && super::detection::symbol_appears_in_stmts(s, remaining) {
        return None;
    }
    ctx.register_variable_type(sb, "TextBytes".to_string());
    crate::optimize::mark_fired(crate::optimization::Opt::TextBytesView);
    let names = RustNames::new(interner);
    let ind = "    ".repeat(indent);
    let code = format!(
        "{}let {} = ({}).as_bytes();\n",
        ind,
        names.ident(sb),
        names.ident(s)
    );
    // `skip` is the count of statements consumed BEYOND `stmts[idx]` (the loop fuses
    // `idx..=idx+skip`); this pattern rewrites only the `Let sb` binding itself, so 0.
    Some((code, 0))
}

/// Every occurrence of `out` in `e` is EITHER the sole argument of a `text_from_bytes(out)` call
/// (the allowed consumption) or absent. A bare `out`, `item i of out`, `out + k`, etc. — anything
/// else — makes it unsafe (returns false). Conservatively complete.
fn expr_bb_ok(e: &Expr, out: Symbol, interner: &Interner) -> bool {
    match e {
        Expr::Call { function, args }
            if interner.resolve(*function) == "text_from_bytes"
                && args.len() == 1
                && matches!(args[0], Expr::Identifier(s) if *s == out) =>
        {
            true
        }
        Expr::Call { args, .. } => args.iter().all(|a| expr_bb_ok(a, out, interner)),
        // Any OTHER expression form is builder-safe only if `out` does not appear in it at all.
        _ => !super::detection::symbol_appears_in_expr(out, e),
    }
}

/// `out` is used ONLY as a byte builder: as the collection of a `Push <byte> to out` (the byte
/// value must not itself mention `out`) or as the sole argument of `text_from_bytes(out)`. ANY
/// other use — a read (`item`/`length`/`for`), `Set out`, `SetIndex`/`Pop`/`Add`/`Remove`/`Swap`/
/// `Give` of `out`, passing `out` bare to a call, re-`Let` — needs the real `LogosSeq<i64>`, so the
/// analysis is conservatively COMPLETE (any construct it cannot prove forces the fallback → false).
/// Twin of `text_bytes_view_safe` (read side).
pub(crate) fn byte_builder_safe(stmts: &[Stmt], out: Symbol, interner: &Interner) -> bool {
    stmts.iter().all(|s| stmt_bb_safe(s, out, interner))
}

fn stmt_bb_safe(s: &Stmt, out: Symbol, interner: &Interner) -> bool {
    match s {
        // The ONE allowed statement naming `out` as target: pushing a byte value.
        Stmt::Push {
            value,
            collection: Expr::Identifier(c),
        } if *c == out => !super::detection::symbol_appears_in_expr(out, value),
        // Pushing/adding/removing to ANOTHER collection: `out` must not ride along.
        Stmt::Push { value, collection }
        | Stmt::Add { value, collection }
        | Stmt::Remove { value, collection } => {
            !super::detection::symbol_appears_in_expr(out, value)
                && !super::detection::symbol_appears_in_expr(out, collection)
        }
        Stmt::Let { var, value, .. } => *var != out && expr_bb_ok(value, out, interner),
        Stmt::Set { target, value } => *target != out && expr_bb_ok(value, out, interner),
        Stmt::Return { value } => value.map_or(true, |v| expr_bb_ok(v, out, interner)),
        Stmt::Break => true,
        Stmt::RuntimeAssert { condition, .. } => {
            !super::detection::symbol_appears_in_expr(out, condition)
        }
        Stmt::Show { object, recipient } => {
            expr_bb_ok(object, out, interner)
                && !super::detection::symbol_appears_in_expr(out, recipient)
        }
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => {
            !super::detection::symbol_appears_in_expr(out, cond)
                && byte_builder_safe(then_block, out, interner)
                && else_block
                    .as_ref()
                    .map_or(true, |e| byte_builder_safe(e, out, interner))
        }
        Stmt::While {
            cond,
            body,
            decreasing,
        } => {
            !super::detection::symbol_appears_in_expr(out, cond)
                && decreasing.map_or(true, |d| !super::detection::symbol_appears_in_expr(out, d))
                && byte_builder_safe(body, out, interner)
        }
        Stmt::Repeat {
            pattern,
            iterable,
            body,
        } => {
            !pattern_binds(pattern, out)
                && !super::detection::symbol_appears_in_expr(out, iterable)
                && byte_builder_safe(body, out, interner)
        }
        Stmt::Zone { body, .. }
        | Stmt::Splice { body }
        | Stmt::Reversibly { body }
        | Stmt::During { body, .. } => byte_builder_safe(body, out, interner),
        // Everything else (SetIndex/Pop/Swap/Give/SetField/Call/Inspect/defs/…): safe only if
        // `out` does not appear at all.
        other => !super::detection::symbol_appears_in_stmt(out, other),
    }
}

/// A statement that DIRECTLY consumes `out` via `text_from_bytes(out)` (in a `Return`, `Let`, or
/// `Set` value, possibly inside a straight-line block). The witness that this Seq is a byte builder.
fn stmt_consumes_tfb(s: &Stmt, out: Symbol, interner: &Interner) -> bool {
    let is_tfb = |e: &Expr| {
        matches!(e, Expr::Call { function, args }
        if interner.resolve(*function) == "text_from_bytes"
            && args.len() == 1
            && matches!(args[0], Expr::Identifier(x) if *x == out))
    };
    match s {
        Stmt::Return { value: Some(v) } => is_tfb(v),
        Stmt::Let { value, .. } => is_tfb(value),
        Stmt::Set { value, .. } => is_tfb(value),
        Stmt::If {
            then_block,
            else_block,
            ..
        } => {
            then_block
                .iter()
                .any(|s| stmt_consumes_tfb(s, out, interner))
                || else_block.as_ref().map_or(false, |e| {
                    e.iter().any(|s| stmt_consumes_tfb(s, out, interner))
                })
        }
        Stmt::While { body, .. } | Stmt::Repeat { body, .. } | Stmt::During { body, .. } => {
            body.iter().any(|s| stmt_consumes_tfb(s, out, interner))
        }
        _ => false,
    }
}

/// Opt::RadixDigitBuffer — the base-N integer→text idiom lowers to one fixed-size stack `[u8; 64]`
/// written BACK-TO-FRONT (the back-to-front write is the reversal), then one `String` from the
/// buffer tail — eliminating BOTH boxed `Seq of Int`s (`digits` + `out`), the reversal loop, and the
/// `text_from_bytes` round-trip. Matches the CONTIGUOUS run at `idx`:
///   [0] Let digits be a new Seq.
///   [1] While <m greater than 0> { …; Push <CODE> to digits; …; Set m to m / base; }
///   [2] Let out be a new Seq.
///   [3] Let k be length of digits.
///   [4] Let i be k.
///   [5] While <i at least 1> { Push item i of digits to out; Set i to i - 1; }
///   [6] Return text_from_bytes(out).
/// `digits` is pushed LSB-first, then reversed into `out` = MSB-first, so writing CODE back-to-front
/// yields the same bytes. Shape-keyed (never the name `toBase`): any well-written pure-LOGOS itoa
/// benefits. SOUND: the generate counter must be a NATIVE i64 (no `__bigint` type) so the digit
/// count is bounded (≤64 for base ≥ 2); a bignum counter bails (keeps the correct heap path).
/// `(CODE) as u8` matches `text_from_bytes`'s per-byte truncation, so the output is byte-identical.
/// Preempts `TextFromBytesBuilder` (it would otherwise fire on the `out` half). Dispatched BEFORE it.
#[allow(clippy::too_many_arguments)]
fn try_emit_radix_digit_buffer<'a>(
    stmts: &[&Stmt<'a>],
    idx: usize,
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
) -> Option<(String, usize)> {
    if !crate::optimize::active_config().is_on(crate::optimization::Opt::RadixDigitBuffer) {
        return None;
    }
    if idx + 6 >= stmts.len() {
        return None;
    }
    let appears_expr = |sym: Symbol, e: &Expr| super::detection::symbol_appears_in_expr(sym, e);
    let appears_stmt = |sym: Symbol, s: &Stmt| super::detection::symbol_appears_in_stmt(sym, s);
    let is_new_seq = |v: &Expr| {
        matches!(v, Expr::New { type_name, init_fields, .. }
        if matches!(interner.resolve(*type_name), "Seq" | "List" | "Vec") && init_fields.is_empty())
    };
    let is_int_lit = |e: &Expr, n: i64| matches!(e, Expr::Literal(Literal::Number(v)) if *v == n);

    // [0] Let digits be a new (empty) Seq.
    let Stmt::Let {
        var: digits,
        value: v0,
        ..
    } = stmts[idx]
    else {
        return None;
    };
    let digits = *digits;
    if !is_new_seq(v0) {
        return None;
    }

    // [1] Generate loop: `While m greater than 0 { … Push CODE to digits … Set m to m/base }`.
    let Stmt::While {
        cond: gen_cond,
        body: gen_body,
        ..
    } = stmts[idx + 1]
    else {
        return None;
    };
    let Expr::BinaryOp {
        op: gop,
        left: gl,
        right: gr,
    } = gen_cond
    else {
        return None;
    };
    if !matches!(gop, BinaryOpKind::Gt) {
        return None;
    }
    let Expr::Identifier(m) = gl else { return None };
    let m = *m;
    if !is_int_lit(gr, 0) {
        return None;
    }
    // Soundness: the formatted counter must be a native i64 (bounded digit count). A bignum-promoted
    // counter would overflow `[u8; 64]`; bail so it keeps the correct heap-Seq path.
    if ctx
        .get_variable_types()
        .get(&m)
        .is_some_and(|t| t.contains("__bigint"))
    {
        return None;
    }
    // `digits` must appear ONLY as the collection of a SINGLE `Push CODE to digits` in the loop.
    let mut push_bi: Option<(usize, &Expr)> = None;
    for (bi, st) in gen_body.iter().enumerate() {
        if let Stmt::Push {
            value,
            collection: Expr::Identifier(c),
        } = st
        {
            if *c == digits {
                if push_bi.is_some() || appears_expr(digits, value) {
                    return None;
                }
                push_bi = Some((bi, value));
                continue;
            }
        }
        if appears_stmt(digits, st) {
            return None;
        }
    }
    let Some((push_bi, code_expr)) = push_bi else {
        return None;
    };
    if appears_expr(digits, gen_cond) {
        return None;
    }

    // [2] Let out be a new (empty) Seq.
    let Stmt::Let {
        var: out,
        value: v2,
        ..
    } = stmts[idx + 2]
    else {
        return None;
    };
    let out = *out;
    if !is_new_seq(v2) {
        return None;
    }
    // [3] Let k be length of digits.
    let Stmt::Let {
        var: k,
        value: Expr::Length { collection: lc },
        ..
    } = stmts[idx + 3]
    else {
        return None;
    };
    let k = *k;
    if !matches!(lc, Expr::Identifier(c) if *c == digits) {
        return None;
    }
    // [4] Let i be k.
    let Stmt::Let {
        var: iv,
        value: Expr::Identifier(kk),
        ..
    } = stmts[idx + 4]
    else {
        return None;
    };
    let iv = *iv;
    if *kk != k {
        return None;
    }
    // [5] Reverse loop: `While i at least 1 { Push item i of digits to out; Set i to i - 1 }`.
    let Stmt::While {
        cond: rcond,
        body: rbody,
        ..
    } = stmts[idx + 5]
    else {
        return None;
    };
    let Expr::BinaryOp {
        op: rop,
        left: rl,
        right: rr,
    } = rcond
    else {
        return None;
    };
    if !matches!(rop, BinaryOpKind::GtEq)
        || !matches!(rl, Expr::Identifier(x) if *x == iv)
        || !is_int_lit(rr, 1)
    {
        return None;
    }
    if rbody.len() != 2 {
        return None;
    }
    let Stmt::Push {
        value: rpv,
        collection: rpc,
    } = &rbody[0]
    else {
        return None;
    };
    if !matches!(rpc, Expr::Identifier(c) if *c == out) {
        return None;
    }
    let Expr::Index {
        collection: ic,
        index: ii,
    } = rpv
    else {
        return None;
    };
    if !matches!(ic, Expr::Identifier(c) if *c == digits)
        || !matches!(ii, Expr::Identifier(x) if *x == iv)
    {
        return None;
    }
    let Stmt::Set {
        target: rt,
        value: rsv,
    } = &rbody[1]
    else {
        return None;
    };
    if *rt != iv {
        return None;
    }
    let Expr::BinaryOp {
        op: sop,
        left: sl,
        right: sr,
    } = rsv
    else {
        return None;
    };
    if !matches!(sop, BinaryOpKind::Subtract)
        || !matches!(sl, Expr::Identifier(x) if *x == iv)
        || !is_int_lit(sr, 1)
    {
        return None;
    }
    // [6] Return text_from_bytes(out).
    let Stmt::Return {
        value: Some(Expr::Call { function, args }),
    } = stmts[idx + 6]
    else {
        return None;
    };
    if interner.resolve(*function) != "text_from_bytes" || args.len() != 1 {
        return None;
    }
    if !matches!(args[0], Expr::Identifier(o) if *o == out) {
        return None;
    }
    // `digits`/`out`/`k` must not be used after the run (the `Return` ends flow, but be defensive).
    for s in &stmts[idx + 7..] {
        if appears_stmt(digits, s) || appears_stmt(out, s) || appears_stmt(k, s) {
            return None;
        }
    }

    // ---- EMIT: stack buffer written back-to-front, then a String from its tail. ----
    crate::optimize::mark_fired(crate::optimization::Opt::RadixDigitBuffer);
    use super::expr::codegen_expr_with_async;
    let names = RustNames::new(interner);
    let ind = "    ".repeat(indent);
    let buf = format!("__radix_buf_{}", names.ident(digits));
    let pos = format!("{buf}_pos");
    let mut out_code = String::new();
    writeln!(out_code, "{ind}let mut {buf}: [u8; 64] = [0u8; 64];").unwrap();
    writeln!(out_code, "{ind}let mut {pos}: usize = 64;").unwrap();
    let cond_str = codegen_expr_with_async(
        gen_cond,
        interner,
        synced_vars,
        async_functions,
        ctx.get_variable_types(),
    );
    writeln!(out_code, "{ind}while {cond_str} {{").unwrap();
    ctx.push_scope();
    for (bi, st) in gen_body.iter().enumerate() {
        if bi == push_bi {
            let code_str = codegen_expr_with_async(
                code_expr,
                interner,
                synced_vars,
                async_functions,
                ctx.get_variable_types(),
            );
            writeln!(out_code, "{ind}    {pos} -= 1;").unwrap();
            writeln!(out_code, "{ind}    {buf}[{pos}] = ({code_str}) as u8;").unwrap();
        } else {
            out_code.push_str(&super::codegen_stmt(
                st,
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
    }
    ctx.pop_scope();
    writeln!(out_code, "{ind}}}").unwrap();
    writeln!(
        out_code,
        "{ind}return String::from_utf8({buf}[{pos}..].to_vec()).unwrap();"
    )
    .unwrap();
    Some((out_code, 6))
}

/// Opt::TextFromBytesBuilder — a `Let out be a new Seq of Int` used ONLY as a byte builder and
/// consumed by `text_from_bytes(out)` lowers to a bare `Vec<u8>` (`out.push((e) as u8)` at each
/// push, `String::from_utf8(out)` at the consume). Kills the `Rc<RefCell<Vec<i64>>>` output (8x
/// memory + per-push borrow + no presize) and the separate `i64`->`u8` rebuild pass. The element
/// type is witnessed by the `text_from_bytes(out)` consumption (its param is `Seq of Int`), so no
/// element-type check is needed. Twin of `try_emit_text_bytes_view` (read side).
fn try_emit_text_from_bytes_builder<'a>(
    stmts: &[&Stmt<'a>],
    idx: usize,
    interner: &Interner,
    indent: usize,
    ctx: &mut RefinementContext<'a>,
) -> Option<(String, usize)> {
    if !crate::optimize::active_config().is_on(crate::optimization::Opt::TextFromBytesBuilder) {
        return None;
    }
    let Stmt::Let {
        var: out, value, ..
    } = stmts[idx]
    else {
        return None;
    };
    let out = *out;
    // `value` must be `a new Seq/List/Vec` (empty) — the byte-list builder.
    let is_new_empty_seq = matches!(value, Expr::New { type_name, init_fields, .. }
        if matches!(interner.resolve(*type_name), "Seq" | "List" | "Vec") && init_fields.is_empty());
    if !is_new_empty_seq {
        return None;
    }
    let remaining = &stmts[idx + 1..];
    // `remaining` is `&[&Stmt]`; iterate the per-statement walkers (each `&&Stmt` deref-coerces to
    // `&Stmt`), matching `try_emit_text_bytes_view`. The block-level `byte_builder_safe` is used for
    // nested `Block`s (real `&[Stmt]`) inside the walkers.
    if !remaining.iter().all(|st| stmt_bb_safe(st, out, interner)) {
        return None;
    }
    if !remaining
        .iter()
        .any(|st| stmt_consumes_tfb(st, out, interner))
    {
        return None;
    }
    ctx.register_variable_type(out, "ByteBuilder".to_string());
    crate::optimize::mark_fired(crate::optimization::Opt::TextFromBytesBuilder);
    let names = RustNames::new(interner);
    let ind = "    ".repeat(indent);
    let code = format!(
        "{}let mut {}: Vec<u8> = Vec::new();\n",
        ind,
        names.ident(out)
    );
    Some((code, 0))
}

/// Recognize the contiguous slice-builder tail of a trim-family fn, at four consecutive statements:
///   s0: `Let out be a new Seq of Int.`
///   s1: `Let mutable i be <LO>.`
///   s2: `While i (is at most|is less than) <HI>: Push item i of sb to out. Set i to i+1.`
///   s3: `Return text_from_bytes(out).`
/// `sb` must be the byte-view symbol (`Let sb be text_bytes(<source param>)`). Returns
/// `(out, lo_expr, hi_expr, inclusive)` — the exact byte range `out` copies from `sb`, so it can be
/// emitted as a zero-copy slice of `s` (`logicaffeine_data::logos_text_run_through`) instead of the O(n)
/// build. Shape-keyed, never name-keyed.
pub(crate) fn match_slice_builder_at<'a>(
    s0: &Stmt<'a>,
    s1: &Stmt<'a>,
    s2: &Stmt<'a>,
    s3: &Stmt<'a>,
    sb: Symbol,
    interner: &Interner,
) -> Option<(Symbol, &'a Expr<'a>, &'a Expr<'a>, bool)> {
    // s0: Let out be a new (empty) Seq/List/Vec.
    let Stmt::Let {
        var: out,
        value: v0,
        ..
    } = s0
    else {
        return None;
    };
    let out = *out;
    if !matches!(v0, Expr::New { type_name, init_fields, .. }
        if matches!(interner.resolve(*type_name), "Seq" | "List" | "Vec") && init_fields.is_empty())
    {
        return None;
    }
    // s1: Let i be <LO>.
    let Stmt::Let {
        var: iv, value: lo, ..
    } = s1
    else {
        return None;
    };
    let iv = *iv;
    // s2: While i <=|< HI { Push item i of sb to out; Set i to i+1 }.
    let Stmt::While { cond, body, .. } = s2 else {
        return None;
    };
    let Expr::BinaryOp { op, left, right } = cond else {
        return None;
    };
    let inclusive = match op {
        BinaryOpKind::LtEq => true,
        BinaryOpKind::Lt => false,
        _ => return None,
    };
    if !matches!(left, Expr::Identifier(x) if *x == iv) {
        return None;
    }
    let hi: &Expr = right;
    if body.len() != 2 {
        return None;
    }
    // body[0]: Push item i of sb to out.
    let Stmt::Push {
        value: pv,
        collection: pc,
    } = &body[0]
    else {
        return None;
    };
    if !matches!(pc, Expr::Identifier(c) if *c == out) {
        return None;
    }
    let Expr::Index {
        collection: icoll,
        index: iidx,
    } = pv
    else {
        return None;
    };
    if !matches!(icoll, Expr::Identifier(c) if *c == sb) {
        return None;
    }
    if !matches!(iidx, Expr::Identifier(x) if *x == iv) {
        return None;
    }
    // body[1]: Set i to i + 1. The loop reads bytes LO, LO+1, … one at a time; a bound that moves while
    // it runs, or a byte view that is the sequence being built, is another loop.
    validate_increment(&body[1], iv)?;
    if sb == out
        || [lo, hi]
            .iter()
            .any(|b| symbol_appears_in_expr(out, b) || symbol_appears_in_expr(iv, b))
    {
        return None;
    }
    // s3: Return text_from_bytes(out).
    let Stmt::Return { value: Some(rv) } = s3 else {
        return None;
    };
    let Expr::Call { function, args } = rv else {
        return None;
    };
    if interner.resolve(*function) != "text_from_bytes" || args.len() != 1 {
        return None;
    }
    if !matches!(args[0], Expr::Identifier(o) if *o == out) {
        return None;
    }
    Some((out, lo, hi, inclusive))
}

/// The sole `Return` of `body` (walking nested blocks) is a contiguous slice-builder of `sb`
/// (`match_slice_builder_at` on the four-statement tail), and `sb` is bound to `text_bytes(<s>)`.
/// Used by the analysis to qualify a fn for a borrowed `&str` return.
pub(crate) fn fn_returns_contiguous_slice(body: &[Stmt], s: Symbol, interner: &Interner) -> bool {
    // sb = text_bytes(s), bound at top level.
    let mut sb = None;
    for st in body {
        if let Stmt::Let {
            var,
            value: Expr::Call { function, args },
            ..
        } = st
        {
            if interner.resolve(*function) == "text_bytes" && args.len() == 1 {
                if matches!(args[0], Expr::Identifier(x) if *x == s) {
                    sb = Some(*var);
                }
            }
        }
    }
    let sb = match sb {
        Some(x) => x,
        None => return false,
    };
    // Exactly one Return in the whole body, and the four-statement tail matches.
    if count_returns(body) != 1 {
        return false;
    }
    let n = body.len();
    if n < 4 {
        return false;
    }
    for j in 0..=n - 4 {
        if match_slice_builder_at(
            &body[j],
            &body[j + 1],
            &body[j + 2],
            &body[j + 3],
            sb,
            interner,
        )
        .is_some()
        {
            return true;
        }
    }
    false
}

fn count_returns(stmts: &[Stmt]) -> usize {
    let mut n = 0;
    for s in stmts {
        match s {
            Stmt::Return { .. } => n += 1,
            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
                n += count_returns(then_block);
                if let Some(e) = else_block {
                    n += count_returns(e);
                }
            }
            Stmt::While { body, .. }
            | Stmt::Repeat { body, .. }
            | Stmt::During { body, .. }
            | Stmt::Zone { body, .. }
            | Stmt::Splice { body }
            | Stmt::Reversibly { body } => {
                n += count_returns(body);
            }
            _ => {}
        }
    }
    n
}

/// Opt::TextSliceReturn analysis — fns that return a contiguous sub-slice of a Text param. Returns
/// `{fn -> source-param-index}`. The slice-builder SHAPE provably only READS the source param, so the
/// param is borrowable by construction — this pass DRIVES borrowing that param (the caller injects the
/// index into `borrow_params_map`), rather than requiring a pre-existing borrow. Caller-gate: drops any
/// fn whose result ESCAPES (bound by `Let/Set`, returned directly, pushed/added/given/stored) — a
/// borrowed `&str` must stay inline/read-only. Inline uses (`length of f(s)`, a condition, an arg) OK.
pub(crate) fn collect_str_slice_return_fns(
    stmts: &[Stmt],
    interner: &Interner,
    borrowed: &super::slice_escape::BorrowedParams,
) -> HashMap<Symbol, usize> {
    if !crate::optimize::active_config().is_on(crate::optimization::Opt::TextSliceReturn) {
        return HashMap::new();
    }
    let mut cand: HashMap<Symbol, usize> = HashMap::new();
    for s in stmts {
        if let Stmt::FunctionDef {
            name,
            body,
            params,
            return_type: Some(rt),
            is_native: false,
            is_exported: false,
            ..
        } = s
        {
            if !super::marshal::is_text_type(rt, interner) {
                continue;
            }
            for (i, (psym, pty)) in params.iter().enumerate() {
                if super::marshal::is_text_type(pty, interner)
                    && fn_returns_contiguous_slice(body, *psym, interner)
                {
                    cand.insert(*name, i);
                    break;
                }
            }
        }
    }
    if cand.is_empty() {
        return HashMap::new();
    }
    let keep = super::slice_escape::surviving_candidates(stmts, &cand, borrowed);
    cand.retain(|f, _| keep.contains(f));
    if !cand.is_empty() {
        crate::optimize::mark_fired(crate::optimization::Opt::TextSliceReturn);
    }
    cand
}

/// Opt::TextSliceReturn — when the current fn returns a borrowed `&str` sub-slice (ctx marks it),
/// lower the `Let out be new Seq … build … Return text_from_bytes(out)` tail to `return &s[lo-1..hi]`.
/// Fires at the `Let out` statement (recovering the source param `s` from the earlier
/// `Let sb be text_bytes(s)`), consumes the four-statement tail (skip = 3). Preempts the write-side
/// builder on this Return.
fn try_emit_str_slice_return<'a>(
    stmts: &[&Stmt<'a>],
    idx: usize,
    interner: &Interner,
    indent: usize,
    ctx: &mut RefinementContext<'a>,
) -> Option<(String, usize)> {
    if !crate::optimize::active_config().is_on(crate::optimization::Opt::TextSliceReturn) {
        return None;
    }
    if ctx.str_slice_source().is_none() {
        return None;
    }
    if idx + 3 >= stmts.len() {
        return None;
    }
    // Recover (sb, s) from an earlier `Let sb be text_bytes(<Identifier s>)` in this block.
    let mut binding: Option<(Symbol, Symbol)> = None;
    for &st in &stmts[..idx] {
        if let Stmt::Let {
            var,
            value: Expr::Call { function, args },
            ..
        } = st
        {
            if interner.resolve(*function) == "text_bytes" && args.len() == 1 {
                if let Expr::Identifier(x) = args[0] {
                    binding = Some((*var, *x));
                }
            }
        }
    }
    let (sb, s) = binding?;
    let (_out, lo, hi, inclusive) = match_slice_builder_at(
        stmts[idx],
        stmts[idx + 1],
        stmts[idx + 2],
        stmts[idx + 3],
        sb,
        interner,
    )?;
    crate::optimize::mark_fired(crate::optimization::Opt::TextSliceReturn);
    crate::optimize::mark_preempted(
        crate::optimization::Opt::TextSliceReturn,
        crate::optimization::Opt::TextFromBytesBuilder,
    );
    let names = RustNames::new(interner);
    let ind = "    ".repeat(indent);
    let s_name = names.ident(s);
    let lo_str = codegen_expr_simple(lo, interner);
    let hi_str = codegen_expr_simple(hi, interner);
    // The loop's bytes under its own rule, read back as `text_from_bytes` reads them.
    let run = if inclusive {
        "logos_text_run_through"
    } else {
        "logos_text_run_before"
    };
    let code = format!("{ind}return logicaffeine_data::{run}(&{s_name}, {lo_str}, {hi_str});\n");
    Some((code, 3))
}

/// The bytes analog of `match_slice_builder_at`: a four-statement contiguous-slice-builder tail whose
/// source is a bare `Seq` symbol (`item i of SB`, extracted from the push) and whose return is the
/// bare Seq `out` (NOT wrapped in `text_from_bytes`). Returns `(out, sb, lo, hi, inclusive)`.
pub(crate) fn match_byte_slice_builder_at<'a>(
    s0: &Stmt<'a>,
    s1: &Stmt<'a>,
    s2: &Stmt<'a>,
    s3: &Stmt<'a>,
    interner: &Interner,
) -> Option<(Symbol, Symbol, &'a Expr<'a>, &'a Expr<'a>, bool)> {
    // s0: Let out be a new (empty) Seq/List/Vec.
    let Stmt::Let {
        var: out,
        value: v0,
        ..
    } = s0
    else {
        return None;
    };
    let out = *out;
    if !matches!(v0, Expr::New { type_name, init_fields, .. }
        if matches!(interner.resolve(*type_name), "Seq" | "List" | "Vec") && init_fields.is_empty())
    {
        return None;
    }
    // s1: Let i be <LO>.
    let Stmt::Let {
        var: iv, value: lo, ..
    } = s1
    else {
        return None;
    };
    let iv = *iv;
    // s2: While i <=|< HI { Push item i of SB to out; Set i to i+1 }.
    let Stmt::While { cond, body, .. } = s2 else {
        return None;
    };
    let Expr::BinaryOp { op, left, right } = cond else {
        return None;
    };
    let inclusive = match op {
        BinaryOpKind::LtEq => true,
        BinaryOpKind::Lt => false,
        _ => return None,
    };
    if !matches!(left, Expr::Identifier(x) if *x == iv) {
        return None;
    }
    let hi: &Expr = right;
    if body.len() != 2 {
        return None;
    }
    let Stmt::Push {
        value: pv,
        collection: pc,
    } = &body[0]
    else {
        return None;
    };
    if !matches!(pc, Expr::Identifier(c) if *c == out) {
        return None;
    }
    let Expr::Index {
        collection: icoll,
        index: iidx,
    } = pv
    else {
        return None;
    };
    let Expr::Identifier(sb) = icoll else {
        return None;
    };
    let sb = *sb;
    if !matches!(iidx, Expr::Identifier(x) if *x == iv) {
        return None;
    }
    // The loop reads items LO, LO+1, … one at a time; a bound that moves while it runs, or a source that
    // is the sequence being built, is another loop.
    validate_increment(&body[1], iv)?;
    if sb == out
        || [lo, hi]
            .iter()
            .any(|b| symbol_appears_in_expr(out, b) || symbol_appears_in_expr(iv, b))
    {
        return None;
    }
    // s3: Return out (the bare Seq — no text_from_bytes wrapper).
    let Stmt::Return {
        value: Some(Expr::Identifier(o)),
    } = s3
    else {
        return None;
    };
    if *o != out {
        return None;
    }
    Some((out, sb, lo, hi, inclusive))
}

/// The sole `Return` of `body` is a contiguous sub-slice builder of the Seq param `xs` (the bytes
/// analog of `fn_returns_contiguous_slice`). Used to qualify a fn for a borrowed `&[i64]` return.
pub(crate) fn fn_returns_contiguous_byte_slice(
    body: &[Stmt],
    xs: Symbol,
    interner: &Interner,
) -> bool {
    if count_returns(body) != 1 {
        return false;
    }
    let n = body.len();
    if n < 4 {
        return false;
    }
    for j in 0..=n - 4 {
        if let Some((_, sb, _, _, _)) = match_byte_slice_builder_at(
            &body[j],
            &body[j + 1],
            &body[j + 2],
            &body[j + 3],
            interner,
        ) {
            if sb == xs {
                return true;
            }
        }
    }
    false
}

/// Opt::ByteSliceReturn analysis — fns returning a contiguous sub-slice of a `Seq of Int` param.
/// Returns `{fn -> source-param-index}`. The slice-builder SHAPE provably only READS the source param
/// (a readonly Seq param is already `&[i64]`), so this pass DRIVES borrowing it (the caller injects
/// the index into `borrow_params_map`) and the signature gains `<'a>`. Caller-gate: drops any fn whose
/// result ESCAPES (bound/returned/pushed/stored) — a borrowed `&[i64]` must stay inline/read-only.
pub(crate) fn collect_byte_slice_return_fns(
    stmts: &[Stmt],
    interner: &Interner,
    borrowed: &super::slice_escape::BorrowedParams,
) -> HashMap<Symbol, usize> {
    if !crate::optimize::active_config().is_on(crate::optimization::Opt::ByteSliceReturn) {
        return HashMap::new();
    }
    let mut cand: HashMap<Symbol, usize> = HashMap::new();
    for s in stmts {
        if let Stmt::FunctionDef {
            name,
            body,
            params,
            return_type: Some(rt),
            is_native: false,
            is_exported: false,
            ..
        } = s
        {
            if !super::detection::is_vec_type_expr(rt, interner) {
                continue;
            }
            for (i, (psym, pty)) in params.iter().enumerate() {
                if super::detection::is_vec_type_expr(pty, interner)
                    && fn_returns_contiguous_byte_slice(body, *psym, interner)
                {
                    cand.insert(*name, i);
                    break;
                }
            }
        }
    }
    if cand.is_empty() {
        return HashMap::new();
    }
    let keep = super::slice_escape::surviving_candidates(stmts, &cand, borrowed);
    cand.retain(|f, _| keep.contains(f));
    // NB: no `mark_fired` here — the pass is marked ONLY at the emitter (`try_emit_byte_slice_return`),
    // so its fired-state tracks its actual effect. If `collect` also marked, disabling `peephole` (which
    // gates the emitter but not this analysis) would leave the pass "fired" without eliminating the
    // build, and the preempts differential could not attribute `peephole → oraclehints/unchecked` to it.
    cand
}

/// Opt::ByteSliceReturn — when the current fn returns a borrowed `&[i64]` sub-slice (ctx marks it),
/// lower the `Let out be new Seq … build … Return out` tail to `return &xs[lo-1..hi]`. Fires at the
/// `Let out` statement, recovering the source param `xs` from the slice loop's `item i of xs`,
/// consumes the four-statement tail (skip = 3).
fn try_emit_byte_slice_return<'a>(
    stmts: &[&Stmt<'a>],
    idx: usize,
    interner: &Interner,
    indent: usize,
    ctx: &mut RefinementContext<'a>,
) -> Option<(String, usize)> {
    if !crate::optimize::active_config().is_on(crate::optimization::Opt::ByteSliceReturn) {
        return None;
    }
    if ctx.byte_slice_source().is_none() {
        return None;
    }
    if idx + 3 >= stmts.len() {
        return None;
    }
    let (_out, sb, lo, hi, inclusive) = match_byte_slice_builder_at(
        stmts[idx],
        stmts[idx + 1],
        stmts[idx + 2],
        stmts[idx + 3],
        interner,
    )?;
    crate::optimize::mark_fired(crate::optimization::Opt::ByteSliceReturn);
    let names = RustNames::new(interner);
    let ind = "    ".repeat(indent);
    let sb_name = names.ident(sb);
    let lo_str = codegen_expr_simple(lo, interner);
    let hi_str = codegen_expr_simple(hi, interner);
    // The items the loop reads, under its own rule.
    let code = format!(
        "{ind}return {};\n",
        copied_run(&sb_name, &lo_str, &hi_str, !inclusive)
    );
    Some((code, 3))
}

/// Opt::StringRepeatLoop — `Let out be ""` followed by a counted loop whose sole body is
/// `Set out to out + <chunk>` (chunk loop-invariant) lowers to `let mut out = chunk.repeat(count)`:
/// one presized alloc + memset, matching Rust's `chunk.repeat(n)`, instead of `count` un-presized
/// `push_str`es. Consumes the `Let out` + the counted loop (init + While). Shape-based.
fn try_emit_string_repeat<'a>(
    stmts: &[&Stmt<'a>],
    idx: usize,
    interner: &Interner,
    indent: usize,
    ctx: &mut RefinementContext<'a>,
) -> Option<(String, usize)> {
    if !crate::optimize::active_config().is_on(crate::optimization::Opt::StringRepeatLoop) {
        return None;
    }
    // stmts[idx]: `Let out be ""` (empty Text literal).
    let Stmt::Let {
        var: out, value, ..
    } = stmts[idx]
    else {
        return None;
    };
    let out = *out;
    match value {
        Expr::Literal(Literal::Text(sym)) if interner.resolve(*sym).is_empty() => {}
        _ => return None,
    }
    // The counted loop at idx+1 (init `Let i = start` + `While i </<= limit { ...; i += 1 }`).
    let (cl, consumed) = crate::loop_shape::extract_counted_while(stmts, idx + 1)?;
    // Body (increment stripped) must be exactly `Set out to (out + chunk)`.
    if cl.body_without_increment.len() != 1 {
        return None;
    }
    let Stmt::Set {
        target,
        value: setval,
    } = &cl.body_without_increment[0]
    else {
        return None;
    };
    if *target != out {
        return None;
    }
    let Expr::BinaryOp {
        op: BinaryOpKind::Concat,
        left,
        right,
    } = setval
    else {
        return None;
    };
    if !matches!(left, Expr::Identifier(l) if *l == out) {
        return None;
    }
    let chunk = *right;
    // chunk must be loop-invariant: independent of the counter AND not self-referential on `out`.
    if super::detection::symbol_appears_in_expr(cl.var, chunk)
        || super::detection::symbol_appears_in_expr(out, chunk)
    {
        return None;
    }
    // chunk must be cleanly emittable by codegen_expr_simple — otherwise it would fall to the `_`
    // sentinel and produce `(_).repeat(..)` (invalid Rust). Anything richer (a call, a slice, a
    // nested concat) safely falls back to the accumulate loop instead of miscompiling.
    if !is_simple_expr(chunk) {
        return None;
    }
    crate::optimize::mark_fired(crate::optimization::Opt::StringRepeatLoop);
    ctx.register_variable_type(out, "String".to_string());
    let names = RustNames::new(interner);
    let ind = "    ".repeat(indent);
    let out_name = names.ident(out);
    let chunk_code = codegen_expr_simple(chunk, interner);
    let start_code = codegen_expr_simple(cl.start, interner);
    let limit_code = codegen_expr_simple(cl.limit, interner);
    // Iteration count = (limit + 1 - start) inclusive, (limit - start) exclusive; clamped ≥ 0 so a
    // non-positive range yields "" exactly as the loop would (zero iterations).
    let count = if cl.inclusive {
        format!("(({limit_code}) + 1 - ({start_code}))")
    } else {
        format!("(({limit_code}) - ({start_code}))")
    };
    let code =
        format!("{ind}let mut {out_name} = ({chunk_code}).repeat(({count}).max(0) as usize);\n");
    Some((code, consumed))
}

pub(crate) fn try_block_peepholes<'a>(
    stmts: &[&Stmt<'a>],
    idx: usize,
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
) -> Option<(String, usize)> {
    // The patterns below are written for machine integers and collections of them; a window that
    // reaches a value of the exact extension is emitted statement by statement on its value model.
    if super::tower::mentions_reached(&stmts[idx..]) {
        return None;
    }
    // Mark Peephole as fired iff some pattern in the dispatch chain below actually
    // emitted (the `Some` is the proof). Dedicated peephole opts (Simd, Cascade,
    // IndexString, CapScale) additionally mark themselves at their own emission.
    let __peephole_result = (|| -> Option<(String, usize)> {
        if let r @ Some(_) = try_emit_text_bytes_view(stmts, idx, interner, indent, ctx) {
            return r;
        }
        if let r @ Some(_) = try_emit_str_slice_return(stmts, idx, interner, indent, ctx) {
            return r;
        }
        if let r @ Some(_) = try_emit_byte_slice_return(stmts, idx, interner, indent, ctx) {
            return r;
        }
        if let r @ Some(_) = try_emit_radix_digit_buffer(
            stmts,
            idx,
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
        ) {
            return r;
        }
        if let r @ Some(_) = try_emit_text_from_bytes_builder(stmts, idx, interner, indent, ctx) {
            return r;
        }
        if let r @ Some(_) = try_emit_string_repeat(stmts, idx, interner, indent, ctx) {
            return r;
        }
        if let r @ Some(_) = try_emit_naive_search(stmts, idx, interner, indent, ctx) {
            return r;
        }
        if let r @ Some(_) = try_emit_affine_cascade(stmts, idx, interner, indent, ctx) {
            return r;
        }
        if let r @ Some(_) = try_emit_seq_from_slice_pattern(
            stmts,
            idx,
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
        ) {
            return r;
        }
        if let r @ Some(_) = try_emit_vec_fill_pattern(stmts, idx, interner, indent, ctx) {
            return r;
        }
        if let r @ Some(_) =
            try_emit_bare_slice_push_pattern(stmts, idx, interner, indent, ctx.get_variable_types())
        {
            return r;
        }
        if let r @ Some(_) = try_emit_vec_with_capacity_pattern(
            stmts,
            idx,
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
        ) {
            return r;
        }
        if let r @ Some(_) = try_emit_merge_capacity_pattern(stmts, idx, interner, indent, ctx) {
            return r;
        }
        if let r @ Some(_) = try_emit_indexed_string_build(
            stmts,
            idx,
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
        ) {
            return r;
        }
        if let r @ Some(_) =
            try_emit_string_with_capacity_pattern(stmts, idx, interner, indent, mutable_vars, ctx)
        {
            return r;
        }
        if let r @ Some(_) = try_emit_buffer_reuse_while(
            stmts,
            idx,
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
        ) {
            return r;
        }
        // SIMD reduction of a `Set acc to acc + trunc(<transcendental in the counter>)` counted loop —
        // tried before the plain for-range so a liftable transcendental accumulator lowers to an AVX2
        // four-lane reduction (the ordinary scalar for-loop covers the <4 tail + the non-AVX2 host).
        if crate::optimize::active_config().is_on(crate::optimization::Opt::SimdReduce) {
            if let Some((cl, _consumed)) = crate::loop_shape::extract_counted_while(stmts, idx) {
                if let Some(plan) = super::simd_reduce::try_lift(&cl, ctx.simd_fns(), interner) {
                    let mut body_code = String::new();
                    for s in cl.body_without_increment.iter() {
                        body_code.push_str(&super::codegen_stmt(
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
                    let names = RustNames::new(interner);
                    let counter_name = names.ident(cl.var);
                    let acc_name = names.ident(plan.acc);
                    let start_str = codegen_expr_simple(cl.start, interner);
                    let limit_str = codegen_expr_simple(cl.limit, interner);
                    crate::optimize::mark_fired(crate::optimization::Opt::SimdReduce);
                    let base_indent = "    ".repeat(indent);
                    let code = super::simd_reduce::emit_block(
                        &plan,
                        &acc_name,
                        &counter_name,
                        &start_str,
                        &limit_str,
                        cl.inclusive,
                        &body_code,
                        &base_indent,
                    );
                    return Some((code, 1));
                }
            }
        }
        if let r @ Some(_) = try_emit_for_range_pattern(
            stmts,
            idx,
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
        ) {
            return r;
        }
        if let r @ Some(_) =
            try_emit_prefix_reverse(stmts, idx, interner, indent, ctx.get_variable_types())
        {
            return r;
        }
        if let r @ Some(_) = try_emit_swap_pattern(
            stmts,
            idx,
            interner,
            indent,
            ctx.get_variable_types(),
            ctx.oracle(),
        ) {
            return r;
        }
        if let r @ Some(_) =
            try_emit_rotate_left_pattern(stmts, idx, interner, indent, ctx.get_variable_types())
        {
            return r;
        }
        None
    })();
    // Several call sites invoke this dispatcher WITHOUT the `no_peephole` guard,
    // and the generic patterns have no internal gate — so mark Peephole only when
    // it actually emitted AND is enabled, keeping `fired ⊆ enabled`. (Dedicated
    // peephole opts — Simd/Cascade/IndexString/CapScale — mark themselves under
    // their own gates.)
    if __peephole_result.is_some()
        && crate::optimize::active_config().is_on(crate::optimization::Opt::Peephole)
    {
        crate::optimize::mark_fired(crate::optimization::Opt::Peephole);
    }
    // A lowering that consumes a `Let` emits its own binder for the name; the name keeps the type its `Let`
    // decided. An Int left untyped reads as no number at all, and the arithmetic after it compiled raw (the sieve's
    // `i * i` overflowed past 2^63 and its unchecked store wrote outside the flags).
    if let Some((_, extra)) = &__peephole_result {
        for consumed in stmts.iter().skip(idx).take(extra + 1) {
            if let Stmt::Let { var, value, .. } = consumed {
                if !ctx.get_variable_types().contains_key(var)
                    && super::types::infer_numeric_type(value, interner, ctx.get_variable_types()) == "i64"
                {
                    ctx.register_variable_type(*var, "i64".to_string());
                }
            }
        }
    }
    __peephole_result
}

/// The byte value of a single-byte constant: a one-char text literal or a small
/// number literal (`0..=255`).
fn byte_const_of(e: &Expr, interner: &Interner) -> Option<i64> {
    match e {
        Expr::Literal(Literal::Text(sym)) => {
            let b = interner.resolve(*sym).as_bytes();
            (b.len() == 1).then(|| b[0] as i64)
        }
        Expr::Literal(Literal::Number(n)) if (0..=255).contains(n) => Some(*n),
        _ => None,
    }
}

/// A side-effect-free integer expression (no calls / collection ops) — safe to
/// re-evaluate as the argument of a folded affine assignment.
fn is_pure_arith(e: &Expr) -> bool {
    match e {
        Expr::Identifier(_) | Expr::Literal(_) => true,
        Expr::BinaryOp { left, right, .. } => is_pure_arith(left) && is_pure_arith(right),
        Expr::Not { operand } => is_pure_arith(operand),
        _ => false,
    }
}

/// Render the affine value `a*E + b` (with the redundant `1*`/`+0` removed).
fn affine_rhs(a: i64, b: i64, e: &str) -> String {
    match (a, b) {
        (0, b) => format!("{b}"),
        (1, 0) => format!("({e})"),
        (1, b) if b > 0 => format!("({e}) + {b}"),
        (1, b) => format!("({e}) - {}", -b),
        (a, 0) => format!("{a} * ({e})"),
        (a, b) if b > 0 => format!("{a} * ({e}) + {b}"),
        (a, b) => format!("{a} * ({e}) - {}", -b),
    }
}

/// Switch-to-affine fold: a default-valued single-byte variable followed by a
/// cascade of guards testing the SAME pure expression `E` against distinct
/// constants and assigning constants:
///
/// ```text
/// Let ch be "a".                 (default d = 'a')
/// If pos % 5 equals 1: Set ch to "b".
/// If pos % 5 equals 2: Set ch to "c".
/// If pos % 5 equals 3: Set ch to "d".
/// If pos % 5 equals 4: Set ch to "e".
/// ```
///
/// defines a function `f(E)` (`f(k_i)=c_i`, else `d`). When the oracle proves
/// `E`'s range `[lo,hi]` is finite and small AND `f` is affine over ALL of it
/// (`f(e) = a*e + b` for every `e in [lo,hi]`), the whole cascade collapses to
/// one arithmetic assignment `ch = (a*E + b) as u8` — the branch chain becomes a
/// register computation. The oracle proof is load-bearing for soundness: `E`
/// (e.g. `pos % 5`) could be negative for a signed `pos`, where the cascade
/// keeps the default but the affine formula would not; proving `E in [0,4]` is
/// exactly what rules that out.
pub(crate) fn try_emit_affine_cascade(
    stmts: &[&Stmt],
    idx: usize,
    interner: &Interner,
    indent: usize,
    ctx: &mut RefinementContext,
) -> Option<(String, usize)> {
    if !crate::optimize::active_config().is_on(crate::optimization::Opt::Cascade) {
        return None;
    }
    // The cascade target must be a single-byte variable so the result type and
    // its downstream uses (`text + ch` → `push(ch as char)`) are unchanged.
    let Stmt::Let {
        var: v,
        value: v0,
        mutable,
        ..
    } = stmts[idx]
    else {
        return None;
    };
    let v = *v;
    if ctx.get_variable_types().get(&v).map(String::as_str) != Some("__single_char_u8") {
        return None;
    }
    let default = byte_const_of(v0, interner)?;
    // Collect the consecutive `If E == k_i { Set v = c_i }` guards.
    let mut e_src: Option<String> = None;
    let mut e_expr: Option<&Expr> = None;
    let mut points: Vec<(i64, i64)> = Vec::new();
    let mut n = 0usize;
    while idx + 1 + n < stmts.len() {
        let Stmt::If {
            cond,
            then_block,
            else_block: None,
        } = stmts[idx + 1 + n]
        else {
            break;
        };
        let Expr::BinaryOp {
            op: BinaryOpKind::Eq,
            left: e,
            right: k,
        } = cond
        else {
            break;
        };
        let Expr::Literal(Literal::Number(k)) = k else {
            break;
        };
        if then_block.len() != 1 {
            break;
        }
        let Stmt::Set { target, value } = &then_block[0] else {
            break;
        };
        if *target != v {
            break;
        }
        let Some(c) = byte_const_of(value, interner) else {
            break;
        };
        let cur = codegen_expr_simple(e, interner);
        match &e_src {
            None => {
                e_src = Some(cur);
                e_expr = Some(e);
            }
            Some(prev) if *prev == cur => {}
            _ => break,
        }
        points.push((*k, c));
        n += 1;
    }
    if n < 2 {
        return None;
    }
    let e_expr = e_expr?;
    if !is_pure_arith(e_expr) {
        return None;
    }
    // The oracle must bound E to a small finite range we can enumerate.
    let (lo, hi) = ctx.oracle().and_then(|o| o.expr_int_range(e_expr))?;
    if hi < lo || hi - lo > 256 {
        return None;
    }
    // Build f over [lo, hi]; guards must be distinct.
    let mut kmap: HashMap<i64, i64> = HashMap::new();
    for (k, c) in &points {
        if kmap.insert(*k, *c).is_some() {
            return None;
        }
    }
    let f = |e: i64| *kmap.get(&e).unwrap_or(&default);
    // Fit a line through the range, then verify it matches f everywhere.
    let (a, b) = if hi == lo {
        (0, f(lo))
    } else {
        let a = f(lo + 1) - f(lo);
        (a, f(lo) - a * lo)
    };
    for e in lo..=hi {
        if a.checked_mul(e).and_then(|x| x.checked_add(b)) != Some(f(e)) {
            return None;
        }
    }
    let vn = RustNames::new(interner).ident(v);
    let mut_kw = if *mutable { "mut " } else { "" };
    let rhs = affine_rhs(a, b, &e_src.unwrap());
    let code = format!(
        "{}let {}{}: u8 = ({}) as u8;\n",
        "    ".repeat(indent),
        mut_kw,
        vn,
        rhs
    );
    crate::optimize::mark_fired(crate::optimization::Opt::Cascade);
    Some((code, n))
}

/// Byte length of an appended operand `X` in `text + X`: a text literal's byte
/// length, or 1 for a single-byte (`__single_char_u8`) variable. None otherwise.
fn append_operand_byte_len(
    x: &Expr,
    variable_types: &HashMap<Symbol, String>,
    interner: &Interner,
) -> Option<i64> {
    match x {
        Expr::Literal(Literal::Text(sym)) => Some(interner.resolve(*sym).len() as i64),
        Expr::Identifier(v) => {
            (variable_types.get(v).map(String::as_str) == Some("__single_char_u8")).then_some(1)
        }
        _ => None,
    }
}

/// `Set text to text + X` → the appended byte length of `X`.
fn string_self_append_len(
    s: &Stmt,
    text: Symbol,
    variable_types: &HashMap<Symbol, String>,
    interner: &Interner,
) -> Option<i64> {
    let Stmt::Set { target, value } = s else {
        return None;
    };
    if *target != text {
        return None;
    }
    let Expr::BinaryOp {
        op: BinaryOpKind::Add,
        left,
        right,
    } = value
    else {
        return None;
    };
    if !matches!(left, Expr::Identifier(s) if *s == text) {
        return None;
    }
    append_operand_byte_len(right, variable_types, interner)
}

/// `Set c to c + K` for the given constant `K`.
fn is_increment_by_const(s: &Stmt, c: Symbol, k: i64) -> bool {
    let Stmt::Set { target, value } = s else {
        return false;
    };
    *target == c
        && matches!(value, Expr::BinaryOp { op: BinaryOpKind::Add, left, right }
            if matches!(left, Expr::Identifier(s) if *s == c)
                && matches!(right, Expr::Literal(Literal::Number(n)) if *n == k))
}

fn expr_mentions_sym(e: &Expr, sym: Symbol) -> bool {
    let mut ids = HashSet::new();
    super::detection::collect_expr_identifiers(e, &mut ids);
    ids.contains(&sym)
}

/// Verify a string `text` is built by CURSOR-LOCKSTEP appends in `body`: every
/// `Set text to text + X` is immediately followed by `Set c to c + len(X)`, `c`
/// is modified nowhere else (so `c` always equals the bytes written so far), and
/// `text` is never otherwise read (so the pre-zeroed buffer is never observed
/// mid-build). Returns the maximum single-append byte length, or None.
fn verify_indexed_string_build(
    body: &[Stmt],
    text: Symbol,
    c: Symbol,
    variable_types: &HashMap<Symbol, String>,
    interner: &Interner,
) -> Option<i64> {
    let mut max_k = 0i64;
    if isb_check_block(body, text, c, variable_types, interner, &mut max_k) && max_k > 0 {
        Some(max_k)
    } else {
        None
    }
}

fn isb_check_block(
    block: &[Stmt],
    text: Symbol,
    c: Symbol,
    vt: &HashMap<Symbol, String>,
    interner: &Interner,
    max_k: &mut i64,
) -> bool {
    let mut i = 0;
    while i < block.len() {
        // Recognized append `Set text=text+X` + its paired `Set c=c+len(X)`.
        if let Some(k) = string_self_append_len(&block[i], text, vt, interner) {
            if i + 1 >= block.len() || !is_increment_by_const(&block[i + 1], c, k) {
                return false;
            }
            *max_k = (*max_k).max(k);
            i += 2;
            continue;
        }
        match &block[i] {
            Stmt::If {
                cond,
                then_block,
                else_block,
            } => {
                if expr_mentions_sym(cond, text) {
                    return false;
                }
                if !isb_check_block(then_block, text, c, vt, interner, max_k) {
                    return false;
                }
                if let Some(eb) = else_block {
                    if !isb_check_block(eb, text, c, vt, interner, max_k) {
                        return false;
                    }
                }
            }
            Stmt::While { cond, body, .. } => {
                if expr_mentions_sym(cond, text)
                    || !isb_check_block(body, text, c, vt, interner, max_k)
                {
                    return false;
                }
            }
            Stmt::Repeat { body, .. } => {
                if !isb_check_block(body, text, c, vt, interner, max_k) {
                    return false;
                }
            }
            // A bare cursor modification (not paired with an append) breaks the
            // lockstep invariant; any other write/read of `text` disqualifies.
            Stmt::Set { target, value } => {
                if *target == text || *target == c || expr_mentions_sym(value, text) {
                    return false;
                }
            }
            Stmt::Let { value, .. } => {
                if expr_mentions_sym(value, text) {
                    return false;
                }
            }
            Stmt::Show { object, .. } => {
                if expr_mentions_sym(object, text) {
                    return false;
                }
            }
            Stmt::SetIndex {
                collection,
                index,
                value,
            } => {
                if expr_mentions_sym(collection, text)
                    || expr_mentions_sym(index, text)
                    || expr_mentions_sym(value, text)
                {
                    return false;
                }
            }
            Stmt::Break => {}
            Stmt::Return { value: None } => {}
            Stmt::Return { value: Some(v) } => {
                if expr_mentions_sym(v, text) {
                    return false;
                }
            }
            // Any other statement shape in the build loop is unanalyzed → bail.
            _ => return false,
        }
        i += 1;
    }
    true
}

/// Cursor-indexed string build: a `""`-initialised string grown ONLY by
/// cursor-lockstep appends in a counted loop is built like C — a pre-sized
/// buffer written at the cursor — instead of `String::push` (a capacity check +
/// length write-back + UTF-8 branch per byte). Recognizes
///
/// ```text
/// Let mutable text be "".
/// Let mutable c be 0.
/// While c < limit:                  (or `<=`)
///     … Set text to text + X. Set c to c + len(X). …   (every append paired)
/// ```
///
/// and emits a zero-filled `String` of `limit + maxlen + 1` bytes (`0u8` is
/// valid UTF-8, so `from_utf8_unchecked` is sound), rewrites each append to a
/// `get_unchecked_mut` write at the cursor (see the `Set` codegen), and
/// `set_len`s to the cursor after the loop. The cursor proof bounds every write
/// inside the buffer; `text` is never read mid-build, so the trailing zeros are
/// never observed.
#[allow(clippy::too_many_arguments)]
pub(crate) fn try_emit_indexed_string_build<'a>(
    stmts: &[&Stmt<'a>],
    idx: usize,
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
) -> Option<(String, usize)> {
    if !crate::optimize::active_config().is_on(crate::optimization::Opt::IndexString) {
        return None;
    }
    if idx + 2 >= stmts.len() {
        return None;
    }
    // s0: `Let mutable text be ""`.
    let Stmt::Let {
        var: text,
        value: tv,
        mutable,
        ..
    } = stmts[idx]
    else {
        return None;
    };
    let text = *text;
    if !*mutable && !mutable_vars.contains(&text) {
        return None;
    }
    match tv {
        Expr::Literal(Literal::Text(s)) if interner.resolve(*s).is_empty() => {}
        _ => return None,
    }
    // s1: `Let c be 0` (cursor starts at the empty-content position).
    let Stmt::Let {
        var: c, value: cv, ..
    } = stmts[idx + 1]
    else {
        return None;
    };
    let c = *c;
    if !matches!(cv, Expr::Literal(Literal::Number(0))) {
        return None;
    }
    // s2: `While c </<= limit { body }`.
    let while_stmt = stmts[idx + 2];
    let Stmt::While { cond, body, .. } = while_stmt else {
        return None;
    };
    let limit = match cond {
        Expr::BinaryOp {
            op: BinaryOpKind::Lt | BinaryOpKind::LtEq,
            left,
            right,
        } if matches!(left, Expr::Identifier(s) if *s == c) => *right,
        _ => return None,
    };
    if !is_simple_expr(limit) {
        return None;
    }
    let max_k = verify_indexed_string_build(body, text, c, ctx.get_variable_types(), interner)?;

    let names = RustNames::new(interner);
    let text_n = names.ident(text);
    let c_n = names.ident(c);
    // `c < limit` ⟹ every write `[c, c+k)` lands below `limit + maxlen`; `+ 1`
    // gives margin for the `<=` form and the final `set_len(c)`.
    let cap = format!(
        "(({}) + {})",
        codegen_expr_simple(limit, interner),
        max_k + 1
    );
    let ind = "    ".repeat(indent);
    let mut out = String::new();
    // Reserve the buffer but DON'T zero it (C's `malloc` doesn't): the appends
    // write into the spare capacity via raw pointer, then `set_len` exposes only
    // the written prefix. This is the standard manual-Vec-init pattern — sound
    // because the cursor proof writes every byte of `[0, c)` before `set_len(c)`,
    // and `text` is never read until then.
    writeln!(
        out,
        "{ind}let mut {text_n} = String::with_capacity(({cap}) as usize);"
    )
    .unwrap();
    writeln!(out, "{ind}let mut {c_n} = 0i64;").unwrap();
    ctx.register_string_var(text);
    ctx.register_indexed_string_build(text, c_n.clone());
    out.push_str(&super::codegen_stmt(
        while_stmt,
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
    writeln!(
        out,
        "{ind}unsafe {{ {text_n}.as_mut_vec().set_len(({c_n}) as usize); }}"
    )
    .unwrap();
    crate::optimize::mark_fired(crate::optimization::Opt::IndexString);
    Some((out, 2))
}

/// Pre-size hints for push-built Vecs: when `V` is index-read by `item counter
/// of V` inside a counted loop `While counter < / <= bound`, its length is
/// `>= bound`, so reserving `bound` up front removes the growth reallocations
/// (C sizes the buffer exactly with `malloc`). The capacity is a HINT, so this
/// is sound regardless of the actual fill count — it never changes semantics.
/// Returns `sym -> capacity expr string` (`(bound).max(0)`, clamped so a
/// negative bound can never request a giant allocation). The de-Rc Vec
/// declaration site applies it only where it would otherwise emit `Vec::new()`.
pub(crate) fn detect_vec_presize<'a>(
    stmts: &[Stmt<'a>],
    interner: &Interner,
) -> HashMap<Symbol, String> {
    // All top-level locals — used to tell a not-yet-declared LOCAL (unsafe to
    // reference at the Vec's decl) from a parameter/global (always in scope).
    let mut all_lets: HashSet<Symbol> = HashSet::new();
    for s in stmts {
        if let Stmt::Let { var, .. } = s {
            all_lets.insert(*var);
        }
    }
    let mut out: HashMap<Symbol, String> = HashMap::new();
    let mut declared: HashSet<Symbol> = HashSet::new();
    // The locals already in scope when each Vec is declared.
    let mut scope_at_decl: HashMap<Symbol, HashSet<Symbol>> = HashMap::new();
    // Top-level forward scan: a Vec is pre-sized only to a capacity that is
    // EVALUABLE AT ITS DECLARATION — a literal, or an identifier that is a
    // parameter/global or was declared before the Vec — and NEVER the Vec's own
    // `length` (use-before-def) or a not-yet-declared local.
    for s in stmts {
        match s {
            Stmt::Let { var, .. } => {
                scope_at_decl.insert(*var, declared.clone());
                declared.insert(*var);
            }
            Stmt::While { cond, body, .. } => {
                let Expr::BinaryOp {
                    op: BinaryOpKind::Lt | BinaryOpKind::LtEq,
                    left,
                    right,
                } = cond
                else {
                    continue;
                };
                if !matches!(&**left, Expr::Identifier(_)) {
                    continue;
                }
                let Expr::Identifier(counter) = &**left else {
                    continue;
                };
                let cap_sym = match &**right {
                    Expr::Identifier(b) => Some(*b),
                    Expr::Literal(Literal::Number(_)) => None,
                    _ => continue, // not declaration-safe (Length, arithmetic, …)
                };
                let cap = format!("({}).max(0)", codegen_expr_simple(right, interner));
                let mut reads: HashSet<Symbol> = HashSet::new();
                for st in body.iter() {
                    presize_reads_in_stmt(st, *counter, &mut reads);
                }
                for v in reads {
                    let in_scope = match cap_sym {
                        None => true, // literal bound — always available
                        Some(b) => {
                            b != v
                                && (!all_lets.contains(&b)
                                    || scope_at_decl.get(&v).is_some_and(|sc| sc.contains(&b)))
                        }
                    };
                    if in_scope {
                        out.entry(v).or_insert_with(|| cap.clone());
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Collect every `V` read as `item counter of V` (`Index { Identifier(V),
/// Identifier(counter) }`) anywhere in `stmt`'s expressions, recursively.
fn presize_reads_in_stmt(stmt: &Stmt, counter: Symbol, out: &mut HashSet<Symbol>) {
    match stmt {
        Stmt::Set { value, .. } | Stmt::Let { value, .. } => {
            presize_reads_in_expr(value, counter, out)
        }
        Stmt::Show { object, .. } => presize_reads_in_expr(object, counter, out),
        Stmt::SetIndex { index, value, .. } => {
            presize_reads_in_expr(index, counter, out);
            presize_reads_in_expr(value, counter, out);
        }
        Stmt::RuntimeAssert { condition, .. } => presize_reads_in_expr(condition, counter, out),
        Stmt::Return { value: Some(v) } => presize_reads_in_expr(v, counter, out),
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => {
            presize_reads_in_expr(cond, counter, out);
            for s in then_block.iter() {
                presize_reads_in_stmt(s, counter, out);
            }
            if let Some(e) = else_block {
                for s in e.iter() {
                    presize_reads_in_stmt(s, counter, out);
                }
            }
        }
        Stmt::While { cond, body, .. } => {
            presize_reads_in_expr(cond, counter, out);
            for s in body.iter() {
                presize_reads_in_stmt(s, counter, out);
            }
        }
        _ => {}
    }
}

fn presize_reads_in_expr(e: &Expr, counter: Symbol, out: &mut HashSet<Symbol>) {
    match e {
        Expr::Index { collection, index } => {
            if let (Expr::Identifier(v), Expr::Identifier(idx)) = (&**collection, &**index) {
                if *idx == counter {
                    out.insert(*v);
                }
            }
            presize_reads_in_expr(collection, counter, out);
            presize_reads_in_expr(index, counter, out);
        }
        Expr::BinaryOp { left, right, .. } => {
            presize_reads_in_expr(left, counter, out);
            presize_reads_in_expr(right, counter, out);
        }
        Expr::Not { operand } => presize_reads_in_expr(operand, counter, out),
        _ => {}
    }
}

/// `arr[index]` with an identifier collection ⟹ `(arr, collection_expr, index)`.
fn index_into<'a>(e: &'a Expr<'a>) -> Option<(Symbol, &'a Expr<'a>, &'a Expr<'a>)> {
    if let Expr::Index { collection, index } = e {
        if let Expr::Identifier(arr) = collection {
            return Some((*arr, collection, index));
        }
    }
    None
}

/// `base + j` or `j + base` (j a bare identifier, `base` free of j) ⟹ `base`.
fn affine_base_in_j<'a>(idx: &'a Expr<'a>, j: Symbol) -> Option<&'a Expr<'a>> {
    let Expr::BinaryOp {
        op: BinaryOpKind::Add,
        left,
        right,
    } = idx
    else {
        return None;
    };
    let is_j = |e: &Expr| matches!(e, Expr::Identifier(s) if *s == j);
    if is_j(left) && !mentions(right, j) {
        Some(right)
    } else if is_j(right) && !mentions(left, j) {
        Some(left)
    } else {
        None
    }
}

fn mentions(e: &Expr, sym: Symbol) -> bool {
    match e {
        Expr::Identifier(s) => *s == sym,
        Expr::BinaryOp { left, right, .. } => mentions(left, sym) || mentions(right, sym),
        Expr::Index { collection, index } => mentions(collection, sym) || mentions(index, sym),
        Expr::Length { collection } => mentions(collection, sym),
        Expr::Not { operand } => mentions(operand, sym),
        _ => false,
    }
}

/// `Set c to c + 1`.
fn is_increment_by_one_of(s: &Stmt, c: Symbol) -> bool {
    let Stmt::Set { target, value } = s else {
        return false;
    };
    if *target != c {
        return false;
    }
    let Expr::BinaryOp {
        op: BinaryOpKind::Add,
        left,
        right,
    } = value
    else {
        return false;
    };
    let is_c = |e: &Expr| matches!(e, Expr::Identifier(s) if *s == c);
    let is_one = |e: &Expr| matches!(e, Expr::Literal(Literal::Number(1)));
    (is_c(left) && is_one(right)) || (is_one(left) && is_c(right))
}

/// A String/`&str` — byte-indexable via `as_bytes()`. Sequences are not.
fn is_byte_indexable(t: Option<&String>) -> bool {
    matches!(t, Some(t) if t.contains("String") || t == &"&str" || t.contains("str"))
}

/// Recognize the in-place converging-swap prefix-reversal idiom and lower it to
/// a single slice `.reverse()`:
///
/// ```text
/// While LO < HI:
///     Let TMP be item LO of C.
///     Set item LO of C to item HI of C.
///     Set item HI of C to TMP.
///     Set LO to LO + 1.
///     Set HI to HI - 1.
/// ```
/// →  `C.logos_reverse_between(LO, HI);`
///
/// `LogosItems::logos_reverse_between` is this loop under the one index rule: when both ends count
/// from the front and name items, it is the slice's own `<[T]>::reverse` (one bounds check instead of N,
/// and the per-element panic branches that block LLVM auto-vectorization are gone, so it is the
/// vectorized reverse a C compiler produces for this loop); any other pair of ends runs the loop index
/// by index, counting from the back and failing where the loop fails.
///
/// Sound because `LO`/`HI`/`TMP` must not be read after the loop (they are left at their entry values,
/// which is then unobservable). General — it fires on any in-place reversal, not just fannkuch's flip.
pub(crate) fn try_emit_prefix_reverse<'a>(
    stmts: &[&Stmt<'a>],
    idx: usize,
    interner: &Interner,
    indent: usize,
    variable_types: &HashMap<Symbol, String>,
) -> Option<(String, usize)> {
    let (cond, body) = match stmts[idx] {
        Stmt::While { cond, body, .. } => (cond, body),
        _ => return None,
    };
    // Guard: LO < HI (both plain loop variables).
    let (lo, hi) = match cond {
        Expr::BinaryOp {
            op: BinaryOpKind::Lt,
            left: Expr::Identifier(l),
            right: Expr::Identifier(h),
        } => (*l, *h),
        _ => return None,
    };
    if lo == hi || body.len() != 5 {
        return None;
    }
    // body[0]: Let TMP be item LO of C.
    let (tmp, coll) = match &body[0] {
        Stmt::Let {
            var,
            value:
                Expr::Index {
                    collection: Expr::Identifier(c),
                    index: Expr::Identifier(li),
                },
            mutable: false,
            ..
        } if *li == lo => (*var, *c),
        _ => return None,
    };
    // body[1]: Set item LO of C to item HI of C.
    match &body[1] {
        Stmt::SetIndex {
            collection: Expr::Identifier(c),
            index: Expr::Identifier(li),
            value:
                Expr::Index {
                    collection: Expr::Identifier(c2),
                    index: Expr::Identifier(hj),
                },
        } if *c == coll && *li == lo && *c2 == coll && *hj == hi => {}
        _ => return None,
    }
    // body[2]: Set item HI of C to TMP.
    match &body[2] {
        Stmt::SetIndex {
            collection: Expr::Identifier(c),
            index: Expr::Identifier(hj),
            value: Expr::Identifier(t),
        } if *c == coll && *hj == hi && *t == tmp => {}
        _ => return None,
    }
    // body[3]/body[4]: Set LO to LO + 1 and Set HI to HI - 1 (either order).
    let is_step = |s: &Stmt, sym: Symbol, op: BinaryOpKind| -> bool {
        matches!(s, Stmt::Set { target, value }
            if *target == sym
            && matches!(value, Expr::BinaryOp { op: o, left: Expr::Identifier(l), right: Expr::Literal(Literal::Number(1)) }
                if *o == op && *l == sym))
    };
    let inc_lo = |s: &Stmt| is_step(s, lo, BinaryOpKind::Add);
    let dec_hi = |s: &Stmt| is_step(s, hi, BinaryOpKind::Subtract);
    if !((inc_lo(&body[3]) && dec_hi(&body[4])) || (inc_lo(&body[4]) && dec_hi(&body[3]))) {
        return None;
    }
    // LO/HI/TMP must not be read after the loop in this block — else leaving them
    // at their entry values (we do not run the loop) would be observable.
    for s in &stmts[idx + 1..] {
        if symbol_appears_in_stmts(lo, &[s])
            || symbol_appears_in_stmts(hi, &[s])
            || symbol_appears_in_stmts(tmp, &[s])
        {
            return None;
        }
    }
    // The collection must be a directly-sliceable Vec / slice (a de-Rc'd buffer);
    // a `LogosSeq` reverses through `.borrow_mut()`.
    let coll_name = interner.resolve(coll);
    let lo_name = interner.resolve(lo);
    let hi_name = interner.resolve(hi);
    let ty = variable_types
        .get(&coll)
        .map(|t| t.split("|__hl:").next().unwrap_or(t).to_string());
    let slice_owner = match ty.as_deref() {
        Some(t) if t.starts_with("LogosSeq") => format!("{}.borrow_mut()", coll_name),
        Some(t) if t.starts_with("Vec") || t.starts_with("&mut [") || t.starts_with('[') => {
            coll_name.to_string()
        }
        _ => return None,
    };
    let indent_str = "    ".repeat(indent);
    let code = format!("{indent_str}{slice_owner}.logos_reverse_between({lo_name}, {hi_name});\n");
    Some((code, 0))
}

/// The swap peephole's checked form would REINTRODUCE the checks the oracle
/// already eliminated. When both swap indices are RELATIONALLY proven in bounds
/// (e.g. quicksort's partition `i`/`j`, where the entry-guard
/// proves `i <= j < hi <= len`), the statements' own `get_unchecked` indexed
/// form is strictly faster — so the swap peephole defers to it. The relational
/// gate is deliberate: literal-index swaps (interval-proven) keep the checked
/// `__swap_tmp` form.
fn swap_unchecked_proven(
    oracle: Option<&crate::optimize::OracleFacts>,
    i: &Expr,
    j: &Expr,
) -> bool {
    oracle.map_or(false, |o| {
        o.index_proven_relational(i) && o.index_proven_relational(j)
    })
}

/// The names the swap peepholes hold the two elements in, first and second in source order. The checked form's
/// first element is `__swap_tmp`, the name that marks that form in the emitted code; the proven, unchecked form
/// holds `__cs_e1` and `__cs_e2`.
fn swap_temps(unchecked: bool) -> (&'static str, &'static str) {
    if unchecked {
        ("__cs_e1", "__cs_e2")
    } else {
        ("__swap_tmp", "__swap_with")
    }
}

/// The read and the store a swap peephole emits for one item of `arr_name`: unchecked where the oracle
/// proved both indices name items, a plain indexed access where the program proves this one does
/// (`proven`), and through the one index rule (`LogosItems`) otherwise. A read clones, so a swap of
/// non-`Copy` items moves nothing out of the sequence; a `Copy` item's clone is a copy.
fn swap_accessors<'n>(
    arr_name: &'n str,
    is_logos_seq: bool,
    unchecked: bool,
    interner: &'n Interner,
    variable_types: &'n HashMap<Symbol, String>,
    oracle: Option<&'n crate::optimize::OracleFacts>,
) -> (
    impl Fn(&Expr, bool) -> String + 'n,
    impl Fn(&Expr, bool, &str) -> String + 'n,
) {
    let offset = move |index: &Expr| simplify_1based_index(index, interner, true, variable_types);
    let read = move |index: &Expr, proven: bool| -> String {
        let owner = if is_logos_seq {
            format!("{}.borrow()", arr_name)
        } else {
            arr_name.to_string()
        };
        if unchecked {
            format!(
                "unsafe {{ {}.get_unchecked({}).clone() }}",
                arr_name,
                offset(index)
            )
        } else if proven {
            format!("{}[{}].clone()", owner, offset(index))
        } else {
            format!(
                "{}.{}({}).clone()",
                owner,
                super::expr::item_accessor(index, oracle, false),
                logos_index(index, interner, variable_types)
            )
        }
    };
    let write = move |index: &Expr, proven: bool, value: &str| -> String {
        let owner = if is_logos_seq {
            format!("{}.borrow_mut()", arr_name)
        } else {
            arr_name.to_string()
        };
        if unchecked {
            format!(
                "unsafe {{ *{}.get_unchecked_mut({}) = {}; }}",
                arr_name,
                offset(index),
                value
            )
        } else if proven {
            format!("{}[{}] = {};", owner, offset(index), value)
        } else {
            format!(
                "*{}.{}({}) = {};",
                owner,
                super::expr::item_accessor(index, oracle, true),
                logos_index(index, interner, variable_types),
                value
            )
        }
    };
    (read, write)
}

pub(crate) fn try_emit_swap_pattern<'a>(
    stmts: &[&Stmt<'a>],
    idx: usize,
    interner: &Interner,
    indent: usize,
    variable_types: &HashMap<Symbol, String>,
    oracle: Option<&crate::optimize::OracleFacts>,
) -> Option<(String, usize)> {
    if idx + 2 >= stmts.len() {
        return None;
    }

    // Statement 1: Let tmp be item I of arr (index expression)
    let (a_sym, arr_sym_1, idx_expr_1, coll_expr_1) = match stmts[idx] {
        Stmt::Let {
            var,
            value: Expr::Index { collection, index },
            mutable: false,
            ..
        } => {
            if let Expr::Identifier(coll_sym) = collection {
                (*var, *coll_sym, *index, *collection)
            } else {
                return None;
            }
        }
        _ => return None,
    };

    // Only optimize for known Vec / &mut [T] / fixed-array (O3) types — all
    // support direct indexing and `.swap()`.
    if let Some(t) = variable_types.get(&arr_sym_1) {
        if !t.starts_with("LogosSeq")
            && !t.starts_with("Vec")
            && !t.starts_with("&mut [")
            && !t.starts_with('[')
        {
            return None;
        }
    } else {
        return None;
    }

    // Try Pattern B first: unconditional 3-statement swap (more general)
    // Statement 2: Set item I of arr to item J of arr.
    // Statement 3: Set item J of arr to tmp.
    if let Some(result) = try_emit_unconditional_swap(
        stmts,
        idx,
        a_sym,
        arr_sym_1,
        idx_expr_1,
        coll_expr_1,
        interner,
        indent,
        variable_types,
        oracle,
    ) {
        return Some(result);
    }

    // Pattern A: conditional swap with any two indices from the same array.
    // Statement 2: Let b be item J of arr (J can be any index, not just I+1)
    let (b_sym, arr_sym_2, idx_expr_2, coll_expr_2) = match stmts[idx + 1] {
        Stmt::Let {
            var,
            value: Expr::Index { collection, index },
            mutable: false,
            ..
        } => {
            if let Expr::Identifier(coll_sym) = collection {
                (*var, *coll_sym, *index, *collection)
            } else {
                return None;
            }
        }
        _ => return None,
    };

    // Must be the same array
    if arr_sym_1 != arr_sym_2 {
        return None;
    }

    // Both index expressions must be simple enough for codegen_expr_simple
    if !is_simple_expr(idx_expr_1) || !is_simple_expr(idx_expr_2) {
        return None;
    }

    // Statement 3: If a OP b: SetIndex arr I b, SetIndex arr J a (cross-swap)
    match stmts[idx + 2] {
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => {
            // Condition must compare a and b
            let compares_a_b = match cond {
                Expr::BinaryOp { op, left, right } => {
                    matches!(
                        op,
                        BinaryOpKind::Gt
                            | BinaryOpKind::Lt
                            | BinaryOpKind::GtEq
                            | BinaryOpKind::LtEq
                            | BinaryOpKind::Eq
                            | BinaryOpKind::NotEq
                    ) && ((matches!(left, Expr::Identifier(s) if *s == a_sym)
                        && matches!(right, Expr::Identifier(s) if *s == b_sym))
                        || (matches!(left, Expr::Identifier(s) if *s == b_sym)
                            && matches!(right, Expr::Identifier(s) if *s == a_sym)))
                }
                _ => false,
            };
            if !compares_a_b {
                return None;
            }

            // Must have no else block
            if else_block.is_some() {
                return None;
            }

            // Then block must have exactly 2 SetIndex statements forming a cross-swap
            if then_block.len() != 2 {
                return None;
            }

            // Check: SetIndex arr idx1 b, SetIndex arr idx2 a (cross pattern)
            let swap_ok = match (&then_block[0], &then_block[1]) {
                (
                    Stmt::SetIndex {
                        collection: c1,
                        index: i1,
                        value: v1,
                    },
                    Stmt::SetIndex {
                        collection: c2,
                        index: i2,
                        value: v2,
                    },
                ) => {
                    // c1 and c2 must be the same array
                    let same_arr = matches!((c1, c2), (Expr::Identifier(s1), Expr::Identifier(s2)) if *s1 == arr_sym_1 && *s2 == arr_sym_1);
                    // Cross pattern: set idx1 to b, set idx2 to a
                    let cross = exprs_equal(i1, idx_expr_1)
                        && exprs_equal(i2, idx_expr_2)
                        && matches!(v1, Expr::Identifier(s) if *s == b_sym)
                        && matches!(v2, Expr::Identifier(s) if *s == a_sym);
                    // Also check reverse: set idx1 to b via idx2/a pattern
                    let cross_rev = exprs_equal(i1, idx_expr_2)
                        && exprs_equal(i2, idx_expr_1)
                        && matches!(v1, Expr::Identifier(s) if *s == a_sym)
                        && matches!(v2, Expr::Identifier(s) if *s == b_sym);
                    same_arr && (cross || cross_rev)
                }
                _ => false,
            };

            if !swap_ok {
                return None;
            }

            // Pattern matched! The program reads item I, then item J, then compares them and stores each
            // into the other's place. The emission keeps that order exactly — both reads first, in the
            // source's order, each through the one index rule — so an index that names no item fails with
            // the sentence and at the read every tier fails at.
            let indent_str = "    ".repeat(indent);
            let arr_name = interner.resolve(arr_sym_1);

            // The source guard may be written `a OP b` OR `b OP a`; the comparison keeps its operands in
            // source order so the swap fires on exactly the condition the program wrote.
            let guard_left_is_a = matches!(
                cond,
                Expr::BinaryOp { left, .. }
                    if matches!(left, Expr::Identifier(s) if *s == a_sym)
            );

            let op_str = match cond {
                Expr::BinaryOp { op, .. } => match op {
                    BinaryOpKind::Gt => ">",
                    BinaryOpKind::Lt => "<",
                    BinaryOpKind::GtEq => ">=",
                    BinaryOpKind::LtEq => "<=",
                    BinaryOpKind::Eq => "==",
                    BinaryOpKind::NotEq => "!=",
                    _ => unreachable!(),
                },
                _ => unreachable!(),
            };

            let is_logos_seq = variable_types
                .get(&arr_sym_1)
                .map_or(false, |t| t.starts_with("LogosSeq"));

            // BCE for the compare-swap: when the bounds-elision oracle proves BOTH
            // swapped indices in range, the swap lowers to `get_unchecked` — killing
            // the per-iteration bounds branch that costs comparison sorts their gap
            // vs C/Zig (the `assert_unchecked` hint alone does not survive LTO; a
            // direct `get_unchecked` does). The proof is the SAME sound machinery the
            // general read path uses (relational SCEV bound from the loop guard, or a
            // concrete interval); if either index is unproven we keep the checked form.
            let swap_unchecked = !is_logos_seq
                && oracle.map_or(false, |o| {
                    o.index_provably_in_bounds(coll_expr_1, idx_expr_1)
                        && o.index_provably_in_bounds(coll_expr_2, idx_expr_2)
                });
            if swap_unchecked {
                crate::optimize::mark_fired(crate::optimization::Opt::Unchecked);
            }
            let (read, write) = swap_accessors(
                arr_name,
                is_logos_seq,
                swap_unchecked,
                interner,
                variable_types,
                oracle,
            );
            let (first, second) = swap_temps(swap_unchecked);
            let (ge_lhs, ge_rhs) = if guard_left_is_a {
                (first, second)
            } else {
                (second, first)
            };
            // Both items are read before either is stored and nothing between changes the length, so a
            // read the program proves in range proves the store at the same index.
            let proven_1 = super::expr::range_proves_index(coll_expr_1, idx_expr_1);
            let proven_2 = super::expr::range_proves_index(coll_expr_2, idx_expr_2);

            let mut output = String::new();
            writeln!(
                output,
                "{}let {} = {};",
                indent_str,
                first,
                read(idx_expr_1, proven_1)
            )
            .unwrap();
            writeln!(
                output,
                "{}let {} = {};",
                indent_str,
                second,
                read(idx_expr_2, proven_2)
            )
            .unwrap();
            // This peephole consumes the `Let a = item I` / `Let b = item J` bindings. A binding read
            // AFTER this block keeps the value it had before the swap.
            if value_read_after(a_sym, stmts, idx + 3) {
                writeln!(
                    output,
                    "{}let {} = {}.clone();",
                    indent_str,
                    interner.resolve(a_sym),
                    first
                )
                .unwrap();
            }
            if value_read_after(b_sym, stmts, idx + 3) {
                writeln!(
                    output,
                    "{}let {} = {}.clone();",
                    indent_str,
                    interner.resolve(b_sym),
                    second
                )
                .unwrap();
            }
            writeln!(
                output,
                "{}if {} {} {} {{",
                indent_str, ge_lhs, op_str, ge_rhs
            )
            .unwrap();
            writeln!(
                output,
                "{}    {}",
                indent_str,
                write(idx_expr_1, proven_1, second)
            )
            .unwrap();
            writeln!(
                output,
                "{}    {}",
                indent_str,
                write(idx_expr_2, proven_2, first)
            )
            .unwrap();
            writeln!(output, "{}}}", indent_str).unwrap();

            Some((output, 2)) // consumed 2 extra statements
        }
        _ => None,
    }
}

/// The items a counted push loop from `start` to `end` reads from `owner` (a sequence expression that
/// indexes as a slice), as one slice under the loop's own rule (`LogosItems::logos_run_through`, or
/// `logos_run_before` for `<`): the same items, and the same stop at the first index that names none.
fn copied_run(owner: &str, start: &str, end: &str, exclusive: bool) -> String {
    let method = if exclusive {
        "logos_run_before"
    } else {
        "logos_run_through"
    };
    format!("{}.{}({}, {})", owner, method, start, end)
}

/// The counter as a counted loop from `start` leaves it: one past `end` (or `end` itself for `<`), or
/// `start` when the loop never ran.
fn counter_after(start: &str, end: &str, exclusive: bool) -> String {
    let past = if exclusive {
        end.to_string()
    } else {
        format!("{} + 1", end)
    };
    format!("std::cmp::max({}, {})", start, past)
}

/// Whether a counted copy from `start` to `end` (`end` itself excluded when `exclusive`) copies exactly one
/// plain slice of its source, the items from `start` through the last one it reads: both bounds are literals,
/// the loop runs at least once, and `reads_proven` says every read names an item. A read in a loop that never
/// runs names an item vacuously, so a proof of it says nothing about the slice, which is evaluated either way.
fn copy_is_a_plain_slice(start: &Expr, end: &Expr, exclusive: bool, reads_proven: bool) -> bool {
    reads_proven
        && match (start, end) {
            (Expr::Literal(Literal::Number(a)), Expr::Literal(Literal::Number(b))) => {
                if exclusive {
                    a < b
                } else {
                    a <= b
                }
            }
            _ => false,
        }
}

/// Peephole optimization: detect element-by-element array copying via push loop
/// and emit slice operations instead.
///
/// Pattern:
///   Let [mutable] dst be a new Seq of T.
///   [intervening statements that don't reference dst]
///   Let/Set counter = start.
///   While counter <= end:
///       Push item counter of source to dst.
///       Set counter to counter + 1.
///
/// Relaxed pattern: allows arbitrary intervening statements between the Seq creation
/// and the counter-init + While pair, as long as they don't reference the Seq variable.
///
/// Full copy (start=1, end=length of source): `let mut dst: Vec<T> = source.to_vec();`
/// Partial slice: `let mut dst: Vec<T> = source[(start-1) as usize..end as usize].to_vec();`
///
/// Returns (generated_code, number_of_extra_statements_consumed) or None.
pub(crate) fn try_emit_seq_from_slice_pattern<'a>(
    stmts: &[&Stmt<'a>],
    idx: usize,
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
) -> Option<(String, usize)> {
    if idx + 2 >= stmts.len() {
        return None;
    }

    // Statement 1: Let mutable dst be a new Seq of T.
    let (dst_sym, elem_type) = match stmts[idx] {
        Stmt::Let {
            var,
            value,
            ty,
            mutable: true,
            ..
        } => {
            let type_from_annotation = if let Some(TypeExpr::Generic { base, params }) = ty {
                let base_name = interner.resolve(*base);
                if matches!(base_name, "Seq" | "List" | "Vec") && !params.is_empty() {
                    Some(codegen_type_expr(&params[0], interner))
                } else {
                    None
                }
            } else {
                None
            };

            let type_from_new = if let Expr::New {
                type_name,
                type_args,
                init_fields,
            } = value
            {
                let tn = interner.resolve(*type_name);
                if matches!(tn, "Seq" | "List" | "Vec") && init_fields.is_empty() {
                    if !type_args.is_empty() {
                        Some(codegen_type_expr(&type_args[0], interner))
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            };

            match type_from_annotation.or(type_from_new) {
                Some(t) => (*var, t),
                None => return None,
            }
        }
        _ => return None,
    };

    // Scan forward from idx+1 for a counter-init + While pair.
    // Skip intervening statements that don't reference dst_sym.
    let mut counter_init_idx: Option<usize> = None;

    for scan in (idx + 1)..stmts.len() {
        let stmt = stmts[scan];

        let is_counter_init = match stmt {
            Stmt::Let { value, .. } if is_simple_expr(value) => true,
            Stmt::Set { value, .. } if is_simple_expr(value) => true,
            _ => false,
        };

        if is_counter_init && scan + 1 < stmts.len() {
            let c_sym = match stmt {
                Stmt::Let { var, .. } => *var,
                Stmt::Set { target, .. } => *target,
                _ => unreachable!(),
            };

            if let Stmt::While { cond, body, .. } = stmts[scan + 1] {
                let cond_ok = match cond {
                    Expr::BinaryOp {
                        op: BinaryOpKind::LtEq | BinaryOpKind::Lt,
                        left,
                        ..
                    } => {
                        matches!(left, Expr::Identifier(sym) if *sym == c_sym)
                    }
                    _ => false,
                };

                if cond_ok && body.len() == 2 {
                    let push_to_dst = match &body[0] {
                        Stmt::Push { collection, value } => {
                            if !matches!(collection, Expr::Identifier(s) if *s == dst_sym) {
                                false
                            } else if let Expr::Index { index, .. } = value {
                                matches!(index, Expr::Identifier(s) if *s == c_sym)
                            } else {
                                false
                            }
                        }
                        _ => false,
                    };

                    let inc_ok = match &body[1] {
                        Stmt::Set { target, value } if *target == c_sym => {
                            matches!(value, Expr::BinaryOp { op: BinaryOpKind::Add, left, right }
                                if (matches!(left, Expr::Identifier(s) if *s == c_sym) && matches!(right, Expr::Literal(Literal::Number(1))))
                                || (matches!(left, Expr::Literal(Literal::Number(1))) && matches!(right, Expr::Identifier(s) if *s == c_sym))
                            )
                        }
                        _ => false,
                    };

                    if push_to_dst && inc_ok {
                        counter_init_idx = Some(scan);
                        break;
                    }
                }
            }
        }

        // Continuation slice: bare While with no counter re-init.
        // The counter was already initialized by a prior loop and carries over.
        if let Stmt::While { cond, body, .. } = stmt {
            if body.len() == 2 {
                if let Some((c_sym, c_end_expr, c_is_exclusive)) = extract_while_cond(cond) {
                    if is_simple_expr(c_end_expr) {
                        if let Some((c_src_sym, c_dst_check)) = extract_push_index_body(body, c_sym)
                        {
                            if c_dst_check == dst_sym
                                && c_src_sym != dst_sym
                                && !symbol_appears_in_expr(dst_sym, c_end_expr)
                                && !symbol_appears_in_expr(c_sym, c_end_expr)
                            {
                                // Continuation slice matched! The counter's current runtime
                                // value is the slice start.
                                let indent_str = "    ".repeat(indent);
                                let names = RustNames::new(interner);
                                let dst_name = interner.resolve(dst_sym);
                                let src_name = interner.resolve(c_src_sym);
                                let counter_name = names.ident(c_sym);
                                let end_str = codegen_expr_simple(c_end_expr, interner);

                                let mut cont_output = String::new();

                                // Check source type to determine if .borrow() is needed
                                let src_is_logos_seq = ctx
                                    .get_variable_types()
                                    .get(&c_src_sym)
                                    .map(|t| t.split("|__hl:").next().unwrap_or(t.as_str()))
                                    .map(|t| t.starts_with("LogosSeq"))
                                    .unwrap_or(true); // default to LogosSeq
                                let borrow_prefix = if src_is_logos_seq {
                                    format!("{}.borrow()", src_name)
                                } else {
                                    src_name.to_string()
                                };

                                // De-Rc-aware: a de-Rc'd target is a plain owned
                                // `Vec<T>` (the `.to_vec()` already yields one), no
                                // `LogosSeq::from_vec` wrapper. Mutable because the
                                // build target is typically reassigned later.
                                let dst_de_rc = ctx.is_de_rc(dst_sym);
                                // The statements between the Seq's creation and the loop run before the loop
                                // does, and may set the counter or the source the copy reads.
                                for si in (idx + 1)..scan {
                                    use super::codegen_stmt;
                                    cont_output.push_str(&codegen_stmt(
                                        stmts[si],
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
                                let run = copied_run(
                                    &borrow_prefix,
                                    &counter_name,
                                    &end_str,
                                    c_is_exclusive,
                                );
                                if dst_de_rc {
                                    writeln!(
                                        cont_output,
                                        "{}let mut {}: Vec<{}> = {}.to_vec();",
                                        indent_str, dst_name, elem_type, run
                                    )
                                    .unwrap();
                                    ctx.register_variable_type(
                                        dst_sym,
                                        format!("Vec<{}>", elem_type),
                                    );
                                } else {
                                    writeln!(
                                        cont_output,
                                        "{}let {}: LogosSeq<{}> = LogosSeq::from_vec({}.to_vec());",
                                        indent_str, dst_name, elem_type, run
                                    )
                                    .unwrap();
                                    ctx.register_variable_type(
                                        dst_sym,
                                        format!("LogosSeq<{}>", elem_type),
                                    );
                                }

                                // Re-emit the counter's post-loop value wherever it can be read.
                                if value_read_after(c_sym, stmts, scan + 1) {
                                    writeln!(
                                        cont_output,
                                        "{}{} = {};",
                                        indent_str,
                                        counter_name,
                                        counter_after(&counter_name, &end_str, c_is_exclusive)
                                    )
                                    .unwrap();
                                }

                                let extra_consumed = scan - idx;
                                return Some((cont_output, extra_consumed));
                            }
                        }
                    }
                }
            }
        }

        // This statement doesn't start the pattern. If it references dst_sym, bail.
        if symbol_appears_in_stmts(dst_sym, &[stmt]) {
            return None;
        }
    }

    let counter_idx = counter_init_idx?;
    let while_idx = counter_idx + 1;

    // Extract counter details
    let (counter_sym, start_expr, counter_is_new_binding) = match stmts[counter_idx] {
        Stmt::Let { var, value, .. } => {
            if is_simple_expr(value) {
                (*var, *value, true)
            } else {
                return None;
            }
        }
        Stmt::Set { target, value } => {
            if is_simple_expr(value) {
                (*target, *value, false)
            } else {
                return None;
            }
        }
        _ => return None,
    };

    // Match the While loop for detailed extraction
    match stmts[while_idx] {
        Stmt::While { cond, body, .. } => {
            let (end_expr, is_exclusive) = match cond {
                Expr::BinaryOp {
                    op: BinaryOpKind::LtEq,
                    left,
                    right,
                } => {
                    if let Expr::Identifier(c) = left {
                        if *c == counter_sym {
                            (Some(*right), false)
                        } else {
                            (None, false)
                        }
                    } else {
                        (None, false)
                    }
                }
                Expr::BinaryOp {
                    op: BinaryOpKind::Lt,
                    left,
                    right,
                } => {
                    if let Expr::Identifier(c) = left {
                        if *c == counter_sym {
                            (Some(*right), true)
                        } else {
                            (None, false)
                        }
                    } else {
                        (None, false)
                    }
                }
                _ => (None, false),
            };
            let end_expr = end_expr?;

            if body.len() != 2 {
                return None;
            }

            let (src_sym, read_coll, read_index) = match &body[0] {
                Stmt::Push { value, collection } => {
                    if !matches!(collection, Expr::Identifier(s) if *s == dst_sym) {
                        return None;
                    }
                    if let Expr::Index {
                        collection: idx_coll,
                        index: idx_expr,
                    } = value
                    {
                        if !matches!(idx_expr, Expr::Identifier(s) if *s == counter_sym) {
                            return None;
                        }
                        if let Expr::Identifier(s) = idx_coll {
                            (*s, *idx_coll, *idx_expr)
                        } else {
                            return None;
                        }
                    } else {
                        return None;
                    }
                }
                _ => return None,
            };

            match &body[1] {
                Stmt::Set { target, value } => {
                    if *target != counter_sym {
                        return None;
                    }
                    match value {
                        Expr::BinaryOp {
                            op: BinaryOpKind::Add,
                            left,
                            right,
                        } => {
                            let ok = match (left, right) {
                                (Expr::Identifier(s), Expr::Literal(Literal::Number(1))) => {
                                    *s == counter_sym
                                }
                                (Expr::Literal(Literal::Number(1)), Expr::Identifier(s)) => {
                                    *s == counter_sym
                                }
                                _ => false,
                            };
                            if !ok {
                                return None;
                            }
                        }
                        _ => return None,
                    }
                }
                _ => return None,
            }

            // The loop reads its bounds every iteration and pushes into `dst` as it reads `src`; one copy
            // means the same only when neither bound reads what the loop changes and `src` is not `dst`.
            if src_sym == dst_sym
                || !is_simple_expr(end_expr)
                || [start_expr, end_expr].iter().any(|e| {
                    symbol_appears_in_expr(dst_sym, e) || symbol_appears_in_expr(counter_sym, e)
                })
            {
                return None;
            }

            // Pattern matched! Determine if it's a full copy or partial slice.
            let indent_str = "    ".repeat(indent);
            let names = RustNames::new(interner);
            let dst_name = interner.resolve(dst_sym);
            let src_name = interner.resolve(src_sym);
            let counter_name = names.ident(counter_sym);

            let is_start_one = matches!(start_expr, Expr::Literal(Literal::Number(1)));
            let is_end_length_of_src = if !is_exclusive {
                matches!(end_expr, Expr::Length { collection } if matches!(collection, Expr::Identifier(s) if *s == src_sym))
            } else {
                false
            };

            let mut output = String::new();

            // Check source type: a de-Rc'd `Vec<T>` or a borrowed slice — `&[T]` from a read-only
            // hoist, `&mut [T]` from a mutable one or a mutably borrowed parameter, a fixed `[T; N]` —
            // indexes directly; a `LogosSeq<T>` (the reference-semantics default) needs `.borrow()`.
            // The hoist restores each handle's type when its region closes, so the registered type is
            // the one in force here. When the source's type is UNKNOWN (a builtin like `text_bytes`
            // whose return type never reached the codegen's legacy map), default to LogosSeq — the
            // same "default to LogosSeq" the continuation-slice path takes — so the slice reads
            // through `.borrow()` instead of trying to index the `LogosSeq` directly (which has no
            // `Index<Range>` impl → E0608).
            let src_type = ctx
                .get_variable_types()
                .get(&src_sym)
                .map(|t| t.split("|__hl:").next().unwrap_or(t.as_str()).to_string())
                .unwrap_or_default();
            let needs_borrow = !(src_type.starts_with("Vec<")
                || src_type.starts_with("&[")
                || src_type.starts_with("&mut [")
                || src_type.starts_with('['));
            let borrow_prefix = if needs_borrow { ".borrow()" } else { "" };

            // The owned-`Vec` initializer for this copy/slice shape. A de-Rc'd
            // target IS that owned Vec (no `LogosSeq::from_vec` wrapper); a
            // LogosSeq target wraps it. Computed once so both emit consistently.
            let runs_proven = copy_is_a_plain_slice(
                start_expr,
                end_expr,
                is_exclusive,
                super::expr::range_proves_index(read_coll, read_index),
            );
            let vec_init = if is_start_one && is_end_length_of_src {
                format!("{}{}.to_vec()", src_name, borrow_prefix)
            } else {
                let start_str = codegen_expr_simple(start_expr, interner);
                let end_str = codegen_expr_simple(end_expr, interner);
                if runs_proven {
                    let stop = if is_exclusive {
                        format!("({} - 1)", end_str)
                    } else {
                        end_str.clone()
                    };
                    format!(
                        "{}{}[({} - 1) as usize..{} as usize].to_vec()",
                        src_name, borrow_prefix, start_str, stop
                    )
                } else {
                    format!(
                        "{}.to_vec()",
                        copied_run(
                            &format!("{}{}", src_name, borrow_prefix),
                            &start_str,
                            &end_str,
                            is_exclusive
                        )
                    )
                }
            };
            // Emit intervening statements (between the `new Seq` and the counter init) BEFORE the
            // slice-init declaration — an intervening `Let vEnd be roff + vlen - 1` DEFINES a binding
            // the slice bounds (`body[(roff-1)..vEnd]`) read, so it must precede the copy, not follow
            // it (emitting it after left `vEnd` used-before-defined → E0425).
            for si in (idx + 1)..counter_idx {
                use super::codegen_stmt;
                output.push_str(&codegen_stmt(
                    stmts[si],
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
            if ctx.is_de_rc(dst_sym) {
                if ctx.is_scratch_hoist(dst_sym) {
                    // The buffer's allocation was hoisted before the enclosing
                    // `while` (see the `Stmt::While` handler). `vec_init` is a
                    // full copy `<slice>.to_vec()`; reuse the buffer in place:
                    // `clear()` keeps capacity, `extend_from_slice` refills it —
                    // value-identical to the fresh `.to_vec()`, zero allocation.
                    let slice = vec_init
                        .strip_suffix(".to_vec()")
                        .unwrap_or(vec_init.as_str());
                    // `&<slice>[..]` is a `&[T]` whatever `<slice>` is: a whole container (`src` /
                    // `src.borrow()`) or the run the loop copies.
                    let slice_ref = format!("&{}[..]", slice);
                    writeln!(output, "{}{}.clear();", indent_str, dst_name).unwrap();
                    writeln!(
                        output,
                        "{}{}.extend_from_slice({});",
                        indent_str, dst_name, slice_ref
                    )
                    .unwrap();
                    // Type was registered as `Vec<T>` at the hoist site.
                } else {
                    writeln!(
                        output,
                        "{}let mut {}: Vec<{}> = {};",
                        indent_str, dst_name, elem_type, vec_init
                    )
                    .unwrap();
                    ctx.register_variable_type(dst_sym, format!("Vec<{}>", elem_type));
                }
            } else {
                if is_start_one && is_end_length_of_src && needs_borrow {
                    // Full deep copy of a nested LogosSeq.
                    writeln!(
                        output,
                        "{}let mut {}: LogosSeq<{}> = {}.deep_clone();",
                        indent_str, dst_name, elem_type, src_name
                    )
                    .unwrap();
                } else {
                    writeln!(
                        output,
                        "{}let mut {}: LogosSeq<{}> = LogosSeq::from_vec({});",
                        indent_str, dst_name, elem_type, vec_init
                    )
                    .unwrap();
                }
                ctx.register_variable_type(dst_sym, format!("LogosSeq<{}>", elem_type));
            }

            // Re-emit the counter wherever its post-loop value can be read.
            if value_read_after(counter_sym, stmts, while_idx + 1) {
                let post_val = counter_after(
                    &codegen_expr_simple(start_expr, interner),
                    &codegen_expr_simple(end_expr, interner),
                    is_exclusive,
                );
                if counter_is_new_binding {
                    writeln!(
                        output,
                        "{}let mut {}: i64 = {};",
                        indent_str, counter_name, post_val
                    )
                    .unwrap();
                } else {
                    writeln!(output, "{}{} = {};", indent_str, counter_name, post_val).unwrap();
                }
            }

            let extra_consumed = while_idx - idx;
            Some((output, extra_consumed))
        }
        _ => None,
    }
}

/// A counted loop `While counter < limit` (or `<= limit`) whose last statement steps the counter by one: the
/// counter, the limit, whether the limit is excluded, and the statements of one pass before the step.
pub(crate) fn counted_loop<'e, 'a>(
    cond: &'e Expr<'a>,
    body: &'e [Stmt<'a>],
) -> Option<(Symbol, &'e Expr<'a>, bool, &'e [Stmt<'a>])> {
    let (counter, limit, exclusive) = match cond {
        Expr::BinaryOp {
            op: op @ (BinaryOpKind::LtEq | BinaryOpKind::Lt),
            left: Expr::Identifier(c),
            right,
        } => (*c, *right, matches!(op, BinaryOpKind::Lt)),
        _ => return None,
    };
    if body.len() < 2 {
        return None;
    }
    let steps = match &body[body.len() - 1] {
        Stmt::Set { target, value } if *target == counter => {
            matches!(value, Expr::BinaryOp { op: BinaryOpKind::Add, left, right }
            if (matches!(left, Expr::Identifier(s) if *s == counter) && matches!(right, Expr::Literal(Literal::Number(1))))
            || (matches!(left, Expr::Literal(Literal::Number(1))) && matches!(right, Expr::Identifier(s) if *s == counter)))
        }
        _ => false,
    };
    steps.then(|| (counter, limit, exclusive, &body[..body.len() - 1]))
}

/// A counted loop (see [`counted_loop`]) that pushes to the sequence `sym` on every pass — directly, or on
/// every path through its branches — so it grows `sym` by a number of elements its bounds fix. The length
/// analysis every tier sizes a push-built sequence by.
pub(crate) struct CountedPush<'e, 'a> {
    pub(crate) counter: Symbol,
    pub(crate) limit: &'e Expr<'a>,
    pub(crate) exclusive: bool,
    /// The direct, unconditional pushes to `sym` per pass (at least one push reaches it on every path).
    pub(crate) pushes: usize,
}

pub(crate) fn counted_push_loop<'e, 'a>(
    cond: &'e Expr<'a>,
    body: &'e [Stmt<'a>],
    sym: Symbol,
) -> Option<CountedPush<'e, 'a>> {
    let (counter, limit, exclusive, pass) = counted_loop(cond, body)?;
    let pushes_every_pass = pass.iter().any(|s| {
        matches!(s, Stmt::Push { collection, .. } if matches!(collection, Expr::Identifier(c) if *c == sym))
    }) || all_paths_push_to(pass, sym);
    pushes_every_pass.then(|| CountedPush {
        counter,
        limit,
        exclusive,
        pushes: pushes_per_iter(pass, sym),
    })
}

/// Peephole optimization: detect `new Seq` followed by a counted push loop and emit
/// `Vec::with_capacity(count)` or `vec![value; N]` instead of `Seq::default()`.
///
/// Relaxed pattern: allows arbitrary intervening statements between the Seq creation
/// and the counter-init + While pair, as long as they don't reference the Seq variable.
/// Scans forward until the pattern is found or a statement touching vec is encountered.
///
/// For Copy types with constant push values, emits `vec![value; N]` which enables
/// LLVM to know the length at the declaration site and elide bounds checks.
///
/// Returns (generated_code, number_of_extra_statements_consumed) or None.

/// How many times `sym` is pushed per loop iteration (direct, unconditional
/// pushes in the loop body). A Seq pushed `k` times per trip needs `k * N`
/// capacity, not `N` — under-sizing it forces a chain of reallocations the
/// pre-size is meant to avoid (graph_bfs's `adj` gets 5 pushes per vertex).
fn pushes_per_iter(body: &[Stmt], sym: Symbol) -> usize {
    // Kill-switch for the Phase-2 capacity scaling (attribution / A/B); returns 1
    // so `scale_capacity` is a no-op.
    if !crate::optimize::active_config().is_on(crate::optimization::Opt::CapScale) {
        return 1;
    }
    body.iter()
        .filter(|s| matches!(s, Stmt::Push { collection: Expr::Identifier(c), .. } if *c == sym))
        .count()
}

/// Scale a trip-count capacity string by the per-iteration push count. `<= 1`
/// leaves it unchanged (the common case); `k > 1` multiplies it.
fn scale_capacity(cap: &str, pushes: usize) -> String {
    if pushes <= 1 {
        cap.to_string()
    } else {
        crate::optimize::mark_fired(crate::optimization::Opt::CapScale);
        format!("({}) * {}", cap, pushes)
    }
}

/// Emit the runtime preconditions a narrowed (`Vec<i32>`) sequence depends on
/// (e.g. a `% m` divisor bound), once at its declaration. No-op when the value
/// range was proven statically.
fn emit_narrow_guards(out: &mut String, sym: Symbol, ctx: &RefinementContext, indent_str: &str) {
    let guards: Vec<String> = ctx.narrow_guards(sym).to_vec();
    for g in guards {
        writeln!(
            out,
            "{}assert!({}, \"LOGOS i32-narrowing guard: element value must fit i32\");",
            indent_str, g
        )
        .unwrap();
    }
}

/// The Rust expression sizing a dense map's direct-addressed array: its
/// oracle-proven key-domain capacity, rendered from the SAME `LinExpr` the key
/// proof used (so the array is exactly large enough for every proven key). `None`
/// only if the map has no recorded cap — the dense gate already declined any cap
/// that does not render, so a dense-typed map always resolves here.
fn dense_cap_rust(m: Symbol, ctx: &RefinementContext, interner: &Interner) -> Option<String> {
    let cap = ctx.oracle()?.map_cap_lin(m)?;
    crate::optimize::lin_to_rust(cap, interner)
}

pub(crate) fn try_emit_vec_with_capacity_pattern<'a>(
    stmts: &[&Stmt<'a>],
    idx: usize,
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
) -> Option<(String, usize)> {
    if idx + 2 >= stmts.len() {
        return None;
    }

    // Statement 1: Let [mutable] vec_var = new Seq of T (or Map of K to V)
    let (vec_sym, collection_info, vec_is_mutable) = match stmts[idx] {
        Stmt::Let {
            var,
            value,
            ty,
            mutable,
        } => {
            let type_from_annotation = if let Some(TypeExpr::Generic { base, params }) = ty {
                let base_name = interner.resolve(*base);
                if matches!(base_name, "Seq" | "List" | "Vec") && !params.is_empty() {
                    Some(CollInfo::Vec(codegen_type_expr(&params[0], interner)))
                } else if matches!(base_name, "Map" | "HashMap") && params.len() >= 2 {
                    Some(CollInfo::Map(
                        codegen_type_expr(&params[0], interner),
                        codegen_type_expr(&params[1], interner),
                    ))
                } else {
                    None
                }
            } else {
                None
            };

            let type_from_new = if let Expr::New {
                type_name,
                type_args,
                init_fields,
            } = value
            {
                let tn = interner.resolve(*type_name);
                if matches!(tn, "Seq" | "List" | "Vec")
                    && init_fields.is_empty()
                    && !type_args.is_empty()
                {
                    Some(CollInfo::Vec(codegen_type_expr(&type_args[0], interner)))
                } else if matches!(tn, "Map" | "HashMap")
                    && init_fields.is_empty()
                    && type_args.len() >= 2
                {
                    Some(CollInfo::Map(
                        codegen_type_expr(&type_args[0], interner),
                        codegen_type_expr(&type_args[1], interner),
                    ))
                } else {
                    None
                }
            } else {
                None
            };

            match type_from_annotation.or(type_from_new) {
                Some(info) => (*var, info, *mutable),
                None => return None,
            }
        }
        _ => return None,
    };

    // An affine read-only array is deleted (reads substitute the closed form), so
    // it must not be re-materialized as a pre-sized push-built Vec here.
    if ctx.affine_array(vec_sym).is_some() {
        return None;
    }

    // Scan forward from idx+1 for a counter-init + While pair.
    // Skip intervening statements that don't reference vec_sym.
    // Stop scanning when we hit a statement that touches vec_sym or find the pattern.
    let mut counter_init_idx: Option<usize> = None;

    for scan in (idx + 1)..stmts.len() {
        let stmt = stmts[scan];

        // Try: is this a counter init (Let/Set with simple expr)?
        let is_counter_init = match stmt {
            Stmt::Let { value, .. } if is_simple_expr(value) => true,
            Stmt::Set { value, .. } if is_simple_expr(value) => true,
            _ => false,
        };

        if is_counter_init && scan + 1 < stmts.len() {
            let counter_sym = match stmt {
                Stmt::Let { var, .. } => *var,
                Stmt::Set { target, .. } => *target,
                _ => unreachable!(),
            };

            if let Stmt::While { cond, body, .. } = stmts[scan + 1] {
                let filled = match &collection_info {
                    CollInfo::Vec(_) => counted_push_loop(cond, body, vec_sym),
                    CollInfo::Map(_, _) => counted_loop(cond, body).filter(|(_, _, _, pass)| {
                        pass.iter().any(|s| {
                            matches!(s, Stmt::SetIndex { collection, .. } if matches!(collection, Expr::Identifier(sym) if *sym == vec_sym))
                        }) || all_paths_set_index_to(pass, vec_sym)
                    })
                    .map(|(counter, limit, exclusive, _)| CountedPush { counter, limit, exclusive, pushes: 1 }),
                };
                if filled.is_some_and(|f| f.counter == counter_sym) {
                    counter_init_idx = Some(scan);
                    break;
                }
            }
        }

        // This statement doesn't start the pattern. If it references vec_sym, bail.
        if symbol_appears_in_stmts(vec_sym, &[stmt]) {
            return None;
        }
    }

    let counter_idx = counter_init_idx?;
    let while_idx = counter_idx + 1;

    let body = match stmts[while_idx] {
        Stmt::While { body, .. } => body,
        _ => return None,
    };

    // Register the map's concrete Rust type BEFORE codegen'ing the loop body
    // below: an in-loop `Set item k of m to v` resolves to the map's `.insert(...)`
    // only if `m`'s type is already registered when the SetIndex is emitted.
    // Otherwise it falls back to the generic `LogosIndexMut::logos_set`, which the
    // dense direct-addressed types do not implement (a compile error). The
    // declaration arm below re-registers the same type (idempotent).
    if let CollInfo::Map(kt, vt) = &collection_info {
        let rust_ty = map_rust_type(kt, vt, vec_sym, ctx);
        ctx.register_variable_type(vec_sym, rust_ty);
    }

    // Delegate counter-init + While to for-range pattern
    let remaining = &stmts[counter_idx..];
    let remaining_refs: Vec<&Stmt> = remaining.iter().copied().collect();
    let loop_result = try_emit_for_range_pattern(
        &remaining_refs,
        0,
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

    let (loop_code, _) = loop_result?;

    // Compute capacity from the for-range bounds
    let start_str = codegen_expr_simple(
        match stmts[counter_idx] {
            Stmt::Let { value, .. } | Stmt::Set { value, .. } => value,
            _ => return None,
        },
        interner,
    );

    let limit_expr = match stmts[while_idx] {
        Stmt::While { cond, .. } => match cond {
            Expr::BinaryOp { right, .. } => *right,
            _ => return None,
        },
        _ => return None,
    };
    let is_exclusive = match stmts[while_idx] {
        Stmt::While { cond, .. } => matches!(
            cond,
            Expr::BinaryOp {
                op: BinaryOpKind::Lt,
                ..
            }
        ),
        _ => false,
    };
    let limit_str = codegen_expr_simple(limit_expr, interner);

    // Use-before-def guard. The `with_capacity(...)` is hoisted to the vec's
    // declaration (`stmts[idx]`), but the loop bound / counter-init value may name
    // a local declared BETWEEN the vec and the loop — `Let out be a new Seq` …
    // `Let hi be ns - np` … `While i is at most hi` — which would emit
    // `with_capacity(hi)` before `hi` exists (invalid Rust; the interpreter never
    // sees the hint so only AOT breaks). Bail — falling back to the plain
    // constructor is always sound — whenever the capacity's operands reference a
    // binding introduced after the vec's own declaration.
    {
        let mut fresh_between: HashSet<Symbol> = HashSet::new();
        for s in &stmts[(idx + 1)..=while_idx] {
            if let Stmt::Let { var, .. } = s {
                fresh_between.insert(*var);
            }
        }
        let mut cap_syms = Vec::new();
        collect_expr_symbols(limit_expr, &mut cap_syms);
        if let Stmt::Let { value, .. } | Stmt::Set { value, .. } = stmts[counter_idx] {
            collect_expr_symbols(value, &mut cap_syms);
        }
        if cap_syms.iter().any(|s| fresh_between.contains(s)) {
            return None;
        }
    }

    let start_lit = match stmts[counter_idx] {
        Stmt::Let {
            value: Expr::Literal(Literal::Number(n)),
            ..
        }
        | Stmt::Set {
            value: Expr::Literal(Literal::Number(n)),
            ..
        } => Some(*n),
        _ => None,
    };
    let limit_lit = match limit_expr {
        Expr::Literal(Literal::Number(n)) => Some(*n),
        _ => None,
    };

    let capacity_expr = match (start_lit, limit_lit) {
        (Some(s), Some(l)) => {
            let count = if is_exclusive { l - s } else { l - s + 1 };
            format!("{}", std::cmp::max(0, count))
        }
        _ => {
            if is_exclusive {
                if start_str == "0" {
                    format!("{} as usize", limit_str)
                } else {
                    format!("({} - {}) as usize", limit_str, start_str)
                }
            } else {
                if start_str == "1" {
                    format!("{} as usize", limit_str)
                } else {
                    format!("({} - {} + 1) as usize", limit_str, start_str)
                }
            }
        }
    };

    let indent_str = "    ".repeat(indent);
    let vec_name = interner.resolve(vec_sym);

    // Detect vec![value; N] opportunity for Copy types with constant push values.
    let vec_fill_literal = if let CollInfo::Vec(ref elem_type) = collection_info {
        let is_copy = matches!(elem_type.as_str(), "i64" | "f64" | "bool");
        if is_copy {
            let body_without_increment = &body[..body.len() - 1];
            if body_without_increment.len() == 1 {
                match &body_without_increment[0] {
                    Stmt::Push { collection, value } => {
                        let is_target =
                            matches!(collection, Expr::Identifier(sym) if *sym == vec_sym);
                        if is_target {
                            match value {
                                Expr::Literal(
                                    lit @ (Literal::Number(_)
                                    | Literal::Float(_)
                                    | Literal::Boolean(_)),
                                ) => Some(super::expr::codegen_literal(lit, interner)),
                                _ => None,
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
        } else {
            None
        }
    } else {
        None
    };

    let mut output = String::new();
    let is_mutable = vec_is_mutable || mutable_vars.contains(&vec_sym);
    let mut_kw = if is_mutable { "mut " } else { "" };

    match &collection_info {
        CollInfo::Vec(elem_type) => {
            // O2 de-Rc: this Seq never needs reference semantics → plain Vec<T>
            // (no Rc/RefCell); otherwise the reference-semantics LogosSeq.
            let de_rc = ctx.is_de_rc(vec_sym);
            // i64→i32 narrowing: a de-Rc'd Seq proven to hold only i32-range
            // values is `Vec<i32>` (half the footprint). Access sites convert.
            let narrow = de_rc && ctx.is_narrowed(vec_sym);
            let elem = if narrow { "i32" } else { elem_type.as_str() };
            let bits = ctx.is_bitpacked(vec_sym);
            let wrap_ty = if bits {
                "logicaffeine_data::LogosBits".to_string()
            } else if de_rc {
                format!("Vec<{}>", elem)
            } else {
                format!("LogosSeq<{}>", elem_type)
            };
            // A Seq pushed `k>1` times per iteration needs `k * N` capacity. (The
            // `vec_fill_literal` fast-path only fires for a single push, so the
            // fill count there is always `N`.)
            let prim_cap = scale_capacity(&capacity_expr, pushes_per_iter(body, vec_sym));
            let rhs = match (&vec_fill_literal, de_rc) {
                (Some(fill), _) if bits => format!(
                    "logicaffeine_data::LogosBits::filled(({}) as usize, {})",
                    capacity_expr, fill
                ),
                (None, _) if bits => {
                    format!(
                        "logicaffeine_data::LogosBits::with_capacity(({}) as usize)",
                        prim_cap
                    )
                }
                (Some(fill), true) => format!("vec![{}; ({}) as usize]", fill, capacity_expr),
                (Some(fill), false) => {
                    format!("LogosSeq::from_vec(vec![{}; {}])", fill, capacity_expr)
                }
                (None, true) => format!("Vec::with_capacity(({}) as usize)", prim_cap),
                (None, false) => format!("LogosSeq::with_capacity({})", prim_cap),
            };
            writeln!(
                output,
                "{}let {}{}: {} = {};",
                indent_str, mut_kw, vec_name, wrap_ty, rhs
            )
            .unwrap();
            emit_narrow_guards(&mut output, vec_sym, ctx, &indent_str);
            let registered = if bits {
                "LogosBits".to_string()
            } else {
                wrap_ty
            };
            ctx.register_variable_type(vec_sym, registered);
        }
        CollInfo::Map(key_type, val_type) => {
            // Phase D: a non-aliased `Map of Int to Int` filled in this loop is
            // an open-addressing `LogosI64Map` (no Rc/RefCell), forced `mut`
            // because it is mutated in place via `&mut self`.
            let rust_ty = map_rust_type(key_type, val_type, vec_sym, ctx);
            if rust_ty.starts_with("LogosDense") {
                // Dense direct-addressed array, sized to the oracle's proven
                // key-domain cap + 1 (offset 0: `data[key]` for `0 <= key <= cap`).
                // The cap is the SAME `LinExpr` the key proof used, so the array
                // is exactly large enough — the dense gate already declined any map
                // whose cap does not render, so this resolves.
                let cap =
                    dense_cap_rust(vec_sym, ctx, interner).unwrap_or_else(|| capacity_expr.clone());
                writeln!(
                    output,
                    "{}let mut {}: {} = {}::with_bounds(0, (({}) + 1) as usize);",
                    indent_str, vec_name, rust_ty, rust_ty, cap
                )
                .unwrap();
            } else if rust_ty.starts_with("LogosI64") || rust_ty.starts_with("LogosI32") {
                writeln!(
                    output,
                    "{}let mut {}: {} = {}::with_capacity({});",
                    indent_str, vec_name, rust_ty, rust_ty, capacity_expr
                )
                .unwrap();
            } else {
                writeln!(
                    output,
                    "{}let {}{}: {} = LogosMap::with_capacity({});",
                    indent_str, mut_kw, vec_name, rust_ty, capacity_expr
                )
                .unwrap();
            }
            ctx.register_variable_type(vec_sym, rust_ty);
        }
    }

    // Emit intervening statements between Seq creation and counter init.
    // Check each intervening Let for sibling Seq/Map creations that are also
    // pushed to in the same While loop — give them with_capacity too.
    let intervening = &stmts[(idx + 1)..counter_idx];
    let body_without_increment = &body[..body.len() - 1];
    for stmt in intervening {
        let sibling_cap = detect_sibling_collection(stmt, body_without_increment, interner);
        if let Some((sib_sym, sib_info, sib_mutable)) = sibling_cap {
            // A deleted affine read-only array emits no declaration here — its
            // build push is suppressed and reads substitute the closed form. (Also
            // preserves its `__affine_array:` type tag from being overwritten.)
            if ctx.affine_array(sib_sym).is_some() {
                continue;
            }
            let sib_name = interner.resolve(sib_sym);
            let sib_mut = if sib_mutable || mutable_vars.contains(&sib_sym) {
                "mut "
            } else {
                ""
            };
            // Sibling collections always use with_capacity, never vec![val; N].
            // The loop still runs (for the primary collection), so vec![val; N]
            // would double the elements — the fill creates N items AND the loop pushes N more.
            match &sib_info {
                CollInfo::Vec(elem_type) => {
                    // O2 de-Rc: a sibling Seq with no reference-semantics need
                    // is a plain Vec<T> too.
                    let de_rc = ctx.is_de_rc(sib_sym);
                    let narrow = de_rc && ctx.is_narrowed(sib_sym);
                    let elem = if narrow { "i32" } else { elem_type.as_str() };
                    let sib_bits = ctx.is_bitpacked(sib_sym);
                    let wrap_ty = if sib_bits {
                        "logicaffeine_data::LogosBits".to_string()
                    } else if de_rc {
                        format!("Vec<{}>", elem)
                    } else {
                        format!("LogosSeq<{}>", elem_type)
                    };
                    let sib_cap = scale_capacity(
                        &capacity_expr,
                        pushes_per_iter(body_without_increment, sib_sym),
                    );
                    let rhs = if sib_bits {
                        format!(
                            "logicaffeine_data::LogosBits::with_capacity(({}) as usize)",
                            sib_cap
                        )
                    } else if de_rc {
                        format!("Vec::with_capacity(({}) as usize)", sib_cap)
                    } else {
                        format!("LogosSeq::with_capacity({})", sib_cap)
                    };
                    writeln!(
                        output,
                        "{}let {}{}: {} = {};",
                        indent_str, sib_mut, sib_name, wrap_ty, rhs
                    )
                    .unwrap();
                    emit_narrow_guards(&mut output, sib_sym, ctx, &indent_str);
                    let registered = if sib_bits {
                        "LogosBits".to_string()
                    } else {
                        wrap_ty
                    };
                    ctx.register_variable_type(sib_sym, registered);
                }
                CollInfo::Map(key_type, val_type) => {
                    let rust_ty = map_rust_type(key_type, val_type, sib_sym, ctx);
                    if rust_ty.starts_with("LogosDense") {
                        let cap = dense_cap_rust(sib_sym, ctx, interner)
                            .unwrap_or_else(|| capacity_expr.clone());
                        writeln!(
                            output,
                            "{}let mut {}: {} = {}::with_bounds(0, (({}) + 1) as usize);",
                            indent_str, sib_name, rust_ty, rust_ty, cap
                        )
                        .unwrap();
                    } else if rust_ty.starts_with("LogosI64") || rust_ty.starts_with("LogosI32") {
                        writeln!(
                            output,
                            "{}let mut {}: {} = {}::with_capacity({});",
                            indent_str, sib_name, rust_ty, rust_ty, capacity_expr
                        )
                        .unwrap();
                    } else {
                        writeln!(
                            output,
                            "{}let {}{}: {} = LogosMap::with_capacity({});",
                            indent_str, sib_mut, sib_name, rust_ty, capacity_expr
                        )
                        .unwrap();
                    }
                    ctx.register_variable_type(sib_sym, rust_ty);
                }
            }
        } else {
            use super::codegen_stmt;
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
    }

    // For vec![value; N], the fill is done by the declaration — skip the loop.
    // Only emit the post-loop counter binding if needed.
    if vec_fill_literal.is_some() {
        // Extract the post-loop counter assignment from the for-range code.
        // The for-range code looks like: `for counter in start..limit {\n  ...\n}\nlet mut counter = ...;\n`
        // We only need the trailing `let mut counter = ...;` part.
        if let Some(closing_pos) = loop_code.rfind("\n    }") {
            let after_loop = &loop_code[closing_pos + 6..]; // skip "}\n"
            let trimmed = after_loop.trim_start_matches('\n');
            if !trimmed.trim().is_empty() {
                output.push_str(trimmed);
            }
        }
    } else {
        output.push_str(&loop_code);
    }

    // Consumed: all statements from idx+1 through while_idx (inclusive)
    let extra_consumed = while_idx - idx;
    Some((output, extra_consumed))
}

/// Detect if an intervening statement is a `Let` creating a new Seq/Map that is
/// pushed to in the given loop body. Returns the symbol and collection info if so.
fn detect_sibling_collection<'a>(
    stmt: &Stmt<'a>,
    body_without_increment: &[Stmt<'a>],
    interner: &Interner,
) -> Option<(Symbol, CollInfo, bool)> {
    let (var, value, ty, is_mutable) = match stmt {
        Stmt::Let {
            var,
            value,
            ty,
            mutable,
        } => (*var, *value, ty.as_ref(), *mutable),
        _ => return None,
    };

    // Extract collection type info from annotation or `new` expression
    let type_from_annotation = if let Some(TypeExpr::Generic { base, params }) = ty {
        let base_name = interner.resolve(*base);
        if matches!(base_name, "Seq" | "List" | "Vec") && !params.is_empty() {
            Some(CollInfo::Vec(codegen_type_expr(&params[0], interner)))
        } else if matches!(base_name, "Map" | "HashMap") && params.len() >= 2 {
            Some(CollInfo::Map(
                codegen_type_expr(&params[0], interner),
                codegen_type_expr(&params[1], interner),
            ))
        } else {
            None
        }
    } else {
        None
    };

    let type_from_new = if let Expr::New {
        type_name,
        type_args,
        init_fields,
    } = value
    {
        let tn = interner.resolve(*type_name);
        if matches!(tn, "Seq" | "List" | "Vec") && init_fields.is_empty() && !type_args.is_empty() {
            Some(CollInfo::Vec(codegen_type_expr(&type_args[0], interner)))
        } else if matches!(tn, "Map" | "HashMap") && init_fields.is_empty() && type_args.len() >= 2
        {
            Some(CollInfo::Map(
                codegen_type_expr(&type_args[0], interner),
                codegen_type_expr(&type_args[1], interner),
            ))
        } else {
            None
        }
    } else {
        None
    };

    let info = type_from_annotation.or(type_from_new)?;

    // Check if this variable is pushed to in the loop body
    let has_push = match &info {
        CollInfo::Vec(_) => {
            body_without_increment.iter().any(|s| {
                matches!(s, Stmt::Push { collection, .. } if matches!(collection, Expr::Identifier(sym) if *sym == var))
            }) || all_paths_push_to(body_without_increment, var)
        }
        CollInfo::Map(_, _) => {
            body_without_increment.iter().any(|s| {
                matches!(s, Stmt::SetIndex { collection, .. } if matches!(collection, Expr::Identifier(sym) if *sym == var))
            }) || all_paths_set_index_to(body_without_increment, var)
        }
    };

    if has_push {
        Some((var, info, is_mutable))
    } else {
        None
    }
}

/// Peephole optimization for merge-style Vec construction.
///
/// Detects a `Let mutable X = new Seq of T` followed by one or more While loops
/// that all push to X from known source Vecs, and emits
/// `Vec::with_capacity((src1.len() + src2.len()) as usize)` instead of `Vec::default()`.
///
/// This pattern is distinct from `try_emit_vec_with_capacity_pattern` which computes
/// capacity from loop iteration count. This pattern computes capacity from the total
/// length of all source collections being merged.
///
/// Pattern:
///   Let mutable result = new Seq of T.
///   ... (counter inits, etc.)
///   While ...: Push item X of SOURCE to result. ...
///   While ...: Push item Y of SOURCE2 to result. ...
///   Return result.
///
/// → `let mut result: Vec<T> = Vec::with_capacity((src1.len() + src2.len()) as usize);`
///
/// Only fires when:
/// - Every While loop between the decl and the first non-While/non-counter-init reference
///   to the target has ALL execution paths pushing to the target
/// - All pushed values come from indexed reads on known source Vecs
/// - The target Vec is not used in any other way between declaration and the While loops
pub(crate) fn try_emit_merge_capacity_pattern<'a>(
    stmts: &[&Stmt<'a>],
    idx: usize,
    interner: &Interner,
    indent: usize,
    ctx: &mut RefinementContext<'a>,
) -> Option<(String, usize)> {
    if idx + 1 >= stmts.len() {
        return None;
    }

    // Statement at idx: Let mutable vec_var = new Seq of T
    let (vec_sym, elem_type) = match stmts[idx] {
        Stmt::Let {
            var,
            value,
            ty,
            mutable: true,
            ..
        } => {
            let type_from_annotation = if let Some(TypeExpr::Generic { base, params }) = ty {
                let base_name = interner.resolve(*base);
                if matches!(base_name, "Seq" | "List" | "Vec") && !params.is_empty() {
                    Some(codegen_type_expr(&params[0], interner))
                } else {
                    None
                }
            } else {
                None
            };

            let type_from_new = if let Expr::New {
                type_name,
                type_args,
                init_fields,
            } = value
            {
                let tn = interner.resolve(*type_name);
                if matches!(tn, "Seq" | "List" | "Vec")
                    && init_fields.is_empty()
                    && !type_args.is_empty()
                {
                    Some(codegen_type_expr(&type_args[0], interner))
                } else {
                    None
                }
            } else {
                None
            };

            match type_from_annotation.or(type_from_new) {
                Some(elem) => (*var, elem),
                None => return None,
            }
        }
        _ => return None,
    };

    // Scan forward: collect While loops that push to vec_sym.
    // Allow intervening Let/Set statements that don't reference vec_sym (counter inits).
    // Stop at Return or any non-Let/Set/While that references vec_sym.
    let mut source_syms: HashSet<Symbol> = HashSet::new();
    let mut last_while_idx = idx;
    let mut found_any_while = false;

    for scan in (idx + 1)..stmts.len() {
        let stmt = stmts[scan];

        match stmt {
            Stmt::While { body, .. } => {
                // Check that all paths push to vec_sym
                if all_paths_push_to(body, vec_sym) {
                    // Collect push sources from this While body
                    if let Some(sources) = collect_push_sources(body, vec_sym) {
                        source_syms.extend(sources);
                        last_while_idx = scan;
                        found_any_while = true;
                    } else {
                        // While pushes to target but sources are not simple indexed reads
                        break;
                    }
                } else {
                    // This While doesn't push to our target on all paths — stop scanning
                    break;
                }
            }
            Stmt::Let { .. } | Stmt::Set { .. } => {
                // Allow intervening counter init / other Let/Set if they don't reference vec_sym
                if symbol_appears_in_stmts(vec_sym, &[stmt]) {
                    break;
                }
            }
            Stmt::Return { .. } => {
                // Return ends the block — stop scanning
                break;
            }
            _ => {
                // Any other statement type — stop scanning
                break;
            }
        }
    }

    if !found_any_while || source_syms.is_empty() {
        return None;
    }

    // Use-before-def guard (mirrors `try_emit_vec_with_capacity_pattern`): the
    // `with_capacity(sum of source.len())` is hoisted to the target's declaration
    // (`stmts[idx]`), so every merge source must be a collection in scope THERE —
    // declared before the target, not bound inside the scanned loop region. A
    // source bound in that region is a per-iteration local: `Push drow to dst`
    // where `drow` is a fresh `Let mutable drow be a new Seq` in the loop body
    // makes `drow` a "source", and `with_capacity(drow.len())` references it
    // before it exists (invalid Rust; only AOT breaks — the interpreter never
    // sees the hint). Bail — the plain constructor is always sound.
    {
        let mut bound_in_region: HashSet<Symbol> = HashSet::new();
        for s in &stmts[(idx + 1)..=last_while_idx] {
            collect_bound_symbols_one(s, &mut bound_in_region);
        }
        if source_syms.iter().any(|s| bound_in_region.contains(s)) {
            return None;
        }
    }

    // Build capacity expression: sum of all source .len() values
    let names = RustNames::new(interner);
    let vec_name = names.ident(vec_sym);
    let indent_str = "    ".repeat(indent);

    let mut capacity_parts: Vec<String> = source_syms
        .iter()
        .map(|sym| format!("{}.len()", names.ident(*sym)))
        .collect();
    // `source_syms` is a `HashSet`, whose iteration order changes between compiles;
    // sort the (commutative) capacity terms so the emitted sum is reproducible.
    capacity_parts.sort();
    let capacity_expr = capacity_parts.join(" + ");

    let mut output = String::new();
    // De-Rc-aware: a de-Rc'd merge target is a plain owned `Vec<T>`.
    if ctx.is_de_rc(vec_sym) {
        writeln!(
            output,
            "{}let mut {}: Vec<{}> = Vec::with_capacity(({}) as usize);",
            indent_str, vec_name, elem_type, capacity_expr
        )
        .unwrap();
        ctx.register_variable_type(vec_sym, format!("Vec<{}>", elem_type));
    } else {
        writeln!(
            output,
            "{}let mut {}: LogosSeq<{}> = LogosSeq::with_capacity(({}) as usize);",
            indent_str, vec_name, elem_type, capacity_expr
        )
        .unwrap();
        ctx.register_variable_type(vec_sym, format!("LogosSeq<{}>", elem_type));
    }

    // Emit intervening statements between the declaration and the first While,
    // then the While loops themselves (all handled by regular codegen).
    // We only consumed statement idx (the Let). The rest are emitted by the caller.
    // Return 0 extra consumed since we only replaced the Let declaration.
    Some((output, 0))
}

/// Every symbol INTRODUCED (bound) by `stmt` and its nested blocks: `Let`
/// bindings and loop-pattern (`Repeat for x`/`for (k, v)`) variables. Used by
/// the merge-capacity guard to reject a source that is a per-iteration local
/// rather than a stable collection in scope at the target's hoisted declaration.
fn collect_bound_symbols_one(stmt: &Stmt, out: &mut HashSet<Symbol>) {
    match stmt {
        Stmt::Let { var, .. } => {
            out.insert(*var);
        }
        Stmt::Repeat { pattern, body, .. } => {
            match pattern {
                Pattern::Identifier(s) => {
                    out.insert(*s);
                }
                Pattern::Tuple(syms) => {
                    for s in syms {
                        out.insert(*s);
                    }
                }
            }
            for s in *body {
                collect_bound_symbols_one(s, out);
            }
        }
        Stmt::While { body, .. } => {
            for s in *body {
                collect_bound_symbols_one(s, out);
            }
        }
        Stmt::If {
            then_block,
            else_block,
            ..
        } => {
            for s in *then_block {
                collect_bound_symbols_one(s, out);
            }
            if let Some(eb) = else_block {
                for s in *eb {
                    collect_bound_symbols_one(s, out);
                }
            }
        }
        _ => {}
    }
}

/// Collect all source collection symbols from Push statements targeting `coll_sym` in a statement block.
/// Returns None if any push to the target doesn't read from a simple indexed collection.
/// Recurses into If/Else branches.
fn collect_push_sources(stmts: &[Stmt], coll_sym: Symbol) -> Option<HashSet<Symbol>> {
    let mut sources = HashSet::new();
    for stmt in stmts {
        match stmt {
            Stmt::Push { value, collection } => {
                if matches!(collection, Expr::Identifier(sym) if *sym == coll_sym) {
                    collect_index_sources_from_expr(value, &mut sources);
                }
            }
            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
                if let Some(then_sources) = collect_push_sources(then_block, coll_sym) {
                    sources.extend(then_sources);
                }
                if let Some(else_stmts) = else_block {
                    if let Some(else_sources) = collect_push_sources(else_stmts, coll_sym) {
                        sources.extend(else_sources);
                    }
                }
            }
            Stmt::While { body, .. } => {
                if let Some(body_sources) = collect_push_sources(body, coll_sym) {
                    sources.extend(body_sources);
                }
            }
            _ => {}
        }
    }
    if sources.is_empty() {
        None
    } else {
        Some(sources)
    }
}

/// Extract source collection identifiers from Index expressions.
/// `Push item i of left to result` → extracts `left`.
fn collect_index_sources_from_expr(expr: &Expr, sources: &mut HashSet<Symbol>) {
    match expr {
        Expr::Index { collection, .. } => {
            if let Expr::Identifier(sym) = collection {
                sources.insert(*sym);
            }
        }
        Expr::Identifier(sym) => {
            sources.insert(*sym);
        }
        _ => {}
    }
}

/// Pre-scan a statement block to identify variables that can use `char` instead of `String`.
///
/// Peephole optimization: detect left-rotation via shift loop and emit `.rotate_left(1)`.
///
/// Pattern (4 statements):
///   Let tmp be item 1 of arr.
///   Set counter to 1.
///   While counter <= limit:
///       Set item counter of arr to item (counter + 1) of arr.
///       Set counter to counter + 1.
///   Set item (limit + 1) of arr to tmp.
/// → `arr[0..=(limit as usize)].rotate_left(1);`
///
/// `arr` must be registered as a `Vec<T>` in variable_types (requires mutable slice).
///
/// Returns (generated_code, number_of_extra_statements_consumed) or None.
pub(crate) fn try_emit_rotate_left_pattern<'a>(
    stmts: &[&Stmt<'a>],
    idx: usize,
    interner: &Interner,
    indent: usize,
    variable_types: &HashMap<Symbol, String>,
) -> Option<(String, usize)> {
    if idx + 3 >= stmts.len() {
        return None;
    }

    // Statement 1: Let tmp be item 1 of arr.  (saves first element)
    let (tmp_sym, arr_sym) = match stmts[idx] {
        Stmt::Let {
            var,
            mutable: false,
            value,
            ..
        } => {
            if let Expr::Index { collection, index } = value {
                if let Expr::Identifier(a) = collection {
                    if matches!(index, Expr::Literal(Literal::Number(1))) {
                        (*var, *a)
                    } else {
                        return None;
                    }
                } else {
                    return None;
                }
            } else {
                return None;
            }
        }
        _ => return None,
    };

    // arr must be a Vec type (rotate_left requires &mut [T])
    match variable_types.get(&arr_sym) {
        Some(t) if t.starts_with("LogosSeq") || t.starts_with("Vec") => {}
        _ => return None,
    }

    // Statement 2: Set counter to 1.
    let counter_sym = match stmts[idx + 1] {
        Stmt::Set {
            target,
            value: Expr::Literal(Literal::Number(1)),
        } => *target,
        _ => return None,
    };

    // Statement 3: While counter <= limit: SetIndex arr[counter] = arr[counter+1]; Set counter++
    let limit_expr = match stmts[idx + 2] {
        Stmt::While { cond, body, .. } => {
            // Condition: counter <= limit
            let limit = match cond {
                Expr::BinaryOp {
                    op: BinaryOpKind::LtEq,
                    left,
                    right,
                } => {
                    if let Expr::Identifier(c) = left {
                        if *c == counter_sym {
                            Some(*right)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
                _ => None,
            }?;

            if body.len() != 2 {
                return None;
            }

            // Body[0]: Set item counter of arr to item (counter + 1) of arr.
            match &body[0] {
                Stmt::SetIndex {
                    collection,
                    index: idx_expr,
                    value,
                } => {
                    if !matches!(collection, Expr::Identifier(s) if *s == arr_sym) {
                        return None;
                    }
                    if !matches!(idx_expr, Expr::Identifier(s) if *s == counter_sym) {
                        return None;
                    }
                    if let Expr::Index {
                        collection: v_coll,
                        index: v_idx,
                    } = value
                    {
                        if !matches!(v_coll, Expr::Identifier(s) if *s == arr_sym) {
                            return None;
                        }
                        match v_idx {
                            Expr::BinaryOp {
                                op: BinaryOpKind::Add,
                                left,
                                right,
                            } => {
                                let ok = (matches!(left, Expr::Identifier(s) if *s == counter_sym)
                                    && matches!(right, Expr::Literal(Literal::Number(1))))
                                    || (matches!(left, Expr::Literal(Literal::Number(1)))
                                        && matches!(right, Expr::Identifier(s) if *s == counter_sym));
                                if !ok {
                                    return None;
                                }
                            }
                            _ => return None,
                        }
                    } else {
                        return None;
                    }
                }
                _ => return None,
            }

            // Body[1]: Set counter to counter + 1.
            match &body[1] {
                Stmt::Set { target, value } => {
                    if *target != counter_sym {
                        return None;
                    }
                    match value {
                        Expr::BinaryOp {
                            op: BinaryOpKind::Add,
                            left,
                            right,
                        } => {
                            let ok = (matches!(left, Expr::Identifier(s) if *s == counter_sym)
                                && matches!(right, Expr::Literal(Literal::Number(1))))
                                || (matches!(left, Expr::Literal(Literal::Number(1)))
                                    && matches!(right, Expr::Identifier(s) if *s == counter_sym));
                            if !ok {
                                return None;
                            }
                        }
                        _ => return None,
                    }
                }
                _ => return None,
            }

            limit
        }
        _ => return None,
    };

    if !is_simple_expr(limit_expr) {
        return None;
    }

    // Statement 4: Set item (limit + 1) of arr to tmp.  (wrap-around)
    match stmts[idx + 3] {
        Stmt::SetIndex {
            collection,
            index,
            value,
        } => {
            if !matches!(collection, Expr::Identifier(s) if *s == arr_sym) {
                return None;
            }
            if !matches!(value, Expr::Identifier(s) if *s == tmp_sym) {
                return None;
            }
            let syntactic_ok = match index {
                Expr::BinaryOp {
                    op: BinaryOpKind::Add,
                    left,
                    right,
                } => {
                    (exprs_equal(left, limit_expr)
                        && matches!(right, Expr::Literal(Literal::Number(1))))
                        || (matches!(left, Expr::Literal(Literal::Number(1)))
                            && exprs_equal(right, limit_expr))
                }
                _ => false,
            };
            // The e-graph simplifies straight-line runs but not the loop
            // condition, so it can fold the wrap index `item (limit + 1)` to a
            // constant while the loop bound stays symbolic. Accept the wrap by
            // VALUE when both sides resolve to constants — `limit` being a
            // single-assignment literal in scope.
            if !syntactic_ok {
                let by_value = matches!(
                    (resolve_const_i64(index, stmts, idx), resolve_const_i64(limit_expr, stmts, idx)),
                    (Some(iv), Some(lv)) if lv.checked_add(1) == Some(iv)
                );
                if !by_value {
                    return None;
                }
            }
        }
        _ => return None,
    }

    // Pattern matched! Emit rotate_left.
    let indent_str = "    ".repeat(indent);
    let arr_name = interner.resolve(arr_sym);
    let tmp_name = interner.resolve(tmp_sym);
    let limit_str = codegen_expr_simple(limit_expr, interner);

    let is_logos_seq = variable_types
        .get(&arr_sym)
        .map_or(false, |t| t.starts_with("LogosSeq"));

    let mut output = String::new();

    // Only emit the tmp binding if it's used after the rotation.
    let remaining = &stmts[idx + 4..];
    if is_logos_seq {
        if symbol_appears_in_stmts(tmp_sym, remaining) {
            writeln!(
                output,
                "{}let {} = {}.borrow()[0].clone();",
                indent_str, tmp_name, arr_name
            )
            .unwrap();
        }
        writeln!(
            output,
            "{}{}.borrow_mut()[0..=({} as usize)].rotate_left(1);",
            indent_str, arr_name, limit_str
        )
        .unwrap();
    } else {
        if symbol_appears_in_stmts(tmp_sym, remaining) {
            writeln!(output, "{}let {} = {}[0];", indent_str, tmp_name, arr_name).unwrap();
        }
        writeln!(
            output,
            "{}{}[0..=({} as usize)].rotate_left(1);",
            indent_str, arr_name, limit_str
        )
        .unwrap();
    }

    Some((output, 3)) // consumed: Set counter + While + SetIndex = 3 extra
}

/// Pattern B: Unconditional 3-statement swap with arbitrary indices.
///   Let tmp be item I of arr.
///   Set item I of arr to item J of arr.
///   Set item J of arr to tmp.
/// → item I and item J read in that order, then each stored in the other's place.
#[allow(clippy::too_many_arguments)]
fn try_emit_unconditional_swap<'a>(
    stmts: &[&Stmt<'a>],
    idx: usize,
    tmp_sym: Symbol,
    arr_sym: Symbol,
    idx_expr_1: &'a Expr<'a>,
    coll_expr_1: &'a Expr<'a>,
    interner: &Interner,
    indent: usize,
    variable_types: &HashMap<Symbol, String>,
    oracle: Option<&crate::optimize::OracleFacts>,
) -> Option<(String, usize)> {
    if idx + 2 >= stmts.len() {
        return None;
    }

    // Statement 2: Set item I of arr to item J of arr.
    let (idx_expr_2, coll_expr_2) = match stmts[idx + 1] {
        Stmt::SetIndex {
            collection,
            index,
            value,
        } => {
            // collection must be the same array
            if !matches!(collection, Expr::Identifier(s) if *s == arr_sym) {
                return None;
            }
            // index must match idx_expr_1
            if !exprs_equal(index, idx_expr_1) {
                return None;
            }
            // value must be an Index into the same array at a different index
            if let Expr::Index {
                collection: v_coll,
                index: v_idx,
            } = value
            {
                if !matches!(v_coll, Expr::Identifier(s) if *s == arr_sym) {
                    return None;
                }
                (*v_idx, *v_coll)
            } else {
                return None;
            }
        }
        _ => return None,
    };

    // Statement 3: Set item J of arr to tmp.
    match stmts[idx + 2] {
        Stmt::SetIndex {
            collection,
            index,
            value,
        } => {
            // collection must be the same array
            if !matches!(collection, Expr::Identifier(s) if *s == arr_sym) {
                return None;
            }
            // index must match idx_expr_2
            if !exprs_equal(index, idx_expr_2) {
                return None;
            }
            // value must be tmp
            if !matches!(value, Expr::Identifier(s) if *s == tmp_sym) {
                return None;
            }
        }
        _ => return None,
    }

    // Both index expressions must be simple enough for codegen_expr_simple
    if !is_simple_expr(idx_expr_1) || !is_simple_expr(idx_expr_2) {
        return None;
    }

    // Defer to the proven-unchecked indexed form rather than bounds-checked `.swap()`.
    if swap_unchecked_proven(oracle, idx_expr_1, idx_expr_2) {
        return None;
    }

    // Pattern matched! The program reads item I, reads item J, stores the latter at I and the former at
    // J; the emission does exactly that, each access through the one index rule.
    let indent_str = "    ".repeat(indent);
    let arr_name = interner.resolve(arr_sym);
    let is_logos_seq = variable_types
        .get(&arr_sym)
        .map_or(false, |t| t.starts_with("LogosSeq"));
    let (read, write) =
        swap_accessors(arr_name, is_logos_seq, false, interner, variable_types, oracle);
    let (first, second) = swap_temps(false);
    // Both items are read before either is stored and nothing between changes the length, so a read the
    // program proves in range proves the store at the same index.
    let proven_1 = super::expr::range_proves_index(coll_expr_1, idx_expr_1);
    let proven_2 = super::expr::range_proves_index(coll_expr_2, idx_expr_2);

    let mut output = String::new();
    writeln!(
        output,
        "{}let {} = {};",
        indent_str,
        first,
        read(idx_expr_1, proven_1)
    )
    .unwrap();
    writeln!(
        output,
        "{}let {} = {};",
        indent_str,
        second,
        read(idx_expr_2, proven_2)
    )
    .unwrap();
    // The peephole consumes `Let tmp be item I of arr`; a program that reads `tmp` afterwards keeps it.
    if symbol_appears_in_stmts(tmp_sym, &stmts[idx + 3..]) {
        writeln!(
            output,
            "{}let {} = {}.clone();",
            indent_str,
            interner.resolve(tmp_sym),
            first
        )
        .unwrap();
    }
    writeln!(
        output,
        "{}{}",
        indent_str,
        write(idx_expr_1, proven_1, second)
    )
    .unwrap();
    writeln!(
        output,
        "{}{}",
        indent_str,
        write(idx_expr_2, proven_2, first)
    )
    .unwrap();

    Some((output, 2)) // consumed 2 extra statements (SetIndex + SetIndex)
}

/// Peephole optimization: detect a contiguous push-copy loop from one array to another
/// and emit `dst.extend_from_slice(...)` instead of individual pushes.
///
/// Matches two forms:
/// 1. **Counter init + While**: `Let/Set counter = start. While counter <= end: Push item counter of src to dst. Set counter to counter + 1.`
/// 2. **Bare While**: `While counter <= end: Push item counter of src to dst. Set counter to counter + 1.`
///    (counter already exists from a previous pattern's post-loop value emission)
///
/// Unlike `try_emit_seq_from_slice_pattern`, this does NOT require a preceding `Let mutable dst = new Seq`
/// anchor. This handles the second-half copy in divide-and-conquer splits where the first half
/// was already consumed by `try_emit_seq_from_slice_pattern`.
///
/// Must be positioned before `try_emit_for_range_pattern` in the peephole chain since both
/// match `Let counter + While` — this pattern is more specific (body must be push + increment).
///
/// Returns (generated_code, number_of_extra_statements_consumed) or None.
pub(crate) fn try_emit_bare_slice_push_pattern<'a>(
    stmts: &[&Stmt<'a>],
    idx: usize,
    interner: &Interner,
    indent: usize,
    variable_types: &HashMap<Symbol, String>,
) -> Option<(String, usize)> {
    // Try form 1: counter init + While (2-statement pattern)
    if let Some(result) =
        try_bare_slice_push_with_init(stmts, idx, interner, indent, variable_types)
    {
        return Some(result);
    }
    // Try form 2: bare While (1-statement pattern, counter already exists)
    try_bare_slice_push_bare_while(stmts, idx, interner, indent, variable_types)
}

fn try_bare_slice_push_with_init<'a>(
    stmts: &[&Stmt<'a>],
    idx: usize,
    interner: &Interner,
    indent: usize,
    variable_types: &HashMap<Symbol, String>,
) -> Option<(String, usize)> {
    if idx + 1 >= stmts.len() {
        return None;
    }

    // Statement 1: Let counter = start OR Set counter to start
    let (counter_sym, start_expr, is_new_binding) = match stmts[idx] {
        Stmt::Let { var, value, .. } => {
            if is_simple_expr(value) {
                (*var, *value, true)
            } else {
                return None;
            }
        }
        Stmt::Set { target, value } => {
            if is_simple_expr(value) {
                (*target, *value, false)
            } else {
                return None;
            }
        }
        _ => return None,
    };

    // Statement 2: While counter <= end OR While counter < end
    let while_info = extract_push_copy_while(stmts[idx + 1], counter_sym)?;

    // Validate types
    validate_slice_push_types(while_info.src_sym, while_info.dst_sym, variable_types)?;
    bounds_hold_still(
        &[start_expr, while_info.end_expr],
        while_info.dst_sym,
        counter_sym,
    )?;

    // Emit the optimized code
    let remaining = &stmts[idx + 2..];
    let src_is_logos_seq = variable_types
        .get(&while_info.src_sym)
        .map(|t| t.split("|__hl:").next().unwrap_or(t.as_str()))
        .map(|t| t.starts_with("LogosSeq"))
        .unwrap_or(true);
    let output = emit_extend_from_slice(
        interner,
        indent,
        while_info.dst_sym,
        while_info.src_sym,
        counter_sym,
        start_expr,
        while_info.end_expr,
        while_info.is_exclusive,
        remaining,
        Some(is_new_binding),
        src_is_logos_seq,
    );

    Some((output, 1)) // consumed 1 extra statement (the While)
}

fn try_bare_slice_push_bare_while<'a>(
    stmts: &[&Stmt<'a>],
    idx: usize,
    interner: &Interner,
    indent: usize,
    variable_types: &HashMap<Symbol, String>,
) -> Option<(String, usize)> {
    // Statement at idx must be a While loop
    let while_stmt = stmts[idx];
    let (counter_sym, end_expr, is_exclusive, src_sym, dst_sym) = match while_stmt {
        Stmt::While { cond, body, .. } => {
            let (counter_sym, end_expr, is_exclusive) = extract_while_cond(cond)?;
            if !is_simple_expr(end_expr) {
                return None;
            }
            if body.len() != 2 {
                return None;
            }
            let (src, dst) = extract_push_index_body(body, counter_sym)?;
            (counter_sym, end_expr, is_exclusive, src, dst)
        }
        _ => return None,
    };

    // Validate types
    validate_slice_push_types(src_sym, dst_sym, variable_types)?;
    bounds_hold_still(&[end_expr], dst_sym, counter_sym)?;

    // For bare While, the counter already has a value from a previous statement.
    // We use the counter identifier as the start expression.
    let start_expr = Expr::Identifier(counter_sym);

    let remaining = &stmts[idx + 1..];
    let src_is_logos_seq = variable_types
        .get(&src_sym)
        .map(|t| t.split("|__hl:").next().unwrap_or(t.as_str()))
        .map(|t| t.starts_with("LogosSeq"))
        .unwrap_or(true);
    let output = emit_extend_from_slice(
        interner,
        indent,
        dst_sym,
        src_sym,
        counter_sym,
        &start_expr,
        end_expr,
        is_exclusive,
        remaining,
        None,
        src_is_logos_seq,
    );

    Some((output, 0)) // no extra statements consumed
}

struct WhileInfo<'a> {
    end_expr: &'a Expr<'a>,
    is_exclusive: bool,
    src_sym: Symbol,
    dst_sym: Symbol,
}

fn extract_push_copy_while<'a>(stmt: &'a Stmt<'a>, counter_sym: Symbol) -> Option<WhileInfo<'a>> {
    let (cond, body) = match stmt {
        Stmt::While { cond, body, .. } => (cond, body),
        _ => return None,
    };

    let (cond_counter, end_expr, is_exclusive) = extract_while_cond(cond)?;
    if cond_counter != counter_sym {
        return None;
    }
    if !is_simple_expr(end_expr) {
        return None;
    }
    if body.len() != 2 {
        return None;
    }

    let (src_sym, dst_sym) = extract_push_index_body(body, counter_sym)?;
    validate_increment(&body[1], counter_sym)?;

    Some(WhileInfo {
        end_expr,
        is_exclusive,
        src_sym,
        dst_sym,
    })
}

fn extract_while_cond<'a>(cond: &'a Expr<'a>) -> Option<(Symbol, &'a Expr<'a>, bool)> {
    match cond {
        Expr::BinaryOp {
            op: BinaryOpKind::LtEq,
            left,
            right,
        } => {
            if let Expr::Identifier(sym) = left {
                Some((*sym, *right, false))
            } else {
                None
            }
        }
        Expr::BinaryOp {
            op: BinaryOpKind::Lt,
            left,
            right,
        } => {
            if let Expr::Identifier(sym) = left {
                Some((*sym, *right, true))
            } else {
                None
            }
        }
        _ => None,
    }
}

fn extract_push_index_body<'a>(body: &[Stmt<'a>], counter_sym: Symbol) -> Option<(Symbol, Symbol)> {
    // body[0]: Push item <counter> of <source> to <target>
    let (src, dst) = match &body[0] {
        Stmt::Push { value, collection } => {
            let dst = if let Expr::Identifier(s) = collection {
                *s
            } else {
                return None;
            };
            if let Expr::Index {
                collection: src_coll,
                index,
            } = value
            {
                if !matches!(index, Expr::Identifier(s) if *s == counter_sym) {
                    return None;
                }
                if let Expr::Identifier(s) = src_coll {
                    (*s, dst)
                } else {
                    return None;
                }
            } else {
                return None;
            }
        }
        _ => return None,
    };

    // body[1]: Set counter to counter + 1
    validate_increment(&body[1], counter_sym)?;

    Some((src, dst))
}

fn validate_increment(stmt: &Stmt, counter_sym: Symbol) -> Option<()> {
    match stmt {
        Stmt::Set { target, value } if *target == counter_sym => match value {
            Expr::BinaryOp {
                op: BinaryOpKind::Add,
                left,
                right,
            } => {
                let ok = matches!((left, right),
                    (Expr::Identifier(s), Expr::Literal(Literal::Number(1))) if *s == counter_sym
                ) || matches!((left, right),
                    (Expr::Literal(Literal::Number(1)), Expr::Identifier(s)) if *s == counter_sym
                );
                if ok {
                    Some(())
                } else {
                    None
                }
            }
            _ => None,
        },
        _ => None,
    }
}

/// A counted copy loop reads its bounds every iteration while it pushes into `dst` and steps `counter`; one
/// copy reads them once, which is the same only when no bound reads either.
fn bounds_hold_still(bounds: &[&Expr], dst: Symbol, counter: Symbol) -> Option<()> {
    let still = bounds
        .iter()
        .all(|b| !symbol_appears_in_expr(dst, b) && !symbol_appears_in_expr(counter, b));
    still.then_some(())
}

fn validate_slice_push_types(
    src_sym: Symbol,
    dst_sym: Symbol,
    variable_types: &HashMap<Symbol, String>,
) -> Option<()> {
    if src_sym == dst_sym {
        return None;
    }

    let dst_type = variable_types.get(&dst_sym)?;
    if !dst_type.starts_with("LogosSeq<") && !dst_type.starts_with("Vec<") {
        return None;
    }

    let src_type = variable_types.get(&src_sym)?;
    if !src_type.starts_with("LogosSeq<")
        && !src_type.starts_with("Vec<")
        && !src_type.starts_with("&[")
        && !src_type.starts_with("&mut [")
    {
        return None;
    }

    Some(())
}

fn emit_extend_from_slice(
    interner: &Interner,
    indent: usize,
    dst_sym: Symbol,
    src_sym: Symbol,
    counter_sym: Symbol,
    start_expr: &Expr,
    end_expr: &Expr,
    is_exclusive: bool,
    remaining: &[&Stmt],
    binding_info: Option<bool>, // Some(true) = new let, Some(false) = reassignment, None = already exists
    src_is_logos_seq: bool,
) -> String {
    let indent_str = "    ".repeat(indent);
    let names = RustNames::new(interner);
    let dst_name = names.ident(dst_sym);
    let src_name = names.ident(src_sym);
    let counter_name = names.ident(counter_sym);
    let start_str = codegen_expr_simple(start_expr, interner);
    let end_str = codegen_expr_simple(end_expr, interner);
    let owner = if src_is_logos_seq {
        format!("{}.borrow()", src_name)
    } else {
        src_name.to_string()
    };

    let mut output = String::new();
    writeln!(
        output,
        "{}{}.extend_from_slice({});",
        indent_str,
        dst_name,
        copied_run(&owner, &start_str, &end_str, is_exclusive)
    )
    .unwrap();

    if symbol_appears_in_stmts(counter_sym, remaining) {
        let post_val = counter_after(&start_str, &end_str, is_exclusive);
        match binding_info {
            Some(true) => {
                writeln!(
                    output,
                    "{}let mut {}: i64 = {};",
                    indent_str, counter_name, post_val
                )
                .unwrap();
            }
            Some(false) | None => {
                writeln!(output, "{}{} = {};", indent_str, counter_name, post_val).unwrap();
            }
        }
    }

    output
}

/// Detect drain-tail pattern in a while loop body.
///
/// When a while loop body starts with `If cond: Push item counter of array to target;
/// Set counter to counter + 1. Otherwise: ...`, and the If condition is loop-invariant
/// within the then-branch (no variable in the condition is modified by the then-branch),
/// we know that once the condition becomes true, ALL remaining iterations will take the
/// then-branch. This is equivalent to copying items `counter` through the loop's bound in one step,
/// leaving the counter one past the bound, and `break`.
///
/// Returns the optimized If code if the pattern matches, or None.
pub(crate) fn try_emit_drain_tail_in_while<'a>(
    stmt: &Stmt<'a>,
    while_cond: &Expr<'a>,
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
    // The while condition must be `counter <= bound` (LtEq only — Lt doesn't apply
    // since drain means "until exhausted").
    let (counter_sym, bound_expr) = match while_cond {
        Expr::BinaryOp {
            op: BinaryOpKind::LtEq,
            left,
            right,
        } => {
            if let Expr::Identifier(sym) = left {
                (*sym, *right)
            } else {
                return None;
            }
        }
        _ => return None,
    };

    // The statement must be an If/Otherwise.
    let (if_cond, then_block, else_block) = match stmt {
        Stmt::If {
            cond,
            then_block,
            else_block: Some(else_block),
        } => (*cond, then_block, else_block),
        _ => return None,
    };

    // The then-block must have exactly 2 statements.
    if then_block.len() != 2 {
        return None;
    }

    // Statement 1: Push item <counter> of <array> to <target>
    let (target_sym, array_sym, push_counter_sym) = match &then_block[0] {
        Stmt::Push { value, collection } => {
            let tgt = if let Expr::Identifier(s) = collection {
                *s
            } else {
                return None;
            };
            if let Expr::Index {
                collection: arr_expr,
                index: idx_expr,
            } = value
            {
                let arr_sym = if let Expr::Identifier(s) = arr_expr {
                    *s
                } else {
                    return None;
                };
                let idx_sym = if let Expr::Identifier(s) = idx_expr {
                    *s
                } else {
                    return None;
                };
                (tgt, arr_sym, idx_sym)
            } else {
                return None;
            }
        }
        _ => return None,
    };

    // Statement 2: Set counter to counter + 1
    validate_increment(&then_block[1], push_counter_sym)?;

    // The push counter must match the while loop counter.
    if push_counter_sym != counter_sym {
        return None;
    }
    // The loop re-reads its bound while it drains into the target; one copy reads it once.
    bounds_hold_still(&[bound_expr], target_sym, counter_sym)?;

    // Validate types: both array and target must be Vec<T>.
    validate_slice_push_types(array_sym, target_sym, ctx.get_variable_types())?;

    // The If condition must be loop-invariant within the then-branch:
    // no variable in the condition is modified by the then-branch.
    let mut cond_syms = Vec::new();
    collect_expr_symbols(if_cond, &mut cond_syms);
    for sym in &cond_syms {
        if body_modifies_var(then_block, *sym) || body_mutates_collection(then_block, *sym) {
            return None;
        }
    }

    // Pattern matched! Emit optimized code.
    let indent_str = "    ".repeat(indent);
    let names = RustNames::new(interner);
    let target_name = names.ident(target_sym);
    let array_name = names.ident(array_sym);
    let counter_name = names.ident(push_counter_sym);
    use super::expr::codegen_expr_with_async;
    let cond_str = codegen_expr_with_async(
        if_cond,
        interner,
        synced_vars,
        async_functions,
        ctx.get_variable_types(),
    );

    let mut output = String::new();

    // Check source type to determine if .borrow() is needed
    let array_is_logos_seq = ctx
        .get_variable_types()
        .get(&array_sym)
        .map(|t| t.split("|__hl:").next().unwrap_or(t.as_str()))
        .map(|t| t.starts_with("LogosSeq"))
        .unwrap_or(true);
    let borrow_expr = if array_is_logos_seq {
        format!("{}.borrow()", array_name)
    } else {
        array_name.to_string()
    };

    // Emit the If with extend_from_slice + break for the then-branch.
    writeln!(output, "{}if {} {{", indent_str, cond_str).unwrap();
    // Copy only up to the loop's upper bound, NOT to the end of the array: `while i <= bound` pushes
    // items i through bound, under the loop's own rule.
    // The loop leaves its counter one past the bound, which the drain reached.
    let bound_str = codegen_expr_with_async(
        bound_expr,
        interner,
        synced_vars,
        async_functions,
        ctx.get_variable_types(),
    );
    writeln!(
        output,
        "{}    {}.extend_from_slice({});",
        indent_str,
        target_name,
        copied_run(&borrow_expr, &counter_name, &bound_str, false)
    )
    .unwrap();
    writeln!(
        output,
        "{}    {} = {} + 1;",
        indent_str, counter_name, bound_str
    )
    .unwrap();
    writeln!(output, "{}    break;", indent_str).unwrap();
    writeln!(output, "{}}} else {{", indent_str).unwrap();

    // Emit the else block normally.
    for else_stmt in else_block.iter() {
        output.push_str(&super::codegen_stmt(
            else_stmt,
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

    writeln!(output, "{}}}", indent_str).unwrap();

    Some(output)
}

/// Result of detecting a buffer-reuse pattern in a loop body.
pub(crate) struct BufferReuseInfo {
    pub inner_sym: Symbol,
    pub outer_sym: Symbol,
    pub inner_elem_type: String,
    pub inner_let_idx: usize,
    pub set_idx: usize,
}

/// Detect buffer reuse pattern in a loop body: a mutable inner buffer is created
/// fresh each iteration, filled, then transferred to an outer variable via `Set`.
/// Returns the detected symbols and indices needed to apply the optimization:
/// hoist the buffer before the loop, emit `.clear()` instead of allocation,
/// and `std::mem::swap()` instead of ownership transfer.
/// Does the buffer `dst`, declared at `let_idx`, get fully overwritten in the
/// statements that follow by a copy of a source slice — the
/// `try_emit_seq_from_slice_pattern` shape (a counter-init, then
/// `While c <= BOUND: Push item c of SRC to dst; c += 1`)? This mirrors that
/// peephole's forward scan so that gating it on `is_scratch_hoist` is sound: if
/// this returns true, the peephole WILL recognize and emit the copy (now as a
/// `clear()` + `extend_from_slice` reuse rather than a fresh `.to_vec()`).
fn followed_by_full_copy_fill(stmts: &[&Stmt], let_idx: usize, dst: Symbol) -> bool {
    for scan in (let_idx + 1)..stmts.len() {
        let stmt = stmts[scan];
        let is_counter_init = match stmt {
            Stmt::Let { value, .. } if is_simple_expr(value) => true,
            Stmt::Set { value, .. } if is_simple_expr(value) => true,
            _ => false,
        };
        if is_counter_init && scan + 1 < stmts.len() {
            let c_sym = match stmt {
                Stmt::Let { var, .. } => *var,
                Stmt::Set { target, .. } => *target,
                _ => unreachable!(),
            };
            if let Stmt::While { cond, body, .. } = stmts[scan + 1] {
                let cond_ok = matches!(cond,
                    Expr::BinaryOp { op: BinaryOpKind::LtEq | BinaryOpKind::Lt, left, .. }
                    if matches!(left, Expr::Identifier(s) if *s == c_sym));
                if cond_ok && body.len() == 2 {
                    let push_ok = match &body[0] {
                        Stmt::Push { value, collection } => {
                            matches!(collection, Expr::Identifier(s) if *s == dst)
                                && matches!(value, Expr::Index { index, .. }
                                    if matches!(index, Expr::Identifier(s) if *s == c_sym))
                        }
                        _ => false,
                    };
                    let inc_ok = match &body[1] {
                        Stmt::Set { target, value } => {
                            *target == c_sym
                                && matches!(value, Expr::BinaryOp { op: BinaryOpKind::Add, left, right }
                                    if (matches!(left, Expr::Identifier(s) if *s == c_sym) && matches!(right, Expr::Literal(Literal::Number(1))))
                                    || (matches!(left, Expr::Literal(Literal::Number(1))) && matches!(right, Expr::Identifier(s) if *s == c_sym)))
                        }
                        _ => false,
                    };
                    if push_ok && inc_ok {
                        return true;
                    }
                }
            }
        }
        // Any other reference to `dst` before the fill is recognized means the
        // shape isn't the clean copy-then-use scratch buffer — be conservative.
        if symbol_appears_in_stmts(dst, &[stmt]) {
            return false;
        }
    }
    false
}

/// Detect loop-local scratch buffers in a `while` body whose per-iteration
/// allocation can be hoisted out of the loop and reused. A buffer `X` qualifies
/// when, in the body, it is:
///   1. declared `Let mutable X be a new Seq of T` (or annotated `Seq of T`);
///   2. de-Rc'd (`ctx.is_de_rc(X)`): uniquely owned, never aliased / returned /
///      stored as an element / passed away — so reuse can never be observed
///      outside this iteration;
///   3. not a `try_emit_buffer_reuse_while` swap partner (mutually exclusive);
///   4. not referenced before its declaration in the body (no outer-scope shadow);
///   5. immediately filled by a FULL copy of a source slice (the
///      `try_emit_seq_from_slice_pattern` shape) — so `clear()` +
///      `extend_from_slice` reproduces exactly the copied contents.
///
/// Returns each qualifying `(buffer_symbol, element_type)`. The `Stmt::While`
/// handler hoists `let mut X: Vec<T> = Vec::new();` before the loop and registers
/// `X` via `register_scratch_hoist`; the fill peephole then emits the reuse form.
/// General by construction: it fires on any loop-local fully-overwritten
/// non-escaping scratch buffer, not just fannkuch's `perm`.
pub(crate) fn detect_scratch_hoist_in_body<'a>(
    body: &[Stmt<'a>],
    interner: &Interner,
    ctx: &RefinementContext<'a>,
) -> Vec<(Symbol, String)> {
    let mut out = Vec::new();
    // The buffer-reuse swap pass owns its partners; never double-claim them.
    let swap = detect_buffer_reuse_in_body(body, interner, ctx);
    let body_refs: Vec<&Stmt> = body.iter().collect();
    for li in 0..body.len() {
        // (1) Let mutable X = new Seq of T (or annotated Seq of T).
        let (dst, elem_type) = match &body[li] {
            Stmt::Let {
                var,
                value,
                ty,
                mutable: true,
            } => {
                let type_from_new = if let Expr::New {
                    type_name,
                    type_args,
                    init_fields,
                } = value
                {
                    let tn = interner.resolve(*type_name);
                    if matches!(tn, "Seq" | "List" | "Vec")
                        && init_fields.is_empty()
                        && !type_args.is_empty()
                    {
                        Some(codegen_type_expr(&type_args[0], interner))
                    } else {
                        None
                    }
                } else {
                    None
                };
                let type_from_annotation = if let Some(TypeExpr::Generic { base, params }) = ty {
                    let base_name = interner.resolve(*base);
                    if matches!(base_name, "Seq" | "List" | "Vec") && !params.is_empty() {
                        Some(codegen_type_expr(&params[0], interner))
                    } else {
                        None
                    }
                } else {
                    None
                };
                match type_from_new.or(type_from_annotation) {
                    Some(t) => (*var, t),
                    None => continue,
                }
            }
            _ => continue,
        };
        // (2) de-Rc proof: uniquely owned, never escapes the iteration.
        if !ctx.is_de_rc(dst) {
            continue;
        }
        // (3) Not a buffer-reuse swap partner.
        if let Some(ref r) = swap {
            if r.inner_sym == dst || r.outer_sym == dst {
                continue;
            }
        }
        // (4) Not referenced before its declaration in the body.
        if symbol_appears_in_stmts(dst, &body_refs[..li]) {
            continue;
        }
        // (5) Fully overwritten by a recognized copy fill.
        if !followed_by_full_copy_fill(&body_refs, li, dst) {
            continue;
        }
        out.push((dst, elem_type));
    }
    out
}

/// A buffer-reuse buffer (`curr`) refilled by the loop-split transform: every
/// push to it lives inside ONE top-level version-guard `If` (the prefix/suffix
/// sub-loops, and the fallback original loop), never at the body's top level.
/// Such a buffer is `resize`d once to the full column count and filled by
/// INDEXED writes (`fill_loop`) across every sub-loop — the single-fill path's
/// per-loop resize cannot, since the fill is spread over two loops.
pub(crate) struct SplitFillInfo {
    /// Index of the version-guard `If` in the outer-loop body.
    pub if_idx: usize,
    /// The full buffer size (the suffix loop's exclusive end), as Rust source.
    pub cols_str: String,
    /// The shared sub-loop counter name (every sub-loop writes `curr[iv]`).
    pub iv_name: String,
    /// The element default for the one-time `resize`.
    pub default: &'static str,
}

/// Recognize the loop-split shape for a buffer-reuse buffer `curr`: all its
/// pushes are nested inside a single top-level `If`, whose then-block's last
/// `While` is the suffix loop (`iv <op> limit`) giving the full size. Returns
/// the info to emit `resize(cols)` once and a body-wide `fill_loop`.
pub(crate) fn detect_split_fill<'a>(
    body: &[Stmt<'a>],
    curr: Symbol,
    inner_elem_type: &str,
    interner: &Interner,
) -> Option<SplitFillInfo> {
    if top_level_push_count(body, curr) != 0 || total_push_count(body, curr) == 0 {
        return None;
    }
    // The unique top-level `If` carrying every curr push.
    let mut found: Option<(usize, &'a [Stmt<'a>])> = None;
    for (i, s) in body.iter().enumerate() {
        if let Stmt::If {
            then_block,
            else_block,
            ..
        } = s
        {
            let here = total_push_count(then_block, curr) > 0
                || else_block.map_or(false, |eb| total_push_count(eb, curr) > 0);
            if here {
                if found.is_some() {
                    return None;
                }
                found = Some((i, then_block));
            }
        }
    }
    let (if_idx, then_block) = found?;
    // The suffix loop is the LAST `While` in the then-block; its limit is the
    // full column count covering every index any sub-loop writes.
    let suffix_cond: &Expr = then_block.iter().rev().find_map(|s| match s {
        Stmt::While { cond, .. } => Some(*cond),
        _ => None,
    })?;
    let (iv_sym, limit_expr, inclusive): (Symbol, &Expr, bool) = match suffix_cond {
        Expr::BinaryOp {
            op: BinaryOpKind::LtEq,
            left,
            right,
        } => match &**left {
            Expr::Identifier(s) => (*s, *right, true),
            _ => return None,
        },
        Expr::BinaryOp {
            op: BinaryOpKind::Lt,
            left,
            right,
        } => match &**left {
            Expr::Identifier(s) => (*s, *right, false),
            _ => return None,
        },
        _ => return None,
    };
    let limit_str = codegen_expr_simple(limit_expr, interner);
    let cols_str = if inclusive {
        match limit_expr {
            Expr::Literal(Literal::Number(n)) => format!("{}", n + 1),
            _ => format!("({} + 1)", limit_str),
        }
    } else {
        limit_str
    };
    let default = default_literal_for_vec_elem(&format!("Vec<{}>", inner_elem_type));
    Some(SplitFillInfo {
        if_idx,
        cols_str,
        iv_name: RustNames::new(interner).ident(iv_sym),
        default,
    })
}

pub(crate) fn detect_buffer_reuse_in_body<'a>(
    body: &[Stmt<'a>],
    interner: &Interner,
    ctx: &RefinementContext<'a>,
) -> Option<BufferReuseInfo> {
    if body.len() < 2 {
        return None;
    }

    // Find Let mutable INNER = new Seq of T in the first two body statements.
    let (inner_sym, inner_elem_type, inner_let_idx) = {
        let mut found = None;
        for (bi, stmt) in body.iter().enumerate() {
            if bi > 1 {
                break;
            }
            if let Stmt::Let {
                var,
                value,
                ty,
                mutable: true,
            } = stmt
            {
                let type_from_annotation = if let Some(TypeExpr::Generic { base, params }) = ty {
                    let base_name = interner.resolve(*base);
                    if matches!(base_name, "Seq" | "List" | "Vec") && !params.is_empty() {
                        Some(codegen_type_expr(&params[0], interner))
                    } else {
                        None
                    }
                } else {
                    None
                };
                let type_from_new = if let Expr::New {
                    type_name,
                    type_args,
                    init_fields,
                } = value
                {
                    let tn = interner.resolve(*type_name);
                    if matches!(tn, "Seq" | "List" | "Vec")
                        && init_fields.is_empty()
                        && !type_args.is_empty()
                    {
                        Some(codegen_type_expr(&type_args[0], interner))
                    } else {
                        None
                    }
                } else {
                    None
                };
                if let Some(t) = type_from_annotation.or(type_from_new) {
                    found = Some((*var, t, bi));
                    break;
                }
            }
        }
        found?
    };

    // Find Set OUTER to Expr::Identifier(INNER) in the body, scanning from the end.
    let (outer_sym, set_idx) = {
        let mut found = None;
        for (bi, stmt) in body.iter().enumerate().rev() {
            if let Stmt::Set { target, value } = stmt {
                if let Expr::Identifier(src) = value {
                    if *src == inner_sym && *target != inner_sym {
                        found = Some((*target, bi));
                        break;
                    }
                }
            }
        }
        found?
    };

    // The Set must be the last or second-to-last body statement.
    // If second-to-last, the last must be a counter increment.
    if set_idx == body.len().wrapping_sub(2) && body.len() >= 2 {
        match &body[body.len() - 1] {
            Stmt::Set { target, value } => {
                let is_increment = matches!(value,
                    Expr::BinaryOp { op: BinaryOpKind::Add, left, right }
                    if (matches!(left, Expr::Identifier(s) if *s == *target) && matches!(right, Expr::Literal(Literal::Number(1))))
                    || (matches!(left, Expr::Literal(Literal::Number(1))) && matches!(right, Expr::Identifier(s) if *s == *target))
                );
                if !is_increment {
                    return None;
                }
            }
            _ => return None,
        }
    } else if set_idx != body.len() - 1 {
        return None;
    }

    // Verify OUTER is a Vec of the same element type as INNER.
    let outer_type = ctx.get_variable_types().get(&outer_sym)?;
    let expected_logos = format!("LogosSeq<{}>", inner_elem_type);
    let expected_vec = format!("Vec<{}>", inner_elem_type);
    if !outer_type.starts_with(&expected_logos) && !outer_type.starts_with(&expected_vec) {
        return None;
    }

    // Verify INNER is not referenced after the Set.
    for bi in (set_idx + 1)..body.len() {
        let stmt_ref: &Stmt = &body[bi];
        if symbol_appears_in_stmts(inner_sym, &[stmt_ref]) {
            return None;
        }
    }

    Some(BufferReuseInfo {
        inner_sym,
        outer_sym,
        inner_elem_type,
        inner_let_idx,
        set_idx,
    })
}

/// Peephole optimization: detect a While whose body creates a fresh Seq each iteration,
/// fills it via an inner loop, and transfers it to an outer variable. Replace with:
/// - Hoisted inner buffer declaration (before the while)
/// - `.clear()` instead of `new Seq` each iteration
/// - `std::mem::swap(&mut outer, &mut inner)` instead of ownership transfer
///
/// This eliminates N allocations + drops for N iterations of the outer loop.
///
/// AST shape:
/// ```text
/// While COND:
///     Let mutable INNER = new Seq of T.   ← replaced with INNER.clear()
///     ... (fill INNER via inner loop) ...
///     Set OUTER to INNER.                 ← replaced with mem::swap
///     [Set counter to counter + 1.]       ← optional counter increment
/// ```
///
/// Conditions:
/// - OUTER must be a Vec<T> with the same element type as INNER
/// - Set OUTER to INNER must be the last meaningful statement (before optional counter increment)
/// - INNER must not appear after the Set in the body
pub(crate) fn try_emit_buffer_reuse_while<'a>(
    stmts: &[&Stmt<'a>],
    idx: usize,
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
) -> Option<(String, usize)> {
    let (cond, body) = match stmts[idx] {
        Stmt::While { cond, body, .. } => (cond, body),
        _ => return None,
    };

    if body.len() < 3 {
        return None;
    }

    let reuse = detect_buffer_reuse_in_body(body, interner, ctx)?;

    // Pattern matched! Generate the transformed While.
    let indent_str = "    ".repeat(indent);
    let names = RustNames::new(interner);
    let inner_name = names.ident(reuse.inner_sym);
    let outer_name = names.ident(reuse.outer_sym);

    let mut output = String::new();

    // O2 de-Rc Phase 2: a buffer-reuse swap pair both de-Rc'd → the hoisted
    // inner buffer is a plain `Vec<T>`, cleared directly, swapped by value.
    // Both partners must agree (the analysis drops the pair otherwise) so the
    // `std::mem::swap` stays well-typed.
    let de_rc_pair = ctx.is_de_rc(reuse.inner_sym) && ctx.is_de_rc(reuse.outer_sym);

    // Hoist inner buffer declaration before the while loop.
    if de_rc_pair {
        writeln!(
            output,
            "{}let mut {}: Vec<{}> = Vec::new();",
            indent_str, inner_name, reuse.inner_elem_type
        )
        .unwrap();
    } else {
        writeln!(
            output,
            "{}let mut {}: LogosSeq<{}> = LogosSeq::new();",
            indent_str, inner_name, reuse.inner_elem_type
        )
        .unwrap();
    }

    // Loop bounds hoisting (replicate from stmt.rs While handler).
    use super::stmt::{collect_length_syms_from_stmts, extract_length_expr_syms};
    let mut all_length_syms_raw = extract_length_expr_syms(cond);
    collect_length_syms_from_stmts(body, &mut all_length_syms_raw);
    let mut seen = HashSet::new();
    let all_length_syms: Vec<Symbol> = all_length_syms_raw
        .into_iter()
        .filter(|s| seen.insert(*s))
        .collect();

    let mut hoisted_syms: Vec<(Symbol, Option<String>)> = Vec::new();
    for len_sym in &all_length_syms {
        if !body_mutates_collection(body, *len_sym) && !body_modifies_var(body, *len_sym) {
            let name = interner.resolve(*len_sym);
            let hoisted_name = format!("{}_len", name);
            writeln!(
                output,
                "{}let {} = ({}.len() as i64);",
                indent_str, hoisted_name, name
            )
            .unwrap();
            let old_type = ctx.get_variable_types().get(len_sym).cloned();
            let new_type = match &old_type {
                Some(existing) => format!("{}|__hl:{}", existing, hoisted_name),
                None => format!("|__hl:{}", hoisted_name),
            };
            ctx.register_variable_type(*len_sym, new_type);
            hoisted_syms.push((*len_sym, old_type));
        }
    }

    // Emit the while condition.
    use super::expr::codegen_expr_with_async;
    let cond_str = codegen_expr_with_async(
        cond,
        interner,
        synced_vars,
        async_functions,
        ctx.get_variable_types(),
    );
    writeln!(output, "{}while {} {{", indent_str, cond_str).unwrap();
    ctx.push_scope();

    // Process body statements with peephole chain, intercepting transformed statements.
    let body_refs: Vec<&Stmt> = body.iter().collect();
    let mut bi = 0;
    while bi < body_refs.len() {
        if bi == reuse.inner_let_idx {
            // Replace Let inner = new Seq with .clear() (Vec or RefCell form).
            if de_rc_pair {
                writeln!(output, "{}    {}.clear();", indent_str, inner_name).unwrap();
                ctx.register_variable_type(
                    reuse.inner_sym,
                    format!("Vec<{}>", reuse.inner_elem_type),
                );
            } else {
                writeln!(
                    output,
                    "{}    {}.borrow_mut().clear();",
                    indent_str, inner_name
                )
                .unwrap();
                ctx.register_variable_type(
                    reuse.inner_sym,
                    format!("LogosSeq<{}>", reuse.inner_elem_type),
                );
            }
            bi += 1;
            continue;
        }
        if bi == reuse.set_idx {
            // Replace Set outer to inner with mem::swap
            writeln!(
                output,
                "{}    std::mem::swap(&mut {}, &mut {});",
                indent_str, outer_name, inner_name
            )
            .unwrap();
            bi += 1;
            continue;
        }

        // Standard peephole chain for body statements.
        if let Some((code, skip)) = try_emit_seq_from_slice_pattern(
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
        if let Some((code, skip)) =
            try_emit_vec_fill_pattern(&body_refs, bi, interner, indent + 1, ctx)
        {
            output.push_str(&code);
            bi += 1 + skip;
            continue;
        }
        if let Some((code, skip)) = try_emit_vec_with_capacity_pattern(
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
        if let Some((code, skip)) =
            try_emit_merge_capacity_pattern(&body_refs, bi, interner, indent + 1, ctx)
        {
            output.push_str(&code);
            bi += 1 + skip;
            continue;
        }
        if let Some((code, skip)) = try_emit_for_range_pattern(
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
        if let Some((code, skip)) = try_emit_prefix_reverse(
            &body_refs,
            bi,
            interner,
            indent + 1,
            ctx.get_variable_types(),
        ) {
            output.push_str(&code);
            bi += 1 + skip;
            continue;
        }
        if let Some((code, skip)) = try_emit_swap_pattern(
            &body_refs,
            bi,
            interner,
            indent + 1,
            ctx.get_variable_types(),
            ctx.oracle(),
        ) {
            output.push_str(&code);
            bi += 1 + skip;
            continue;
        }
        if let Some((code, skip)) = try_emit_rotate_left_pattern(
            &body_refs,
            bi,
            interner,
            indent + 1,
            ctx.get_variable_types(),
        ) {
            output.push_str(&code);
            bi += 1 + skip;
            continue;
        }

        // Fallback: normal codegen.
        use super::codegen_stmt;
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

    // Restore hoisted length symbols.
    for (sym, old_type) in hoisted_syms {
        if let Some(old) = old_type {
            ctx.register_variable_type(sym, old);
        } else {
            ctx.get_variable_types_mut().remove(&sym);
        }
    }

    writeln!(output, "{}}}", indent_str).unwrap();

    Some((output, 0))
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen/peephole/tests.rs"]
mod tests;
