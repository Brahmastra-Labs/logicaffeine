use std::collections::{HashMap, HashSet};
use std::fmt::Write;

use crate::analysis::registry::TypeRegistry;
use crate::analysis::types::RustNames;
use crate::ast::stmt::{BinaryOpKind, Expr, Literal, Stmt, TypeExpr};
use crate::intern::{Interner, Symbol};
use super::context::{RefinementContext, VariableCapabilities};
use super::detection::symbol_appears_in_stmts;
use super::i64_map::map_rust_type;
use super::types::codegen_type_expr;

/// Collection type information for with_capacity pattern detection.
enum CollInfo { Vec(String), Map(String, String) }

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
            Stmt::If { then_block, else_block, .. } => {
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
            Stmt::Push { collection: Expr::Identifier(b), .. } if *b == buf => n += 1,
            Stmt::If { then_block, else_block, .. } => {
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
        if let Stmt::Push { collection: Expr::Identifier(buf), .. } = stmt {
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

    // OPT-TILE: Tiled loop nest optimization for triple-nested loops.
    // Detect i<N / k<N / j<N pattern (matrix multiplication) and emit 6-level
    // tiled loop nest with step_by for L1 cache locality.
    if counter_start_literal == Some(0) && is_exclusive {
        if let Some(tiled) = try_emit_tiled_inner(
            counter_sym, limit_expr, body_without_increment,
            stmts, idx, is_new_binding, interner, indent,
            mutable_vars, ctx, lww_fields, mv_fields, synced_vars,
            var_caps, async_functions, pipe_vars, boxed_fields,
            registry, type_env,
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
    let limit_str = codegen_expr_simple(limit_expr, interner);
    let start_str = codegen_expr_simple(counter_start_expr, interner);

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
    let range_str = if use_zero_based {
        // 0-based: rebase inclusive `start..=limit` (1-based, indexing arr[i-1])
        // to `(start-1)..limit` indexing arr[i]. For start == 1 this is `0..limit`.
        let lo = match counter_start_expr {
            Expr::Literal(Literal::Number(n)) => format!("{}", n - 1),
            _ => format!("({} - 1)", start_str),
        };
        if let Expr::Literal(Literal::Number(n)) = limit_expr {
            format!("{}..{}", lo, n)
        } else {
            format!("{}..{}", lo, limit_str)
        }
    } else if is_exclusive {
        format!("{}..{}", start_str, limit_str)
    } else {
        // For literal limits, compute limit+1 at compile time
        if let Expr::Literal(Literal::Number(n)) = limit_expr {
            format!("{}..{}", start_str, n + 1)
        } else {
            format!("{}..({} + 1)", start_str, limit_str)
        }
    };

    let mut output = String::new();

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

    // AOT BCE-hoist: row-major affine-offset guards (`c[i*n+j]` — `i*n` is
    // invariant w.r.t. this loop). A preheader `assert!` proves the max index
    // in range so the per-iteration access elides soundly. The offset/index are
    // loop-invariant arithmetic, so empty sync/async sets suffice for codegen.
    let affine_empty: HashSet<Symbol> = HashSet::new();
    let affine_guards = super::stmt::plan_affine_offset_guards(
        body_without_increment,
        counter_sym,
        is_exclusive,
        ctx.get_variable_types(),
    );
    super::stmt::emit_affine_offset_preheader(
        &affine_guards, &limit_str, interner, &affine_empty, &affine_empty,
        ctx.get_variable_types(), &indent_str, &mut output,
    );

    // Hoist inner buffer if buffer reuse detected (stays outside any borrow
    // scope; the swap rebinds it across iterations).
    // O2 de-Rc Phase 2: both partners of a buffer-reuse swap de-Rc'd → the
    // hoisted inner buffer is a plain `Vec<T>`, cleared directly, swapped by
    // value. Both must agree (the analysis drops the pair otherwise).
    let de_rc_reuse = buffer_reuse
        .as_ref()
        .map_or(false, |r| ctx.is_de_rc(r.inner_sym) && ctx.is_de_rc(r.outer_sym));
    // Loop-split fill: the reused buffer is filled across the prefix/suffix
    // sub-loops of a version-guard `If`, so the single-fill path (one 0-based
    // resize) cannot size it. Resize once to the full column count and drive
    // every sub-loop's push through `fill_loop` (indexed unchecked writes).
    let split_fill = buffer_reuse
        .as_ref()
        .filter(|_| de_rc_reuse)
        .and_then(|r| detect_split_fill(body_without_increment, r.inner_sym, &r.inner_elem_type, interner));
    if let Some(ref reuse) = buffer_reuse {
        let reuse_inner = names.ident(reuse.inner_sym);
        if de_rc_reuse {
            writeln!(output, "{}let mut {}: Vec<{}> = Vec::new();", indent_str, reuse_inner, reuse.inner_elem_type).unwrap();
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
            writeln!(output, "{}let mut {}: LogosSeq<{}> = LogosSeq::new();", indent_str, reuse_inner, reuse.inner_elem_type).unwrap();
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
        writeln!(output, "{}{}.resize(({}) as usize, {});", indent_str, buf_name, count_str, default).unwrap();
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
                writeln!(output, "{}let mut {}: Vec<{}> = Vec::new();",
                    indent_str, interner.resolve(dst), elem_type).unwrap();
                ctx.register_variable_type(dst, format!("Vec<{}>", elem_type));
                ctx.register_scratch_hoist(dst);
                dst
            })
            .collect();

    writeln!(output, "{}for {} in {} {{", indent_str, counter_name, range_str).unwrap();
    emit_bounds_hint_header(&bounds_hints, interner, &"    ".repeat(indent + 1), &mut output);
    super::stmt::emit_affine_offset_header(
        &affine_guards, interner, &affine_empty, &affine_empty,
        ctx.get_variable_types(), &"    ".repeat(indent + 1), &mut output,
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
                        writeln!(output, "{}{}.resize(({}) as usize, {});", "    ".repeat(indent + 1), reuse_inner, sf.cols_str, sf.default).unwrap();
                    } else if !ctx.is_buffer_reuse_fill(reuse.inner_sym) {
                        // A1: a fill-converted buffer is sized by the inner loop's
                        // `resize` and fully overwritten by index — no `clear()`
                        // (which would force the resize to re-init N defaults every
                        // outer round). Plain reused buffers still clear.
                        writeln!(output, "{}{}.clear();", "    ".repeat(indent + 1), reuse_inner).unwrap();
                    }
                    ctx.register_variable_type(reuse.inner_sym, format!("Vec<{}>", reuse.inner_elem_type));
                } else {
                    writeln!(output, "{}{}.borrow_mut().clear();", "    ".repeat(indent + 1), reuse_inner).unwrap();
                    ctx.register_variable_type(reuse.inner_sym, format!("LogosSeq<{}>", reuse.inner_elem_type));
                }
                bi += 1;
                continue;
            }
            if bi == reuse.set_idx {
                let reuse_inner = names.ident(reuse.inner_sym);
                let reuse_outer = names.ident(reuse.outer_sym);
                writeln!(output, "{}std::mem::swap(&mut {}, &mut {});", "    ".repeat(indent + 1), reuse_outer, reuse_inner).unwrap();
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
                    output.push_str(&super::codegen_stmt(body_refs[bi], interner, indent + 1, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry, type_env));
                    ctx.clear_fill_loop();
                    bi += 1;
                    continue;
                }
            }
        }
        if let Some((code, skip)) = try_emit_vec_fill_pattern(&body_refs, bi, interner, indent + 1, ctx) {
            output.push_str(&code);
            bi += 1 + skip;
            continue;
        }
        if let Some((code, skip)) = try_emit_for_range_pattern(&body_refs, bi, interner, indent + 1, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry, type_env) {
            output.push_str(&code);
            bi += 1 + skip;
            continue;
        }
        if let Some((code, skip)) = try_emit_prefix_reverse(&body_refs, bi, interner, indent + 1, ctx.get_variable_types()) {
            output.push_str(&code);
            bi += 1 + skip;
            continue;
        }
        if let Some((code, skip)) = try_emit_swap_pattern(&body_refs, bi, interner, indent + 1, ctx.get_variable_types(), ctx.oracle()) {
            output.push_str(&code);
            bi += 1 + skip;
            continue;
        }
        if let Some((code, skip)) = try_emit_seq_from_slice_pattern(&body_refs, bi, interner, indent + 1, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry, type_env) {
            output.push_str(&code);
            bi += 1 + skip;
            continue;
        }
        if let Some((code, skip)) = try_emit_seq_copy_pattern(&body_refs, bi, interner, indent + 1, ctx) {
            output.push_str(&code);
            bi += 1 + skip;
            continue;
        }
        if let Some((code, skip)) = try_emit_rotate_left_pattern(&body_refs, bi, interner, indent + 1, ctx.get_variable_types()) {
            output.push_str(&code);
            bi += 1 + skip;
            continue;
        }
        output.push_str(&super::codegen_stmt(body_refs[bi], interner, indent + 1, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry, type_env));
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
    // Close the borrow-hoist scope and restore the handles' tracked types.
    super::hoist::emit_hoist_close(&hoist_plan, &indent_str, ctx, &mut output);

    // Emit post-loop counter value only if the counter is used after the loop.
    let remaining_stmts = &stmts[idx + 2..];
    if symbol_appears_in_stmts(counter_sym, remaining_stmts) {
        // OPT-7: If the next statement immediately overwrites the counter,
        // skip the max() computation and just declare the variable (if needed).
        let next_stmt_overwrites_counter = remaining_stmts.first().map_or(false, |s| {
            matches!(s, Stmt::Set { target, .. } if *target == counter_sym)
        });

        if next_stmt_overwrites_counter {
            // For Let-based counters, we still need to declare the variable
            // so it exists in scope for the overwriting Set statement.
            if is_new_binding {
                writeln!(output, "{}let mut {} = 0;", indent_str, counter_name).unwrap();
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
                writeln!(output, "{}let mut {} = {};", indent_str, counter_name, post_value).unwrap();
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
pub(crate) fn body_modifies_var(stmts: &[Stmt], sym: Symbol) -> bool {
    for stmt in stmts {
        match stmt {
            Stmt::Set { target, .. } if *target == sym => return true,
            Stmt::If { then_block, else_block, .. } => {
                if body_modifies_var(then_block, sym) {
                    return true;
                }
                if let Some(else_stmts) = else_block {
                    if body_modifies_var(else_stmts, sym) {
                        return true;
                    }
                }
            }
            Stmt::While { body, .. } => {
                if body_modifies_var(body, sym) {
                    return true;
                }
            }
            Stmt::Repeat { body, .. } => {
                if body_modifies_var(body, sym) {
                    return true;
                }
            }
            Stmt::Zone { body, .. } => {
                if body_modifies_var(body, sym) {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

/// True if `stmt` (recursively, through nested blocks) `Set`s `sym`.
fn stmt_modifies_var(stmt: &Stmt, sym: Symbol) -> bool {
    match stmt {
        Stmt::Set { target, .. } => *target == sym,
        Stmt::If { then_block, else_block, .. } => {
            body_modifies_var(then_block, sym)
                || else_block.is_some_and(|e| body_modifies_var(e, sym))
        }
        Stmt::While { body, .. } | Stmt::Repeat { body, .. } | Stmt::Zone { body, .. } => {
            body_modifies_var(body, sym)
        }
        _ => false,
    }
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
            if count == 1 { bound } else { None }
        }
        Expr::BinaryOp { op: BinaryOpKind::Add, left, right } => {
            resolve_const_i64(left, stmts, upto)?.checked_add(resolve_const_i64(right, stmts, upto)?)
        }
        Expr::BinaryOp { op: BinaryOpKind::Subtract, left, right } => {
            resolve_const_i64(left, stmts, upto)?.checked_sub(resolve_const_i64(right, stmts, upto)?)
        }
        Expr::BinaryOp { op: BinaryOpKind::Multiply, left, right } => {
            resolve_const_i64(left, stmts, upto)?.checked_mul(resolve_const_i64(right, stmts, upto)?)
        }
        _ => None,
    }
}

/// Check if a loop body mutates a specific collection (used for iterator optimization).
/// Scans for Push, Pop, SetIndex, Remove, Set, and Add targeting the collection.
/// Recursively walks into nested If/While/Repeat/Zone blocks.
pub(crate) fn body_mutates_collection(stmts: &[Stmt], coll_sym: Symbol) -> bool {
    for stmt in stmts {
        match stmt {
            Stmt::Push { collection, .. } | Stmt::Pop { collection, .. }
            | Stmt::Add { collection, .. } | Stmt::Remove { collection, .. } => {
                if let Expr::Identifier(sym) = collection {
                    if *sym == coll_sym {
                        return true;
                    }
                }
            }
            Stmt::SetIndex { collection, .. } => {
                if let Expr::Identifier(sym) = collection {
                    if *sym == coll_sym {
                        return true;
                    }
                }
            }
            Stmt::Set { target, .. } if *target == coll_sym => return true,
            Stmt::If { then_block, else_block, .. } => {
                if body_mutates_collection(then_block, coll_sym) {
                    return true;
                }
                if let Some(else_stmts) = else_block {
                    if body_mutates_collection(else_stmts, coll_sym) {
                        return true;
                    }
                }
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } => {
                if body_mutates_collection(body, coll_sym) {
                    return true;
                }
            }
            Stmt::Zone { body, .. } => {
                if body_mutates_collection(body, coll_sym) {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

/// Check if a loop body resizes a specific collection via Push, Pop, Add, or Remove.
/// Unlike `body_mutates_collection`, this does NOT flag SetIndex (element-level writes),
/// making it suitable for detecting double-buffer patterns where SetIndex is expected.
/// Recursively walks into nested If/While/Repeat/Zone blocks.
pub(crate) fn body_resizes_collection(stmts: &[Stmt], coll_sym: Symbol) -> bool {
    for stmt in stmts {
        match stmt {
            Stmt::Push { collection, .. } | Stmt::Pop { collection, .. }
            | Stmt::Add { collection, .. } | Stmt::Remove { collection, .. } => {
                if let Expr::Identifier(sym) = collection {
                    if *sym == coll_sym {
                        return true;
                    }
                }
            }
            Stmt::Set { target, .. } if *target == coll_sym => return true,
            Stmt::If { then_block, else_block, .. } => {
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
        Stmt::If { then_block, else_block, .. } => {
            if let Some(else_stmts) = else_block {
                all_paths_push_to(then_block, coll_sym)
                    && all_paths_push_to(else_stmts, coll_sym)
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
        Stmt::If { then_block, else_block, .. } => {
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
    stmts.iter().any(|s| stmt_has_counter_index_use(s, counter_sym))
}

fn stmt_has_counter_index_use(stmt: &Stmt, counter_sym: Symbol) -> bool {
    match stmt {
        Stmt::Let { value, .. } => expr_has_counter_index_use(value, counter_sym),
        Stmt::Set { value, .. } => expr_has_counter_index_use(value, counter_sym),
        Stmt::Show { object, .. } => expr_has_counter_index_use(object, counter_sym),
        Stmt::Push { value, .. } => expr_has_counter_index_use(value, counter_sym),
        Stmt::SetIndex { index, value, .. } => {
            matches!(index, Expr::Identifier(sym) if *sym == counter_sym)
                || expr_has_counter_index_use(value, counter_sym)
        }
        Stmt::If { cond, then_block, else_block } => {
            expr_has_counter_index_use(cond, counter_sym)
                || counter_has_index_uses(then_block, counter_sym)
                || else_block.as_ref().map_or(false, |eb| counter_has_index_uses(eb, counter_sym))
        }
        Stmt::While { body, .. } => counter_has_index_uses(body, counter_sym),
        Stmt::Return { value } => value.map_or(false, |v| expr_has_counter_index_use(v, counter_sym)),
        Stmt::Call { args, .. } => args.iter().any(|a| expr_has_counter_index_use(a, counter_sym)),
        Stmt::Repeat { body, .. } => counter_has_index_uses(body, counter_sym),
        _ => false,
    }
}

fn expr_has_counter_index_use(expr: &Expr, counter_sym: Symbol) -> bool {
    match expr {
        Expr::Index { collection, index } => {
            matches!(index, Expr::Identifier(sym) if *sym == counter_sym)
                || expr_has_counter_index_use(collection, counter_sym)
                || expr_has_counter_index_use(index, counter_sym)
        }
        Expr::BinaryOp { left, right, .. } => {
            expr_has_counter_index_use(left, counter_sym) || expr_has_counter_index_use(right, counter_sym)
        }
        Expr::Call { args, .. } => args.iter().any(|a| expr_has_counter_index_use(a, counter_sym)),
        Expr::Length { collection } => expr_has_counter_index_use(collection, counter_sym),
        Expr::List(items) => items.iter().any(|e| expr_has_counter_index_use(e, counter_sym)),
        Expr::Not { operand } => expr_has_counter_index_use(operand, counter_sym),
        Expr::Copy { expr: inner } => expr_has_counter_index_use(inner, counter_sym),
        Expr::Slice { collection, start, end } => {
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
fn counter_indexes_only_vec_types(stmts: &[Stmt], counter_sym: Symbol, ctx: &RefinementContext) -> bool {
    for stmt in stmts {
        if !stmt_counter_indexes_vec_types(stmt, counter_sym, ctx) {
            return false;
        }
    }
    true
}

fn stmt_counter_indexes_vec_types(stmt: &Stmt, counter_sym: Symbol, ctx: &RefinementContext) -> bool {
    match stmt {
        Stmt::Let { value, .. } => expr_counter_indexes_vec_types(value, counter_sym, ctx),
        Stmt::Set { value, .. } => expr_counter_indexes_vec_types(value, counter_sym, ctx),
        Stmt::Show { object, .. } => expr_counter_indexes_vec_types(object, counter_sym, ctx),
        Stmt::Push { value, .. } => expr_counter_indexes_vec_types(value, counter_sym, ctx),
        Stmt::SetIndex { collection, index, value } => {
            let idx_uses_counter = matches!(index, Expr::Identifier(sym) if *sym == counter_sym);
            if idx_uses_counter {
                if let Expr::Identifier(coll_sym) = collection {
                    let is_vec = ctx.get_variable_types().get(coll_sym)
                        .map_or(false, |t| t.starts_with("LogosSeq") || t.starts_with("Vec") || t.starts_with("&[") || t.starts_with("&mut [") || t.starts_with("["));
                    if !is_vec { return false; }
                } else {
                    return false;
                }
            }
            expr_counter_indexes_vec_types(value, counter_sym, ctx)
        }
        Stmt::If { cond, then_block, else_block } => {
            expr_counter_indexes_vec_types(cond, counter_sym, ctx)
                && counter_indexes_only_vec_types(then_block, counter_sym, ctx)
                && else_block.as_ref().map_or(true, |eb| counter_indexes_only_vec_types(eb, counter_sym, ctx))
        }
        Stmt::While { body, .. } => counter_indexes_only_vec_types(body, counter_sym, ctx),
        Stmt::Repeat { body, .. } => counter_indexes_only_vec_types(body, counter_sym, ctx),
        Stmt::Return { value } => value.map_or(true, |v| expr_counter_indexes_vec_types(v, counter_sym, ctx)),
        Stmt::Call { args, .. } => args.iter().all(|a| expr_counter_indexes_vec_types(a, counter_sym, ctx)),
        _ => true,
    }
}

fn expr_counter_indexes_vec_types(expr: &Expr, counter_sym: Symbol, ctx: &RefinementContext) -> bool {
    match expr {
        Expr::Index { collection, index } => {
            let idx_uses_counter = matches!(index, Expr::Identifier(sym) if *sym == counter_sym);
            if idx_uses_counter {
                if let Expr::Identifier(coll_sym) = collection {
                    let is_vec = ctx.get_variable_types().get(coll_sym)
                        .map_or(false, |t| t.starts_with("LogosSeq") || t.starts_with("Vec") || t.starts_with("&[") || t.starts_with("&mut [") || t.starts_with("["));
                    if !is_vec { return false; }
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
        Expr::Call { args, .. } => args.iter().all(|a| expr_counter_indexes_vec_types(a, counter_sym, ctx)),
        Expr::Not { operand } => expr_counter_indexes_vec_types(operand, counter_sym, ctx),
        Expr::Copy { expr: inner } => expr_counter_indexes_vec_types(inner, counter_sym, ctx),
        Expr::Length { collection } => expr_counter_indexes_vec_types(collection, counter_sym, ctx),
        Expr::List(items) => items.iter().all(|e| expr_counter_indexes_vec_types(e, counter_sym, ctx)),
        Expr::Slice { collection, start, end } => {
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
        Stmt::SetIndex { collection: _, index, value } => {
            // The index position of SetIndex IS a valid index use.
            // Check: is the index expression either Identifier(counter) or does it not
            // reference counter at all? For simplicity, allow only direct Identifier(counter).
            let index_ok = match index {
                Expr::Identifier(sym) if *sym == counter_sym => true,
                _ => !expr_contains_symbol(index, counter_sym),
            };
            let value_ok = expr_uses_counter_only_in_index(value, counter_sym);
            index_ok && value_ok
        }
        Stmt::If { cond, then_block, else_block } => {
            expr_uses_counter_only_in_index(cond, counter_sym)
                && counter_only_used_for_indexing(then_block, counter_sym)
                && else_block.as_ref().map_or(true, |eb| counter_only_used_for_indexing(eb, counter_sym))
        }
        Stmt::While { cond, body, .. } => {
            expr_uses_counter_only_in_index(cond, counter_sym)
                && counter_only_used_for_indexing(body, counter_sym)
        }
        Stmt::Repeat { body, .. } => counter_only_used_for_indexing(body, counter_sym),
        Stmt::Call { args, .. } => {
            args.iter().all(|a| expr_uses_counter_only_in_index(a, counter_sym))
        }
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
            let index_ok = match index {
                Expr::Identifier(sym) if *sym == counter_sym => true,
                _ => expr_uses_counter_only_in_index(index, counter_sym),
            };
            collection_ok && index_ok
        }
        Expr::BinaryOp { op, left, right } => {
            // For comparison operators, also allow the counter as a bare operand.
            // This enables zero-based normalization for loops like:
            //   While i is at most n: If i is greater than k: ... item i of arr ...
            // The counter `i` appears in both an index and a comparison. In codegen,
            // the comparison operand gets `(i + 1)` to compensate for the 0-based shift.
            match op {
                BinaryOpKind::Lt | BinaryOpKind::LtEq | BinaryOpKind::Gt
                | BinaryOpKind::GtEq | BinaryOpKind::Eq | BinaryOpKind::NotEq => {
                    let left_is_counter = matches!(left, Expr::Identifier(s) if *s == counter_sym);
                    let right_is_counter = matches!(right, Expr::Identifier(s) if *s == counter_sym);
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
        Expr::Call { args, .. } => {
            args.iter().all(|a| expr_uses_counter_only_in_index(a, counter_sym))
        }
        Expr::Length { collection } => expr_uses_counter_only_in_index(collection, counter_sym),
        Expr::Literal(_) => true,
        Expr::List(items) => items.iter().all(|e| expr_uses_counter_only_in_index(e, counter_sym)),
        Expr::Slice { collection, start, end } => {
            expr_uses_counter_only_in_index(collection, counter_sym)
                && expr_uses_counter_only_in_index(start, counter_sym)
                && expr_uses_counter_only_in_index(end, counter_sym)
        }
        Expr::Copy { expr: inner } => expr_uses_counter_only_in_index(inner, counter_sym),
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
        Expr::Slice { collection, start, end } => {
            expr_contains_symbol(collection, sym)
                || expr_contains_symbol(start, sym)
                || expr_contains_symbol(end, sym)
        }
        Expr::Copy { expr: inner } => expr_contains_symbol(inner, sym),
        Expr::Literal(_) => false,
        _ => false,
    }
}

/// Check if a statement is `Set counter to counter + 1`.
pub(crate) fn is_counter_increment(stmt: &Stmt, counter: Symbol) -> bool {
    match stmt {
        Stmt::Set { target, value } if *target == counter => {
            match value {
                Expr::BinaryOp { op: BinaryOpKind::Add, left, right } => {
                    match (left, right) {
                        (Expr::Identifier(s), Expr::Literal(Literal::Number(1))) => *s == counter,
                        (Expr::Literal(Literal::Number(1)), Expr::Identifier(s)) => *s == counter,
                        _ => false,
                    }
                }
                _ => false,
            }
        }
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
    let is_rs = |e: &Expr| matches!(
        e,
        Expr::BinaryOp { op: BinaryOpKind::Multiply, left: r, right: s }
            if is_counter(r) && exprs_equal(s, stride)
    );
    if let Expr::BinaryOp { op: BinaryOpKind::Add, left, right } = index {
        if matches!(right, Expr::Literal(Literal::Number(1))) {
            if let Expr::BinaryOp { op: BinaryOpKind::Add, left: la, right: lb } = left {
                if (is_rs(la) && is_counter(lb)) || (is_counter(la) && is_rs(lb)) {
                    return Some(left);
                }
            }
        }
    }
    None
}

/// AOT BCE-hoist (tiled): for each array indexed by a row-major bilinear form
/// (`a[i*n+k]`, or `c[idx-1]` where `idx = i*n+j+1`) in the tile body, collect
/// the EMITTED index strings. Every such index is `R*S+C` with the tile
/// counters `R,C ∈ [0, bound)` and stride `S = bound`, so its maximum is
/// `(bound-1)*bound + (bound-1)`; a single preheader `assert!` of that max
/// (plus `bound >= 0`) lets every access be elided soundly. Returns one entry
/// per array: `(arr, [emitted index strings])`.
fn collect_tiled_bilinear_guards<'a>(
    body: &'a [Stmt<'a>],
    counters: &[Symbol],
    stride: &Expr,
    interner: &Interner,
    var_types: &HashMap<Symbol, String>,
) -> Vec<(Symbol, Vec<String>)> {
    let names = RustNames::new(interner);
    // Vars bound to a bilinear 1-based index (`Let idx be i*n+j+1`), mapped to
    // the RESOLVED emitted index `R*S+C` (`i*n+j`). Resolving avoids referencing
    // the binding (which is emitted INSIDE the body, after the hint) — the
    // counters are loop variables always in scope, and LLVM GVNs `idx-1` to it.
    let mut bilinear_vars: HashMap<Symbol, &Expr> = HashMap::new();
    for stmt in body {
        if let Stmt::Let { var, value, .. } = stmt {
            if let Some(a) = match_tiled_bilinear(value, counters, stride) {
                bilinear_vars.insert(*var, a);
            }
        }
    }
    let mut order: Vec<Symbol> = Vec::new();
    let mut per_arr: HashMap<Symbol, Vec<String>> = HashMap::new();
    let mut record = |arr: Symbol, emitted: String, order: &mut Vec<Symbol>, per_arr: &mut HashMap<Symbol, Vec<String>>| {
        per_arr.entry(arr).or_insert_with(|| { order.push(arr); Vec::new() }).push(emitted);
    };
    let mut accesses: Vec<(&Expr, &Expr)> = Vec::new();
    for stmt in body {
        collect_index_pairs_in_stmt(stmt, &mut accesses);
    }
    for (coll, index) in accesses {
        let Expr::Identifier(arr) = coll else { continue };
        // Only `Vec`/slice arrays (`.len()` is valid and stable). A LogosSeq
        // would need `.borrow()`; a resized array's length would go stale.
        let qualifies = var_types.get(arr).map_or(false, |t| {
            let t = t.split("|__hl:").next().unwrap_or(t.as_str());
            t.starts_with("Vec<") || t.starts_with("&[") || t.starts_with("&mut [")
        });
        if !qualifies || body_resizes_collection(body, *arr) {
            continue;
        }
        // Inline bilinear `item (R*S+C+1) of arr`.
        if let Some(a) = match_tiled_bilinear(index, counters, stride) {
            record(*arr, codegen_expr_simple(a, interner), &mut order, &mut per_arr);
        } else if let Expr::Identifier(v) = index {
            // `item idx of arr` where idx was bound to a bilinear index — use
            // the resolved `R*S+C` so the hint references in-scope counters.
            if let Some(a) = bilinear_vars.get(v) {
                record(*arr, codegen_expr_simple(a, interner), &mut order, &mut per_arr);
            }
        }
    }
    order.into_iter().map(|a| (a, per_arr.remove(&a).unwrap())).collect()
}

/// Collect `(collection, index)` pairs from every index access in a statement
/// (SetIndex and `item _ of _` reads anywhere in its expressions).
fn collect_index_pairs_in_stmt<'a>(stmt: &'a Stmt<'a>, out: &mut Vec<(&'a Expr<'a>, &'a Expr<'a>)>) {
    fn in_expr<'a>(e: &'a Expr<'a>, out: &mut Vec<(&'a Expr<'a>, &'a Expr<'a>)>) {
        match e {
            Expr::Index { collection, index } => { out.push((collection, index)); in_expr(collection, out); in_expr(index, out); }
            Expr::BinaryOp { left, right, .. } => { in_expr(left, out); in_expr(right, out); }
            Expr::Not { operand } => in_expr(operand, out),
            Expr::Call { args, .. } => for a in args.iter() { in_expr(a, out); },
            Expr::Length { collection } => in_expr(collection, out),
            _ => {}
        }
    }
    match stmt {
        Stmt::Let { value, .. } | Stmt::Set { value, .. } => in_expr(value, out),
        Stmt::SetIndex { collection, index, value } => { out.push((collection, index)); in_expr(index, out); in_expr(value, out); }
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
            Expr::BinaryOp { op: BinaryOpKind::Lt, left, right } => {
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
            Expr::BinaryOp { op: BinaryOpKind::Lt, left, right } => {
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
    let hoist_plan = super::hoist::plan_borrow_hoist(
        inner_loop_stmt, None, inner_body_sans_inc, ctx, interner,
    );
    super::hoist::emit_hoist_open(&hoist_plan, interner, &tile_indent, ctx, &mut out);

    // AOT BCE-hoist (tiled): row-major bilinear bounds guards. The tile
    // counters are all in `[0, bound)`, so every `R*S+C` index's maximum is
    // `(bound-1)*bound + (bound-1)`. One preheader `assert!` per array proves it
    // in range (PANICS, never UB, if the array is too short), after which the
    // per-FMA `assert_unchecked`s in the body let LLVM drop the checks.
    let bilinear_guards = collect_tiled_bilinear_guards(
        inner_body_sans_inc,
        &[outer_sym, mid_sym, inner_sym],
        outer_bound,
        interner,
        ctx.get_variable_types(),
    );
    for (arr, _) in &bilinear_guards {
        let arr_name = names.ident(*arr);
        writeln!(
            out,
            "{}    assert!(({n}) >= 0 && ((({n}) - 1) * ({n}) + (({n}) - 1)) < ({arr}.len() as i64), \"LOGOS bounds guard: indexing `{arr}` (tiled row-major) out of range\");",
            pad, n = bound_str, arr = arr_name
        ).unwrap();
    }

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
        for (arr, idxs) in &bilinear_guards {
            let arr_name = names.ident(*arr);
            for idx in idxs {
                writeln!(
                    out,
                    "{}unsafe {{ std::hint::assert_unchecked(({idx}) >= 0 && ({idx}) < ({arr}.len() as i64)); }}",
                    bi, idx = idx, arr = arr_name
                ).unwrap();
            }
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
    if symbol_appears_in_stmts(outer_sym, remaining) {
        let next_overwrites = remaining
            .first()
            .map_or(false, |s| matches!(s, Stmt::Set { target, .. } if *target == outer_sym));
        if next_overwrites {
            if is_new_binding {
                writeln!(out, "{}let mut {} = 0;", pad, outer_name).unwrap();
            }
        } else {
            writeln!(out, "{}let mut {} = {};", pad, outer_name, bound_str).unwrap();
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
        Stmt::Let { var, value, ty, mutable } => {
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
            let type_from_new = if let Expr::New { type_name, type_args, init_fields } = value {
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
                        Expr::Literal(Literal::Number(n)) => Some(format!("{}", n)),
                        Expr::Literal(Literal::Float(f)) => Some(format!("{:.1}", f)),
                        Expr::Literal(Literal::Boolean(b)) => Some(format!("{}", b)),
                        Expr::Literal(Literal::Char(c)) => Some(format!("'{}'", c)),
                        Expr::Literal(Literal::Text(s)) => {
                            Some(format!("String::from(\"{}\")", interner.resolve(*s)))
                        }
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
        Stmt::Let { var, value: Expr::Literal(Literal::Number(n)), .. } => {
            (*var, *n)
        }
        Stmt::Set { target, value: Expr::Literal(Literal::Number(n)) } => {
            (*target, *n)
        }
        _ => return None,
    };

    // While loop: counter <= limit (or counter < limit): Push const_val to vec_var. Set counter to counter + 1.
    match stmts[cursor + 1] {
        Stmt::While { cond, body, .. } => {
            // Check condition: counter <= limit OR counter < limit
            let (limit_expr, is_exclusive) = match cond {
                Expr::BinaryOp { op: BinaryOpKind::LtEq, left, right } => {
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
                Expr::BinaryOp { op: BinaryOpKind::Lt, left, right } => {
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
                Expr::Literal(Literal::Number(n)) => format!("{}", n),
                Expr::Literal(Literal::Float(f)) => format!("{:.1}", f),
                Expr::Literal(Literal::Boolean(b)) => format!("{}", b),
                Expr::Literal(Literal::Char(c)) => format!("'{}'", c),
                Expr::Literal(Literal::Text(s)) => {
                    format!("String::from(\"{}\")", interner.resolve(*s))
                }
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
                        Expr::BinaryOp { op: BinaryOpKind::Add, left, right } => {
                            let is_counter_plus_1 = match (left, right) {
                                (Expr::Identifier(s), Expr::Literal(Literal::Number(1))) if *s == counter_sym => true,
                                (Expr::Literal(Literal::Number(1)), Expr::Identifier(s)) if *s == counter_sym => true,
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

            // Total count = prefix elements + loop iterations
            let count_expr = if prefix_count == 0 {
                format!("{} as usize", raw_loop_count)
            } else {
                format!("({} + {}) as usize", prefix_count, raw_loop_count)
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
            if de_rc {
                writeln!(output, "{}let {}{}: Vec<{}> = vec![{}; {}];",
                    indent_str, mut_kw, vec_name, elem, fill_val_str, count_expr).unwrap();
                emit_narrow_guards(&mut output, vec_sym, ctx, &indent_str);
                ctx.register_variable_type(vec_sym, format!("Vec<{}>", elem));
            } else {
                writeln!(output, "{}let {}{}: LogosSeq<{}> = LogosSeq::from_vec(vec![{}; {}]);",
                    indent_str, mut_kw, vec_name, elem_type, fill_val_str, count_expr).unwrap();
                ctx.register_variable_type(vec_sym, format!("LogosSeq<{}>", elem_type));
            }
            let narrow_prefix = narrow;

            // Emit prefix element overrides (only for values different from fill)
            for (i, prefix_val) in prefix_values.iter().enumerate() {
                if *prefix_val != fill_val_str {
                    // A narrowed buffer truncates the override (lossless by proof).
                    let pv = if narrow_prefix { format!("({}) as i32", prefix_val) } else { prefix_val.clone() };
                    if de_rc {
                        writeln!(output, "{}{}[{}] = {};",
                            indent_str, vec_name, i, pv).unwrap();
                    } else {
                        writeln!(output, "{}{}.borrow_mut()[{}] = {};",
                            indent_str, vec_name, i, pv).unwrap();
                    }
                }
            }

            // Re-emit counter variable (it may be reused after the fill loop)
            let names = RustNames::new(interner);
            let counter_name = names.ident(counter_sym);
            if counter_is_new_binding {
                writeln!(output, "{}let mut {} = {};",
                    indent_str, counter_name, counter_start).unwrap();
            } else {
                writeln!(output, "{}{} = {};",
                    indent_str, counter_name, counter_start).unwrap();
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
        | Expr::Identifier(_) => true,
        Expr::BinaryOp { op, left, right } => {
            matches!(op,
                BinaryOpKind::Add | BinaryOpKind::Subtract |
                BinaryOpKind::Multiply | BinaryOpKind::Divide | BinaryOpKind::Modulo
            ) && is_simple_expr(left) && is_simple_expr(right)
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
        Expr::Literal(Literal::Float(f)) => format!("{:.1}", f),
        Expr::Literal(Literal::Boolean(b)) => format!("{}", b),
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

    // Statement 1: Let mutable text be "".
    let str_sym = match stmts[idx] {
        Stmt::Let { var, value, mutable, .. } => {
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
                    Expr::BinaryOp { op: BinaryOpKind::Lt, left, right } => {
                        if matches!(left, Expr::Identifier(sym) if *sym == counter_sym) {
                            (Some(*right), true)
                        } else {
                            (None, false)
                        }
                    }
                    Expr::BinaryOp { op: BinaryOpKind::LtEq, left, right } => {
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
                                writeln!(output, "{}let mut {} = String::with_capacity({} as usize);",
                                    indent_str, var_name, capacity_expr).unwrap();

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
                if let Expr::BinaryOp { op: BinaryOpKind::Add, left, .. } = value {
                    if matches!(left, Expr::Identifier(sym) if *sym == str_sym) {
                        return true;
                    }
                }
            }
            Stmt::If { then_block, else_block, .. } => {
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
/// where the caller adds it).
pub(crate) fn simplify_1based_index(expr: &Expr, interner: &Interner, include_as_usize: bool) -> String {
    let cast = if include_as_usize { " as usize" } else { "" };

    match expr {
        // Literal(1) → 0
        Expr::Literal(Literal::Number(1)) => "0".to_string(),
        // Literal(N) → N-1 (compile-time constant, no cast needed)
        Expr::Literal(Literal::Number(n)) => format!("{}", n - 1),
        // (X + K) where K is a literal
        Expr::BinaryOp { op: BinaryOpKind::Add, left, right } => {
            match (left, right) {
                // (X + 1) → X
                (_, Expr::Literal(Literal::Number(1))) => {
                    let inner = codegen_expr_simple(left, interner);
                    if include_as_usize {
                        format!("({}){}", inner, cast)
                    } else {
                        inner
                    }
                }
                // (1 + X) → X
                (Expr::Literal(Literal::Number(1)), _) => {
                    let inner = codegen_expr_simple(right, interner);
                    if include_as_usize {
                        format!("({}){}", inner, cast)
                    } else {
                        inner
                    }
                }
                // (X + K) where K > 1 → (X + K-1)
                (_, Expr::Literal(Literal::Number(k))) if *k > 1 => {
                    let inner = codegen_expr_simple(left, interner);
                    format!("({} + {}){}", inner, k - 1, cast)
                }
                // (K + X) where K > 1 → (X + K-1)
                (Expr::Literal(Literal::Number(k)), _) if *k > 1 => {
                    let inner = codegen_expr_simple(right, interner);
                    format!("({} + {}){}", inner, k - 1, cast)
                }
                // Fallback: (expr - 1)
                _ => {
                    let full = codegen_expr_simple(expr, interner);
                    format!("({} - 1){}", full, cast)
                }
            }
        }
        // Fallback: (expr - 1)
        _ => {
            let full = codegen_expr_simple(expr, interner);
            format!("({} - 1){}", full, cast)
        }
    }
}

/// Comparison strength of a bounds hint: `bound < len` vs `bound <= len`.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum HintStrength { Lt, Le }

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
        Expr::BinaryOp { op: BinaryOpKind::Add, left, right } => match (left, right) {
            (Expr::Identifier(s), Expr::Literal(Literal::Number(1))) if *s == counter_sym => Some(true),
            (Expr::Literal(Literal::Number(1)), Expr::Identifier(s)) if *s == counter_sym => Some(true),
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
        Expr::BinaryOp { op: BinaryOpKind::And | BinaryOpKind::Or, left, .. } => {
            classify_expr_accesses(left, counter_sym, order, kinds);
        }
        Expr::BinaryOp { left, right, .. } => {
            classify_expr_accesses(left, counter_sym, order, kinds);
            classify_expr_accesses(right, counter_sym, order, kinds);
        }
        Expr::Not { operand } => classify_expr_accesses(operand, counter_sym, order, kinds),
        Expr::Length { collection } => classify_expr_accesses(collection, counter_sym, order, kinds),
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
            Stmt::SetIndex { collection, index, value } => {
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
            Stmt::If { then_block, else_block, .. } => {
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
        plans.push(BoundsHintPlan { arr_sym, bound_str, strength });
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
        writeln!(output, "{}let __{}_bhl = {}.len() as i64;", indent_str, arr, arr).unwrap();
    }
}

/// Emit the in-loop `debug_assert!` + `assert_unchecked` pairs for planned
/// bounds hints. Placed at the top of the loop body so they never execute
/// for an empty loop; LLVM hoists the loop-invariant assume.
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
            "{}debug_assert!({} {} __{}_bhl, \"LOGOS bounds hint violated: loop indexes `{}` past its length\");",
            indent_str, p.bound_str, op, arr, arr
        )
        .unwrap();
        writeln!(
            output,
            "{}unsafe {{ std::hint::assert_unchecked({} {} __{}_bhl); }}",
            indent_str, p.bound_str, op, arr
        )
        .unwrap();
    }
}

/// Check if two expressions are structurally equal (for swap pattern detection, sentinel detection).
pub(crate) fn exprs_equal(a: &Expr, b: &Expr) -> bool {
    match (a, b) {
        (Expr::Identifier(s1), Expr::Identifier(s2)) => s1 == s2,
        (Expr::Literal(Literal::Number(n1)), Expr::Literal(Literal::Number(n2))) => n1 == n2,
        (Expr::BinaryOp { op: op1, left: l1, right: r1 }, Expr::BinaryOp { op: op2, left: l2, right: r2 }) => {
            op1 == op2 && exprs_equal(l1, l2) && exprs_equal(r1, r2)
        }
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
pub(crate) fn try_emit_byte_compare_window(
    stmt: &Stmt,
    interner: &Interner,
    indent: usize,
    variable_types: &HashMap<Symbol, String>,
    oracle: Option<&crate::optimize::OracleFacts>,
) -> Option<String> {
    let Stmt::While { cond, body, .. } = stmt else { return None };
    // Loop counter + bound: `j < LEN`.
    let Expr::BinaryOp { op: BinaryOpKind::Lt, left: j_e, right: len_e } = cond else { return None };
    let Expr::Identifier(j) = &**j_e else { return None };
    let j = *j;
    // Body is exactly `[If mismatch {…}, Set j to j + 1]`.
    if body.len() != 2 || !is_increment_by_one_of(&body[1], j) {
        return None;
    }
    let Stmt::If { cond: mism, then_block, else_block: None } = &body[0] else { return None };
    // Mismatch: `TEXT[a+j] != NEEDLE[b+j]` (LOGOS `is not` lowers to `!=`).
    let Expr::BinaryOp { op: BinaryOpKind::NotEq, left: t_e, right: n_e } = mism else { return None };
    let (text, t_coll, t_idx) = index_into(t_e)?;
    let (needle, n_coll, n_idx) = index_into(n_e)?;
    if !is_byte_indexable(variable_types.get(&text)) || !is_byte_indexable(variable_types.get(&needle)) {
        return None;
    }
    // Both indices must be `base + j` (unit stride in j) for a contiguous window.
    let base_t = affine_base_in_j(t_idx, j)?;
    let base_n = affine_base_in_j(n_idx, j)?;
    // The then-block sets the FLAG and breaks (`Set j to LEN`); order-independent.
    // A CONSTANT window length lets LLVM expand the comparison inline (a
    // fixed-size `bcmp` becomes loads + compares, like C's unrolled byte loop)
    // instead of emitting a runtime `bcmp` CALL per position — the string_search
    // gap. Resolve the length from the oracle when it is provably constant.
    let const_len: Option<i64> = oracle
        .and_then(|o| o.expr_int_range(len_e))
        .and_then(|(lo, hi)| (lo == hi && lo > 0 && lo <= 16).then_some(lo));
    let len_str = match const_len {
        Some(k) => k.to_string(),
        None => codegen_expr_simple(len_e, interner),
    };
    let mut flag: Option<(Symbol, &Expr)> = None;
    let mut saw_break = false;
    for s in then_block.iter() {
        match s {
            // The break — either a real `Break` (the optimizer rewrites `Set j to
            // LEN` to this) or the raw `Set j to LEN` (so `j < LEN` goes false).
            Stmt::Break => saw_break = true,
            Stmt::Set { target, value } if *target == j => {
                if codegen_expr_simple(value, interner) != len_str {
                    return None;
                }
                saw_break = true;
            }
            Stmt::Set { target, value } if is_simple_expr(value) => {
                if flag.replace((*target, value)).is_some() {
                    return None; // only one flag write
                }
            }
            _ => return None,
        }
    }
    let (flag_sym, flag_val) = flag?;
    if !saw_break {
        return None;
    }
    let names = RustNames::new(interner);
    let text_n = names.ident(text);
    let needle_n = names.ident(needle);
    let flag_n = names.ident(flag_sym);
    let bt = codegen_expr_simple(base_t, interner);
    let bn = codegen_expr_simple(base_n, interner);
    let fv = codegen_expr_simple(flag_val, interner);
    let ind = "    ".repeat(indent);
    let ind1 = "    ".repeat(indent + 1);
    // Byte index = `item` index - 1 (1-based to 0-based). Window length = LEN.
    let mut out = String::new();
    // Carry the window's bounds proof forward: when the oracle proved the
    // PER-BYTE accesses in range (the BCE it would have hinted on the scalar
    // loop), the whole window `[base-1 .. base-1+LEN)` is in bounds, so the
    // slice indexing is too — `assert_unchecked` it so LLVM elides the slice
    // bound check (and the access stays as cheap as the elided byte loop was).
    let proven = |o: &crate::optimize::OracleFacts, coll, idx| o.index_provably_in_bounds(coll, idx);
    if oracle.map_or(false, |o| proven(o, t_coll, t_idx)) {
        writeln!(
            out,
            "{ind}unsafe {{ std::hint::assert_unchecked((({bt}) - 1) >= 0 && ((({bt}) - 1) + ({len_str})) <= ({text_n}.len() as i64)); }}"
        )
        .unwrap();
    }
    if oracle.map_or(false, |o| proven(o, n_coll, n_idx)) {
        writeln!(
            out,
            "{ind}unsafe {{ std::hint::assert_unchecked((({bn}) - 1) >= 0 && ((({bn}) - 1) + ({len_str})) <= ({needle_n}.len() as i64)); }}"
        )
        .unwrap();
    }
    if let Some(k) = const_len {
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
    needle_len: usize,
}

/// Most recent constant bound to `e` before `before`: a literal, or an
/// identifier whose last `Let`/`Set` before the loop is a number literal.
fn resolve_const_before(e: &Expr, stmts: &[&Stmt], before: usize) -> Option<i64> {
    if let Expr::Literal(Literal::Number(n)) = e {
        return Some(*n);
    }
    let Expr::Identifier(sym) = e else { return None };
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
fn needle_literal_len(needle: Symbol, stmts: &[&Stmt], before: usize, interner: &Interner) -> Option<usize> {
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
    let Stmt::Let { var: count, value: c0, .. } = stmts[idx] else { return None };
    if !matches!(c0, Expr::Literal(Literal::Number(0))) {
        return None;
    }
    let count = *count;
    // s1: `Let i be <literal>`.
    let Stmt::Let { var: i, value: iv, .. } = stmts[idx + 1] else { return None };
    let Expr::Literal(Literal::Number(i_init)) = iv else { return None };
    let (i, i_init) = (*i, *i_init);
    // s2: `While i </<= HI { body }`.
    let Stmt::While { cond, body, .. } = stmts[idx + 2] else { return None };
    let (inclusive, hi) = match cond {
        Expr::BinaryOp { op: BinaryOpKind::LtEq, left, right } if matches!(left, Expr::Identifier(s) if *s == i) => (true, *right),
        Expr::BinaryOp { op: BinaryOpKind::Lt, left, right } if matches!(left, Expr::Identifier(s) if *s == i) => (false, *right),
        _ => return None,
    };
    if body.len() != 5 {
        return None;
    }
    // body[0]: `Let match be 1`.
    let Stmt::Let { var: match_var, value: m1, .. } = &body[0] else { return None };
    if !matches!(m1, Expr::Literal(Literal::Number(1))) {
        return None;
    }
    let match_var = *match_var;
    // body[1]: `Let j be 0`.
    let Stmt::Let { var: j, value: j0, .. } = &body[1] else { return None };
    if !matches!(j0, Expr::Literal(Literal::Number(0))) {
        return None;
    }
    let j = *j;
    // body[2]: the inner fixed-window byte compare (bcmp shape).
    let Stmt::While { cond: inner, body: inner_body, .. } = &body[2] else { return None };
    let Expr::BinaryOp { op: BinaryOpKind::Lt, left: j_e, right: m_expr } = inner else { return None };
    if !matches!(j_e, Expr::Identifier(s) if *s == j) {
        return None;
    }
    if inner_body.len() != 2 || !is_increment_by_one_of(&inner_body[1], j) {
        return None;
    }
    let Stmt::If { cond: mism, then_block, else_block: None } = &inner_body[0] else { return None };
    let Expr::BinaryOp { op: BinaryOpKind::NotEq, left: t_e, right: n_e } = mism else { return None };
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
    let Stmt::If { cond: mc, then_block: inc, else_block: None } = &body[3] else { return None };
    match mc {
        Expr::BinaryOp { op: BinaryOpKind::Eq, left, right }
            if matches!(left, Expr::Identifier(s) if *s == match_var)
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
        needle_len,
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
    if symbol_appears_in_stmts(m.i, &stmts[idx + 3..]) {
        return None;
    }
    let names = RustNames::new(interner);
    let text_n = names.ident(m.text);
    let needle_n = names.ident(m.needle);
    let count_n = names.ident(m.count);
    // 0-based start positions: `p = i - 1` for `i` in [i_init, HI] (inclusive)
    // or [i_init, HI) (exclusive). Half-open `[start, end)`:
    let start = m.i_init - 1;
    let end_src = if m.inclusive { m.hi_src.clone() } else { format!("({}) - 1", m.hi_src) };
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
        Stmt::If { then_block, else_block, .. } => {
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
    // Mark Peephole as fired iff some pattern in the dispatch chain below actually
    // emitted (the `Some` is the proof). Dedicated peephole opts (Simd, Cascade,
    // IndexString, CapScale) additionally mark themselves at their own emission.
    let __peephole_result = (|| -> Option<(String, usize)> {
    if let r @ Some(_) = try_emit_naive_search(stmts, idx, interner, indent, ctx) {
        return r;
    }
    if let r @ Some(_) = try_emit_affine_cascade(stmts, idx, interner, indent, ctx) {
        return r;
    }
    if let r @ Some(_) = try_emit_seq_from_slice_pattern(stmts, idx, interner, indent, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry, type_env) {
        return r;
    }
    if let r @ Some(_) = try_emit_vec_fill_pattern(stmts, idx, interner, indent, ctx) {
        return r;
    }
    if let r @ Some(_) = try_emit_bare_slice_push_pattern(stmts, idx, interner, indent, ctx.get_variable_types()) {
        return r;
    }
    if let r @ Some(_) = try_emit_vec_with_capacity_pattern(stmts, idx, interner, indent, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry, type_env) {
        return r;
    }
    if let r @ Some(_) = try_emit_merge_capacity_pattern(stmts, idx, interner, indent, ctx) {
        return r;
    }
    if let r @ Some(_) = try_emit_indexed_string_build(stmts, idx, interner, indent, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry, type_env) {
        return r;
    }
    if let r @ Some(_) = try_emit_string_with_capacity_pattern(stmts, idx, interner, indent, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry, type_env) {
        return r;
    }
    if let r @ Some(_) = try_emit_buffer_reuse_while(stmts, idx, interner, indent, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry, type_env) {
        return r;
    }
    if let r @ Some(_) = try_emit_for_range_pattern(stmts, idx, interner, indent, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry, type_env) {
        return r;
    }
    if let r @ Some(_) = try_emit_prefix_reverse(stmts, idx, interner, indent, ctx.get_variable_types()) {
        return r;
    }
    if let r @ Some(_) = try_emit_swap_pattern(stmts, idx, interner, indent, ctx.get_variable_types(), ctx.oracle()) {
        return r;
    }
    if let r @ Some(_) = try_emit_seq_copy_pattern(stmts, idx, interner, indent, ctx) {
        return r;
    }
    if let r @ Some(_) = try_emit_rotate_left_pattern(stmts, idx, interner, indent, ctx.get_variable_types()) {
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
    let Stmt::Let { var: v, value: v0, mutable, .. } = stmts[idx] else { return None };
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
        let Stmt::If { cond, then_block, else_block: None } = stmts[idx + 1 + n] else { break };
        let Expr::BinaryOp { op: BinaryOpKind::Eq, left: e, right: k } = cond else { break };
        let Expr::Literal(Literal::Number(k)) = k else { break };
        if then_block.len() != 1 {
            break;
        }
        let Stmt::Set { target, value } = &then_block[0] else { break };
        if *target != v {
            break;
        }
        let Some(c) = byte_const_of(value, interner) else { break };
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
    let code = format!("{}let {}{}: u8 = ({}) as u8;\n", "    ".repeat(indent), mut_kw, vn, rhs);
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
    let Stmt::Set { target, value } = s else { return None };
    if *target != text {
        return None;
    }
    let Expr::BinaryOp { op: BinaryOpKind::Add, left, right } = value else { return None };
    if !matches!(left, Expr::Identifier(s) if *s == text) {
        return None;
    }
    append_operand_byte_len(right, variable_types, interner)
}

/// `Set c to c + K` for the given constant `K`.
fn is_increment_by_const(s: &Stmt, c: Symbol, k: i64) -> bool {
    let Stmt::Set { target, value } = s else { return false };
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
            Stmt::If { cond, then_block, else_block } => {
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
                if expr_mentions_sym(cond, text) || !isb_check_block(body, text, c, vt, interner, max_k) {
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
            Stmt::SetIndex { collection, index, value } => {
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
    let Stmt::Let { var: text, value: tv, mutable, .. } = stmts[idx] else { return None };
    let text = *text;
    if !*mutable && !mutable_vars.contains(&text) {
        return None;
    }
    match tv {
        Expr::Literal(Literal::Text(s)) if interner.resolve(*s).is_empty() => {}
        _ => return None,
    }
    // s1: `Let c be 0` (cursor starts at the empty-content position).
    let Stmt::Let { var: c, value: cv, .. } = stmts[idx + 1] else { return None };
    let c = *c;
    if !matches!(cv, Expr::Literal(Literal::Number(0))) {
        return None;
    }
    // s2: `While c </<= limit { body }`.
    let while_stmt = stmts[idx + 2];
    let Stmt::While { cond, body, .. } = while_stmt else { return None };
    let limit = match cond {
        Expr::BinaryOp { op: BinaryOpKind::Lt | BinaryOpKind::LtEq, left, right }
            if matches!(left, Expr::Identifier(s) if *s == c) =>
        {
            *right
        }
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
    let cap = format!("(({}) + {})", codegen_expr_simple(limit, interner), max_k + 1);
    let ind = "    ".repeat(indent);
    let mut out = String::new();
    // Reserve the buffer but DON'T zero it (C's `malloc` doesn't): the appends
    // write into the spare capacity via raw pointer, then `set_len` exposes only
    // the written prefix. This is the standard manual-Vec-init pattern — sound
    // because the cursor proof writes every byte of `[0, c)` before `set_len(c)`,
    // and `text` is never read until then.
    writeln!(out, "{ind}let mut {text_n} = String::with_capacity(({cap}) as usize);").unwrap();
    writeln!(out, "{ind}let mut {c_n} = 0i64;").unwrap();
    ctx.register_string_var(text);
    ctx.register_indexed_string_build(text, c_n.clone());
    out.push_str(&super::codegen_stmt(
        while_stmt, interner, indent, mutable_vars, ctx, lww_fields, mv_fields, synced_vars,
        var_caps, async_functions, pipe_vars, boxed_fields, registry, type_env,
    ));
    writeln!(out, "{ind}unsafe {{ {text_n}.as_mut_vec().set_len(({c_n}) as usize); }}").unwrap();
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
                let Expr::Identifier(counter) = &**left else { continue };
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
        Stmt::Set { value, .. } | Stmt::Let { value, .. } => presize_reads_in_expr(value, counter, out),
        Stmt::Show { object, .. } => presize_reads_in_expr(object, counter, out),
        Stmt::SetIndex { index, value, .. } => {
            presize_reads_in_expr(index, counter, out);
            presize_reads_in_expr(value, counter, out);
        }
        Stmt::RuntimeAssert { condition, .. } => presize_reads_in_expr(condition, counter, out),
        Stmt::Return { value: Some(v) } => presize_reads_in_expr(v, counter, out),
        Stmt::If { cond, then_block, else_block } => {
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
    let Expr::BinaryOp { op: BinaryOpKind::Add, left, right } = idx else { return None };
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
    let Stmt::Set { target, value } = s else { return false };
    if *target != c {
        return false;
    }
    let Expr::BinaryOp { op: BinaryOpKind::Add, left, right } = value else { return false };
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
/// →  `C[(LO - 1) as usize..HI as usize].reverse();`
///
/// 1-based positions `[LO, HI]` of `C` are reversed, exactly as the pairwise
/// swap does. Two payoffs over the scalar loop: (1) one slice-bounds check
/// instead of N per-element checks; (2) the per-element panic branches that
/// block LLVM auto-vectorization are gone, so `<[T]>::reverse` emits the same
/// vectorized reverse a C compiler produces for this loop.
///
/// Sound because: the slice `C[(LO-1)..HI]` reverses precisely the elements the
/// swap loop touches; and `LO`/`HI`/`TMP` must not be read after the loop (they
/// are left at their entry values, which is then unobservable). General — it
/// fires on any in-place reversal, not just fannkuch's flip.
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
        Expr::BinaryOp { op: BinaryOpKind::Lt, left: Expr::Identifier(l), right: Expr::Identifier(h) } => (*l, *h),
        _ => return None,
    };
    if lo == hi || body.len() != 5 {
        return None;
    }
    // body[0]: Let TMP be item LO of C.
    let (tmp, coll) = match &body[0] {
        Stmt::Let { var, value: Expr::Index { collection: Expr::Identifier(c), index: Expr::Identifier(li) }, mutable: false, .. }
            if *li == lo => (*var, *c),
        _ => return None,
    };
    // body[1]: Set item LO of C to item HI of C.
    match &body[1] {
        Stmt::SetIndex {
            collection: Expr::Identifier(c),
            index: Expr::Identifier(li),
            value: Expr::Index { collection: Expr::Identifier(c2), index: Expr::Identifier(hj) },
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
    let ty = variable_types.get(&coll).map(|t| t.split("|__hl:").next().unwrap_or(t).to_string());
    let slice_owner = match ty.as_deref() {
        Some(t) if t.starts_with("LogosSeq") => format!("{}.borrow_mut()", coll_name),
        Some(t) if t.starts_with("Vec") || t.starts_with("&mut [") || t.starts_with('[') => coll_name.to_string(),
        _ => return None,
    };
    let indent_str = "    ".repeat(indent);
    let code = format!(
        "{indent_str}{slice_owner}[({lo_name} - 1) as usize..{hi_name} as usize].reverse();\n"
    );
    Some((code, 0))
}

/// `.swap()` is bounds-checked, so rewriting an indexed swap to it would
/// REINTRODUCE the checks the oracle already eliminated. When both swap indices
/// are RELATIONALLY proven in bounds (e.g. quicksort's partition `i`/`j`, where
/// the entry-guard proves `i <= j < hi <= len`), the manual `get_unchecked`
/// indexed form is strictly faster — so the swap peephole must defer to it. The
/// relational gate is deliberate: literal-index swaps (interval-proven) still
/// lower to `.swap()`, which is the win there.
fn swap_unchecked_proven(
    oracle: Option<&crate::optimize::OracleFacts>,
    i: &Expr,
    j: &Expr,
) -> bool {
    oracle.map_or(false, |o| o.index_proven_relational(i) && o.index_proven_relational(j))
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
    let (a_sym, arr_sym_1, idx_expr_1) = match stmts[idx] {
        Stmt::Let { var, value: Expr::Index { collection, index }, mutable: false, .. } => {
            if let Expr::Identifier(coll_sym) = collection {
                (*var, *coll_sym, *index)
            } else {
                return None;
            }
        }
        _ => return None,
    };

    // Only optimize for known Vec / &mut [T] / fixed-array (O3) types — all
    // support direct indexing and `.swap()`.
    if let Some(t) = variable_types.get(&arr_sym_1) {
        if !t.starts_with("LogosSeq") && !t.starts_with("Vec") && !t.starts_with("&mut [") && !t.starts_with('[') {
            return None;
        }
    } else {
        return None;
    }

    // Try Pattern B first: unconditional 3-statement swap (more general)
    // Statement 2: Set item I of arr to item J of arr.
    // Statement 3: Set item J of arr to tmp.
    if let Some(result) = try_emit_unconditional_swap(stmts, idx, a_sym, arr_sym_1, idx_expr_1, interner, indent, variable_types, oracle) {
        return Some(result);
    }

    // Pattern A: conditional swap with any two indices from the same array.
    // Statement 2: Let b be item J of arr (J can be any index, not just I+1)
    let (b_sym, arr_sym_2, idx_expr_2) = match stmts[idx + 1] {
        Stmt::Let { var, value: Expr::Index { collection, index }, mutable: false, .. } => {
            if let Expr::Identifier(coll_sym) = collection {
                (*var, *coll_sym, *index)
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
        Stmt::If { cond, then_block, else_block } => {
            // Condition must compare a and b
            let compares_a_b = match cond {
                Expr::BinaryOp { op, left, right } => {
                    matches!(op, BinaryOpKind::Gt | BinaryOpKind::Lt | BinaryOpKind::GtEq | BinaryOpKind::LtEq | BinaryOpKind::Eq | BinaryOpKind::NotEq) &&
                    ((matches!(left, Expr::Identifier(s) if *s == a_sym) && matches!(right, Expr::Identifier(s) if *s == b_sym)) ||
                     (matches!(left, Expr::Identifier(s) if *s == b_sym) && matches!(right, Expr::Identifier(s) if *s == a_sym)))
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
                    Stmt::SetIndex { collection: c1, index: i1, value: v1 },
                    Stmt::SetIndex { collection: c2, index: i2, value: v2 },
                ) => {
                    // c1 and c2 must be the same array
                    let same_arr = matches!((c1, c2), (Expr::Identifier(s1), Expr::Identifier(s2)) if *s1 == arr_sym_1 && *s2 == arr_sym_1);
                    // Cross pattern: set idx1 to b, set idx2 to a
                    let cross = exprs_equal(i1, idx_expr_1) && exprs_equal(i2, idx_expr_2) &&
                        matches!(v1, Expr::Identifier(s) if *s == b_sym) &&
                        matches!(v2, Expr::Identifier(s) if *s == a_sym);
                    // Also check reverse: set idx1 to b via idx2/a pattern
                    let cross_rev = exprs_equal(i1, idx_expr_2) && exprs_equal(i2, idx_expr_1) &&
                        matches!(v1, Expr::Identifier(s) if *s == a_sym) &&
                        matches!(v2, Expr::Identifier(s) if *s == b_sym);
                    same_arr && (cross || cross_rev)
                }
                _ => false,
            };

            if !swap_ok {
                return None;
            }

            // Pattern matched! Emit optimized swap
            let indent_str = "    ".repeat(indent);
            let arr_name = interner.resolve(arr_sym_1);
            let idx1_simplified = simplify_1based_index(idx_expr_1, interner, true);
            let idx2_simplified = simplify_1based_index(idx_expr_2, interner, true);

            // The source guard may be written `a OP b` OR `b OP a`; emit the
            // comparison with operands in SOURCE order so the swap fires on
            // exactly the condition the program wrote. (The swap body itself is
            // symmetric in the two indices, so only the guard needs ordering.)
            let guard_left_is_a = matches!(
                cond,
                Expr::BinaryOp { left, .. }
                    if matches!(left, Expr::Identifier(s) if *s == a_sym)
            );
            let (g_lhs, g_rhs) = if guard_left_is_a {
                (idx1_simplified.as_str(), idx2_simplified.as_str())
            } else {
                (idx2_simplified.as_str(), idx1_simplified.as_str())
            };

            let op_str = match cond {
                Expr::BinaryOp { op, .. } => match op {
                    BinaryOpKind::Gt => ">", BinaryOpKind::Lt => "<",
                    BinaryOpKind::GtEq => ">=", BinaryOpKind::LtEq => "<=",
                    BinaryOpKind::Eq => "==", BinaryOpKind::NotEq => "!=",
                    _ => unreachable!(),
                },
                _ => unreachable!(),
            };

            let is_logos_seq = variable_types.get(&arr_sym_1)
                .map_or(false, |t| t.starts_with("LogosSeq"));

            let mut output = String::new();

            // This peephole consumes the `Let a = item I` / `Let b = item J`
            // bindings. If either captured value is read AFTER this block, re-bind
            // it to the PRE-swap element value first (the swap below mutates the
            // array in place), so the residual program still references a defined
            // local — mirroring the liveness guard in the sibling optimizers.
            let remaining = &stmts[idx + 3..];
            let elem_access = |idx_s: &str| -> String {
                if is_logos_seq {
                    format!("{}.borrow()[{}]", arr_name, idx_s)
                } else {
                    format!("{}[{}]", arr_name, idx_s)
                }
            };
            if symbol_appears_in_stmts(a_sym, remaining) {
                writeln!(output, "{}let {} = {};",
                    indent_str, interner.resolve(a_sym), elem_access(&idx1_simplified)).unwrap();
            }
            if symbol_appears_in_stmts(b_sym, remaining) {
                writeln!(output, "{}let {} = {};",
                    indent_str, interner.resolve(b_sym), elem_access(&idx2_simplified)).unwrap();
            }

            if is_logos_seq {
                writeln!(output, "{}{{ let mut __bm = {}.borrow_mut();", indent_str, arr_name).unwrap();
                writeln!(output, "{}if __bm[{}] {} __bm[{}] {{",
                    indent_str, g_lhs, op_str, g_rhs,
                ).unwrap();
                writeln!(output, "{}    let __swap_tmp = __bm[{}];",
                    indent_str, idx1_simplified).unwrap();
                writeln!(output, "{}    __bm[{}] = __bm[{}];",
                    indent_str, idx1_simplified, idx2_simplified).unwrap();
                writeln!(output, "{}    __bm[{}] = __swap_tmp;",
                    indent_str, idx2_simplified).unwrap();
                writeln!(output, "{}}}", indent_str).unwrap();
                writeln!(output, "{}}}", indent_str).unwrap();
            } else {
                writeln!(output, "{}if {}[{}] {} {}[{}] {{",
                    indent_str, arr_name, g_lhs, op_str, arr_name, g_rhs,
                ).unwrap();
                writeln!(output, "{}    let __swap_tmp = {}[{}];",
                    indent_str, arr_name, idx1_simplified).unwrap();
                writeln!(output, "{}    {}[{}] = {}[{}];",
                    indent_str, arr_name, idx1_simplified, arr_name, idx2_simplified).unwrap();
                writeln!(output, "{}    {}[{}] = __swap_tmp;",
                    indent_str, arr_name, idx2_simplified).unwrap();
                writeln!(output, "{}}}", indent_str).unwrap();
            }

            Some((output, 2)) // consumed 2 extra statements
        }
        _ => None,
    }
}

/// Peephole optimization: detect full-array copy via push loop and emit `.to_vec()`.
///
/// Pattern:
///   Let [mutable] dst be a new Seq of T.
///   Set counter to 1.                           (counter already declared)
///   While counter <= length of src:
///       Push item counter of src to dst.
///       Set counter to counter + 1.
/// → `let mut dst: Vec<T> = src.to_vec();`
///
/// If the counter appears in subsequent statements, the post-loop value
/// `src.len() as i64 + 1` is emitted so callers can rely on it.
///
/// Returns (generated_code, number_of_extra_statements_consumed) or None.
pub(crate) fn try_emit_seq_copy_pattern<'a>(
    stmts: &[&Stmt<'a>],
    idx: usize,
    interner: &Interner,
    indent: usize,
    ctx: &mut RefinementContext<'a>,
) -> Option<(String, usize)> {
    if idx + 2 >= stmts.len() {
        return None;
    }

    // Statement 1: Let mutable dst be a new Seq of T.
    let (dst_sym, elem_type) = match stmts[idx] {
        Stmt::Let { var, value, mutable: true, .. } => {
            if let Expr::New { type_name, type_args, init_fields } = value {
                let tn = interner.resolve(*type_name);
                if matches!(tn, "Seq" | "List" | "Vec") && init_fields.is_empty() && !type_args.is_empty() {
                    (*var, codegen_type_expr(&type_args[0], interner))
                } else {
                    return None;
                }
            } else {
                return None;
            }
        }
        _ => return None,
    };

    // Statement 2: Set counter to 1  (counter must already be declared — this is a reset)
    let counter_sym = match stmts[idx + 1] {
        Stmt::Set { target, value: Expr::Literal(Literal::Number(1)) } => *target,
        _ => return None,
    };

    // Statement 3: While counter <= length of src: Push item counter of src to dst; Set counter++
    match stmts[idx + 2] {
        Stmt::While { cond, body, .. } => {
            // Condition: counter <= length of src
            let src_sym = match cond {
                Expr::BinaryOp { op: BinaryOpKind::LtEq, left, right } => {
                    if let Expr::Identifier(c) = left {
                        if *c == counter_sym {
                            if let Expr::Length { collection } = right {
                                if let Expr::Identifier(s) = collection {
                                    *s
                                } else {
                                    return None;
                                }
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

            // Body must have exactly 2 statements
            if body.len() != 2 {
                return None;
            }

            // Body[0]: Push item counter of src to dst
            match &body[0] {
                Stmt::Push { value, collection } => {
                    if !matches!(collection, Expr::Identifier(s) if *s == dst_sym) {
                        return None;
                    }
                    if let Expr::Index { collection: idx_coll, index: idx_expr } = value {
                        if !matches!(idx_coll, Expr::Identifier(s) if *s == src_sym) {
                            return None;
                        }
                        if !matches!(idx_expr, Expr::Identifier(s) if *s == counter_sym) {
                            return None;
                        }
                    } else {
                        return None;
                    }
                }
                _ => return None,
            }

            // Body[1]: Set counter to counter + 1
            match &body[1] {
                Stmt::Set { target, value } => {
                    if *target != counter_sym {
                        return None;
                    }
                    match value {
                        Expr::BinaryOp { op: BinaryOpKind::Add, left, right } => {
                            let ok = match (left, right) {
                                (Expr::Identifier(s), Expr::Literal(Literal::Number(1))) => *s == counter_sym,
                                (Expr::Literal(Literal::Number(1)), Expr::Identifier(s)) => *s == counter_sym,
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

            // Pattern matched! Emit: let mut dst: Vec<T> = src.to_vec();
            let indent_str = "    ".repeat(indent);
            let dst_name = interner.resolve(dst_sym);
            let src_name = interner.resolve(src_sym);
            let names = RustNames::new(interner);
            let counter_name = names.ident(counter_sym);

            let mut output = String::new();
            writeln!(output, "{}let {}: LogosSeq<{}> = {}.deep_clone();",
                indent_str, dst_name, elem_type, src_name).unwrap();
            ctx.register_variable_type(dst_sym, format!("LogosSeq<{}>", elem_type));

            // If the counter appears after the loop, emit its post-loop value.
            let remaining = &stmts[idx + 3..];
            if symbol_appears_in_stmts(counter_sym, remaining) {
                writeln!(output, "{}{} = {}.len() as i64 + 1;",
                    indent_str, counter_name, src_name).unwrap();
            }

            Some((output, 2)) // consumed: counter-reset + While = 2 extra
        }
        _ => None,
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
        Stmt::Let { var, value, ty, mutable: true, .. } => {
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

            let type_from_new = if let Expr::New { type_name, type_args, init_fields } = value {
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
                    Expr::BinaryOp { op: BinaryOpKind::LtEq | BinaryOpKind::Lt, left, .. } => {
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
                        if let Some((c_src_sym, c_dst_check)) = extract_push_index_body(body, c_sym) {
                            if c_dst_check == dst_sym {
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
                                let src_is_logos_seq = ctx.get_variable_types().get(&c_src_sym)
                                    .map(|t| t.split("|__hl:").next().unwrap_or(t.as_str()))
                                    .map(|t| t.starts_with("LogosSeq"))
                                    .unwrap_or(true); // default to LogosSeq
                                let borrow_prefix = if src_is_logos_seq { format!("{}.borrow()", src_name) } else { src_name.to_string() };

                                // De-Rc-aware: a de-Rc'd target is a plain owned
                                // `Vec<T>` (the `.to_vec()` already yields one), no
                                // `LogosSeq::from_vec` wrapper. Mutable because the
                                // build target is typically reassigned later.
                                let dst_de_rc = ctx.is_de_rc(dst_sym);
                                let end_slice = if c_is_exclusive {
                                    format!("({} - 1) as usize", end_str)
                                } else {
                                    format!("{} as usize", end_str)
                                };
                                if dst_de_rc {
                                    writeln!(cont_output, "{}let mut {}: Vec<{}> = {}[({} - 1) as usize..{}].to_vec();",
                                        indent_str, dst_name, elem_type, borrow_prefix, counter_name, end_slice).unwrap();
                                    ctx.register_variable_type(dst_sym, format!("Vec<{}>", elem_type));
                                } else {
                                    writeln!(cont_output, "{}let {}: LogosSeq<{}> = LogosSeq::from_vec({}[({} - 1) as usize..{}].to_vec());",
                                        indent_str, dst_name, elem_type, borrow_prefix, counter_name, end_slice).unwrap();
                                    ctx.register_variable_type(dst_sym, format!("LogosSeq<{}>", elem_type));
                                }

                                // Emit intervening statements between Seq creation and the While
                                for si in (idx + 1)..scan {
                                    use super::codegen_stmt;
                                    cont_output.push_str(&codegen_stmt(stmts[si], interner, indent, mutable_vars, ctx,
                                        lww_fields, mv_fields, synced_vars, var_caps, async_functions,
                                        pipe_vars, boxed_fields, registry, type_env));
                                }

                                // Re-emit counter post-loop value if used after the While
                                let remaining = &stmts[scan + 1..];
                                if symbol_appears_in_stmts(c_sym, remaining) {
                                    let post_val = if c_is_exclusive {
                                        end_str.to_string()
                                    } else {
                                        if let Expr::Literal(Literal::Number(n)) = c_end_expr {
                                            format!("{}", n + 1)
                                        } else {
                                            format!("{} + 1", end_str)
                                        }
                                    };
                                    writeln!(cont_output, "{}{} = {};", indent_str, counter_name, post_val).unwrap();
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
                Expr::BinaryOp { op: BinaryOpKind::LtEq, left, right } => {
                    if let Expr::Identifier(c) = left {
                        if *c == counter_sym { (Some(*right), false) } else { (None, false) }
                    } else { (None, false) }
                }
                Expr::BinaryOp { op: BinaryOpKind::Lt, left, right } => {
                    if let Expr::Identifier(c) = left {
                        if *c == counter_sym { (Some(*right), true) } else { (None, false) }
                    } else { (None, false) }
                }
                _ => (None, false),
            };
            let end_expr = end_expr?;

            if body.len() != 2 {
                return None;
            }

            let src_sym = match &body[0] {
                Stmt::Push { value, collection } => {
                    if !matches!(collection, Expr::Identifier(s) if *s == dst_sym) {
                        return None;
                    }
                    if let Expr::Index { collection: idx_coll, index: idx_expr } = value {
                        if !matches!(idx_expr, Expr::Identifier(s) if *s == counter_sym) {
                            return None;
                        }
                        if let Expr::Identifier(s) = idx_coll {
                            *s
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
                    if *target != counter_sym { return None; }
                    match value {
                        Expr::BinaryOp { op: BinaryOpKind::Add, left, right } => {
                            let ok = match (left, right) {
                                (Expr::Identifier(s), Expr::Literal(Literal::Number(1))) => *s == counter_sym,
                                (Expr::Literal(Literal::Number(1)), Expr::Identifier(s)) => *s == counter_sym,
                                _ => false,
                            };
                            if !ok { return None; }
                        }
                        _ => return None,
                    }
                }
                _ => return None,
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

            // Check source type: &[T] and Vec<T> don't need .borrow()
            let src_type = ctx.get_variable_types().get(&src_sym).cloned().unwrap_or_default();
            let needs_borrow = src_type.starts_with("LogosSeq");
            let borrow_prefix = if needs_borrow { ".borrow()" } else { "" };

            // The owned-`Vec` initializer for this copy/slice shape. A de-Rc'd
            // target IS that owned Vec (no `LogosSeq::from_vec` wrapper); a
            // LogosSeq target wraps it. Computed once so both emit consistently.
            let vec_init = if is_start_one && is_end_length_of_src {
                format!("{}{}.to_vec()", src_name, borrow_prefix)
            } else {
                let start_str = codegen_expr_simple(start_expr, interner);
                let end_str = codegen_expr_simple(end_expr, interner);
                let hi = if is_exclusive {
                    format!("({} - 1) as usize", end_str)
                } else {
                    format!("{} as usize", end_str)
                };
                let range = if matches!(start_expr, Expr::Literal(Literal::Number(1))) {
                    format!("[..{}]", hi)
                } else {
                    format!("[({} - 1) as usize..{}]", start_str, hi)
                };
                format!("{}{}{}.to_vec()", src_name, borrow_prefix, range)
            };
            if ctx.is_de_rc(dst_sym) {
                if ctx.is_scratch_hoist(dst_sym) {
                    // The buffer's allocation was hoisted before the enclosing
                    // `while` (see the `Stmt::While` handler). `vec_init` is a
                    // full copy `<slice>.to_vec()`; reuse the buffer in place:
                    // `clear()` keeps capacity, `extend_from_slice` refills it —
                    // value-identical to the fresh `.to_vec()`, zero allocation.
                    let slice = vec_init.strip_suffix(".to_vec()").unwrap_or(vec_init.as_str());
                    // `&<slice>` must be a `&[T]`: an indexed slice (`src[..hi]`)
                    // already is; a whole container (`src` / `src.borrow()`) needs
                    // `[..]` to coerce.
                    let slice_ref = if slice.ends_with(']') {
                        format!("&{}", slice)
                    } else {
                        format!("&{}[..]", slice)
                    };
                    writeln!(output, "{}{}.clear();", indent_str, dst_name).unwrap();
                    writeln!(output, "{}{}.extend_from_slice({});", indent_str, dst_name, slice_ref).unwrap();
                    // Type was registered as `Vec<T>` at the hoist site.
                } else {
                    writeln!(output, "{}let mut {}: Vec<{}> = {};", indent_str, dst_name, elem_type, vec_init).unwrap();
                    ctx.register_variable_type(dst_sym, format!("Vec<{}>", elem_type));
                }
            } else {
                if is_start_one && is_end_length_of_src && needs_borrow {
                    // Full deep copy of a nested LogosSeq.
                    writeln!(output, "{}let mut {}: LogosSeq<{}> = {}.deep_clone();",
                        indent_str, dst_name, elem_type, src_name).unwrap();
                } else {
                    writeln!(output, "{}let mut {}: LogosSeq<{}> = LogosSeq::from_vec({});",
                        indent_str, dst_name, elem_type, vec_init).unwrap();
                }
                ctx.register_variable_type(dst_sym, format!("LogosSeq<{}>", elem_type));
            }

            // Emit intervening statements between Seq creation and counter init
            for si in (idx + 1)..counter_idx {
                use super::codegen_stmt;
                output.push_str(&codegen_stmt(stmts[si], interner, indent, mutable_vars, ctx,
                    lww_fields, mv_fields, synced_vars, var_caps, async_functions,
                    pipe_vars, boxed_fields, registry, type_env));
            }

            // Re-emit counter if it's used after the While
            let remaining = &stmts[while_idx + 1..];
            if symbol_appears_in_stmts(counter_sym, remaining) {
                let end_str = codegen_expr_simple(end_expr, interner);
                let post_val = if is_exclusive {
                    end_str.to_string()
                } else {
                    format!("{} + 1", end_str)
                };
                if counter_is_new_binding {
                    writeln!(output, "{}let mut {} = {};", indent_str, counter_name, post_val).unwrap();
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
        Stmt::Let { var, value, ty, mutable } => {
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

            let type_from_new = if let Expr::New { type_name, type_args, init_fields } = value {
                let tn = interner.resolve(*type_name);
                if matches!(tn, "Seq" | "List" | "Vec") && init_fields.is_empty() && !type_args.is_empty() {
                    Some(CollInfo::Vec(codegen_type_expr(&type_args[0], interner)))
                } else if matches!(tn, "Map" | "HashMap") && init_fields.is_empty() && type_args.len() >= 2 {
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
                let loop_matches = match cond {
                    Expr::BinaryOp { op: BinaryOpKind::LtEq | BinaryOpKind::Lt, left, .. } => {
                        matches!(left, Expr::Identifier(sym) if *sym == counter_sym)
                    }
                    _ => false,
                };

                if loop_matches && body.len() >= 2 {
                    // Check last statement is counter++
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
                        let body_without_increment = &body[..body.len() - 1];
                        // Check for pushes/inserts. For top-level pushes, any push suffices.
                        // For nested pushes (inside If/Otherwise), require ALL paths to push
                        // so the count is deterministic (filter patterns excluded).
                        let has_push = match &collection_info {
                            CollInfo::Vec(_) => {
                                body_without_increment.iter().any(|s| {
                                    matches!(s, Stmt::Push { collection, .. } if matches!(collection, Expr::Identifier(sym) if *sym == vec_sym))
                                }) || all_paths_push_to(body_without_increment, vec_sym)
                            }
                            CollInfo::Map(_, _) => {
                                body_without_increment.iter().any(|s| {
                                    matches!(s, Stmt::SetIndex { collection, .. } if matches!(collection, Expr::Identifier(sym) if *sym == vec_sym))
                                }) || all_paths_set_index_to(body_without_increment, vec_sym)
                            }
                        };

                        if has_push {
                            counter_init_idx = Some(scan);
                            break;
                        }
                    }
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
        &remaining_refs, 0, interner, indent, mutable_vars, ctx,
        lww_fields, mv_fields, synced_vars, var_caps, async_functions,
        pipe_vars, boxed_fields, registry, type_env,
    );

    let (loop_code, _) = loop_result?;

    // Compute capacity from the for-range bounds
    let start_str = codegen_expr_simple(match stmts[counter_idx] {
        Stmt::Let { value, .. } | Stmt::Set { value, .. } => value,
        _ => return None,
    }, interner);

    let limit_expr = match stmts[while_idx] {
        Stmt::While { cond, .. } => match cond {
            Expr::BinaryOp { right, .. } => *right,
            _ => return None,
        },
        _ => return None,
    };
    let is_exclusive = match stmts[while_idx] {
        Stmt::While { cond, .. } => matches!(cond, Expr::BinaryOp { op: BinaryOpKind::Lt, .. }),
        _ => false,
    };
    let limit_str = codegen_expr_simple(limit_expr, interner);

    let start_lit = match stmts[counter_idx] {
        Stmt::Let { value: Expr::Literal(Literal::Number(n)), .. }
        | Stmt::Set { value: Expr::Literal(Literal::Number(n)), .. } => Some(*n),
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
                        let is_target = matches!(collection, Expr::Identifier(sym) if *sym == vec_sym);
                        if is_target {
                            match value {
                                Expr::Literal(Literal::Number(n)) => Some(format!("{}", n)),
                                Expr::Literal(Literal::Float(f)) => Some(format!("{:.1}", f)),
                                Expr::Literal(Literal::Boolean(b)) => Some(format!("{}", b)),
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
            let wrap_ty = if de_rc { format!("Vec<{}>", elem) } else { format!("LogosSeq<{}>", elem_type) };
            // A Seq pushed `k>1` times per iteration needs `k * N` capacity. (The
            // `vec_fill_literal` fast-path only fires for a single push, so the
            // fill count there is always `N`.)
            let prim_cap = scale_capacity(&capacity_expr, pushes_per_iter(body, vec_sym));
            let rhs = match (&vec_fill_literal, de_rc) {
                (Some(fill), true) => format!("vec![{}; ({}) as usize]", fill, capacity_expr),
                (Some(fill), false) => format!("LogosSeq::from_vec(vec![{}; {}])", fill, capacity_expr),
                (None, true) => format!("Vec::with_capacity(({}) as usize)", prim_cap),
                (None, false) => format!("LogosSeq::with_capacity({})", prim_cap),
            };
            writeln!(output, "{}let {}{}: {} = {};",
                indent_str, mut_kw, vec_name, wrap_ty, rhs).unwrap();
            emit_narrow_guards(&mut output, vec_sym, ctx, &indent_str);
            ctx.register_variable_type(vec_sym, wrap_ty);
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
                let cap = dense_cap_rust(vec_sym, ctx, interner)
                    .unwrap_or_else(|| capacity_expr.clone());
                writeln!(output, "{}let mut {}: {} = {}::with_bounds(0, (({}) + 1) as usize);",
                    indent_str, vec_name, rust_ty, rust_ty, cap).unwrap();
            } else if rust_ty.starts_with("LogosI64") || rust_ty.starts_with("LogosI32") {
                writeln!(output, "{}let mut {}: {} = {}::with_capacity({});",
                    indent_str, vec_name, rust_ty, rust_ty, capacity_expr).unwrap();
            } else {
                writeln!(output, "{}let {}{}: {} = LogosMap::with_capacity({});",
                    indent_str, mut_kw, vec_name, rust_ty, capacity_expr).unwrap();
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
            let sib_mut = if sib_mutable || mutable_vars.contains(&sib_sym) { "mut " } else { "" };
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
                    let wrap_ty = if de_rc { format!("Vec<{}>", elem) } else { format!("LogosSeq<{}>", elem_type) };
                    let sib_cap = scale_capacity(&capacity_expr, pushes_per_iter(body_without_increment, sib_sym));
                    let rhs = if de_rc {
                        format!("Vec::with_capacity(({}) as usize)", sib_cap)
                    } else {
                        format!("LogosSeq::with_capacity({})", sib_cap)
                    };
                    writeln!(output, "{}let {}{}: {} = {};",
                        indent_str, sib_mut, sib_name, wrap_ty, rhs).unwrap();
                    emit_narrow_guards(&mut output, sib_sym, ctx, &indent_str);
                    ctx.register_variable_type(sib_sym, wrap_ty);
                }
                CollInfo::Map(key_type, val_type) => {
                    let rust_ty = map_rust_type(key_type, val_type, sib_sym, ctx);
                    if rust_ty.starts_with("LogosDense") {
                        let cap = dense_cap_rust(sib_sym, ctx, interner)
                            .unwrap_or_else(|| capacity_expr.clone());
                        writeln!(output, "{}let mut {}: {} = {}::with_bounds(0, (({}) + 1) as usize);",
                            indent_str, sib_name, rust_ty, rust_ty, cap).unwrap();
                    } else if rust_ty.starts_with("LogosI64") || rust_ty.starts_with("LogosI32") {
                        writeln!(output, "{}let mut {}: {} = {}::with_capacity({});",
                            indent_str, sib_name, rust_ty, rust_ty, capacity_expr).unwrap();
                    } else {
                        writeln!(output, "{}let {}{}: {} = LogosMap::with_capacity({});",
                            indent_str, sib_mut, sib_name, rust_ty, capacity_expr).unwrap();
                    }
                    ctx.register_variable_type(sib_sym, rust_ty);
                }
            }
        } else {
            use super::codegen_stmt;
            output.push_str(&codegen_stmt(stmt, interner, indent, mutable_vars, ctx,
                lww_fields, mv_fields, synced_vars, var_caps, async_functions,
                pipe_vars, boxed_fields, registry, type_env));
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
        Stmt::Let { var, value, ty, mutable } => (*var, *value, ty.as_ref(), *mutable),
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

    let type_from_new = if let Expr::New { type_name, type_args, init_fields } = value {
        let tn = interner.resolve(*type_name);
        if matches!(tn, "Seq" | "List" | "Vec") && init_fields.is_empty() && !type_args.is_empty() {
            Some(CollInfo::Vec(codegen_type_expr(&type_args[0], interner)))
        } else if matches!(tn, "Map" | "HashMap") && init_fields.is_empty() && type_args.len() >= 2 {
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

    if has_push { Some((var, info, is_mutable)) } else { None }
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
        Stmt::Let { var, value, ty, mutable: true, .. } => {
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

            let type_from_new = if let Expr::New { type_name, type_args, init_fields } = value {
                let tn = interner.resolve(*type_name);
                if matches!(tn, "Seq" | "List" | "Vec") && init_fields.is_empty() && !type_args.is_empty() {
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

    // Build capacity expression: sum of all source .len() values
    let names = RustNames::new(interner);
    let vec_name = names.ident(vec_sym);
    let indent_str = "    ".repeat(indent);

    let mut capacity_parts: Vec<String> = source_syms.iter().map(|sym| {
        format!("{}.len()", names.ident(*sym))
    }).collect();
    // `source_syms` is a `HashSet`, whose iteration order changes between compiles;
    // sort the (commutative) capacity terms so the emitted sum is reproducible.
    capacity_parts.sort();
    let capacity_expr = capacity_parts.join(" + ");

    let mut output = String::new();
    // De-Rc-aware: a de-Rc'd merge target is a plain owned `Vec<T>`.
    if ctx.is_de_rc(vec_sym) {
        writeln!(output, "{}let mut {}: Vec<{}> = Vec::with_capacity(({}) as usize);",
            indent_str, vec_name, elem_type, capacity_expr).unwrap();
        ctx.register_variable_type(vec_sym, format!("Vec<{}>", elem_type));
    } else {
        writeln!(output, "{}let mut {}: LogosSeq<{}> = LogosSeq::with_capacity(({}) as usize);",
            indent_str, vec_name, elem_type, capacity_expr).unwrap();
        ctx.register_variable_type(vec_sym, format!("LogosSeq<{}>", elem_type));
    }

    // Emit intervening statements between the declaration and the first While,
    // then the While loops themselves (all handled by regular codegen).
    // We only consumed statement idx (the Let). The rest are emitted by the caller.
    // Return 0 extra consumed since we only replaced the Let declaration.
    Some((output, 0))
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
            Stmt::If { then_block, else_block, .. } => {
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
        Stmt::Let { var, mutable: false, value, .. } => {
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
        Stmt::Set { target, value: Expr::Literal(Literal::Number(1)) } => *target,
        _ => return None,
    };

    // Statement 3: While counter <= limit: SetIndex arr[counter] = arr[counter+1]; Set counter++
    let limit_expr = match stmts[idx + 2] {
        Stmt::While { cond, body, .. } => {
            // Condition: counter <= limit
            let limit = match cond {
                Expr::BinaryOp { op: BinaryOpKind::LtEq, left, right } => {
                    if let Expr::Identifier(c) = left {
                        if *c == counter_sym { Some(*right) } else { None }
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
                Stmt::SetIndex { collection, index: idx_expr, value } => {
                    if !matches!(collection, Expr::Identifier(s) if *s == arr_sym) {
                        return None;
                    }
                    if !matches!(idx_expr, Expr::Identifier(s) if *s == counter_sym) {
                        return None;
                    }
                    if let Expr::Index { collection: v_coll, index: v_idx } = value {
                        if !matches!(v_coll, Expr::Identifier(s) if *s == arr_sym) {
                            return None;
                        }
                        match v_idx {
                            Expr::BinaryOp { op: BinaryOpKind::Add, left, right } => {
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
                        Expr::BinaryOp { op: BinaryOpKind::Add, left, right } => {
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
        Stmt::SetIndex { collection, index, value } => {
            if !matches!(collection, Expr::Identifier(s) if *s == arr_sym) {
                return None;
            }
            if !matches!(value, Expr::Identifier(s) if *s == tmp_sym) {
                return None;
            }
            let syntactic_ok = match index {
                Expr::BinaryOp { op: BinaryOpKind::Add, left, right } => {
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

    let is_logos_seq = variable_types.get(&arr_sym)
        .map_or(false, |t| t.starts_with("LogosSeq"));

    let mut output = String::new();

    // Only emit the tmp binding if it's used after the rotation.
    let remaining = &stmts[idx + 4..];
    if is_logos_seq {
        if symbol_appears_in_stmts(tmp_sym, remaining) {
            writeln!(output, "{}let {} = {}.borrow()[0].clone();", indent_str, tmp_name, arr_name).unwrap();
        }
        writeln!(output, "{}{}.borrow_mut()[0..=({} as usize)].rotate_left(1);",
            indent_str, arr_name, limit_str).unwrap();
    } else {
        if symbol_appears_in_stmts(tmp_sym, remaining) {
            writeln!(output, "{}let {} = {}[0];", indent_str, tmp_name, arr_name).unwrap();
        }
        writeln!(output, "{}{}[0..=({} as usize)].rotate_left(1);",
            indent_str, arr_name, limit_str).unwrap();
    }

    Some((output, 3)) // consumed: Set counter + While + SetIndex = 3 extra
}

/// Pattern B: Unconditional 3-statement swap with arbitrary indices.
///   Let tmp be item I of arr.
///   Set item I of arr to item J of arr.
///   Set item J of arr to tmp.
/// → `arr.swap((I-1) as usize, (J-1) as usize);`
fn try_emit_unconditional_swap<'a>(
    stmts: &[&Stmt<'a>],
    idx: usize,
    tmp_sym: Symbol,
    arr_sym: Symbol,
    idx_expr_1: &'a Expr<'a>,
    interner: &Interner,
    indent: usize,
    variable_types: &HashMap<Symbol, String>,
    oracle: Option<&crate::optimize::OracleFacts>,
) -> Option<(String, usize)> {
    if idx + 2 >= stmts.len() {
        return None;
    }

    // Statement 2: Set item I of arr to item J of arr.
    let idx_expr_2 = match stmts[idx + 1] {
        Stmt::SetIndex { collection, index, value } => {
            // collection must be the same array
            if !matches!(collection, Expr::Identifier(s) if *s == arr_sym) {
                return None;
            }
            // index must match idx_expr_1
            if !exprs_equal(index, idx_expr_1) {
                return None;
            }
            // value must be an Index into the same array at a different index
            if let Expr::Index { collection: v_coll, index: v_idx } = value {
                if !matches!(v_coll, Expr::Identifier(s) if *s == arr_sym) {
                    return None;
                }
                *v_idx
            } else {
                return None;
            }
        }
        _ => return None,
    };

    // Statement 3: Set item J of arr to tmp.
    match stmts[idx + 2] {
        Stmt::SetIndex { collection, index, value } => {
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

    // Pattern matched! Emit manual swap with direct indexing
    let indent_str = "    ".repeat(indent);
    let arr_name = interner.resolve(arr_sym);
    let idx1_simplified = simplify_1based_index(idx_expr_1, interner, true);
    let idx2_simplified = simplify_1based_index(idx_expr_2, interner, true);

    let is_logos_seq = variable_types.get(&arr_sym)
        .map_or(false, |t| t.starts_with("LogosSeq"));

    let mut output = String::new();
    if is_logos_seq {
        writeln!(output, "{}{{ let mut __bm = {}.borrow_mut();", indent_str, arr_name).unwrap();
        writeln!(output, "{}let __swap_tmp = __bm[{}];",
            indent_str, idx1_simplified).unwrap();
        writeln!(output, "{}__bm[{}] = __bm[{}];",
            indent_str, idx1_simplified, idx2_simplified).unwrap();
        writeln!(output, "{}__bm[{}] = __swap_tmp;",
            indent_str, idx2_simplified).unwrap();
        writeln!(output, "{}}}", indent_str).unwrap();
    } else {
        writeln!(output, "{}let __swap_tmp = {}[{}];",
            indent_str, arr_name, idx1_simplified).unwrap();
        writeln!(output, "{}{}[{}] = {}[{}];",
            indent_str, arr_name, idx1_simplified, arr_name, idx2_simplified).unwrap();
        writeln!(output, "{}{}[{}] = __swap_tmp;",
            indent_str, arr_name, idx2_simplified).unwrap();
    }

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
    if let Some(result) = try_bare_slice_push_with_init(stmts, idx, interner, indent, variable_types) {
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

    // Emit the optimized code
    let remaining = &stmts[idx + 2..];
    let src_is_logos_seq = variable_types.get(&while_info.src_sym)
        .map(|t| t.split("|__hl:").next().unwrap_or(t.as_str()))
        .map(|t| t.starts_with("LogosSeq"))
        .unwrap_or(true);
    let output = emit_extend_from_slice(
        interner, indent, while_info.dst_sym, while_info.src_sym, counter_sym,
        start_expr, while_info.end_expr, while_info.is_exclusive,
        remaining, Some(is_new_binding), src_is_logos_seq,
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
            if !is_simple_expr(end_expr) { return None; }
            if body.len() != 2 { return None; }
            let (src, dst) = extract_push_index_body(body, counter_sym)?;
            (counter_sym, end_expr, is_exclusive, src, dst)
        }
        _ => return None,
    };

    // Validate types
    validate_slice_push_types(src_sym, dst_sym, variable_types)?;

    // For bare While, the counter already has a value from a previous statement.
    // We use the counter identifier as the start expression.
    let start_expr = Expr::Identifier(counter_sym);

    let remaining = &stmts[idx + 1..];
    let src_is_logos_seq = variable_types.get(&src_sym)
        .map(|t| t.split("|__hl:").next().unwrap_or(t.as_str()))
        .map(|t| t.starts_with("LogosSeq"))
        .unwrap_or(true);
    let output = emit_extend_from_slice(
        interner, indent, dst_sym, src_sym, counter_sym,
        &start_expr, end_expr, is_exclusive,
        remaining, None, src_is_logos_seq,
    );

    Some((output, 0)) // no extra statements consumed
}

struct WhileInfo<'a> {
    end_expr: &'a Expr<'a>,
    is_exclusive: bool,
    src_sym: Symbol,
    dst_sym: Symbol,
}

fn extract_push_copy_while<'a>(
    stmt: &'a Stmt<'a>,
    counter_sym: Symbol,
) -> Option<WhileInfo<'a>> {
    let (cond, body) = match stmt {
        Stmt::While { cond, body, .. } => (cond, body),
        _ => return None,
    };

    let (cond_counter, end_expr, is_exclusive) = extract_while_cond(cond)?;
    if cond_counter != counter_sym { return None; }
    if !is_simple_expr(end_expr) { return None; }
    if body.len() != 2 { return None; }

    let (src_sym, dst_sym) = extract_push_index_body(body, counter_sym)?;
    validate_increment(&body[1], counter_sym)?;

    Some(WhileInfo { end_expr, is_exclusive, src_sym, dst_sym })
}

fn extract_while_cond<'a>(cond: &'a Expr<'a>) -> Option<(Symbol, &'a Expr<'a>, bool)> {
    match cond {
        Expr::BinaryOp { op: BinaryOpKind::LtEq, left, right } => {
            if let Expr::Identifier(sym) = left {
                Some((*sym, *right, false))
            } else {
                None
            }
        }
        Expr::BinaryOp { op: BinaryOpKind::Lt, left, right } => {
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
            let dst = if let Expr::Identifier(s) = collection { *s } else { return None; };
            if let Expr::Index { collection: src_coll, index } = value {
                if !matches!(index, Expr::Identifier(s) if *s == counter_sym) {
                    return None;
                }
                if let Expr::Identifier(s) = src_coll { (*s, dst) } else { return None; }
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
        Stmt::Set { target, value } if *target == counter_sym => {
            match value {
                Expr::BinaryOp { op: BinaryOpKind::Add, left, right } => {
                    let ok = matches!((left, right),
                        (Expr::Identifier(s), Expr::Literal(Literal::Number(1))) if *s == counter_sym
                    ) || matches!((left, right),
                        (Expr::Literal(Literal::Number(1)), Expr::Identifier(s)) if *s == counter_sym
                    );
                    if ok { Some(()) } else { None }
                }
                _ => None,
            }
        }
        _ => None,
    }
}

fn validate_slice_push_types(
    src_sym: Symbol,
    dst_sym: Symbol,
    variable_types: &HashMap<Symbol, String>,
) -> Option<()> {
    if src_sym == dst_sym { return None; }

    let dst_type = variable_types.get(&dst_sym)?;
    if !dst_type.starts_with("LogosSeq<") && !dst_type.starts_with("Vec<") { return None; }

    let src_type = variable_types.get(&src_sym)?;
    if !src_type.starts_with("LogosSeq<") && !src_type.starts_with("Vec<") && !src_type.starts_with("&[") && !src_type.starts_with("&mut [") {
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
    let borrow_expr = if src_is_logos_seq { format!("{}.borrow()", src_name) } else { src_name.to_string() };

    let mut output = String::new();

    if is_exclusive {
        writeln!(output, "{}if {} < {} {{", indent_str, start_str, end_str).unwrap();
        writeln!(output, "{}    {}.extend_from_slice(&{}[({} - 1) as usize..({} - 1) as usize]);",
            indent_str, dst_name, borrow_expr, start_str, end_str).unwrap();
        writeln!(output, "{}}}", indent_str).unwrap();
    } else {
        writeln!(output, "{}if {} <= {} {{", indent_str, start_str, end_str).unwrap();
        writeln!(output, "{}    {}.extend_from_slice(&{}[({} - 1) as usize..{} as usize]);",
            indent_str, dst_name, borrow_expr, start_str, end_str).unwrap();
        writeln!(output, "{}}}", indent_str).unwrap();
    }

    if symbol_appears_in_stmts(counter_sym, remaining) {
        let post_val = if is_exclusive {
            end_str.to_string()
        } else {
            if let Expr::Literal(Literal::Number(n)) = end_expr {
                format!("{}", n + 1)
            } else {
                format!("{} + 1", end_str)
            }
        };
        match binding_info {
            Some(true) => {
                writeln!(output, "{}let mut {} = {};", indent_str, counter_name, post_val).unwrap();
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
/// then-branch. This is equivalent to `target.extend_from_slice(&array[counter-1..])` + `break`.
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
        Expr::BinaryOp { op: BinaryOpKind::LtEq, left, right } => {
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
        Stmt::If { cond, then_block, else_block: Some(else_block) } => {
            (*cond, then_block, else_block)
        }
        _ => return None,
    };

    // The then-block must have exactly 2 statements.
    if then_block.len() != 2 {
        return None;
    }

    // Statement 1: Push item <counter> of <array> to <target>
    let (target_sym, array_sym, push_counter_sym) = match &then_block[0] {
        Stmt::Push { value, collection } => {
            let tgt = if let Expr::Identifier(s) = collection { *s } else { return None; };
            if let Expr::Index { collection: arr_expr, index: idx_expr } = value {
                let arr_sym = if let Expr::Identifier(s) = arr_expr { *s } else { return None; };
                let idx_sym = if let Expr::Identifier(s) = idx_expr { *s } else { return None; };
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
    let cond_str = codegen_expr_with_async(if_cond, interner, synced_vars, async_functions, ctx.get_variable_types());

    let mut output = String::new();

    // Check source type to determine if .borrow() is needed
    let array_is_logos_seq = ctx.get_variable_types().get(&array_sym)
        .map(|t| t.split("|__hl:").next().unwrap_or(t.as_str()))
        .map(|t| t.starts_with("LogosSeq"))
        .unwrap_or(true);
    let borrow_expr = if array_is_logos_seq { format!("{}.borrow()", array_name) } else { array_name.to_string() };

    // Emit the If with extend_from_slice + break for the then-branch.
    writeln!(output, "{}if {} {{", indent_str, cond_str).unwrap();
    // Copy only up to the loop's 1-based upper bound, NOT to the end of the
    // array: `while i <= bound` pushes items i..=bound, i.e. 0-based
    // `[(i-1)..bound)`. Draining to the array end over-copies when bound < len.
    let bound_str = codegen_expr_with_async(bound_expr, interner, synced_vars, async_functions, ctx.get_variable_types());
    writeln!(output, "{}    {}.extend_from_slice(&{}[({} - 1) as usize..({}) as usize]);",
        indent_str, target_name, borrow_expr, counter_name, bound_str).unwrap();
    writeln!(output, "{}    break;", indent_str).unwrap();
    writeln!(output, "{}}} else {{", indent_str).unwrap();

    // Emit the else block normally.
    for else_stmt in else_block.iter() {
        output.push_str(&super::codegen_stmt(else_stmt, interner, indent + 1, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry, type_env));
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
            Stmt::Let { var, value, ty, mutable: true } => {
                let type_from_new = if let Expr::New { type_name, type_args, init_fields } = value {
                    let tn = interner.resolve(*type_name);
                    if matches!(tn, "Seq" | "List" | "Vec") && init_fields.is_empty() && !type_args.is_empty() {
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
        if let Stmt::If { then_block, else_block, .. } = s {
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
        Expr::BinaryOp { op: BinaryOpKind::LtEq, left, right } => match &**left {
            Expr::Identifier(s) => (*s, *right, true),
            _ => return None,
        },
        Expr::BinaryOp { op: BinaryOpKind::Lt, left, right } => match &**left {
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
            if bi > 1 { break; }
            if let Stmt::Let { var, value, ty, mutable: true } = stmt {
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
                let type_from_new = if let Expr::New { type_name, type_args, init_fields } = value {
                    let tn = interner.resolve(*type_name);
                    if matches!(tn, "Seq" | "List" | "Vec") && init_fields.is_empty() && !type_args.is_empty() {
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
        writeln!(output, "{}let mut {}: Vec<{}> = Vec::new();", indent_str, inner_name, reuse.inner_elem_type).unwrap();
    } else {
        writeln!(output, "{}let mut {}: LogosSeq<{}> = LogosSeq::new();", indent_str, inner_name, reuse.inner_elem_type).unwrap();
    }

    // Loop bounds hoisting (replicate from stmt.rs While handler).
    use super::stmt::{extract_length_expr_syms, collect_length_syms_from_stmts};
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
            writeln!(output, "{}let {} = ({}.len() as i64);", indent_str, hoisted_name, name).unwrap();
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
    let cond_str = codegen_expr_with_async(cond, interner, synced_vars, async_functions, ctx.get_variable_types());
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
                ctx.register_variable_type(reuse.inner_sym, format!("Vec<{}>", reuse.inner_elem_type));
            } else {
                writeln!(output, "{}    {}.borrow_mut().clear();", indent_str, inner_name).unwrap();
                ctx.register_variable_type(reuse.inner_sym, format!("LogosSeq<{}>", reuse.inner_elem_type));
            }
            bi += 1;
            continue;
        }
        if bi == reuse.set_idx {
            // Replace Set outer to inner with mem::swap
            writeln!(output, "{}    std::mem::swap(&mut {}, &mut {});", indent_str, outer_name, inner_name).unwrap();
            bi += 1;
            continue;
        }

        // Standard peephole chain for body statements.
        if let Some((code, skip)) = try_emit_seq_from_slice_pattern(&body_refs, bi, interner, indent + 1, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry, type_env) {
            output.push_str(&code);
            bi += 1 + skip;
            continue;
        }
        if let Some((code, skip)) = try_emit_vec_fill_pattern(&body_refs, bi, interner, indent + 1, ctx) {
            output.push_str(&code);
            bi += 1 + skip;
            continue;
        }
        if let Some((code, skip)) = try_emit_vec_with_capacity_pattern(&body_refs, bi, interner, indent + 1, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry, type_env) {
            output.push_str(&code);
            bi += 1 + skip;
            continue;
        }
        if let Some((code, skip)) = try_emit_merge_capacity_pattern(&body_refs, bi, interner, indent + 1, ctx) {
            output.push_str(&code);
            bi += 1 + skip;
            continue;
        }
        if let Some((code, skip)) = try_emit_for_range_pattern(&body_refs, bi, interner, indent + 1, mutable_vars, ctx, lww_fields, mv_fields, synced_vars, var_caps, async_functions, pipe_vars, boxed_fields, registry, type_env) {
            output.push_str(&code);
            bi += 1 + skip;
            continue;
        }
        if let Some((code, skip)) = try_emit_prefix_reverse(&body_refs, bi, interner, indent + 1, ctx.get_variable_types()) {
            output.push_str(&code);
            bi += 1 + skip;
            continue;
        }
        if let Some((code, skip)) = try_emit_swap_pattern(&body_refs, bi, interner, indent + 1, ctx.get_variable_types(), ctx.oracle()) {
            output.push_str(&code);
            bi += 1 + skip;
            continue;
        }
        if let Some((code, skip)) = try_emit_seq_copy_pattern(&body_refs, bi, interner, indent + 1, ctx) {
            output.push_str(&code);
            bi += 1 + skip;
            continue;
        }
        if let Some((code, skip)) = try_emit_rotate_left_pattern(&body_refs, bi, interner, indent + 1, ctx.get_variable_types()) {
            output.push_str(&code);
            bi += 1 + skip;
            continue;
        }

        // Fallback: normal codegen.
        use super::codegen_stmt;
        output.push_str(&codegen_stmt(body_refs[bi], interner, indent + 1, mutable_vars, ctx,
            lww_fields, mv_fields, synced_vars, var_caps, async_functions,
            pipe_vars, boxed_fields, registry, type_env));
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
