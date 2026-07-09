use std::collections::{HashMap, HashSet};

use crate::analysis::registry::TypeRegistry;
use crate::analysis::types::RustNames;
use crate::ast::logic::{LogicExpr, NumberKind, Term};
use crate::ast::stmt::{BinaryOpKind, Expr, Literal, Stmt, TypeExpr};
use crate::formatter::RustFormatter;
use crate::intern::{Interner, Symbol};
use crate::registry::SymbolRegistry;

use super::context::RefinementContext;
use super::detection::{collect_mutable_vars, expr_debug_prefix, parse_aos_tag};
use super::i64_map::is_logos_map_type;
use super::types::{codegen_type_expr, infer_logos_type, infer_numeric_type};
use super::{
    codegen_stmt, get_root_identifier, has_copy_element_type, has_copy_value_type, is_copy_type,
};

use std::sync::LazyLock;

// Stable empty contexts so a sparsely-populated [`ExprCtx`] can borrow `'static`
// placeholders instead of stack temporaries at every call site.
static EMPTY_SYMS: LazyLock<HashSet<Symbol>> = LazyLock::new(HashSet::new);
static EMPTY_TYPES: LazyLock<HashMap<Symbol, String>> = LazyLock::new(HashMap::new);
static EMPTY_BOXED_FIELDS: LazyLock<HashSet<(String, String, String)>> =
    LazyLock::new(HashSet::new);
static EMPTY_REGISTRY: LazyLock<TypeRegistry> = LazyLock::new(TypeRegistry::new);

/// All borrowed context an expression lowering threads through its recursion.
///
/// This bundles what were eight positional parameters passed by hand through
/// every recursive `codegen_expr` call. It is `Copy` (every field is a shared
/// reference), so recursion passes `ctx` unchanged and a sub-scope that needs to
/// vary one field uses struct-update: `ExprCtx { boxed_bindings: &b, ..*ctx }`.
/// New cross-cutting context becomes one field here instead of a parameter on
/// hundreds of call sites.
#[derive(Clone, Copy)]
pub(crate) struct ExprCtx<'a> {
    pub interner: &'a Interner,
    pub synced_vars: &'a HashSet<Symbol>,
    pub boxed_fields: &'a HashSet<(String, String, String)>,
    pub registry: &'a TypeRegistry,
    pub async_functions: &'a HashSet<Symbol>,
    pub boxed_bindings: &'a HashSet<Symbol>,
    pub string_vars: &'a HashSet<Symbol>,
    pub variable_types: &'a HashMap<Symbol, String>,
    /// Divisor symbol → precomputed `LogosDivU64` helper variable name, for the
    /// loop-invariant libdivide rewrite of `% n` / `/ n` (empty when inactive).
    pub fast_div: &'a HashMap<Symbol, String>,
    /// Bounds-elision oracle: when present, an `item E of arr` read the oracle
    /// proves in range lowers to `get_unchecked` (no bounds branch) instead of
    /// checked indexing. `None` ⟹ every access keeps its check. Threaded only
    /// from the statement arms that lower indexed reads (`Let`/`Set`/`If`/
    /// `SetIndex` values), where the `RefinementContext`'s oracle is in hand.
    pub oracle: Option<&'a crate::optimize::OracleFacts>,
    /// Overflow ruling v2 (stage 2): when true, an unproven Int op may emit the
    /// bare checked-exact helper (result `LogosInt` — the promoting boxed
    /// path); when false the result NARROWS back to i64 with a loud canonical
    /// error if it no longer fits (never a silent wrap). True under `Show` and
    /// down the operand chains of exactable ops; false at i64 storage sinks.
    pub int_exact_tolerant: bool,
    /// True while codegen'ing an INDEX subexpression (`arr[<here>]`). Index
    /// arithmetic is a `usize` computation the interpreter also requires to fit
    /// i64 (a promoted BigInt can't index) — so it stays raw, never exact-wrapped.
    pub int_index_context: bool,
    /// True while codegen'ing the argument of a `wordN(...)` truncation. The
    /// result keeps only the low N bits, so Int add/sub/mul lower to raw i64
    /// `wrapping_*` (identical low bits to the checked-exact helper, matching the
    /// crate's wrapping word semantics) instead of `logos_*_exact`. Propagates
    /// through the additive chain but is RESET at a call boundary (a call's
    /// argument is a full-width value, not truncated).
    pub int_wrapping: bool,
}

impl<'a> ExprCtx<'a> {
    /// The minimal context: just the interner and synced vars, every richer
    /// field empty. Richer wrappers layer their fields on with struct-update.
    pub(crate) fn bare(interner: &'a Interner, synced_vars: &'a HashSet<Symbol>) -> Self {
        ExprCtx {
            interner,
            synced_vars,
            boxed_fields: &EMPTY_BOXED_FIELDS,
            registry: &EMPTY_REGISTRY,
            async_functions: &EMPTY_SYMS,
            boxed_bindings: &EMPTY_SYMS,
            string_vars: &EMPTY_SYMS,
            variable_types: &EMPTY_TYPES,
            fast_div: &EMPTY_TYPES,
            oracle: None,
            int_exact_tolerant: false,
            int_index_context: false,
            int_wrapping: false,
        }
    }
}

/// True when the bounds-elision oracle proves `index` in range for `collection`,
/// so the access can lower to `get_unchecked`. The `LOGOS_ORACLE_UNCHECKED=0`
/// kill switch forces the checked form (A/B against the `assert_unchecked` hint
/// path). Soundness rests on the kernel-LIA-certified proof plus the caller's
/// entry precondition guard; a `debug_assert!` (emitted by the statement-level
/// hint pass) traps an unsound proof in debug builds.
/// A compile-time Int op on two literals (overflow ruling v2, stage 2).
thread_local! {
    /// The whole-program set of `Int`-returning functions whose value can exceed i64, so their
    /// return type is the promoting `LogosInt` (`bigint_promote::bigint_returning_fns`). Set once
    /// per program at codegen entry by [`set_bigint_returning_fns`]; read by `mentions_bigint_var`
    /// so an INLINE call to such a function (e.g. `mlkemBit(a) + mlkemBit(b)`) is recognised as a
    /// promoted operand and routes through the exact helper. A bare `LogosInt + LogosInt` has no
    /// operator impl and would not compile — this closes the call-operand hole the variable-only
    /// detector left open. Codegen is single-threaded per program (nextest gives each test its own
    /// thread), and each program overwrites the set, so no state leaks across programs or tests.
    static BIGINT_RETURNING_FNS: std::cell::RefCell<HashSet<Symbol>> =
        std::cell::RefCell::new(HashSet::new());
}

/// Record the program's bignum-returning function set for the current codegen (see
/// [`BIGINT_RETURNING_FNS`]). Called once at the top of `generate_rust_code`.
pub(crate) fn set_bigint_returning_fns(fns: &HashSet<Symbol>) {
    BIGINT_RETURNING_FNS.with(|c| *c.borrow_mut() = fns.clone());
}

fn is_bigint_returning_call(function: Symbol) -> bool {
    BIGINT_RETURNING_FNS.with(|c| c.borrow().contains(&function))
}

/// Does `e` PRODUCE a value stored as the overflow-promoting `LogosInt`? — a variable typed with
/// the `|__bigint` sentinel, or an inline call to a bignum-returning function. Such an operand
/// forces the exact-helper path so the result stays `LogosInt` rather than a raw `i64` operator
/// (or `as u64` cast) that `LogosInt` has no impl for.
fn mentions_bigint_var(e: &Expr, variable_types: &HashMap<Symbol, String>) -> bool {
    match e {
        Expr::Identifier(s) => variable_types.get(s).map_or(false, |t| t.contains("__bigint")),
        Expr::Call { function, .. } => is_bigint_returning_call(*function),
        Expr::BinaryOp { left, right, .. } => {
            mentions_bigint_var(left, variable_types) || mentions_bigint_var(right, variable_types)
        }
        Expr::Not { operand } => mentions_bigint_var(operand, variable_types),
        _ => false,
    }
}

enum ConstExact {
    /// The exact result fits i64 — emit today's raw text, byte-identical.
    InRange,
    /// The exact result escapes i64 — the promoted `LogosInt` literal.
    Promoted(String),
}

/// Evaluate `a op b` exactly at compile time (i128 covers any single i64 op).
/// `None` = not decidable here (a zero divisor falls to the runtime helper,
/// which raises the canonical catchable error).
fn const_exact_int(op: BinaryOpKind, a: i64, b: i64) -> Option<ConstExact> {
    let exact: i128 = match op {
        BinaryOpKind::Add => a as i128 + b as i128,
        BinaryOpKind::Subtract => a as i128 - b as i128,
        BinaryOpKind::Multiply => a as i128 * b as i128,
        BinaryOpKind::Divide => {
            if b == 0 {
                return None;
            }
            a as i128 / b as i128
        }
        BinaryOpKind::Modulo => {
            if b == 0 {
                return None;
            }
            a as i128 % b as i128
        }
        _ => return None,
    };
    Some(if i64::try_from(exact).is_ok() {
        ConstExact::InRange
    } else {
        ConstExact::Promoted(format!("LogosInt::from_literal(\"{}\")", exact))
    })
}

/// True when the range-analysis Oracle bounds BOTH operands such that the
/// exact result provably fits i64 — the license for raw (unchecked) i64
/// emission. Add/Sub are monotone in each operand; Mul takes the corner
/// products. Div/Mod never qualify here (their rims are divisor-shaped,
/// not range-shaped) — they go through the checked helper when non-const.
fn oracle_proves_int_op_in_range(
    ecx: &ExprCtx,
    whole: &Expr,
    op: BinaryOpKind,
    left: &Expr,
    right: &Expr,
) -> bool {
    // A constructing pass (e.g. `try_defer_modulus`) may have PROVEN this very
    // node in-range when it built it — its version guard is the proof the
    // interval fixpoint cannot re-derive (accumulators widen to ±∞ by design).
    if crate::optimize::expr_proven_raw_int_op(whole) {
        return true;
    }
    let Some(o) = ecx.oracle else { return false };
    // If the oracle bounds the WHOLE expression to an i64 range (the affine
    // index proof `prev[w - wi]` gives `w - wi ∈ [0, len)`), it cannot
    // overflow — raw, no promotion. This keeps proven index arithmetic clean.
    if let Some((lo, hi)) = o.expr_int_range(whole) {
        let _ = (lo, hi); // in range by construction (i64 bounds)
        return true;
    }
    let (Some((la, lb)), Some((ra, rb))) = (o.expr_int_range(left), o.expr_int_range(right))
    else {
        return false;
    };
    let (la, lb, ra, rb) = (la as i128, lb as i128, ra as i128, rb as i128);
    let (lo, hi) = match op {
        BinaryOpKind::Add => (la + ra, lb + rb),
        BinaryOpKind::Subtract => (la - rb, lb - ra),
        BinaryOpKind::Multiply => {
            let c = [la * ra, la * rb, lb * ra, lb * rb];
            (*c.iter().min().unwrap(), *c.iter().max().unwrap())
        }
        _ => return false,
    };
    lo >= i64::MIN as i128 && hi <= i64::MAX as i128
}

/// Is `e` an Int-exactable arithmetic interior node — the shape the tolerant
/// `LogosInt` chain would own?
fn is_exact_arith_node(e: &Expr) -> bool {
    matches!(
        e,
        Expr::BinaryOp {
            op: BinaryOpKind::Add
                | BinaryOpKind::Subtract
                | BinaryOpKind::Multiply
                | BinaryOpKind::Divide
                | BinaryOpKind::Modulo,
            ..
        }
    )
}

/// Worst-case MAGNITUDE of an exact Int chain — the i128-lowering gate. A
/// literal costs its own magnitude, every other PROVEN-i64 leaf costs the
/// full `2^63` (covers `i64::MIN`); add/sub sum magnitudes, mul multiplies
/// them, div keeps the dividend's, rem is bounded by both. Exact tracking
/// (not bit-widths, which compound over-estimates: `(i·n + j) + 1` is
/// `2^126 + 2^63 + 1`, comfortably inside i128, where width arithmetic
/// rounds every step up). `None` = some node is not provably an i64-valued
/// Int-chain member, or the bound escapes u128 — only the promoting
/// `LogosInt` chain is exact there.
fn exact_chain_max_magnitude(e: &Expr, ecx: &ExprCtx) -> Option<u128> {
    if mentions_bigint_var(e, ecx.variable_types) {
        return None;
    }
    match e {
        Expr::Literal(Literal::Number(n)) => Some(n.unsigned_abs() as u128),
        Expr::BinaryOp { op, left, right } if is_exact_arith_node(e) => {
            let l = exact_chain_max_magnitude(left, ecx)?;
            let r = exact_chain_max_magnitude(right, ecx)?;
            match op {
                BinaryOpKind::Add | BinaryOpKind::Subtract => l.checked_add(r),
                BinaryOpKind::Multiply => l.checked_mul(r),
                BinaryOpKind::Divide => Some(l),
                BinaryOpKind::Modulo => Some(r.min(l)),
                _ => unreachable!("is_exact_arith_node covers exactly these five"),
            }
        }
        _ => (infer_numeric_type(e, ecx.interner, ecx.variable_types) == "i64")
            .then_some(1u128 << 63),
    }
}

/// Emit the i128 form of a width-bounded exact chain: leaves cast to i128,
/// interiors native (`/`/`%` keep the canonical zero-divisor error via the
/// guarded helpers unless the divisor is a nonzero literal).
fn emit_exact_chain_i128(e: &Expr, ecx: &ExprCtx) -> String {
    match e {
        Expr::Literal(Literal::Number(n)) => format!("{}i128", n),
        Expr::BinaryOp { op, left, right } if is_exact_arith_node(e) => {
            let l = emit_exact_chain_i128(left, ecx);
            let r = emit_exact_chain_i128(right, ecx);
            let nonzero_lit = matches!(&**right, Expr::Literal(Literal::Number(d)) if *d != 0);
            match op {
                BinaryOpKind::Add => format!("({} + {})", l, r),
                BinaryOpKind::Subtract => format!("({} - {})", l, r),
                BinaryOpKind::Multiply => format!("({} * {})", l, r),
                BinaryOpKind::Divide if nonzero_lit => format!("({} / {})", l, r),
                BinaryOpKind::Divide => format!("logos_div_i128({}, {})", l, r),
                BinaryOpKind::Modulo if nonzero_lit => format!("({} % {})", l, r),
                BinaryOpKind::Modulo => format!("logos_rem_i128({}, {})", l, r),
                _ => unreachable!("is_exact_arith_node covers exactly these five"),
            }
        }
        _ => {
            let plain = ExprCtx { int_exact_tolerant: false, ..*ecx };
            format!("(({}) as i128)", codegen_expr_ctx(e, &plain))
        }
    }
}

/// The register-shaped lowerings for a root-NARROWED exact Int op. Both
/// tiers preserve the interpreter's semantics bit for bit — same values,
/// same canonical error text:
///
///  - **depth-1** (no exact-chain interior): one fused checked machine op
///    (`logos_add_i64` …), overflow to the cold canonical panic;
///  - **width-bounded chain**: native i128 with a single root narrow — every
///    intermediate is exact by construction, so an oversized intermediate
///    that returns to range still succeeds, exactly like the `LogosInt`
///    chain it replaces.
///
/// `None`: the chain's worst case may exceed 127 bits — the promoting
/// `LogosInt` chain is the only exact lowering.
fn try_fuse_narrowed_int_op(
    op: BinaryOpKind,
    left: &Expr,
    right: &Expr,
    ecx: &ExprCtx,
) -> Option<String> {
    let plain = ExprCtx { int_exact_tolerant: false, ..*ecx };
    if !is_exact_arith_node(left) && !is_exact_arith_node(right) {
        let helper = match op {
            BinaryOpKind::Add => "logos_add_i64",
            BinaryOpKind::Subtract => "logos_sub_i64",
            BinaryOpKind::Multiply => "logos_mul_i64",
            BinaryOpKind::Divide => "logos_div_i64",
            BinaryOpKind::Modulo => "logos_rem_i64",
            _ => return None,
        };
        let l = codegen_expr_ctx(left, &plain);
        let r = codegen_expr_ctx(right, &plain);
        return Some(format!("{}({}, {})", helper, l, r));
    }
    None
}

/// Tier 3 — the width-bounded i128 chain (see [`try_fuse_narrowed_int_op`]
/// for tier 1, [`try_guarded_dual_chain`] for the preferred tier 2 on deeper
/// chains: raw i64 behind one predictable leaf guard beats branchless i128
/// wherever it applies, so this runs LAST).
fn try_i128_chain(
    op: BinaryOpKind,
    left: &Expr,
    right: &Expr,
    ecx: &ExprCtx,
) -> Option<String> {
    let l = exact_chain_max_magnitude(left, ecx)?;
    let r = exact_chain_max_magnitude(right, ecx)?;
    let root_magnitude = match op {
        BinaryOpKind::Add | BinaryOpKind::Subtract => l.checked_add(r)?,
        BinaryOpKind::Multiply => l.checked_mul(r)?,
        BinaryOpKind::Divide => l,
        BinaryOpKind::Modulo => r.min(l),
        _ => return None,
    };
    if root_magnitude > (i128::MAX as u128) {
        return None;
    }
    let le = emit_exact_chain_i128(left, ecx);
    let re = emit_exact_chain_i128(right, ecx);
    let nonzero_lit = matches!(right, Expr::Literal(Literal::Number(d)) if *d != 0);
    let chain = match op {
        BinaryOpKind::Add => format!("({} + {})", le, re),
        BinaryOpKind::Subtract => format!("({} - {})", le, re),
        BinaryOpKind::Multiply => format!("({} * {})", le, re),
        BinaryOpKind::Divide if nonzero_lit => format!("({} / {})", le, re),
        BinaryOpKind::Divide => format!("logos_div_i128({}, {})", le, re),
        BinaryOpKind::Modulo if nonzero_lit => format!("({} % {})", le, re),
        BinaryOpKind::Modulo => format!("logos_rem_i128({}, {})", le, re),
        _ => unreachable!("root op matched above"),
    };
    Some(format!("logos_narrow_i128({})", chain))
}

/// Chain magnitude with every non-literal leaf capped at `bound` — the
/// solver's evaluation function for the guarded-dual tier.
fn chain_magnitude_with_leaf_bound(e: &Expr, bound: u128) -> Option<u128> {
    match e {
        Expr::Literal(Literal::Number(n)) => Some(n.unsigned_abs() as u128),
        Expr::BinaryOp { op, left, right } if is_exact_arith_node(e) => {
            let l = chain_magnitude_with_leaf_bound(left, bound)?;
            let r = chain_magnitude_with_leaf_bound(right, bound)?;
            match op {
                BinaryOpKind::Add | BinaryOpKind::Subtract => l.checked_add(r),
                BinaryOpKind::Multiply => l.checked_mul(r),
                BinaryOpKind::Divide => Some(l),
                BinaryOpKind::Modulo => Some(r.min(l)),
                _ => unreachable!("is_exact_arith_node covers exactly these five"),
            }
        }
        _ => Some(bound),
    }
}

/// Emit a chain over PRE-BOUND leaf names. `raw` picks plain i64 ops (the
/// guarded fast branch — proven in range by the leaf-bound guard); otherwise
/// the promoting `LogosInt` helpers (the exact slow branch). `/`/`%` keep
/// their canonical zero-divisor errors in both forms.
fn emit_chain_with_bound_leaves(
    e: &Expr,
    names: &HashMap<usize, String>,
    raw: bool,
) -> String {
    match e {
        Expr::Literal(Literal::Number(n)) => format!("{}i64", n),
        Expr::BinaryOp { op, left, right } if is_exact_arith_node(e) => {
            let l = emit_chain_with_bound_leaves(left, names, raw);
            let r = emit_chain_with_bound_leaves(right, names, raw);
            let nonzero_lit = matches!(&**right, Expr::Literal(Literal::Number(d)) if *d != 0);
            if raw {
                match op {
                    BinaryOpKind::Add => format!("({} + {})", l, r),
                    BinaryOpKind::Subtract => format!("({} - {})", l, r),
                    BinaryOpKind::Multiply => format!("({} * {})", l, r),
                    BinaryOpKind::Divide if nonzero_lit => format!("({} / {})", l, r),
                    BinaryOpKind::Divide => format!("logos_div_i64({}, {})", l, r),
                    BinaryOpKind::Modulo if nonzero_lit => format!("({} % {})", l, r),
                    BinaryOpKind::Modulo => format!("logos_rem_i64({}, {})", l, r),
                    _ => unreachable!("is_exact_arith_node covers exactly these five"),
                }
            } else {
                let helper = match op {
                    BinaryOpKind::Add => "logos_add_exact",
                    BinaryOpKind::Subtract => "logos_sub_exact",
                    BinaryOpKind::Multiply => "logos_mul_exact",
                    BinaryOpKind::Divide => "logos_div_exact",
                    BinaryOpKind::Modulo => "logos_rem_exact",
                    _ => unreachable!("is_exact_arith_node covers exactly these five"),
                };
                format!("{}({}, {})", helper, l, r)
            }
        }
        Expr::Identifier(s) => names[&(usize::MAX - s.index())].clone(),
        _ => names[&(e as *const Expr as usize)].clone(),
    }
}

/// Tier 3.5 — the GUARDED DUAL for a narrowed chain the i128 gate refused:
/// bind every leaf once (evaluation order preserved), test them against the
/// largest bound `B` under which the whole chain provably fits i64, and run
/// the RAW i64 chain under the guard — the exact `LogosInt` chain otherwise.
/// Value- and error-identical to the exact chain (the raw branch is proven
/// overflow-free; the slow branch IS the exact chain); what it buys is a
/// register-shaped, vectorizable hot path behind one predictable branch.
fn try_guarded_dual_chain<'e>(
    op: BinaryOpKind,
    left: &'e Expr<'e>,
    right: &'e Expr<'e>,
    whole_ecx: &ExprCtx,
) -> Option<String> {
    fn walk<'e>(
        e: &'e Expr<'e>,
        ecx: &ExprCtx,
        addrs: &mut Vec<usize>,
        exprs: &mut Vec<&'e Expr<'e>>,
        seen: &mut std::collections::HashSet<usize>,
    ) -> Option<()> {
        if mentions_bigint_var(e, ecx.variable_types) {
            return None;
        }
        match e {
            Expr::Literal(Literal::Number(_)) => Some(()),
            Expr::BinaryOp { left, right, .. } if is_exact_arith_node(e) => {
                walk(left, ecx, addrs, exprs, seen)?;
                walk(right, ecx, addrs, exprs, seen)
            }
            _ => {
                if infer_numeric_type(e, ecx.interner, ecx.variable_types) != "i64" {
                    return None;
                }
                // A bare identifier is pure — no binding needed, and every
                // occurrence shares one guard term (dedupe by symbol via the
                // seen-key). Anything else binds once per node.
                let addr = match e {
                    Expr::Identifier(s) => usize::MAX - s.index(),
                    _ => e as *const Expr as usize,
                };
                if seen.insert(addr) {
                    addrs.push(addr);
                    exprs.push(e);
                }
                Some(())
            }
        }
    }
    // Bound the emission: tiny chains fused earlier; huge ones would bloat
    // the guard past its value.
    let mut leaves: Vec<usize> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut leaf_exprs: Vec<&Expr> = Vec::new();
    walk(left, whole_ecx, &mut leaves, &mut leaf_exprs, &mut seen)?;
    walk(right, whole_ecx, &mut leaves, &mut leaf_exprs, &mut seen)?;
    if leaves.is_empty() || leaves.len() > 6 {
        return None;
    }
    // Solve for the largest power-of-two leaf bound that keeps the chain in
    // i64 (descending scan; the chain magnitude is monotone in the bound).
    let root_mag = |b: u128| -> Option<u128> {
        let l = chain_magnitude_with_leaf_bound(left, b)?;
        let r = chain_magnitude_with_leaf_bound(right, b)?;
        match op {
            BinaryOpKind::Add | BinaryOpKind::Subtract => l.checked_add(r),
            BinaryOpKind::Multiply => l.checked_mul(r),
            BinaryOpKind::Divide => Some(l),
            BinaryOpKind::Modulo => Some(r.min(l)),
            _ => None,
        }
    };
    let mut bound: u128 = 0;
    for exp in (16..=62).rev() {
        let b = 1u128 << exp;
        if root_mag(b).is_some_and(|m| m <= i64::MAX as u128) {
            bound = b;
            break;
        }
    }
    if bound == 0 {
        return None;
    }
    let plain = ExprCtx { int_exact_tolerant: false, ..*whole_ecx };
    let mut names: HashMap<usize, String> = HashMap::new();
    let mut binds = String::new();
    for (k, (addr, le)) in leaves.iter().zip(leaf_exprs.iter()).enumerate() {
        // A bare identifier needs no binding — its emitted name is pure and
        // the guard/chain reference it directly (one guard term per SYMBOL).
        if matches!(le, Expr::Identifier(_)) {
            names.insert(*addr, codegen_expr_ctx(le, &plain));
            continue;
        }
        let name = format!("__chain_l{}", k);
        binds.push_str(&format!("let {} = {}; ", name, codegen_expr_ctx(le, &plain)));
        names.insert(*addr, name);
    }
    // Guard terms in LEAF ORDER (the deduped evaluation order) — HashMap
    // iteration would make the emitted text nondeterministic across compiles.
    // A leaf the oracle proves non-negative needs only the UPPER bound: its
    // `>= -bound` term is vacuous, so we drop it. This halves the per-iteration
    // guard on hot multiply chains whose operand is an inductive `>= 0` (e.g.
    // collatz's `3*k+1`, where `k`'s loop range fixes to `[0, ∞)`).
    let guard = leaves
        .iter()
        .zip(leaf_exprs.iter())
        .map(|(addr, le)| {
            let n = &names[addr];
            let b = bound as i64;
            let nonneg = whole_ecx
                .oracle
                .and_then(|o| o.expr_int_range(le))
                .is_some_and(|(lo, _)| lo >= 0);
            if nonneg {
                format!("({n} <= {b}i64)")
            } else {
                format!("({n} >= -{b}i64 && {n} <= {b}i64)")
            }
        })
        .collect::<Vec<_>>()
        .join(" && ");
    let root = |raw: bool| -> String {
        let l = emit_chain_with_bound_leaves(left, &names, raw);
        let r = emit_chain_with_bound_leaves(right, &names, raw);
        let nonzero_lit = matches!(right, Expr::Literal(Literal::Number(d)) if *d != 0);
        if raw {
            match op {
                BinaryOpKind::Add => format!("({} + {})", l, r),
                BinaryOpKind::Subtract => format!("({} - {})", l, r),
                BinaryOpKind::Multiply => format!("({} * {})", l, r),
                BinaryOpKind::Divide if nonzero_lit => format!("({} / {})", l, r),
                BinaryOpKind::Divide => format!("logos_div_i64({}, {})", l, r),
                BinaryOpKind::Modulo if nonzero_lit => format!("({} % {})", l, r),
                BinaryOpKind::Modulo => format!("logos_rem_i64({}, {})", l, r),
                _ => unreachable!(),
            }
        } else {
            let helper = match op {
                BinaryOpKind::Add => "logos_add_exact",
                BinaryOpKind::Subtract => "logos_sub_exact",
                BinaryOpKind::Multiply => "logos_mul_exact",
                BinaryOpKind::Divide => "logos_div_exact",
                BinaryOpKind::Modulo => "logos_rem_exact",
                _ => unreachable!(),
            };
            format!("{}({}, {}).expect_i64(\"Int\")", helper, l, r)
        }
    };
    Some(format!(
        "{{ {binds}if {guard} {{ {fast} }} else {{ {slow} }} }}",
        fast = root(true),
        slow = root(false),
    ))
}

fn oracle_proves_index(ecx: &ExprCtx, collection: &Expr, index: &Expr) -> bool {
    if !crate::optimize::active_config().is_on(crate::optimization::Opt::Unchecked) {
        return false;
    }
    let proven = ecx.oracle.map_or(false, |o| o.index_provably_in_bounds(collection, index));
    if proven {
        crate::optimize::mark_fired(crate::optimization::Opt::Unchecked);
    }
    proven
}

/// [`codegen_expr_with_async`] plus the bounds-elision oracle, so proven indexed
/// reads in the lowered expression become `get_unchecked`. Used by the `If`-cond
/// statement arm.
pub(crate) fn codegen_expr_with_async_oracle<'a>(
    expr: &Expr,
    interner: &'a Interner,
    synced_vars: &'a HashSet<Symbol>,
    async_functions: &'a HashSet<Symbol>,
    variable_types: &'a HashMap<Symbol, String>,
    oracle: Option<&'a crate::optimize::OracleFacts>,
) -> String {
    codegen_expr_ctx(
        expr,
        &ExprCtx { async_functions, variable_types, oracle, ..ExprCtx::bare(interner, synced_vars) },
    )
}

/// [`codegen_expr_boxed_with_types`] plus the bounds-elision oracle, so proven
/// indexed reads in the lowered expression become `get_unchecked`. Used by the
/// `Let`/`Set`/`SetIndex` value statement arms.
#[allow(clippy::too_many_arguments)]
pub(crate) fn codegen_expr_boxed_with_types_oracle<'a>(
    expr: &Expr,
    interner: &'a Interner,
    synced_vars: &'a HashSet<Symbol>,
    boxed_fields: &'a HashSet<(String, String, String)>,
    registry: &'a TypeRegistry,
    async_functions: &'a HashSet<Symbol>,
    string_vars: &'a HashSet<Symbol>,
    variable_types: &'a HashMap<Symbol, String>,
    fast_div: &'a HashMap<Symbol, String>,
    oracle: Option<&'a crate::optimize::OracleFacts>,
) -> String {
    codegen_expr_ctx(
        expr,
        &ExprCtx {
            boxed_fields,
            registry,
            async_functions,
            string_vars,
            variable_types,
            fast_div,
            oracle,
            ..ExprCtx::bare(interner, synced_vars)
        },
    )
}

/// Like [`codegen_expr_boxed_with_types_oracle`] but TOLERANT of overflow: the
/// root exact-arithmetic result stays a `LogosInt` (no `.expect_i64` narrow).
/// Used for the RHS of a binding whose target is stored as `LogosInt`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn codegen_expr_boxed_with_types_oracle_tolerant<'a>(
    expr: &Expr,
    interner: &'a Interner,
    synced_vars: &'a HashSet<Symbol>,
    boxed_fields: &'a HashSet<(String, String, String)>,
    registry: &'a TypeRegistry,
    async_functions: &'a HashSet<Symbol>,
    string_vars: &'a HashSet<Symbol>,
    variable_types: &'a HashMap<Symbol, String>,
    fast_div: &'a HashMap<Symbol, String>,
    oracle: Option<&'a crate::optimize::OracleFacts>,
) -> String {
    codegen_expr_ctx(
        expr,
        &ExprCtx {
            boxed_fields,
            registry,
            async_functions,
            string_vars,
            variable_types,
            fast_div,
            oracle,
            int_exact_tolerant: true,
            ..ExprCtx::bare(interner, synced_vars)
        },
    )
}

pub fn codegen_expr(expr: &Expr, interner: &Interner, synced_vars: &HashSet<Symbol>) -> String {
    codegen_expr_ctx(expr, &ExprCtx::bare(interner, synced_vars))
}

/// Phase 54+: Codegen expression with async function tracking.
/// Adds .await to async function calls at the expression level, handling nested calls.
pub(crate) fn codegen_expr_with_async(
    expr: &Expr,
    interner: &Interner,
    synced_vars: &HashSet<Symbol>,
    async_functions: &HashSet<Symbol>,
    variable_types: &HashMap<Symbol, String>,
) -> String {
    codegen_expr_ctx(
        expr,
        &ExprCtx { async_functions, variable_types, ..ExprCtx::bare(interner, synced_vars) },
    )
}

/// Codegen expression with async support, string variable tracking, and the
/// loop-invariant libdivide map (`fast_div`).
pub(crate) fn codegen_expr_with_async_and_strings(
    expr: &Expr,
    interner: &Interner,
    synced_vars: &HashSet<Symbol>,
    async_functions: &HashSet<Symbol>,
    string_vars: &HashSet<Symbol>,
    variable_types: &HashMap<Symbol, String>,
    fast_div: &HashMap<Symbol, String>,
) -> String {
    codegen_expr_ctx(
        expr,
        &ExprCtx {
            async_functions,
            string_vars,
            variable_types,
            fast_div,
            // Every caller is a `Show` sink (plain object or an interpolated
            // part): a display accepts the promoting `LogosInt`, so overflow
            // prints the EXACT value instead of narrowing.
            int_exact_tolerant: true,
            ..ExprCtx::bare(interner, synced_vars)
        },
    )
}

/// Check if an expression is definitely numeric (safe to use + operator).
/// This is conservative for Add operations - treats it as string concat only
/// when clearly dealing with strings (string literals).
pub(crate) fn is_definitely_numeric_expr(expr: &Expr) -> bool {
    match expr {
        Expr::Literal(Literal::Number(_)) => true,
        Expr::Literal(Literal::Float(_)) => true,
        Expr::Literal(Literal::Duration(_)) => true,
        // Identifiers might be strings, but without a string literal nearby,
        // assume numeric (Rust will catch type errors)
        Expr::Identifier(_) => true,
        // Arithmetic operations are numeric
        Expr::BinaryOp { op: BinaryOpKind::Subtract, .. } => true,
        Expr::BinaryOp { op: BinaryOpKind::Multiply, .. } => true,
        Expr::BinaryOp { op: BinaryOpKind::Divide, .. } => true,
        Expr::BinaryOp { op: BinaryOpKind::Modulo, .. } => true,
        // Length always returns a number
        Expr::Length { .. } => true,
        // Add is numeric if both operands seem numeric
        Expr::BinaryOp { op: BinaryOpKind::Add, left, right } => {
            is_definitely_numeric_expr(left) && is_definitely_numeric_expr(right)
        }
        // Function calls - assume numeric (Rust type checker will validate)
        Expr::Call { .. } => true,
        // Index expressions - assume numeric
        Expr::Index { .. } => true,
        _ => true,
    }
}

/// Check if an expression is definitely a string (needs format! for concatenation).
/// Takes a set of known string variable symbols for identifier lookup.
pub(crate) fn is_definitely_string_expr_with_vars(expr: &Expr, string_vars: &HashSet<Symbol>) -> bool {
    match expr {
        // String literals are definitely strings
        Expr::Literal(Literal::Text(_)) => true,
        // Variables known to be strings
        Expr::Identifier(sym) => string_vars.contains(sym),
        // Concat always produces strings
        Expr::BinaryOp { op: BinaryOpKind::Concat, .. } => true,
        // Add with a string operand produces a string
        Expr::BinaryOp { op: BinaryOpKind::Add, left, right } => {
            is_definitely_string_expr_with_vars(left, string_vars)
                || is_definitely_string_expr_with_vars(right, string_vars)
        }
        // WithCapacity wrapping a string value is a string
        Expr::WithCapacity { value, .. } => is_definitely_string_expr_with_vars(value, string_vars),
        // Interpolated strings always produce strings
        Expr::InterpolatedString(_) => true,
        _ => false,
    }
}

/// Check if an expression is definitely a string (without variable tracking).
/// This is a fallback for contexts where string_vars isn't available.
pub(crate) fn is_definitely_string_expr(expr: &Expr) -> bool {
    let empty = HashSet::new();
    is_definitely_string_expr_with_vars(expr, &empty)
}

/// True when the expression is definitely a SEQUENCE — a list literal, a
/// Seq-typed variable, a slice, a concat/repeat result, or a fill. Drives the
/// `+` (concat) and `*` (repeat) operator overloads onto the sequence
/// emissions instead of the numeric ones.
fn is_definitely_seq_expr(
    expr: &Expr,
    variable_types: &HashMap<Symbol, String>,
    interner: &Interner,
) -> bool {
    match expr {
        Expr::List(_) | Expr::Slice { .. } => true,
        Expr::Identifier(sym) => variable_types
            .get(sym)
            .map(|t| {
                let t = t.split("|__hl:").next().unwrap_or(t.as_str());
                t.starts_with("LogosSeq") || t.starts_with("Vec<") || t.starts_with("&[")
            })
            .unwrap_or(false),
        Expr::BinaryOp { op: BinaryOpKind::SeqConcat, .. } => true,
        Expr::BinaryOp { op: BinaryOpKind::Add | BinaryOpKind::Multiply, left, right } => {
            is_definitely_seq_expr(left, variable_types, interner)
                || is_definitely_seq_expr(right, variable_types, interner)
        }
        Expr::Call { function, .. } => interner.resolve(*function) == "repeatSeq",
        Expr::New { type_name, .. } => interner.resolve(*type_name) == "Seq",
        _ => false,
    }
}

/// Collect leaf operands from a chain of string Add/Concat operations.
///
/// Walks left-leaning trees of `+` (on strings) and `Concat` operations,
/// collecting all leaf expressions into a flat Vec. This enables emitting
/// a single `format!("{}{}{}", a, b, c)` instead of nested
/// `format!("{}{}", format!("{}{}", a, b), c)`, avoiding O(n^2) allocation.
pub(crate) fn collect_string_concat_operands<'a, 'b>(
    expr: &'b Expr<'a>,
    string_vars: &HashSet<Symbol>,
    operands: &mut Vec<&'b Expr<'a>>,
) {
    match expr {
        Expr::BinaryOp { op: BinaryOpKind::Concat, left, right } => {
            collect_string_concat_operands(left, string_vars, operands);
            collect_string_concat_operands(right, string_vars, operands);
        }
        Expr::BinaryOp { op: BinaryOpKind::Add, left, right } => {
            let has_string = is_definitely_string_expr_with_vars(left, string_vars)
                || is_definitely_string_expr_with_vars(right, string_vars);
            if has_string {
                collect_string_concat_operands(left, string_vars, operands);
                collect_string_concat_operands(right, string_vars, operands);
            } else {
                operands.push(expr);
            }
        }
        _ => {
            operands.push(expr);
        }
    }
}

/// Phase 102: Codegen with boxed field support for recursive enums.
/// Phase 103: Added registry for polymorphic enum type inference.
/// Phase 54+: Added async_functions for proper .await on nested async calls.
pub(crate) fn codegen_expr_boxed(
    expr: &Expr,
    interner: &Interner,
    synced_vars: &HashSet<Symbol>,
    boxed_fields: &HashSet<(String, String, String)>,  // (EnumName, VariantName, FieldName)
    registry: &TypeRegistry,  // Phase 103: For type annotations on polymorphic enums
    async_functions: &HashSet<Symbol>,  // Phase 54+: Functions that are async
) -> String {
    codegen_expr_ctx(
        expr,
        &ExprCtx {
            boxed_fields,
            registry,
            async_functions,
            ..ExprCtx::bare(interner, synced_vars)
        },
    )
}

/// Codegen with string variable tracking for proper string concatenation.
pub(crate) fn codegen_expr_boxed_with_strings(
    expr: &Expr,
    interner: &Interner,
    synced_vars: &HashSet<Symbol>,
    boxed_fields: &HashSet<(String, String, String)>,
    registry: &TypeRegistry,
    async_functions: &HashSet<Symbol>,
    string_vars: &HashSet<Symbol>,
) -> String {
    codegen_expr_ctx(
        expr,
        &ExprCtx {
            boxed_fields,
            registry,
            async_functions,
            string_vars,
            ..ExprCtx::bare(interner, synced_vars)
        },
    )
}

/// Codegen with variable type tracking for direct collection indexing
/// optimization, plus the loop-invariant libdivide map (`fast_div`) so `% n` /
/// `/ n` sites can lower to a precomputed magic multiply.
pub(crate) fn codegen_expr_boxed_with_types(
    expr: &Expr,
    interner: &Interner,
    synced_vars: &HashSet<Symbol>,
    boxed_fields: &HashSet<(String, String, String)>,
    registry: &TypeRegistry,
    async_functions: &HashSet<Symbol>,
    string_vars: &HashSet<Symbol>,
    variable_types: &HashMap<Symbol, String>,
    fast_div: &HashMap<Symbol, String>,
) -> String {
    codegen_expr_ctx(
        expr,
        &ExprCtx {
            boxed_fields,
            registry,
            async_functions,
            string_vars,
            variable_types,
            fast_div,
            ..ExprCtx::bare(interner, synced_vars)
        },
    )
}

/// The expression-lowering workhorse. Takes the whole [`ExprCtx`] and unpacks it
/// into the locals the match arms use, so the body reads exactly as it did when
/// these were eight positional parameters; recursion just forwards `ctx`.
/// Parse an `__affine_array:coeff:offset:trip` type tag into `(coeff, offset)`,
/// the constants of the deleted CSR array's closed form `A[p] = coeff*p+offset`.
fn affine_array_coeff_offset(ty: Option<&String>) -> Option<(i64, i64)> {
    let rest = ty?.strip_prefix("__affine_array:")?;
    let mut parts = rest.splitn(3, ':');
    let coeff: i64 = parts.next()?.parse().ok()?;
    let offset: i64 = parts.next()?.parse().ok()?;
    Some((coeff, offset))
}

/// The trip-count Rust expression (the value of `length of A`) from the tag.
fn affine_array_trip(ty: Option<&String>) -> Option<String> {
    let rest = ty?.strip_prefix("__affine_array:")?;
    let mut parts = rest.splitn(3, ':');
    parts.next()?; // coeff
    parts.next()?; // offset
    Some(parts.next()?.to_string())
}

/// Does `expr` evaluate to a `LogosRational` at runtime? True for an `ExactDivide`, for
/// `+ − ×` over a Rational operand, and for an identifier bound to a `LogosRational`.
/// Mirrors `resolve_division::Cx::is_rat_expr` at the codegen level so operand coercion
/// matches the resolve pass that produced the `ExactDivide` in the first place.
pub(super) fn is_rational_expr(expr: &Expr, variable_types: &HashMap<Symbol, String>) -> bool {
    match expr {
        Expr::BinaryOp { op: BinaryOpKind::ExactDivide, .. } => true,
        Expr::BinaryOp {
            op: BinaryOpKind::Add | BinaryOpKind::Subtract | BinaryOpKind::Multiply,
            left,
            right,
        } => is_rational_expr(left, variable_types) || is_rational_expr(right, variable_types),
        Expr::Identifier(sym) => {
            variable_types.get(sym).map_or(false, |t| t == "LogosRational")
        }
        _ => false,
    }
}

/// A `LogosRational`-producing Rust string for an operand of exact arithmetic: a value
/// that is already Rational is used verbatim; an integer operand is wrapped as
/// `LogosRational::from_i64(..)` so the exact methods type-check.
fn rational_operand(expr: &Expr, code: &str, variable_types: &HashMap<Symbol, String>) -> String {
    if is_rational_expr(expr, variable_types) {
        code.to_string()
    } else {
        format!("LogosRational::from_i64(({}) as i64)", code)
    }
}

/// Does `expr` evaluate to a `LogosDecimal` at runtime? True for a `decimal(..)` call, for
/// `+ − ×` over a Decimal operand, and for an identifier bound to a `LogosDecimal`. Mirrors
/// `is_rational_expr` so operand coercion lines up with the inferred `Decimal` type.
pub(super) fn is_decimal_expr(
    expr: &Expr,
    variable_types: &HashMap<Symbol, String>,
    interner: &Interner,
) -> bool {
    match expr {
        Expr::Call { function, .. } => interner.resolve(*function) == "decimal",
        Expr::BinaryOp {
            op: BinaryOpKind::Add | BinaryOpKind::Subtract | BinaryOpKind::Multiply,
            left,
            right,
        } => {
            is_decimal_expr(left, variable_types, interner)
                || is_decimal_expr(right, variable_types, interner)
        }
        Expr::Identifier(sym) => variable_types.get(sym).map_or(false, |t| t == "LogosDecimal"),
        _ => false,
    }
}

/// A `LogosDecimal`-producing Rust string for an operand of exact decimal arithmetic: a
/// value that is already a Decimal is used verbatim; an integer operand is widened with
/// `LogosDecimal::from_i64(..)` so the exact methods type-check (`price * 3`).
fn decimal_operand(
    expr: &Expr,
    code: &str,
    variable_types: &HashMap<Symbol, String>,
    interner: &Interner,
) -> String {
    if is_decimal_expr(expr, variable_types, interner) {
        code.to_string()
    } else {
        format!("LogosDecimal::from_i64(({}) as i64)", code)
    }
}

/// Does `expr` evaluate to a `LogosComplex`? True for a `complex(..)` call, for `+ − × ÷`
/// over a Complex operand, and for an identifier bound to a `LogosComplex`.
pub(super) fn is_complex_expr(
    expr: &Expr,
    variable_types: &HashMap<Symbol, String>,
    interner: &Interner,
) -> bool {
    match expr {
        Expr::Call { function, .. } => interner.resolve(*function) == "complex",
        Expr::BinaryOp {
            op: BinaryOpKind::Add | BinaryOpKind::Subtract | BinaryOpKind::Multiply | BinaryOpKind::Divide,
            left,
            right,
        } => {
            is_complex_expr(left, variable_types, interner)
                || is_complex_expr(right, variable_types, interner)
        }
        Expr::Identifier(sym) => variable_types.get(sym).map_or(false, |t| t == "LogosComplex"),
        _ => false,
    }
}

/// A `LogosComplex`-producing operand: a value already Complex is used verbatim; an integer
/// is embedded as `LogosComplex::from_i64(..)` (a real `re + 0i`).
fn complex_operand(
    expr: &Expr,
    code: &str,
    variable_types: &HashMap<Symbol, String>,
    interner: &Interner,
) -> String {
    if is_complex_expr(expr, variable_types, interner) {
        code.to_string()
    } else {
        format!("LogosComplex::from_i64(({}) as i64)", code)
    }
}

/// Does `expr` evaluate to a `LogosModular`? True for a `modular(..)` call, for `+ − × ÷` over
/// two Modular operands, and for an identifier bound to a `LogosModular`. (No auto-lift: a bare
/// integer has no modulus, so modular arithmetic fires only when BOTH operands are modular.)
pub(super) fn is_modular_expr(
    expr: &Expr,
    variable_types: &HashMap<Symbol, String>,
    interner: &Interner,
) -> bool {
    match expr {
        Expr::Call { function, .. } => interner.resolve(*function) == "modular",
        Expr::BinaryOp {
            op: BinaryOpKind::Add | BinaryOpKind::Subtract | BinaryOpKind::Multiply | BinaryOpKind::Divide,
            left,
            right,
        } => {
            is_modular_expr(left, variable_types, interner)
                && is_modular_expr(right, variable_types, interner)
        }
        Expr::Identifier(sym) => variable_types.get(sym).map_or(false, |t| t == "LogosModular"),
        _ => false,
    }
}

/// Does `expr` evaluate to a `LogosQuantity`? True for a `quantity(..)`/`convert(..)` call, an
/// identifier bound to a `LogosQuantity`, `+ −` over two quantities, and `× ÷` where AT LEAST one
/// operand is a quantity (the other scales it). Mirrors the interpreter's quantity dispatch.
pub(super) fn is_quantity_expr(
    expr: &Expr,
    variable_types: &HashMap<Symbol, String>,
    interner: &Interner,
) -> bool {
    match expr {
        Expr::Call { function, .. } => {
            matches!(interner.resolve(*function), "quantity" | "convert")
        }
        Expr::BinaryOp { op: BinaryOpKind::Add | BinaryOpKind::Subtract, left, right } => {
            is_quantity_expr(left, variable_types, interner)
                && is_quantity_expr(right, variable_types, interner)
        }
        Expr::BinaryOp { op: BinaryOpKind::Multiply | BinaryOpKind::Divide, left, right } => {
            is_quantity_expr(left, variable_types, interner)
                || is_quantity_expr(right, variable_types, interner)
        }
        Expr::Identifier(sym) => variable_types.get(sym).map_or(false, |t| t == "LogosQuantity"),
        _ => false,
    }
}

/// Does `expr` evaluate to a `LogosMoney`? True for a `money(..)` call, an identifier bound to a
/// `LogosMoney`, `+ −` over two monies, and `× ÷` where AT LEAST one operand is money (the other
/// scales it). Mirrors the interpreter's money dispatch.
pub(super) fn is_money_expr(
    expr: &Expr,
    variable_types: &HashMap<Symbol, String>,
    interner: &Interner,
) -> bool {
    match expr {
        Expr::Call { function, .. } => interner.resolve(*function) == "money",
        Expr::BinaryOp { op: BinaryOpKind::Add | BinaryOpKind::Subtract, left, right } => {
            is_money_expr(left, variable_types, interner)
                && is_money_expr(right, variable_types, interner)
        }
        Expr::BinaryOp { op: BinaryOpKind::Multiply | BinaryOpKind::Divide, left, right } => {
            is_money_expr(left, variable_types, interner)
                || is_money_expr(right, variable_types, interner)
        }
        Expr::Identifier(sym) => variable_types.get(sym).map_or(false, |t| t == "LogosMoney"),
        _ => false,
    }
}

fn codegen_expr_ctx(expr: &Expr, ecx: &ExprCtx) -> String {
    // Bind the fields the match arms read directly; `registry` rides along in
    // `ecx` and reaches recursion via the `recurse!` macro.
    let ExprCtx {
        interner,
        synced_vars,
        boxed_fields,
        async_functions,
        boxed_bindings,
        string_vars,
        variable_types,
        fast_div,
        ..
    } = *ecx;
    let names = RustNames::new(interner);
    // Helper macro for recursive calls with all context
    macro_rules! recurse {
        ($e:expr) => {
            codegen_expr_ctx($e, ecx)
        };
    }
    // Recurse into an INDEX subexpression: arithmetic there is a `usize`
    // computation (raw i64, never exact-promoted). The flag propagates through
    // nested arithmetic automatically.
    macro_rules! irecurse {
        ($e:expr) => {
            codegen_expr_ctx($e, &ExprCtx { int_index_context: true, ..*ecx })
        };
    }

    match expr {
        Expr::Literal(lit) => codegen_literal(lit, interner),

        Expr::Identifier(sym) => {
            let name = names.ident(*sym);
            // Dereference boxed bindings from enum destructuring
            let base = if boxed_bindings.contains(sym) {
                format!("(*{})", name)
            } else {
                name
            };
            // An overflow-promoting `LogosInt` variable: in an INDEX position it must narrow to
            // an i64 (`.expect_i64`, then `- 1 as usize` downstream); in any other value position
            // it is read by value, so `.clone()` avoids a move (LogosInt is not Copy).
            if variable_types.get(sym).map_or(false, |t| t.contains("__bigint")) {
                if ecx.int_index_context {
                    format!("{}.expect_i64(\"Int\")", base)
                } else {
                    format!("{}.clone()", base)
                }
            } else {
                base
            }
        }

        Expr::BinaryOp { op, left, right } => {
            // O9 libdivide: a loop-invariant POSITIVE divisor lowers to a
            // precomputed `LogosDivU64` magic multiply (`helper.rem(x)` /
            // `helper.div(x)`) instead of a hardware `div`/`idiv`. `detect_fast_div`
            // already proved the divisor immutable and `>= 1` and the dividend
            // `>= 0` at every site, so the `i64`→`u64` reinterpretation is
            // value-preserving.
            // A promoted (`LogosInt`) dividend can hold a value beyond i64, so the `i64`→`u64`
            // libdivide reinterpretation is neither valid (no `as` cast on `LogosInt`) nor correct;
            // it falls through to the exact helper below.
            if matches!(op, BinaryOpKind::Modulo | BinaryOpKind::Divide)
                && !mentions_bigint_var(left, variable_types)
            {
                if let Expr::Identifier(n) = right {
                    if let Some(helper) = fast_div.get(n) {
                        let dividend = recurse!(left);
                        let method = if matches!(op, BinaryOpKind::Modulo) { "rem" } else { "div" };
                        return format!("({}.{}(({}) as u64) as i64)", helper, method, dividend);
                    }
                }
            }

            // Flatten chained string concat/add into a single format! call.
            // Turns O(n^2) nested format! into O(n) single-allocation.
            let is_string_concat = matches!(op, BinaryOpKind::Concat)
                || (matches!(op, BinaryOpKind::Add)
                    && (is_definitely_string_expr_with_vars(left, string_vars)
                        || is_definitely_string_expr_with_vars(right, string_vars)));

            if is_string_concat {
                let mut operands = Vec::new();
                collect_string_concat_operands(expr, string_vars, &mut operands);
                let placeholders: String = operands.iter().map(|_| "{}").collect::<Vec<_>>().join("");
                let values: Vec<String> = operands.iter().map(|e| {
                    // String literals can be &str inside format!() — no heap allocation needed
                    if let Expr::Literal(Literal::Text(sym)) = e {
                        format!("\"{}\"", interner.resolve(*sym))
                    } else {
                        recurse!(e)
                    }
                }).collect();
                return format!("format!(\"{}\", {})", placeholders, values.join(", "));
            }

            // `a followed by b` — merge two sequences into one fresh `LogosSeq` (the default repr
            // for a `Seq of T`, matching list literals and slices). `.to_vec()` clones from either
            // representation (a plain `Vec<T>` or a `LogosSeq<T>`), so the operands are
            // representation-agnostic.
            if matches!(op, BinaryOpKind::SeqConcat) {
                let l = recurse!(left);
                let r = recurse!(right);
                return format!(
                    "LogosSeq::from_vec({{ let mut __sc = ({}).to_vec(); __sc.extend(({}).to_vec()); __sc }})",
                    l, r
                );
            }

            // `a is approximately b` — the ONE shared isclose definition
            // (both operands promoted to f64; approximation is inherently
            // tolerant, so the lossy view is correct here).
            if matches!(op, BinaryOpKind::ApproxEq) {
                let l = recurse!(left);
                let r = recurse!(right);
                return format!("logos_approx_eq(({}) as f64, ({}) as f64)", l, r);
            }

            // `| & ^ -` on SET operands are the set operations (insertion-
            // ordered IndexSet methods; `-` reaches here only for Sets — the
            // numeric subtract keeps the plain emission below).
            if matches!(op, BinaryOpKind::BitAnd | BinaryOpKind::BitOr | BinaryOpKind::BitXor | BinaryOpKind::Subtract) {
                let set_typed = |e: &Expr| -> bool {
                    matches!(e, Expr::Identifier(sym)
                        if variable_types.get(sym).is_some_and(|t| t.starts_with("Set<") || t.starts_with("FxHashSet<")))
                        || matches!(e, Expr::Call { function, .. } if names.raw(*function) == "setOf")
                };
                if set_typed(left) || set_typed(right) {
                    let l = recurse!(left);
                    let r = recurse!(right);
                    let method = match op {
                        BinaryOpKind::BitAnd => "intersection",
                        BinaryOpKind::BitOr => "union",
                        BinaryOpKind::BitXor => "symmetric_difference",
                        _ => "difference",
                    };
                    return format!("({}).{}(&({})).cloned().collect::<Set<_>>()", l, method, r);
                }
            }

            // `xs + ys` on sequences IS `followed by` — the same fresh-merge emission.
            if matches!(op, BinaryOpKind::Add)
                && (is_definitely_seq_expr(left, variable_types, interner)
                    || is_definitely_seq_expr(right, variable_types, interner))
            {
                let l = recurse!(left);
                let r = recurse!(right);
                return format!(
                    "LogosSeq::from_vec({{ let mut __sc = ({}).to_vec(); __sc.extend(({}).to_vec()); __sc }})",
                    l, r
                );
            }

            // `xs * n` / `n * xs` — repeat into a fresh sequence; every slot is
            // an independent `fill_clone` of its element (n rows, never n
            // aliases of one). n ≤ 0 is empty, matching the tree-walker.
            if matches!(op, BinaryOpKind::Multiply) {
                let seq_first = is_definitely_seq_expr(left, variable_types, interner);
                let seq_second = is_definitely_seq_expr(right, variable_types, interner);
                if seq_first || seq_second {
                    let (seq_e, n_e) = if seq_first { (left, right) } else { (right, left) };
                    let s = recurse!(seq_e);
                    let n = recurse!(n_e);
                    return format!(
                        "{{ let __rp_src = ({}).to_vec(); let __rp_n = (({}) as i64).max(0) as usize; \
                         let mut __rp = Vec::with_capacity(__rp_src.len() * __rp_n); \
                         for _ in 0..__rp_n {{ __rp.extend(__rp_src.iter().map(|__e| __e.fill_clone())); }} \
                         LogosSeq::from_vec(__rp) }}",
                        s, n
                    );
                }
            }

            // Optimize HashMap .get() for equality comparisons to avoid cloning
            if matches!(op, BinaryOpKind::Eq | BinaryOpKind::NotEq) {
                let neg = matches!(op, BinaryOpKind::NotEq);
                // Check if left side is a HashMap/LogosMap index
                if let Expr::Index { collection, index } = left {
                    if let Expr::Identifier(sym) = collection {
                        if let Some(t) = variable_types.get(sym) {
                            if is_logos_map_type(t) {
                                let coll_str = recurse!(collection);
                                let key_str = irecurse!(index);
                                let val_str = recurse!(right);
                                let cmp = if neg { "!=" } else { "==" };
                                return format!("({}.get(&({})) {} Some({}))", coll_str, key_str, cmp, val_str);
                            } else if t.starts_with("std::collections::HashMap") || t.starts_with("HashMap") || t.starts_with("rustc_hash::FxHashMap") || t.starts_with("FxHashMap") {
                                let coll_str = recurse!(collection);
                                let key_str = irecurse!(index);
                                let val_str = recurse!(right);
                                let cmp = if neg { "!=" } else { "==" };
                                if has_copy_value_type(t) {
                                    return format!("({}.get(&({})).copied() {} Some({}))", coll_str, key_str, cmp, val_str);
                                } else {
                                    return format!("({}.get(&({})) {} Some(&({})))", coll_str, key_str, cmp, val_str);
                                }
                            }
                        }
                    }
                }
                // Check if right side is a HashMap/LogosMap index
                if let Expr::Index { collection, index } = right {
                    if let Expr::Identifier(sym) = collection {
                        if let Some(t) = variable_types.get(sym) {
                            if is_logos_map_type(t) {
                                let coll_str = recurse!(collection);
                                let key_str = irecurse!(index);
                                let val_str = recurse!(left);
                                let cmp = if neg { "!=" } else { "==" };
                                return format!("(Some({}) {} {}.get(&({})))", val_str, cmp, coll_str, key_str);
                            } else if t.starts_with("std::collections::HashMap") || t.starts_with("HashMap") || t.starts_with("rustc_hash::FxHashMap") || t.starts_with("FxHashMap") {
                                let coll_str = recurse!(collection);
                                let key_str = irecurse!(index);
                                let val_str = recurse!(left);
                                let cmp = if neg { "!=" } else { "==" };
                                if has_copy_value_type(t) {
                                    return format!("(Some({}) {} {}.get(&({})).copied())", val_str, cmp, coll_str, key_str);
                                } else {
                                    return format!("(Some(&({})) {} {}.get(&({})))", val_str, cmp, coll_str, key_str);
                                }
                            }
                        }
                    }
                }

                // Optimize string-index-vs-string-index comparison to use direct
                // byte comparison via as_bytes() instead of logos_get_char().
                // Byte equality is correct for UTF-8: two characters are equal
                // iff their byte representations are equal, and the logos_get_char
                // function already uses byte-level indexing for the ASCII fast path.
                if let (Expr::Index { collection: left_coll, index: left_idx },
                        Expr::Index { collection: right_coll, index: right_idx }) = (left, right) {
                    let left_is_string = if let Expr::Identifier(sym) = left_coll {
                        string_vars.contains(sym) || variable_types.get(sym).map_or(false, |t| t == "String")
                    } else { false };
                    let right_is_string = if let Expr::Identifier(sym) = right_coll {
                        string_vars.contains(sym) || variable_types.get(sym).map_or(false, |t| t == "String")
                    } else { false };
                    if left_is_string && right_is_string {
                        let cmp = if neg { "!=" } else { "==" };
                        let left_coll_str = recurse!(left_coll);
                        let right_coll_str = recurse!(right_coll);
                        let left_idx_simplified = super::peephole::simplify_1based_index(left_idx, interner, true, variable_types);
                        let right_idx_simplified = super::peephole::simplify_1based_index(right_idx, interner, true, variable_types);
                        return format!("({}.as_bytes()[{}] {} {}.as_bytes()[{}])",
                            left_coll_str, left_idx_simplified, cmp, right_coll_str, right_idx_simplified);
                    }
                }

                // Optimize string-index-vs-single-char-literal comparison to use
                // logos_get_char() == 'c' instead of LogosIndex::logos_get() == String::from("c").
                // Avoids two heap allocations per comparison in hot loops.
                let is_string_index = |expr: &Expr| -> bool {
                    if let Expr::Index { collection, .. } = expr {
                        if let Expr::Identifier(sym) = collection {
                            return string_vars.contains(sym) || variable_types.get(sym).map_or(false, |t| t == "String");
                        }
                    }
                    false
                };
                let single_char_literal = |expr: &Expr| -> Option<char> {
                    if let Expr::Literal(Literal::Text(sym)) = expr {
                        let s = interner.resolve(*sym);
                        let mut chars = s.chars();
                        if let Some(c) = chars.next() {
                            if chars.next().is_none() {
                                return Some(c);
                            }
                        }
                    }
                    None
                };

                // Left is string index, right is single-char literal
                if is_string_index(left) {
                    if let Some(ch) = single_char_literal(right) {
                        if let Expr::Index { collection, index } = left {
                            let coll_str = recurse!(collection);
                            let idx_str = irecurse!(index);
                            let cmp = if neg { "!=" } else { "==" };
                            let ch_escaped = match ch {
                                '\'' => "\\'".to_string(),
                                '\\' => "\\\\".to_string(),
                                '\n' => "\\n".to_string(),
                                '\t' => "\\t".to_string(),
                                '\r' => "\\r".to_string(),
                                _ => ch.to_string(),
                            };
                            return format!("({}.logos_get_char({}) {} '{}')",
                                coll_str, idx_str, cmp, ch_escaped);
                        }
                    }
                }
                // Right is string index, left is single-char literal
                if is_string_index(right) {
                    if let Some(ch) = single_char_literal(left) {
                        if let Expr::Index { collection, index } = right {
                            let coll_str = recurse!(collection);
                            let idx_str = irecurse!(index);
                            let cmp = if neg { "!=" } else { "==" };
                            let ch_escaped = match ch {
                                '\'' => "\\'".to_string(),
                                '\\' => "\\\\".to_string(),
                                '\n' => "\\n".to_string(),
                                '\t' => "\\t".to_string(),
                                '\r' => "\\r".to_string(),
                                _ => ch.to_string(),
                            };
                            return format!("('{}' {} {}.logos_get_char({}))",
                                ch_escaped, cmp, coll_str, idx_str);
                        }
                    }
                }
            }

            // OPT-8b: Zero-based counter in comparison.
            // When a __zero_based_i64 counter appears as a bare operand in a comparison,
            // emit (counter + 1) to compensate for the 0-based range shift.
            // E.g., `If i > 3` with 0-based `i` becomes `if (i + 1) > 3`.
            if matches!(op, BinaryOpKind::Lt | BinaryOpKind::LtEq | BinaryOpKind::Gt
                | BinaryOpKind::GtEq | BinaryOpKind::Eq | BinaryOpKind::NotEq)
            {
                let left_zb = if let Expr::Identifier(sym) = left {
                    variable_types.get(sym).map_or(false, |t| t == "__zero_based_i64")
                } else { false };
                let right_zb = if let Expr::Identifier(sym) = right {
                    variable_types.get(sym).map_or(false, |t| t == "__zero_based_i64")
                } else { false };
                if left_zb || right_zb {
                    let left_str = if left_zb {
                        format!("({} + 1)", recurse!(left))
                    } else { recurse!(left) };
                    let right_str = if right_zb {
                        format!("({} + 1)", recurse!(right))
                    } else { recurse!(right) };
                    let op_str = match op {
                        BinaryOpKind::Lt => "<", BinaryOpKind::LtEq => "<=",
                        BinaryOpKind::Gt => ">", BinaryOpKind::GtEq => ">=",
                        BinaryOpKind::Eq => "==", BinaryOpKind::NotEq => "!=",
                        _ => unreachable!(),
                    };
                    return format!("({} {} {})", left_str, op_str, right_str);
                }
            }

            let left_str = recurse!(left);
            let right_str = recurse!(right);

            // And/Or are logical: truthiness in, Bool out, short-circuit (Rust's
            // `&&`/`||` keep the right operand lazy). Bool×Bool skips the trait call.
            if matches!(op, BinaryOpKind::And | BinaryOpKind::Or) {
                let op_str = match op {
                    BinaryOpKind::And => "&&",
                    BinaryOpKind::Or => "||",
                    _ => unreachable!(),
                };
                let bools = matches!(
                    infer_logos_type(left, interner, variable_types),
                    crate::analysis::types::LogosType::Bool
                ) && matches!(
                    infer_logos_type(right, interner, variable_types),
                    crate::analysis::types::LogosType::Bool
                );
                return if bools {
                    format!("({} {} {})", left_str, op_str, right_str)
                } else {
                    format!(
                        "(logos_truthy(&({})) {} logos_truthy(&({})))",
                        left_str, op_str, right_str
                    )
                };
            }

            // Dimensioned quantity arithmetic. `+ −` need two quantities (same dimension, checked
            // at runtime), `× ÷` combine dimensions (two quantities) or scale a quantity by an
            // integer; magnitudes stay exact on the rational tower. Comparison/equality use the
            // derived PartialOrd/PartialEq (same-dimension; physical equality).
            if matches!(
                op,
                BinaryOpKind::Add | BinaryOpKind::Subtract | BinaryOpKind::Multiply | BinaryOpKind::Divide
                    | BinaryOpKind::Lt | BinaryOpKind::Gt | BinaryOpKind::LtEq | BinaryOpKind::GtEq
                    | BinaryOpKind::Eq | BinaryOpKind::NotEq
            ) && (is_quantity_expr(left, variable_types, interner)
                || is_quantity_expr(right, variable_types, interner))
            {
                let l_q = is_quantity_expr(left, variable_types, interner);
                let r_q = is_quantity_expr(right, variable_types, interner);
                let code: Option<std::string::String> = match (op, l_q, r_q) {
                    (BinaryOpKind::Add, true, true) => Some(format!("{}.add(&{})", left_str, right_str)),
                    (BinaryOpKind::Subtract, true, true) => Some(format!("{}.sub(&{})", left_str, right_str)),
                    (BinaryOpKind::Multiply, true, true) => Some(format!("{}.mul(&{})", left_str, right_str)),
                    (BinaryOpKind::Divide, true, true) => Some(format!("{}.div_exact(&{})", left_str, right_str)),
                    // Scale a quantity by an integer (the common case), commuting for ×.
                    (BinaryOpKind::Multiply, true, false) => Some(format!("{}.scale_int({})", left_str, right_str)),
                    (BinaryOpKind::Multiply, false, true) => Some(format!("{}.scale_int({})", right_str, left_str)),
                    (BinaryOpKind::Divide, true, false) => Some(format!("{}.div_int({})", left_str, right_str)),
                    // Comparison / equality over two quantities (PartialOrd / physical PartialEq).
                    (BinaryOpKind::Lt, true, true) => Some(format!("({} < {})", left_str, right_str)),
                    (BinaryOpKind::Gt, true, true) => Some(format!("({} > {})", left_str, right_str)),
                    (BinaryOpKind::LtEq, true, true) => Some(format!("({} <= {})", left_str, right_str)),
                    (BinaryOpKind::GtEq, true, true) => Some(format!("({} >= {})", left_str, right_str)),
                    (BinaryOpKind::Eq, true, true) => Some(format!("({} == {})", left_str, right_str)),
                    (BinaryOpKind::NotEq, true, true) => Some(format!("({} != {})", left_str, right_str)),
                    _ => None,
                };
                if let Some(c) = code {
                    return c;
                }
            }

            // Money arithmetic. `+ −` require the same currency, `× ÷` scale by an integer, and a
            // same-currency `Money ÷ Money` is the dimensionless ratio. Mirrors the interpreter.
            if matches!(
                op,
                BinaryOpKind::Add | BinaryOpKind::Subtract | BinaryOpKind::Multiply | BinaryOpKind::Divide
                    | BinaryOpKind::Lt | BinaryOpKind::Gt | BinaryOpKind::LtEq | BinaryOpKind::GtEq
                    | BinaryOpKind::Eq | BinaryOpKind::NotEq
            ) && (is_money_expr(left, variable_types, interner)
                || is_money_expr(right, variable_types, interner))
            {
                let l_m = is_money_expr(left, variable_types, interner);
                let r_m = is_money_expr(right, variable_types, interner);
                let code: Option<std::string::String> = match (op, l_m, r_m) {
                    (BinaryOpKind::Add, true, true) => Some(format!("{}.add(&{})", left_str, right_str)),
                    (BinaryOpKind::Subtract, true, true) => Some(format!("{}.sub(&{})", left_str, right_str)),
                    // Same-currency ratio → an exact Rational.
                    (BinaryOpKind::Divide, true, true) => Some(format!("{}.ratio(&{})", left_str, right_str)),
                    // Scale money by an integer (commutes for ×); split money by an integer.
                    (BinaryOpKind::Multiply, true, false) => Some(format!("{}.scale_int({})", left_str, right_str)),
                    (BinaryOpKind::Multiply, false, true) => Some(format!("{}.scale_int({})", right_str, left_str)),
                    (BinaryOpKind::Divide, true, false) => Some(format!("{}.div_int({})", left_str, right_str)),
                    (BinaryOpKind::Lt, true, true) => Some(format!("({} < {})", left_str, right_str)),
                    (BinaryOpKind::Gt, true, true) => Some(format!("({} > {})", left_str, right_str)),
                    (BinaryOpKind::LtEq, true, true) => Some(format!("({} <= {})", left_str, right_str)),
                    (BinaryOpKind::GtEq, true, true) => Some(format!("({} >= {})", left_str, right_str)),
                    (BinaryOpKind::Eq, true, true) => Some(format!("({} == {})", left_str, right_str)),
                    (BinaryOpKind::NotEq, true, true) => Some(format!("({} != {})", left_str, right_str)),
                    _ => None,
                };
                if let Some(c) = code {
                    return c;
                }
            }

            // Modular (ℤ/nℤ) arithmetic. `+ − × ÷` wrap in the ring when BOTH operands are
            // modular (same modulus; no auto-lift). `÷` multiplies by the modular inverse.
            if matches!(
                op,
                BinaryOpKind::Add | BinaryOpKind::Subtract | BinaryOpKind::Multiply | BinaryOpKind::Divide
            ) && is_modular_expr(left, variable_types, interner)
                && is_modular_expr(right, variable_types, interner)
            {
                let method = match op {
                    BinaryOpKind::Add => "add",
                    BinaryOpKind::Subtract => "sub",
                    BinaryOpKind::Multiply => "mul",
                    BinaryOpKind::Divide => "div_exact",
                    _ => unreachable!(),
                };
                return format!("{}.{}(&{})", left_str, method, right_str);
            }

            // Exact Complex arithmetic. `+ − × ÷` stay an exact `LogosComplex` (the field is
            // closed) the moment one operand is Complex; an integer operand embeds as `re + 0i`.
            if matches!(
                op,
                BinaryOpKind::Add | BinaryOpKind::Subtract | BinaryOpKind::Multiply | BinaryOpKind::Divide
            ) && (is_complex_expr(left, variable_types, interner)
                || is_complex_expr(right, variable_types, interner))
            {
                let l = complex_operand(left, &left_str, variable_types, interner);
                let r = complex_operand(right, &right_str, variable_types, interner);
                let method = match op {
                    BinaryOpKind::Add => "add",
                    BinaryOpKind::Subtract => "sub",
                    BinaryOpKind::Multiply => "mul",
                    BinaryOpKind::Divide => "div_exact",
                    _ => unreachable!(),
                };
                return format!("{}.{}(&{})", l, method, r);
            }

            // Exact Decimal (money) arithmetic. `+ − ×` stay an exact `LogosDecimal` the
            // moment one operand is a Decimal (scale preserved); an integer operand is widened.
            // (`÷` widening to Rational is a separate follow-up; not emitted here.)
            if matches!(op, BinaryOpKind::Add | BinaryOpKind::Subtract | BinaryOpKind::Multiply)
                && (is_decimal_expr(left, variable_types, interner)
                    || is_decimal_expr(right, variable_types, interner))
            {
                let l = decimal_operand(left, &left_str, variable_types, interner);
                let r = decimal_operand(right, &right_str, variable_types, interner);
                let method = match op {
                    BinaryOpKind::Add => "add",
                    BinaryOpKind::Subtract => "sub",
                    BinaryOpKind::Multiply => "mul",
                    _ => unreachable!(),
                };
                return format!("{}.{}(&{})", l, method, r);
            }

            // Decimal division widens to an exact `LogosRational` (base-10 division need not
            // terminate) — `19.99 / 3` is exact, never a lossy float or a floored int.
            if matches!(op, BinaryOpKind::Divide)
                && (is_decimal_expr(left, variable_types, interner)
                    || is_decimal_expr(right, variable_types, interner))
            {
                let l = decimal_operand(left, &left_str, variable_types, interner);
                let r = decimal_operand(right, &right_str, variable_types, interner);
                return format!("{}.to_rational().div_exact(&{}.to_rational())", l, r);
            }

            // Decimal comparison / equality: coerce each operand to `LogosDecimal` (which
            // derives Ord/Eq) so `price > 10` and `a == b` compile exactly, cross-type included.
            if matches!(
                op,
                BinaryOpKind::Lt | BinaryOpKind::Gt | BinaryOpKind::LtEq | BinaryOpKind::GtEq
                    | BinaryOpKind::Eq | BinaryOpKind::NotEq
            ) && (is_decimal_expr(left, variable_types, interner)
                || is_decimal_expr(right, variable_types, interner))
            {
                let l = decimal_operand(left, &left_str, variable_types, interner);
                let r = decimal_operand(right, &right_str, variable_types, interner);
                let op_str = match op {
                    BinaryOpKind::Lt => "<",
                    BinaryOpKind::Gt => ">",
                    BinaryOpKind::LtEq => "<=",
                    BinaryOpKind::GtEq => ">=",
                    BinaryOpKind::Eq => "==",
                    BinaryOpKind::NotEq => "!=",
                    _ => unreachable!(),
                };
                return format!("({} {} {})", l, op_str, r);
            }

            // Complex equality: coerce each operand to `LogosComplex` (derives Eq). Complex
            // has NO order, so relational `< >` are deliberately left to fail to compile.
            if matches!(op, BinaryOpKind::Eq | BinaryOpKind::NotEq)
                && (is_complex_expr(left, variable_types, interner)
                    || is_complex_expr(right, variable_types, interner))
            {
                let l = complex_operand(left, &left_str, variable_types, interner);
                let r = complex_operand(right, &right_str, variable_types, interner);
                let op_str = if matches!(op, BinaryOpKind::Eq) { "==" } else { "!=" };
                return format!("({} {} {})", l, op_str, r);
            }

            // Exact Rational arithmetic. `ExactDivide` only ever appears in a Rational
            // context (the `resolve_divisions` invariant), and `+ − ×` become Rational the
            // moment one operand is. Coerce each operand to `LogosRational` and call the
            // exact method, so `Let x: Rational be 7 / 2` compiles to `7/2`, not floored `3`.
            if matches!(op, BinaryOpKind::ExactDivide)
                || (matches!(
                    op,
                    BinaryOpKind::Add | BinaryOpKind::Subtract | BinaryOpKind::Multiply
                ) && (is_rational_expr(left, variable_types)
                    || is_rational_expr(right, variable_types)))
            {
                let l = rational_operand(left, &left_str, variable_types);
                let r = rational_operand(right, &right_str, variable_types);
                let method = match op {
                    BinaryOpKind::Add => "add",
                    BinaryOpKind::Subtract => "sub",
                    BinaryOpKind::Multiply => "mul",
                    BinaryOpKind::ExactDivide => "div_exact",
                    _ => unreachable!(),
                };
                return format!("{}.{}(&{})", l, method, r);
            }

            // `**` exponentiation has no native Rust operator, so it never
            // reaches the operator-string match below. Integer power is EXACT —
            // overflow promotes to BigInt via `logos_pow_exact` (a negative Int
            // exponent panics loudly, the interp's error). Any Float operand
            // switches to `powf`, widening the Int side to f64. Mirrors
            // `semantics::arith::power`.
            if matches!(op, BinaryOpKind::Pow) {
                let lt = infer_numeric_type(left, interner, variable_types);
                let rt = infer_numeric_type(right, interner, variable_types);
                if lt == "f64" || rt == "f64" {
                    let l = if lt == "f64" { left_str.clone() } else { format!("(({}) as f64)", left_str) };
                    let r = if rt == "f64" { right_str.clone() } else { format!("(({}) as f64)", right_str) };
                    return format!("({}).powf({})", l, r);
                }
                // Both operands constant: fold at compile time. The i64 fast path
                // (`checked_pow`) emits a plain literal; overflow or a negative/oversized
                // exponent falls through to the runtime helper (BigInt promotion, or the
                // loud negative-exponent error), never a giant compile-time BigInt.
                if let (Expr::Literal(Literal::Number(a)), Expr::Literal(Literal::Number(b))) = (left, right) {
                    if let Ok(e) = u32::try_from(*b) {
                        if let Some(v) = a.checked_pow(e) {
                            return format!("{}i64", v);
                        }
                    }
                }
                let tol = ExprCtx { int_exact_tolerant: true, ..*ecx };
                let l = codegen_expr_ctx(left, &tol);
                let r = codegen_expr_ctx(right, &tol);
                // A bare integer-literal operand needs its `i64` suffix: only `i64: Into<LogosInt>`,
                // so a bare `{integer}` (default i32) would fail the `logos_pow_exact` bound.
                let l = if matches!(left, Expr::Literal(Literal::Number(_))) { format!("{}i64", l) } else { l };
                let r = if matches!(right, Expr::Literal(Literal::Number(_))) { format!("{}i64", r) } else { r };
                let inner = format!("logos_pow_exact({}, {})", l, r);
                return if ecx.int_exact_tolerant {
                    inner
                } else {
                    format!("{}.expect_i64(\"Int\")", inner)
                };
            }

            // `//` floor division rounds toward NEGATIVE INFINITY, which Rust's `/`
            // (truncation) does not, so it never reaches the operator-string match
            // below. Integer floor is EXACT — overflow promotes to BigInt via
            // `logos_floordiv_exact` (a zero divisor panics loudly, the interp's
            // error). Any Float operand floors the float quotient, staying f64.
            // Mirrors `semantics::arith::floor_divide`.
            if matches!(op, BinaryOpKind::FloorDivide) {
                let lt = infer_numeric_type(left, interner, variable_types);
                let rt = infer_numeric_type(right, interner, variable_types);
                if lt == "f64" || rt == "f64" {
                    let l = if lt == "f64" { left_str.clone() } else { format!("(({}) as f64)", left_str) };
                    let r = if rt == "f64" { right_str.clone() } else { format!("(({}) as f64)", right_str) };
                    return format!("({} / {}).floor()", l, r);
                }
                // Both operands constant: fold the floored quotient at compile time when
                // it fits i64 (a zero divisor or the `i64::MIN // -1` overflow falls
                // through to the promoting runtime helper).
                if let (Expr::Literal(Literal::Number(a)), Expr::Literal(Literal::Number(b))) = (left, right) {
                    if *b != 0 {
                        if let (Some(q), Some(r)) = (a.checked_div(*b), a.checked_rem(*b)) {
                            let floored = if r != 0 && (r < 0) != (*b < 0) { q - 1 } else { q };
                            return format!("{}i64", floored);
                        }
                    }
                }
                let tol = ExprCtx { int_exact_tolerant: true, ..*ecx };
                let l = codegen_expr_ctx(left, &tol);
                let r = codegen_expr_ctx(right, &tol);
                let l = if matches!(left, Expr::Literal(Literal::Number(_))) { format!("{}i64", l) } else { l };
                let r = if matches!(right, Expr::Literal(Literal::Number(_))) { format!("{}i64", r) } else { r };
                let inner = format!("logos_floordiv_exact({}, {})", l, r);
                return if ecx.int_exact_tolerant {
                    inner
                } else {
                    format!("{}.expect_i64(\"Int\")", inner)
                };
            }

            let op_str = match op {
                BinaryOpKind::Add => "+",
                BinaryOpKind::Subtract => "-",
                BinaryOpKind::Multiply => "*",
                // Floor division (the integer default); `ExactDivide` is handled above.
                BinaryOpKind::Divide | BinaryOpKind::ExactDivide => "/",
                BinaryOpKind::Modulo => "%",
                BinaryOpKind::Eq => "==",
                BinaryOpKind::NotEq => "!=",
                BinaryOpKind::Lt => "<",
                BinaryOpKind::Gt => ">",
                BinaryOpKind::LtEq => "<=",
                BinaryOpKind::GtEq => ">=",
                BinaryOpKind::And | BinaryOpKind::Or => unreachable!(), // handled above
                BinaryOpKind::Concat => unreachable!(), // handled above
                BinaryOpKind::SeqConcat => unreachable!(), // handled above
                BinaryOpKind::ApproxEq => unreachable!(), // handled above
                BinaryOpKind::Pow => unreachable!(), // handled above
                BinaryOpKind::FloorDivide => unreachable!(), // handled above
                BinaryOpKind::BitXor => "^",
                BinaryOpKind::BitAnd => "&",
                BinaryOpKind::BitOr => "|",
                BinaryOpKind::Shl => "<<",
                BinaryOpKind::Shr => ">>",
            };

            // Mixed Float*Int handling. ARITHMETIC promotes the Int side to
            // f64 (float math is the result type). COMPARISON is EXACT —
            // mathematical values via `logos_cmp_i64_f64`, never a lossy cast
            // (`i as f64` rounds above 2^53, which would call
            // `9007199254740993` equal to the float `9007199254740992.0`).
            // Same-type compares are untouched: hot loops pay nothing.
            if !matches!(op, BinaryOpKind::And | BinaryOpKind::Or) {
                let left_type = infer_numeric_type(left, interner, variable_types);
                let right_type = infer_numeric_type(right, interner, variable_types);
                let is_cmp = matches!(
                    op,
                    BinaryOpKind::Eq
                        | BinaryOpKind::NotEq
                        | BinaryOpKind::Lt
                        | BinaryOpKind::Gt
                        | BinaryOpKind::LtEq
                        | BinaryOpKind::GtEq
                );
                // `(int_expr, float_expr)` operand order for the exact helper;
                // `flip` marks that the FLOAT was on the left, so the ordering
                // reverses (`f < i` ⟺ `cmp(i, f) == Greater`).
                let mixed_cmp = if is_cmp && left_type == "f64" && right_type == "i64" {
                    Some((right_str.clone(), left_str.clone(), true))
                } else if is_cmp && left_type == "i64" && right_type == "f64" {
                    Some((left_str.clone(), right_str.clone(), false))
                } else {
                    None
                };
                if let Some((int_e, float_e, flip)) = mixed_cmp {
                    let pat = match (op, flip) {
                        (BinaryOpKind::Lt, false) | (BinaryOpKind::Gt, true) => "Some(core::cmp::Ordering::Less)",
                        (BinaryOpKind::Gt, false) | (BinaryOpKind::Lt, true) => "Some(core::cmp::Ordering::Greater)",
                        (BinaryOpKind::LtEq, false) | (BinaryOpKind::GtEq, true) => {
                            "Some(core::cmp::Ordering::Less | core::cmp::Ordering::Equal)"
                        }
                        (BinaryOpKind::GtEq, false) | (BinaryOpKind::LtEq, true) => {
                            "Some(core::cmp::Ordering::Greater | core::cmp::Ordering::Equal)"
                        }
                        (BinaryOpKind::Eq, _) => {
                            return format!("logos_i64_eq_f64({}, {})", int_e, float_e);
                        }
                        (BinaryOpKind::NotEq, _) => {
                            return format!("(!logos_i64_eq_f64({}, {}))", int_e, float_e);
                        }
                        _ => unreachable!("is_cmp covers exactly the six comparison ops"),
                    };
                    return format!("matches!(logos_cmp_i64_f64({}, {}), {})", int_e, float_e, pat);
                }
                if left_type == "f64" && right_type != "f64" {
                    return format!("({} {} (({}) as f64))", left_str, op_str, right_str);
                } else if right_type == "f64" && left_type != "f64" {
                    return format!("((({}) as f64) {} {})", left_str, op_str, right_str);
                }

                // Under a `wordN(...)` truncation the result is mod 2^N, so Int add/sub/mul lower to raw
                // i64 `wrapping_*` — identical low bits to the checked-exact helper, at a fraction of the
                // cost (the MD5 byte→word schedule was ~half the runtime as `logos_*_exact`). The flag
                // propagates down the additive chain (recurse with `ecx`) and is reset at call boundaries.
                if ecx.int_wrapping
                    && left_type == "i64"
                    && right_type == "i64"
                    && !ecx.int_index_context
                    && matches!(op, BinaryOpKind::Add | BinaryOpKind::Subtract | BinaryOpKind::Multiply)
                {
                    let method = match op {
                        BinaryOpKind::Add => "wrapping_add",
                        BinaryOpKind::Subtract => "wrapping_sub",
                        BinaryOpKind::Multiply => "wrapping_mul",
                        _ => unreachable!("matches! guards exactly these three"),
                    };
                    let l = codegen_expr_ctx(left, ecx);
                    let r = codegen_expr_ctx(right, ecx);
                    // Pin the receiver to `i64`: the codegen already proved both sides are i64, but a
                    // syntactically-unconstrained operand (e.g. a loop counter `for i in 1..=8` whose
                    // only use is this op) is an ambiguous `{integer}` in the emitted Rust, and
                    // `wrapping_add` exists on every integer type → E0689. `as i64` is a no-op on an
                    // already-i64 value and resolves the method. The result is i64, as before.
                    return format!("(({}) as i64).{}({})", l, method, r);
                }

                // Overflow ruling v2 (stage 2): compiled Int arithmetic is EXACT.
                // Constant operands promote at COMPILE time; Oracle-proven-in-range
                // arithmetic stays raw i64 (hot loops pay nothing); everything else
                // emits the checked-exact helper — an inlined checked op whose
                // overflow spills to the promoting `LogosInt`. `%` rides along for
                // its own rims (zero divisor message, `i64::MIN % -1`).
                let is_exactable = matches!(
                    op,
                    BinaryOpKind::Add
                        | BinaryOpKind::Subtract
                        | BinaryOpKind::Multiply
                        | BinaryOpKind::Divide
                        | BinaryOpKind::Modulo
                );
                // A `LogosInt`-typed (promoted) operand forces the exact helper even when the
                // OTHER operand's type is unknown (e.g. a `while`→`for` loop counter that was never
                // registered): the result must stay `LogosInt`, and a raw `LogosInt * i64` has no
                // operator. `logos_*_exact` takes `impl Into<LogosInt>`, so a bare `i64` flows in.
                let has_bigint_operand = mentions_bigint_var(left, variable_types)
                    || mentions_bigint_var(right, variable_types);
                if is_exactable
                    && (left_type == "i64" && right_type == "i64" || has_bigint_operand)
                    && !ecx.int_index_context
                {
                    let narrow = |e: String| {
                        if ecx.int_exact_tolerant {
                            e
                        } else {
                            format!("{}.expect_i64(\"Int\")", e)
                        }
                    };
                    // `/` and `%` by a NONZERO literal can never overflow i64
                    // (a remainder is bounded by the divisor; a quotient shrinks
                    // — the sole exception `i64::MIN / -1` is excluded) and can
                    // never divide by zero, so raw matches the interpreter and
                    // keeps the mod-pow2 strength reduction (`x % 1024 → x & 1023`)
                    // legible. A zero (or `-1` for `/`) literal falls through to
                    // the checked helper for the canonical loud error.
                    if let Expr::Literal(Literal::Number(d)) = right {
                        let safe_div = matches!(op, BinaryOpKind::Divide) && *d != 0 && *d != -1;
                        let safe_mod = matches!(op, BinaryOpKind::Modulo) && *d != 0;
                        // A promoted (`LogosInt`) dividend has no raw `/`/`%` operator and may exceed
                        // i64, so it must use the exact helper below rather than this raw fast-path.
                        if (safe_div || safe_mod) && !has_bigint_operand {
                            // The raw i64 `/`/`%` needs an i64 DIVIDEND. In a tolerant context `left_str` is
                            // a promoted `LogosInt` (no `Div<i64>`/`Rem<i64>`) — e.g. `(a + b) / 64` inside
                            // an exact op — so recompute the dividend as i64; the i64 result flows back into
                            // the enclosing exact op via `Into`. (The divisor `d` is a literal, always i64.)
                            let l = if ecx.int_exact_tolerant {
                                codegen_expr_ctx(left, &ExprCtx { int_exact_tolerant: false, ..*ecx })
                            } else {
                                left_str.clone()
                            };
                            return format!("({} {} {})", l, op_str, right_str);
                        }
                    }
                    if let (Expr::Literal(Literal::Number(a)), Expr::Literal(Literal::Number(b))) =
                        (left, right)
                    {
                        match const_exact_int(*op, *a, *b) {
                            Some(ConstExact::InRange) => {
                                return format!("({} {} {})", left_str, op_str, right_str);
                            }
                            Some(ConstExact::Promoted(lit)) => return narrow(lit),
                            // A constant zero divisor falls to the runtime
                            // helper: the loud canonical error, the interp's.
                            None => {}
                        }
                    } else if !has_bigint_operand && oracle_proves_int_op_in_range(ecx, expr, *op, left, right) {
                        // A raw `left op right` needs both sides to be i64. With a promoted operand
                        // (`LogosInt`, no arithmetic operators) the exact helper below is the only
                        // valid lowering, even when the result provably fits i64.
                        return format!("({} {} {})", left_str, op_str, right_str);
                    }
                    // A root-NARROWED op (non-tolerant, no promoted operand)
                    // has two register-shaped exact lowerings that skip the
                    // `LogosInt` round-trip entirely; a TOLERANT root must
                    // stay on the promoting chain so an oversized
                    // intermediate flows exact into the enclosing op.
                    if !ecx.int_exact_tolerant && !has_bigint_operand {
                        if let Some(fused) = try_fuse_narrowed_int_op(*op, left, right, ecx) {
                            return fused;
                        }
                        if let Some(dual) = try_guarded_dual_chain(*op, left, right, ecx) {
                            return dual;
                        }
                        if let Some(chain) = try_i128_chain(*op, left, right, ecx) {
                            return chain;
                        }
                    }
                    let helper = match op {
                        BinaryOpKind::Add => "logos_add_exact",
                        BinaryOpKind::Subtract => "logos_sub_exact",
                        BinaryOpKind::Multiply => "logos_mul_exact",
                        BinaryOpKind::Divide => "logos_div_exact",
                        BinaryOpKind::Modulo => "logos_rem_exact",
                        _ => unreachable!("is_exactable covers exactly these five"),
                    };
                    // Exactness flows DOWN the operand chain: intermediates stay
                    // `LogosInt` (Into-chained), narrowing happens once at the root.
                    let tol = ExprCtx { int_exact_tolerant: true, ..*ecx };
                    let l = codegen_expr_ctx(left, &tol);
                    let r = codegen_expr_ctx(right, &tol);
                    return narrow(format!("{}({}, {})", helper, l, r));
                }
            }

            format!("({} {} {})", left_str, op_str, right_str)
        }

        Expr::Call { function, args } => {
            let func_name = names.ident(*function);
            let raw_name = names.raw(*function);
            // Callee param-role indices (packed one slot per function): a readonly
            // borrow (`&[T]`) and a value-semantics `mutable` collection param can
            // BOTH be present, so read each role independently rather than assuming
            // one clobbers the other.
            let callee_slot = variable_types.get(function);
            let callee_borrow_indices: HashSet<usize> =
                super::fn_role_indices(callee_slot, super::FnRole::Borrow);
            // `mutable` collection params (value semantics): pass the caller's
            // collection by shared reference (no clone) so the callee mutates it
            // in place — the explicit propagation escape hatch.
            let callee_value_mutable: HashSet<usize> =
                super::fn_role_indices(callee_slot, super::FnRole::ValueMutable);
            // Recursively codegen args with full context.
            // Borrow params: pass &name (or pass through if already a slice).
            // Non-borrow params: clone non-Copy identifiers to avoid move-after-use.
            // A `wordN(...)` argument is codegen'd in `int_wrapping` mode (its Int arithmetic is mod 2^N);
            // every OTHER call resets the flag — a callee's argument is a full-width value, not truncated,
            // so `wordN(f(a * b))` must NOT wrap `a * b`.
            let arg_wrapping = matches!(raw_name, "word8" | "word16" | "word32" | "word64");
            let args_str: Vec<String> = args.iter()
                .enumerate()
                .map(|(i, a)| {
                    let s = codegen_expr_ctx(a, &ExprCtx { int_wrapping: arg_wrapping, ..*ecx });
                    if callee_value_mutable.contains(&i) {
                        // Pass the caller's collection by shared reference (no clone)
                        // so the callee mutates it in place. If the arg is already a
                        // reference (a `mutable` param threaded through a nested call),
                        // pass it as-is rather than re-referencing.
                        if let Expr::Identifier(sym) = a {
                            if variable_types.get(sym).is_some_and(|t| t.starts_with('&')) {
                                return s;
                            }
                        }
                        return format!("&{}", s);
                    }
                    if callee_borrow_indices.contains(&i) {
                        // Borrow param: pass &[T] reference instead of cloning
                        if let Expr::Identifier(sym) = a {
                            if let Some(ty) = variable_types.get(sym) {
                                let ty = ty.split("|__hl:").next().unwrap_or(ty.as_str());
                                if ty.starts_with("&[") || ty.starts_with("&mut [") {
                                    return s; // Already a slice — pass through
                                }
                                if ty.starts_with("Vec<") {
                                    return format!("&{}", s); // Vec<T> derefs to &[T]
                                }
                                if ty.starts_with('[') {
                                    return format!("&{}", s); // fixed array [T; N] coerces to &[T]
                                }
                                if ty.starts_with("LogosSeq") {
                                    return format!("&*{}.borrow()", s);
                                }
                            }
                            // Unknown type at borrow position — default to LogosSeq conversion.
                            // All LOGOS Seq variables are LogosSeq<T> unless materialized as Vec<T>
                            // (which is always tracked). Safe because borrow positions only accept Seq<T>.
                            return format!("&*{}.borrow()", s);
                        }
                        // Non-identifier at borrow position (e.g. list literal, function call).
                        // The expression evaluates to LogosSeq<T>; borrow the temporary.
                        format!("&*{}.borrow()", s)
                    } else {
                        // Regular param: clone non-Copy identifiers
                        if let Expr::Identifier(sym) = a {
                            // A `LogosInt` (promoted) var passed to a scalar `i64` param narrows to
                            // the param's declared width — a loud panic if it exceeds i64 (the
                            // i64-param contract). Functions do not yet take `LogosInt` params.
                            if variable_types.get(sym).map_or(false, |t| t.contains("__bigint")) {
                                return format!("{}.expect_i64(\"Int\")", names.ident(*sym));
                            }
                            if let Some(ty) = variable_types.get(sym) {
                                if !is_copy_type(ty) {
                                    return format!("{}.clone()", s);
                                }
                            } else {
                                // Unknown type (e.g. pattern-bound variable from Inspect):
                                // clone defensively to avoid move-after-use in loops
                                return format!("{}.clone()", s);
                            }
                        }
                        s
                    }
                })
                .collect();
            // Builtin math functions → Rust method call syntax
            match raw_name {
                "sqrt" if args_str.len() == 1 => {
                    format!("(({}) as f64).sqrt()", args_str[0])
                }
                // `{k: v, …}` map literal — flat pairs into an insertion-ordered
                // LogosMap (block expression; `insert` takes `&self`).
                "mapOf" if !args_str.is_empty() && args_str.len() % 2 == 0 => {
                    let mut s = String::from("{ let __map_lit = LogosMap::new(); ");
                    for pair in args_str.chunks(2) {
                        s.push_str(&format!("__map_lit.insert({}, {}); ", pair[0], pair[1]));
                    }
                    s.push_str("__map_lit }");
                    s
                }
                // `repeatSeq(x, n)` — the `n copies of x` fill: n independent
                // `fill_clone` slots (the element evaluates once).
                "repeatSeq" if args_str.len() == 2 => {
                    format!(
                        "{{ let __rp_x = ({}); let __rp_n = (({}) as i64).max(0) as usize; \
                         let mut __rp = Vec::with_capacity(__rp_n); \
                         for _ in 0..__rp_n {{ __rp.push(__rp_x.fill_clone()); }} \
                         LogosSeq::from_vec(__rp) }}",
                        args_str[0], args_str[1]
                    )
                }
                // `{a, b, …}` set literal — elements into the Set repr
                // (hash-set semantics: dedup by value equality).
                "setOf" if !args_str.is_empty() => {
                    let mut s = String::from("{ let mut __set_lit = Set::default(); ");
                    for a in &args_str {
                        s.push_str(&format!("__set_lit.insert({}); ", a));
                    }
                    s.push_str("__set_lit }");
                    s
                }
                // Exact base-10 money: parse the literal text into a `LogosDecimal`. The
                // `.to_string()` accepts the arg whether it lowered to `&str` or `String`.
                "decimal" if args_str.len() == 1 => {
                    format!("LogosDecimal::parse(&({}).to_string())", args_str[0])
                }
                // Exact complex `re + im·i` from two exact reals (integers embed as `n/1`).
                "complex" if args_str.len() == 2 => {
                    let re = rational_operand(&args[0], &args_str[0], variable_types);
                    let im = rational_operand(&args[1], &args_str[1], variable_types);
                    format!("LogosComplex::new({}, {})", re, im)
                }
                // A ℤ/nℤ element from a value and a modulus (both integers).
                "modular" if args_str.len() == 2 => {
                    format!("LogosModular::new(({}) as i64, ({}) as i64)", args_str[0], args_str[1])
                }
                // A dimensioned quantity from an exact magnitude and a unit name (`quantity(2, "inch")`).
                // The magnitude rides the rational tower (integers embed as `n/1`), so it never truncates.
                "quantity" if args_str.len() == 2 => {
                    let mag = rational_operand(&args[0], &args_str[0], variable_types);
                    format!("LogosQuantity::from_rational({}, &({}).to_string())", mag, args_str[1])
                }
                // Re-express a quantity in another unit of the same dimension (`convert(q, "foot")`).
                "convert" if args_str.len() == 2 => {
                    format!("({}).convert(&({}).to_string())", args_str[0], args_str[1])
                }
                // Construct money: a Decimal amount → `LogosMoney::of`, an Int amount → `from_i64`.
                "money" if args_str.len() == 2 => {
                    if is_decimal_expr(&args[0], variable_types, interner) {
                        let amt = decimal_operand(&args[0], &args_str[0], variable_types, interner);
                        format!("LogosMoney::of({}, &({}).to_string())", amt, args_str[1])
                    } else {
                        format!("LogosMoney::from_i64({}, &({}).to_string())", args_str[0], args_str[1])
                    }
                }
                // UUID — parse, the two special ids, the well-known namespaces, name-based v3/v5, and
                // the version accessor. All lower to `LogosUuid` associated functions.
                "uuid" if args_str.len() == 1 => {
                    format!("LogosUuid::parse(&({}).to_string())", args_str[0])
                }
                "uuid_nil" if args_str.is_empty() => "LogosUuid::nil()".to_string(),
                "uuid_max" if args_str.is_empty() => "LogosUuid::max()".to_string(),
                "uuid_dns" if args_str.is_empty() => "LogosUuid::namespace_dns()".to_string(),
                "uuid_url" if args_str.is_empty() => "LogosUuid::namespace_url()".to_string(),
                "uuid_oid" if args_str.is_empty() => "LogosUuid::namespace_oid()".to_string(),
                "uuid_x500" if args_str.is_empty() => "LogosUuid::namespace_x500()".to_string(),
                "uuid_version" if args_str.len() == 1 => format!("({}).version()", args_str[0]),
                // Byte-level primitives backing the Logos-written uuid.lg constructors. (`md5`/`sha1`/
                // `uuid_v3`/`uuid_v5` are Logos stdlib functions now, emitted as normal calls.)
                "text_bytes" if args_str.len() == 1 => format!("text_bytes(&({}))", args_str[0]),
                "text_from_bytes" if args_str.len() == 1 => format!("text_from_bytes(&({}))", args_str[0]),
                "readWireProgram" if args_str.is_empty() => {
                    "{ use std::io::Read as _; let mut __len = [0u8; 4]; if std::io::stdin().read_exact(&mut __len).is_err() { std::process::exit(0); } let __n = u32::from_le_bytes(__len) as usize; let mut __wb = vec![0u8; __n]; std::io::stdin().read_exact(&mut __wb).expect(\"readWireProgram: frame\"); <CProgram as logicaffeine_data::wire::WireDecode>::wire_decode(&__wb, &mut 0usize).expect(\"readWireProgram: decode\") }".to_string()
                }
                "writeWireResidual" if args_str.len() == 1 => {
                    format!("{{ use std::io::Write as _; let __s: String = ({}).into(); let __b = __s.as_bytes(); let __o = std::io::stdout(); let mut __h = __o.lock(); __h.write_all(&(__b.len() as u32).to_le_bytes()).unwrap(); __h.write_all(__b).unwrap(); __h.flush().unwrap(); __b.len() as i64 }}", args_str[0])
                }
                "uuid_bytes" if args_str.len() == 1 => format!("({}).byte_seq()", args_str[0]),
                "uuid_from_bytes" if args_str.len() == 1 => {
                    format!("LogosUuid::from_byte_seq(&({}))", args_str[0])
                }
                // `chr(code)` → a one-char String (the char code → text; used building the canonical
                // UUID hex in Logos). Fully qualified — it lives in the `text` submodule, not the glob.
                "chr" if args_str.len() == 1 => {
                    format!("logicaffeine_system::text::chr(({}) as i64)", args_str[0])
                }
                // Parse an RFC 3339 timestamp into a `LogosMoment` (delegates to base::temporal).
                "parse_timestamp" if args_str.len() == 1 => {
                    format!("LogosMoment::parse_rfc3339(&({}).to_string())", args_str[0])
                }
                // Render a `LogosMoment` as an RFC 3339 string.
                "format_timestamp" if args_str.len() == 1 => {
                    format!("({}).format_rfc3339()", args_str[0])
                }
                // UTC calendar component extractors on a `LogosMoment`.
                "year_of" if args_str.len() == 1 => format!("({}).year()", args_str[0]),
                "month_of" if args_str.len() == 1 => format!("({}).month()", args_str[0]),
                "day_of" if args_str.len() == 1 => format!("({}).day()", args_str[0]),
                "weekday_of" if args_str.len() == 1 => format!("({}).weekday()", args_str[0]),
                "hour_of" if args_str.len() == 1 => format!("({}).hour()", args_str[0]),
                "minute_of" if args_str.len() == 1 => format!("({}).minute()", args_str[0]),
                "second_of" if args_str.len() == 1 => format!("({}).second()", args_str[0]),
                // The ISO-8601 week number (1..=53) of the Moment/Date.
                "week_of" if args_str.len() == 1 => format!("({}).iso_week()", args_str[0]),
                // The calendar quarter (1..=4) of the Moment/Date.
                "quarter_of" if args_str.len() == 1 => format!("({}).quarter()", args_str[0]),
                // The calendar day (a `LogosDate`) the Moment falls on.
                "date_of" if args_str.len() == 1 => format!("({}).date()", args_str[0]),
                // The wall-clock time-of-day (a `LogosTime`) of the Moment.
                "time_of" if args_str.len() == 1 => format!("({}).time_of_day()", args_str[0]),
                // Moment arithmetic.
                "seconds_between" if args_str.len() == 2 => {
                    format!("({}).seconds_until(&({}))", args_str[0], args_str[1])
                }
                "months_between" if args_str.len() == 2 => {
                    format!("({}).months_until(&({}))", args_str[0], args_str[1])
                }
                "years_between" if args_str.len() == 2 => {
                    format!("({}).years_until(&({}))", args_str[0], args_str[1])
                }
                "add_seconds" if args_str.len() == 2 => {
                    format!("({}).add_seconds({})", args_str[0], args_str[1])
                }
                "in_zone" if args_str.len() == 2 => {
                    format!("({}).in_zone(&({}).to_string())", args_str[0], args_str[1])
                }
                "local_instant" if args_str.len() == 2 => {
                    format!("({}).local_instant(&({}).to_string())", args_str[0], args_str[1])
                }
                // SHA-1 SHA-NI lane (`Lanes4Word32` = one `__m128i`): pack/unpack + the four SHA ops,
                // which lower to the `sha1rnds4`/`sha1msg1/2`/`sha1nexte` hardware instructions.
                "lanes4Word32" if args_str.len() == 1 => {
                    format!("lanes4_word32(&({}))", args_str[0])
                }
                "lanes4Of" if args_str.len() == 4 => {
                    format!("lanes4_of({}, {}, {}, {})", args_str[0], args_str[1], args_str[2], args_str[3])
                }
                // Byte-shuffle lane (`Lanes16Word8`): pshufb + per-byte shift + interleaves — the SIMD
                // hex codec written in Logos lowers to these.
                "lanes16Word8" if args_str.len() == 1 => format!("lanes16_word8(&({}))", args_str[0]),
                "seqOfLanes16W8" if args_str.len() == 1 => format!("seq_of_lanes16w8({})", args_str[0]),
                "splat16Word8" if args_str.len() == 1 => format!("splat16_word8({})", args_str[0]),
                "shuffle16" if args_str.len() == 2 => {
                    format!("shuffle16({}, {})", args_str[0], args_str[1])
                }
                "shrBytes16" if args_str.len() == 2 => {
                    format!("shr_bytes16({}, {})", args_str[0], args_str[1])
                }
                "interleaveLo16" if args_str.len() == 2 => {
                    format!("interleave_lo16({}, {})", args_str[0], args_str[1])
                }
                "interleaveHi16" if args_str.len() == 2 => {
                    format!("interleave_hi16({}, {})", args_str[0], args_str[1])
                }
                "byteAdd16" if args_str.len() == 2 => {
                    format!("byte_add16({}, {})", args_str[0], args_str[1])
                }
                "maddubs16" if args_str.len() == 2 => {
                    format!("maddubs16({}, {})", args_str[0], args_str[1])
                }
                "packus16" if args_str.len() == 2 => {
                    format!("packus16({}, {})", args_str[0], args_str[1])
                }
                "seqOfLanes4W32" if args_str.len() == 1 => {
                    format!("seq_of_lanes4w32({})", args_str[0])
                }
                "sha1rnds4" if args_str.len() == 3 => {
                    format!("sha1rnds4({}, {}, {})", args_str[0], args_str[1], args_str[2])
                }
                "sha1msg1" if args_str.len() == 2 => {
                    format!("sha1msg1({}, {})", args_str[0], args_str[1])
                }
                "sha1msg2" if args_str.len() == 2 => {
                    format!("sha1msg2({}, {})", args_str[0], args_str[1])
                }
                "sha1nexte" if args_str.len() == 2 => {
                    format!("sha1nexte({}, {})", args_str[0], args_str[1])
                }
                // SIMD lane vector: pack a Seq of Word32 into one `__m256i`, or read its lanes back.
                "lanes8Word32" if args_str.len() == 1 => {
                    format!("lanes8_word32(&({}))", args_str[0])
                }
                "seqOfLanes8" if args_str.len() == 1 => {
                    format!("seq_of_lanes8({})", args_str[0])
                }
                "splat8Word32" if args_str.len() == 1 => {
                    format!("splat8_word32({})", args_str[0])
                }
                "intOfWord32" if args_str.len() == 1 => {
                    format!("int_of_word32({})", args_str[0])
                }
                "intOfWord64" if args_str.len() == 1 => {
                    format!("int_of_word64({})", args_str[0])
                }
                "word64Shl" if args_str.len() == 2 => {
                    format!("word64_shl({}, {})", args_str[0], args_str[1])
                }
                "word64Shr" if args_str.len() == 2 => {
                    format!("word64_shr({}, {})", args_str[0], args_str[1])
                }
                "word32Shr" if args_str.len() == 2 => {
                    format!("word32_shr({}, {})", args_str[0], args_str[1])
                }
                "word64And" if args_str.len() == 2 => {
                    format!("word64_and({}, {})", args_str[0], args_str[1])
                }
                "word16" if args_str.len() == 1 => {
                    format!("word16({})", args_str[0])
                }
                "intOfWord16" if args_str.len() == 1 => {
                    format!("int_of_word16({})", args_str[0])
                }
                "lanes4Word64" if args_str.len() == 1 => {
                    format!("lanes4_word64(&({}))", args_str[0])
                }
                "seqOfLanes4" if args_str.len() == 1 => {
                    format!("seq_of_lanes4({})", args_str[0])
                }
                "mul32x32to64" if args_str.len() == 2 => {
                    format!("mul32x32to64({}, {})", args_str[0], args_str[1])
                }
                "hsumLanes4" if args_str.len() == 1 => {
                    format!("hsum_lanes4({})", args_str[0])
                }
                "splat4Word64" if args_str.len() == 1 => {
                    format!("splat4_word64({})", args_str[0])
                }
                "andNot4" if args_str.len() == 2 => {
                    format!("and_not4({}, {})", args_str[0], args_str[1])
                }
                "lanes16Word16" if args_str.len() == 1 => {
                    format!("lanes16_word16(&({}))", args_str[0])
                }
                "seqOfLanes16" if args_str.len() == 1 => {
                    format!("seq_of_lanes16({})", args_str[0])
                }
                "splat16Word16" if args_str.len() == 1 => {
                    format!("splat16_word16({})", args_str[0])
                }
                "mulhi16" if args_str.len() == 2 => {
                    format!("mulhi16({}, {})", args_str[0], args_str[1])
                }
                "montmul32" if args_str.len() == 4 => {
                    format!("montmul32({}, {}, {}, {})", args_str[0], args_str[1], args_str[2], args_str[3])
                }
                // Method calls so Rust resolves the right lane impl by type (Lanes16Word16 i16 NTT
                // strides 8/4/2; Lanes8Word32 i32 NTT strides 4/2/1).
                "nttBcastLo" if args_str.len() == 2 => {
                    format!("({}).ntt_bcast_lo(({}) as usize)", args_str[0], args_str[1])
                }
                "nttBcastHi" if args_str.len() == 2 => {
                    format!("({}).ntt_bcast_hi(({}) as usize)", args_str[0], args_str[1])
                }
                "nttBlend" if args_str.len() == 3 => {
                    format!("({}).ntt_blend({}, ({}) as usize)", args_str[0], args_str[1], args_str[2])
                }
                // Modular exponentiation: `pow(m, e)` where the base is a ℤ/nℤ element.
                "pow" if args_str.len() == 2 && is_modular_expr(&args[0], variable_types, interner) => {
                    format!("({}).pow(({}) as u64)", args_str[0], args_str[1])
                }
                // A Rational argument takes the EXACT path (BigInt num/den), never a
                // lossy `as f64`: `|·|` stays rational; floor/ceil/round give the exact Int.
                "abs" if args_str.len() == 1 && is_rational_expr(&args[0], variable_types) => {
                    format!("({}).abs()", args_str[0])
                }
                "floor" if args_str.len() == 1 && is_rational_expr(&args[0], variable_types) => {
                    format!("({}).floor()", args_str[0])
                }
                "ceil" if args_str.len() == 1 && is_rational_expr(&args[0], variable_types) => {
                    format!("({}).ceil()", args_str[0])
                }
                "round" if args_str.len() == 1 && is_rational_expr(&args[0], variable_types) => {
                    format!("({}).round()", args_str[0])
                }
                "abs" if args_str.len() == 1 => {
                    let arg_type = infer_numeric_type(&args[0], interner, variable_types);
                    if arg_type == "f64" {
                        format!("(({}) as f64).abs()", args_str[0])
                    } else {
                        format!("(({}) as i64).abs()", args_str[0])
                    }
                }
                "floor" if args_str.len() == 1 => {
                    format!("((({}) as f64).floor() as i64)", args_str[0])
                }
                "ceil" if args_str.len() == 1 => {
                    format!("((({}) as f64).ceil() as i64)", args_str[0])
                }
                "round" if args_str.len() == 1 => {
                    format!("((({}) as f64).round() as i64)", args_str[0])
                }
                "pow" if args_str.len() == 2 => {
                    format!("((({}) as f64).powf(({}) as f64))", args_str[0], args_str[1])
                }
                "min" if args_str.len() == 2 => {
                    format!("({}).min({})", args_str[0], args_str[1])
                }
                "max" if args_str.len() == 2 => {
                    format!("({}).max({})", args_str[0], args_str[1])
                }
                _ => {
                    // Add .await if this function is async
                    if async_functions.contains(function) {
                        format!("{}({}).await", func_name, args_str.join(", "))
                    } else {
                        format!("{}({})", func_name, args_str.join(", "))
                    }
                }
            }
        }

        // Affine read-only array (deleted CSR offset array): `item k of A` is the
        // closed form `coeff * (k-1) + offset` — no load, no array. The 1-based→
        // 0-based `-1` cancels a `+1` in the index, so `item (v+1) of A` with
        // `A[p]=5*p` becomes `v * 5` (C's `v*MAX_EDGES`).
        Expr::Index { collection: Expr::Identifier(sym), index }
            if affine_array_coeff_offset(variable_types.get(sym)).is_some() =>
        {
            let (coeff, offset) = affine_array_coeff_offset(variable_types.get(sym)).unwrap();
            enum Pos {
                Const(i64),
                Expr(String),
            }
            let pos = match index {
                Expr::Literal(Literal::Number(n)) => Pos::Const(n - 1),
                Expr::BinaryOp { op: BinaryOpKind::Add, left, right } => match (left, right) {
                    (_, Expr::Literal(Literal::Number(1))) => Pos::Expr(recurse!(left)),
                    (Expr::Literal(Literal::Number(1)), _) => Pos::Expr(recurse!(right)),
                    (_, Expr::Literal(Literal::Number(k))) if *k > 1 => {
                        Pos::Expr(format!("({} + {})", recurse!(left), k - 1))
                    }
                    (Expr::Literal(Literal::Number(k)), _) if *k > 1 => {
                        Pos::Expr(format!("({} + {})", recurse!(right), k - 1))
                    }
                    _ => Pos::Expr(format!("(({}) - 1)", irecurse!(index))),
                },
                _ => Pos::Expr(format!("(({}) - 1)", irecurse!(index))),
            };
            match pos {
                Pos::Const(p) => format!("{}i64", coeff.wrapping_mul(p).wrapping_add(offset)),
                Pos::Expr(_) if coeff == 0 => format!("{}i64", offset),
                Pos::Expr(p) => {
                    let base = if coeff == 1 { p } else { format!("(({}) * {})", p, coeff) };
                    if offset == 0 {
                        base
                    } else {
                        format!("({} + {})", base, offset)
                    }
                }
            }
        }

        // AoS interleaving: `item i of member` reads column `col` of the fused
        // backing's `(i-1)`-th row — `backing[(i-1)][col]`. Adjacent columns are
        // memory-adjacent, so LLVM packs them (C's struct-array load).
        Expr::Index { collection: Expr::Identifier(sym), index }
            if parse_aos_tag(variable_types.get(sym)).is_some() =>
        {
            let tag = parse_aos_tag(variable_types.get(sym)).unwrap();
            let row = match index {
                Expr::Literal(Literal::Number(1)) => "0".to_string(),
                Expr::Literal(Literal::Number(n)) => format!("{}", n - 1),
                Expr::BinaryOp { op: BinaryOpKind::Add, left, right } => match (left, right) {
                    (_, Expr::Literal(Literal::Number(1))) => format!("({}) as usize", recurse!(left)),
                    (Expr::Literal(Literal::Number(1)), _) => format!("({}) as usize", recurse!(right)),
                    _ => format!("(({}) - 1) as usize", irecurse!(index)),
                },
                _ => format!("(({}) - 1) as usize", irecurse!(index)),
            };
            format!("{}[{}][{}]", tag.backing, row, tag.col)
        }

        Expr::Index { collection, index } => {
            let coll_str = recurse!(collection);
            // Direct indexing for known collection types (avoids trait dispatch)
            // Strip |__hl: hoisting suffix so type parsing (strip_suffix, etc.) works correctly.
            let known_type = if let Expr::Identifier(sym) = collection {
                variable_types.get(sym).map(|s| s.split("|__hl:").next().unwrap_or(s.as_str()))
            } else {
                None
            };
            // A NEGATIVE literal index is end-relative (`item -1 of xs` is the
            // last element). The direct-`[...]` fast paths below assume a
            // positive 1-based index and would compute `(-1 - 1) as usize` — a
            // giant offset. Route it through the `LogosIndex` trait, whose
            // `resolve_logos_index` carries the one end-relative rule shared
            // with the interpreter. (A narrowed `Vec<i32>` sign-extends.)
            if let Expr::Literal(Literal::Number(n)) = index {
                if *n < 0 {
                    if let Some(t) = known_type {
                        let idx_capable = t.starts_with("LogosSeq")
                            || t.starts_with("Vec")
                            || t.starts_with("&[")
                            || t.starts_with("&mut [")
                            || t.starts_with('[')
                            || t == "String";
                        if idx_capable {
                            let read = format!(
                                "logicaffeine_data::LogosIndex::logos_get(&{}, {}i64)",
                                coll_str, n
                            );
                            return if t == "Vec<i32>" {
                                format!("(({}) as i64)", read)
                            } else {
                                read
                            };
                        }
                    }
                }
            }
            match known_type {
                Some(t) if t.starts_with("LogosSeq") || t.starts_with("Vec") => {
                    let is_logos_seq = t.starts_with("LogosSeq");
                    let suffix = if has_copy_element_type(t) { "" } else { ".clone()" };
                    // OPT-8: Check if index is a zero-based counter (already 0-based, no -1 needed)
                    let is_zero_based_counter = if let Expr::Identifier(idx_sym) = index {
                        variable_types.get(idx_sym).map_or(false, |t| t == "__zero_based_i64")
                    } else {
                        false
                    };
                    let index_part = if is_zero_based_counter {
                        let idx_name = irecurse!(index);
                        format!("{} as usize", idx_name)
                    } else { match index {
                        // Literal(1) → 0
                        Expr::Literal(Literal::Number(1)) => "0".to_string(),
                        // Literal(N) → N-1
                        Expr::Literal(Literal::Number(n)) => format!("{}", n - 1),
                        // (X + K) patterns: +1 cancels the -1 from 1-based indexing
                        Expr::BinaryOp { op: BinaryOpKind::Add, left, right } => {
                            match (left, right) {
                                (_, Expr::Literal(Literal::Number(1))) => {
                                    let left_str = irecurse!(left);
                                    if matches!(left, Expr::Identifier(_)) {
                                        format!("{} as usize", left_str)
                                    } else {
                                        format!("({}) as usize", left_str)
                                    }
                                }
                                (Expr::Literal(Literal::Number(1)), _) => {
                                    let right_str = irecurse!(right);
                                    if matches!(right, Expr::Identifier(_)) {
                                        format!("{} as usize", right_str)
                                    } else {
                                        format!("({}) as usize", right_str)
                                    }
                                }
                                (_, Expr::Literal(Literal::Number(k))) if *k > 1 => {
                                    format!("({} + {}) as usize", irecurse!(left), k - 1)
                                }
                                (Expr::Literal(Literal::Number(k)), _) if *k > 1 => {
                                    format!("({} + {}) as usize", irecurse!(right), k - 1)
                                }
                                _ => {
                                    format!("({} - 1) as usize", irecurse!(index))
                                }
                            }
                        }
                        _ => {
                            format!("({} - 1) as usize", irecurse!(index))
                        }
                    } };
                    // BCE: when the oracle proves this index in range, emit an
                    // unchecked load (no bounds branch). The statement-level hint
                    // pass emits a `debug_assert!` net for debug builds.
                    let read = if oracle_proves_index(ecx, collection, index) {
                        match (is_logos_seq, suffix.is_empty()) {
                            (true, true) => format!("unsafe {{ *{}.borrow().get_unchecked({}) }}", coll_str, index_part),
                            (true, false) => format!("unsafe {{ {}.borrow().get_unchecked({}){} }}", coll_str, index_part, suffix),
                            (false, true) => format!("unsafe {{ *{}.get_unchecked({}) }}", coll_str, index_part),
                            (false, false) => format!("unsafe {{ {}.get_unchecked({}){} }}", coll_str, index_part, suffix),
                        }
                    } else if is_logos_seq {
                        format!("{}.borrow()[{}]{}", coll_str, index_part, suffix)
                    } else {
                        format!("{}[{}]{}", coll_str, index_part, suffix)
                    };
                    // A narrowed (`Vec<i32>`) read sign-extends to the i64 the rest
                    // of the program expects (lossless).
                    if t == "Vec<i32>" {
                        format!("(({}) as i64)", read)
                    } else {
                        read
                    }
                }
                Some(t) if t.starts_with("&[") || t.starts_with("&mut [") || t.starts_with('[') => {
                    // Slice (&[T] / &mut [T]) or O3 fixed array [T; N] — direct
                    // indexing with the same 1-based simplification.
                    let elem = if t.starts_with('[') {
                        t.trim_start_matches('[').split("; ").next().unwrap_or("_")
                    } else {
                        t.strip_prefix("&mut [")
                            .or_else(|| t.strip_prefix("&["))
                            .and_then(|s| s.strip_suffix(']'))
                            .unwrap_or("_")
                    };
                    let suffix = if is_copy_type(elem) { "" } else { ".clone()" };
                    // OPT-8: Check if index is a zero-based counter
                    let is_zero_based_counter = if let Expr::Identifier(idx_sym) = index {
                        variable_types.get(idx_sym).map_or(false, |t| t == "__zero_based_i64")
                    } else {
                        false
                    };
                    let index_part = if is_zero_based_counter {
                        let idx_name = irecurse!(index);
                        format!("{} as usize", idx_name)
                    } else { match index {
                        Expr::Literal(Literal::Number(1)) => "0".to_string(),
                        Expr::Literal(Literal::Number(n)) => format!("{}", n - 1),
                        Expr::BinaryOp { op: BinaryOpKind::Add, left, right } => {
                            match (left, right) {
                                (_, Expr::Literal(Literal::Number(1))) => {
                                    let left_str = irecurse!(left);
                                    if matches!(left, Expr::Identifier(_)) {
                                        format!("{} as usize", left_str)
                                    } else {
                                        format!("({}) as usize", left_str)
                                    }
                                }
                                (Expr::Literal(Literal::Number(1)), _) => {
                                    let right_str = irecurse!(right);
                                    if matches!(right, Expr::Identifier(_)) {
                                        format!("{} as usize", right_str)
                                    } else {
                                        format!("({}) as usize", right_str)
                                    }
                                }
                                (_, Expr::Literal(Literal::Number(k))) if *k > 1 => {
                                    format!("({} + {}) as usize", irecurse!(left), k - 1)
                                }
                                (Expr::Literal(Literal::Number(k)), _) if *k > 1 => {
                                    format!("({} + {}) as usize", irecurse!(right), k - 1)
                                }
                                _ => {
                                    format!("({} - 1) as usize", irecurse!(index))
                                }
                            }
                        }
                        _ => {
                            format!("({} - 1) as usize", irecurse!(index))
                        }
                    } };
                    // BCE: oracle-proven index → unchecked load (no bounds branch).
                    let read = if oracle_proves_index(ecx, collection, index) {
                        if suffix.is_empty() {
                            format!("unsafe {{ *{}.get_unchecked({}) }}", coll_str, index_part)
                        } else {
                            format!("unsafe {{ {}.get_unchecked({}){} }}", coll_str, index_part, suffix)
                        }
                    } else {
                        format!("{}[{}]{}", coll_str, index_part, suffix)
                    };
                    // A narrowed slice (`&[i32]`/`&mut [i32]`, from a hoisted
                    // `Vec<i32>`) sign-extends to i64 (lossless).
                    if elem == "i32" {
                        format!("(({}) as i64)", read)
                    } else {
                        read
                    }
                }
                Some(t) if is_logos_map_type(t) => {
                    let index_str = irecurse!(index);
                    // Use .get() which borrows the key (avoids moving String keys)
                    format!("{}.get(&({})).expect(\"Key not found in map\")", coll_str, index_str)
                }
                Some(t) if t.starts_with("std::collections::HashMap") || t.starts_with("HashMap") || t.starts_with("rustc_hash::FxHashMap") || t.starts_with("FxHashMap") => {
                    let index_str = irecurse!(index);
                    let suffix = if has_copy_value_type(t) { "" } else { ".clone()" };
                    format!("{}[&({})]{}", coll_str, index_str, suffix)
                }
                Some("String") => {
                    let index_str = irecurse!(index);
                    format!("LogosIndex::logos_get(&{}, {})", coll_str, index_str)
                }
                _ => {
                    let index_str = irecurse!(index);
                    format!("LogosIndex::logos_get(&{}, {})", coll_str, index_str)
                }
            }
        }

        Expr::Slice { collection, start, end } => {
            let coll_str = recurse!(collection);
            let start_str = recurse!(start);
            let end_str = recurse!(end);
            // For LogosSeq, need to borrow the inner Vec first
            let known_type = if let Expr::Identifier(sym) = collection {
                variable_types.get(sym).map(|s| s.split("|__hl:").next().unwrap_or(s.as_str()))
            } else {
                None
            };
            if matches!(known_type, Some(t) if t.starts_with("LogosSeq")) {
                format!("&{}.borrow()[({} - 1) as usize..{} as usize]", coll_str, start_str, end_str)
            } else {
                format!("&{}[({} - 1) as usize..{} as usize]", coll_str, start_str, end_str)
            }
        }

        Expr::Copy { expr: inner } => {
            // Special case: Copy of Slice → emit arr[range].to_vec() wrapped in LogosSeq
            if let Expr::Slice { collection, start, end } = inner {
                let coll_str = recurse!(collection);
                let start_str = recurse!(start);
                let end_str = recurse!(end);
                let known_type = if let Expr::Identifier(sym) = collection {
                    variable_types.get(sym).map(|s| s.split("|__hl:").next().unwrap_or(s.as_str()))
                } else {
                    None
                };
                if matches!(known_type, Some(t) if t.starts_with("LogosSeq")) {
                    format!("LogosSeq::from_vec({}.borrow()[({} - 1) as usize..{} as usize].to_vec())", coll_str, start_str, end_str)
                } else if matches!(known_type, Some(t) if t.starts_with("&[") || t.starts_with("Vec<")) {
                    format!("LogosSeq::from_vec({}[({} - 1) as usize..{} as usize].to_vec())", coll_str, start_str, end_str)
                } else {
                    format!("{}[({} - 1) as usize..{} as usize].to_vec()", coll_str, start_str, end_str)
                }
            } else {
                // Check if the inner expression is a LogosSeq/LogosMap → deep_clone()
                let known_type = if let Expr::Identifier(sym) = inner {
                    variable_types.get(sym).map(|s| s.split("|__hl:").next().unwrap_or(s.as_str()))
                } else {
                    None
                };
                let expr_str = recurse!(inner);
                if matches!(known_type, Some(t) if t.starts_with("Vec<")) {
                    // Slice variable stored as Vec<T> — wrap with LogosSeq::from_vec.
                    // One copy (.clone()) + zero-cost wrap, same as old pre-ref-semantics .to_vec().
                    format!("LogosSeq::from_vec({}.clone())", expr_str)
                } else if matches!(known_type, Some(t) if t.starts_with("&[")) {
                    format!("LogosSeq::from_vec({}.to_vec())", expr_str)
                } else if matches!(known_type, Some(t) if t.starts_with("LogosSeq") || t.starts_with("LogosMap")) {
                    format!("{}.deep_clone()", expr_str)
                } else {
                    format!("{}.to_owned()", expr_str)
                }
            }
        }

        Expr::Give { value } => {
            // Ownership transfer: emit value without .clone()
            // The move semantics are implicit in Rust - no special syntax needed
            recurse!(value)
        }

        // Affine read-only array: `length of A` is its trip count (the array is
        // deleted, so there is no `.len()` to read).
        Expr::Length { collection: Expr::Identifier(sym) }
            if affine_array_trip(variable_types.get(sym)).is_some() =>
        {
            format!("(({}) as i64)", affine_array_trip(variable_types.get(sym)).unwrap())
        }

        // AoS-interleaved member: its length is the backing's fixed row count.
        Expr::Length { collection: Expr::Identifier(sym) }
            if parse_aos_tag(variable_types.get(sym)).is_some() =>
        {
            format!("{}i64", parse_aos_tag(variable_types.get(sym)).unwrap().len)
        }

        Expr::Length { collection } => {
            if let Expr::Identifier(sym) = collection {
                if let Some(t) = variable_types.get(sym) {
                    if let Some(pos) = t.find("|__hl:") {
                        return t[pos + "|__hl:".len()..].to_string();
                    }
                }
            }
            let coll_str = recurse!(collection);
            // Phase 43D: Collection length - cast to i64 for LOGOS integer semantics
            format!("({}.len() as i64)", coll_str)
        }

        Expr::Contains { collection, value } => {
            let coll_str = recurse!(collection);
            let val_str = recurse!(value);
            // Numeric-unified map key: a Float indexing an `Int`-keyed map coerces to its Int
            // (`1.0` → key `1`; a non-integral `1.5` matches nothing), mirroring the interpreter's
            // `1 == 1.0` key rule. An `i64`-keyed `logos_contains` otherwise rejects the `f64` operand.
            if let crate::analysis::types::LogosType::Map(k, _) =
                super::types::infer_logos_type(collection, interner, variable_types)
            {
                if matches!(*k, crate::analysis::types::LogosType::Int)
                    && super::types::infer_numeric_type(value, interner, variable_types) == "f64"
                {
                    return format!(
                        "logicaffeine_data::logos_i64_key_of_f64({}).map_or(false, |__k| {}.logos_contains(&__k))",
                        val_str, coll_str
                    );
                }
            }
            // Use LogosContains trait for unified contains across List, Set, Map, Text
            format!("{}.logos_contains(&{})", coll_str, val_str)
        }

        Expr::Union { left, right } => {
            let left_str = recurse!(left);
            let right_str = recurse!(right);
            format!("{}.union(&{}).cloned().collect::<Set<_>>()", left_str, right_str)
        }

        Expr::Intersection { left, right } => {
            let left_str = recurse!(left);
            let right_str = recurse!(right);
            format!("{}.intersection(&{}).cloned().collect::<Set<_>>()", left_str, right_str)
        }

        // Phase 48: Sipping Protocol expressions
        Expr::ManifestOf { zone } => {
            let zone_str = recurse!(zone);
            format!("logicaffeine_system::network::FileSipper::from_zone(&{}).manifest()", zone_str)
        }

        Expr::ChunkAt { index, zone } => {
            let zone_str = recurse!(zone);
            let index_str = irecurse!(index);
            // LOGOS uses 1-indexed, Rust uses 0-indexed
            format!("logicaffeine_system::network::FileSipper::from_zone(&{}).get_chunk(({} - 1) as usize)", zone_str, index_str)
        }

        Expr::List(ref items) => {
            let item_strs: Vec<String> = items.iter()
                .map(|i| recurse!(i))
                .collect();
            format!("LogosSeq::from_vec(vec![{}])", item_strs.join(", "))
        }

        Expr::Tuple(ref items) => {
            let item_strs: Vec<String> = items.iter()
                .map(|i| format!("Value::from({})", recurse!(i)))
                .collect();
            // Tuples as Vec<Value> for heterogeneous support
            format!("vec![{}]", item_strs.join(", "))
        }

        Expr::Range { start, end } => {
            let start_str = recurse!(start);
            let end_str = recurse!(end);
            format!("({}..={})", start_str, end_str)
        }

        Expr::FieldAccess { object, field } => {
            let field_name = interner.resolve(*field);

            // Phase 52: Check if root object is synced - use .get().await
            let root_sym = get_root_identifier(object);
            if let Some(sym) = root_sym {
                if synced_vars.contains(&sym) {
                    let obj_name = interner.resolve(sym);
                    return format!("{}.get().await.{}", obj_name, field_name);
                }
            }

            let obj_str = recurse!(object);
            format!("{}.{}", obj_str, field_name)
        }

        Expr::New { type_name, type_args, init_fields } => {
            let type_str = interner.resolve(*type_name);
            if !init_fields.is_empty() {
                // Struct initialization with fields: Point { x: 10, y: 20, ..Default::default() }
                // Always add ..Default::default() to handle partial initialization (e.g., CRDT fields)
                let fields_str = init_fields.iter()
                    .map(|(name, value)| {
                        let field_name = interner.resolve(*name);
                        let value_str = recurse!(value);
                        format!("{}: {}", field_name, value_str)
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{} {{ {}, ..Default::default() }}", type_str, fields_str)
            } else if type_args.is_empty() {
                format!("{}::default()", type_str)
            } else {
                // Phase 34: Turbofish syntax for generic instantiation
                // Bug fix: Use codegen_type_expr to support nested types like Seq of (Seq of Int)
                let args_str = type_args.iter()
                    .map(|t| codegen_type_expr(t, interner))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{}::<{}>::default()", type_str, args_str)
            }
        }

        Expr::NewVariant { enum_name, variant, fields } => {
            let enum_str = interner.resolve(*enum_name);
            let variant_str = interner.resolve(*variant);
            if fields.is_empty() {
                // Unit variant: Shape::Point
                format!("{}::{}", enum_str, variant_str)
            } else {
                // Phase 103: Count identifier usage to handle cloning for reused values
                // We need to clone on all uses except the last one
                let mut identifier_counts: HashMap<Symbol, usize> = HashMap::new();
                for (_, value) in fields.iter() {
                    if let Expr::Identifier(sym) = value {
                        *identifier_counts.entry(*sym).or_insert(0) += 1;
                    }
                }

                // Track remaining uses for each identifier
                let mut remaining_uses: HashMap<Symbol, usize> = identifier_counts.clone();

                // Struct variant: Shape::Circle { radius: 10 }
                // Phase 102: Check if any field is recursive and needs Box::new()
                let fields_str: Vec<String> = fields.iter()
                    .map(|(field_name, value)| {
                        let name = interner.resolve(*field_name);

                        // Phase 103: Clone identifiers that are used multiple times
                        // Clone on all uses except the last one (to allow move on final use)
                        let val = if let Expr::Identifier(sym) = value {
                            let total = identifier_counts.get(sym).copied().unwrap_or(0);
                            let remaining = remaining_uses.get_mut(sym);
                            let base_name = if boxed_bindings.contains(sym) {
                                format!("(*{})", interner.resolve(*sym))
                            } else {
                                interner.resolve(*sym).to_string()
                            };
                            if total > 1 {
                                if let Some(r) = remaining {
                                    *r -= 1;
                                    if *r > 0 {
                                        // Not the last use, need to clone
                                        format!("{}.clone()", base_name)
                                    } else {
                                        // Last use, can move
                                        base_name
                                    }
                                } else {
                                    base_name
                                }
                            } else {
                                base_name
                            }
                        } else {
                            recurse!(value)
                        };

                        // Check if this field needs to be boxed (recursive type)
                        let key = (enum_str.to_string(), variant_str.to_string(), name.to_string());
                        if boxed_fields.contains(&key) {
                            format!("{}: Box::new({})", name, val)
                        } else {
                            format!("{}: {}", name, val)
                        }
                    })
                    .collect();
                format!("{}::{} {{ {} }}", enum_str, variant_str, fields_str.join(", "))
            }
        }

        Expr::OptionSome { value } => {
            format!("Some({})", recurse!(value))
        }

        Expr::OptionNone => {
            "None".to_string()
        }

        Expr::Escape { code, .. } => {
            let raw_code = interner.resolve(*code);
            let mut block = String::from("{\n");
            for line in raw_code.lines() {
                block.push_str("    ");
                block.push_str(line);
                block.push('\n');
            }
            block.push('}');
            block
        }

        Expr::WithCapacity { value, capacity } => {
            let cap_str = recurse!(capacity);
            match value {
                // Empty string → String::with_capacity(cap)
                Expr::Literal(Literal::Text(sym)) if interner.resolve(*sym).is_empty() => {
                    format!("String::with_capacity(({}) as usize)", cap_str)
                }
                // Non-empty string → { let mut __s = String::with_capacity(cap); __s.push_str("..."); __s }
                Expr::Literal(Literal::Text(sym)) => {
                    let text = interner.resolve(*sym);
                    format!("{{ let mut __s = String::with_capacity(({}) as usize); __s.push_str(\"{}\"); __s }}", cap_str, text)
                }
                // Collection Expr::New → Type::with_capacity(cap)
                Expr::New { type_name, type_args, .. } => {
                    let type_str = interner.resolve(*type_name);
                    match type_str {
                        "Seq" | "List" | "Vec" => {
                            let elem = if !type_args.is_empty() {
                                codegen_type_expr(&type_args[0], interner)
                            } else { "()".to_string() };
                            format!("LogosSeq::<{}>::with_capacity(({}) as usize)", elem, cap_str)
                        }
                        "Map" | "HashMap" => {
                            let (k, v) = if type_args.len() >= 2 {
                                (codegen_type_expr(&type_args[0], interner),
                                 codegen_type_expr(&type_args[1], interner))
                            } else { ("String".to_string(), "String".to_string()) };
                            format!("LogosMap::<{}, {}>::with_capacity(({}) as usize)", k, v, cap_str)
                        }
                        "Set" | "HashSet" => {
                            let elem = if !type_args.is_empty() {
                                codegen_type_expr(&type_args[0], interner)
                            } else { "()".to_string() };
                            format!("{{ let __s: Set<{}> = Set::with_capacity_and_hasher(({}) as usize, Default::default()); __s }}", elem, cap_str)
                        }
                        _ => recurse!(value) // Unknown type — ignore capacity
                    }
                }
                // Other expressions — ignore capacity hint
                _ => recurse!(value)
            }
        }

        Expr::Closure { params, body, .. } => {
            use crate::ast::stmt::ClosureBody;
            let params_str: Vec<String> = params.iter()
                .map(|(name, ty)| {
                    let param_name = names.ident(*name);
                    let param_type = codegen_type_expr(ty, interner);
                    format!("{}: {}", param_name, param_type)
                })
                .collect();

            match body {
                ClosureBody::Expression(expr) => {
                    let body_str = recurse!(expr);
                    format!("move |{}| {{ {} }}", params_str.join(", "), body_str)
                }
                ClosureBody::Block(stmts) => {
                    let mut body_str = String::new();
                    let mut ctx = RefinementContext::new();
                    let empty_mutable = collect_mutable_vars(stmts);
                    let empty_lww = HashSet::new();
                    let empty_mv = HashSet::new();
                    let mut empty_synced = HashSet::new();
                    let empty_caps = HashMap::new();
                    let empty_pipes = HashSet::new();
                    let empty_boxed = HashSet::new();
                    let empty_registry = TypeRegistry::new();
                    let type_env = crate::analysis::types::TypeEnv::new();
                    for stmt in stmts.iter() {
                        body_str.push_str(&codegen_stmt(
                            stmt, interner, 2, &empty_mutable, &mut ctx,
                            &empty_lww, &empty_mv, &mut empty_synced, &empty_caps,
                            async_functions, &empty_pipes, &empty_boxed, &empty_registry,
                            &type_env,
                        ));
                    }
                    format!("move |{}| {{\n{}{}}}", params_str.join(", "), body_str, "    ")
                }
            }
        }

        Expr::CallExpr { callee, args } => {
            let callee_str = recurse!(callee);
            let args_str: Vec<String> = args.iter().map(|a| recurse!(a)).collect();
            format!("({})({})", callee_str, args_str.join(", "))
        }

        Expr::InterpolatedString(parts) => {
            codegen_interpolated_string(parts, ecx)
        }

        Expr::Not { operand } => {
            // Logical negation of truthiness (`~` is the bitwise complement).
            let operand_str = recurse!(operand);
            if matches!(
                infer_logos_type(operand, interner, variable_types),
                crate::analysis::types::LogosType::Bool
            ) {
                format!("!({})", operand_str)
            } else {
                format!("(!logos_truthy(&({})))", operand_str)
            }
        }
    }
}

pub(crate) fn codegen_interpolated_string(
    parts: &[crate::ast::stmt::StringPart],
    ecx: &ExprCtx,
) -> String {
    use crate::ast::stmt::StringPart;
    let interner = ecx.interner;

    let mut fmt_str = String::new();
    let mut args = Vec::new();

    for part in parts {
        match part {
            StringPart::Literal(sym) => {
                let text = interner.resolve(*sym);
                // Escape braces and special chars in the format string
                for ch in text.chars() {
                    match ch {
                        '{' => fmt_str.push_str("{{"),
                        '}' => fmt_str.push_str("}}"),
                        '\n' => fmt_str.push_str("\\n"),
                        '\t' => fmt_str.push_str("\\t"),
                        '\r' => fmt_str.push_str("\\r"),
                        _ => fmt_str.push(ch),
                    }
                }
            }
            StringPart::Expr { value, format_spec, debug } => {
                if *debug {
                    let debug_prefix = expr_debug_prefix(value, interner);
                    for ch in debug_prefix.chars() {
                        match ch {
                            '{' => fmt_str.push_str("{{"),
                            '}' => fmt_str.push_str("}}"),
                            _ => fmt_str.push(ch),
                        }
                    }
                    fmt_str.push('=');
                }
                let needs_float_cast = if let Some(spec) = format_spec {
                    let spec_str = interner.resolve(*spec);
                    if spec_str == "$" {
                        fmt_str.push('$');
                        fmt_str.push_str("{:.2}");
                        true
                    } else if spec_str.starts_with('.') {
                        fmt_str.push_str(&format!("{{:{}}}", spec_str));
                        true
                    } else {
                        fmt_str.push_str(&format!("{{:{}}}", spec_str));
                        false
                    }
                } else {
                    fmt_str.push_str("{}");
                    false
                };
                let arg_str = codegen_expr_ctx(value, ecx);
                if needs_float_cast {
                    args.push(format!("{} as f64", arg_str));
                } else {
                    args.push(arg_str);
                }
            }
        }
    }

    if args.is_empty() {
        // No holes — emit raw String::from (no format! escaping needed).
        // Reconstruct the raw text from parts without brace escaping.
        let mut raw = String::new();
        for part in parts {
            if let StringPart::Literal(sym) = part {
                let text = interner.resolve(*sym);
                for ch in text.chars() {
                    match ch {
                        '\n' => raw.push_str("\\n"),
                        '\t' => raw.push_str("\\t"),
                        '\r' => raw.push_str("\\r"),
                        '"' => raw.push_str("\\\""),
                        '\\' => raw.push_str("\\\\"),
                        _ => raw.push(ch),
                    }
                }
            }
        }
        format!("String::from(\"{}\")", raw)
    } else {
        format!("format!(\"{}\"{})", fmt_str, args.iter().map(|a| format!(", {}", a)).collect::<String>())
    }
}

pub(crate) fn codegen_literal(lit: &Literal, interner: &Interner) -> String {
    match lit {
        Literal::Number(n) => {
            if *n > i32::MAX as i64 || *n < i32::MIN as i64 {
                format!("{}_i64", n)
            } else {
                n.to_string()
            }
        }
        // Non-finite values have no numeric-literal spelling in Rust
        // (`Display` gives "inf"/"NaN", which would emit invalid tokens).
        Literal::Float(f) if f.is_nan() => "f64::NAN".to_string(),
        Literal::Float(f) if f.is_infinite() && *f > 0.0 => "f64::INFINITY".to_string(),
        Literal::Float(f) if f.is_infinite() => "f64::NEG_INFINITY".to_string(),
        Literal::Float(f) => format!("{}f64", f),
        // String literals are converted to String for consistent Text type handling
        Literal::Text(sym) => {
            let raw = interner.resolve(*sym);
            let escaped: String = raw.chars().map(|c| match c {
                '\n' => "\\n".to_string(),
                '\r' => "\\r".to_string(),
                '\t' => "\\t".to_string(),
                '\\' => "\\\\".to_string(),
                '"' => "\\\"".to_string(),
                other => other.to_string(),
            }).collect();
            format!("String::from(\"{}\")", escaped)
        }
        Literal::Boolean(b) => b.to_string(),
        Literal::Nothing => "()".to_string(),
        // Character literals
        Literal::Char(c) => {
            // Handle escape sequences for special characters
            match c {
                '\n' => "'\\n'".to_string(),
                '\t' => "'\\t'".to_string(),
                '\r' => "'\\r'".to_string(),
                '\\' => "'\\\\'".to_string(),
                '\'' => "'\\''".to_string(),
                '\0' => "'\\0'".to_string(),
                c => format!("'{}'", c),
            }
        }
        // Temporal literals: Duration stored as nanoseconds (i64)
        Literal::Duration(nanos) => format!("std::time::Duration::from_nanos({}u64)", nanos),
        // Date stored as days since Unix epoch (i32)
        Literal::Date(days) => format!("LogosDate({})", days),
        // Moment stored as nanoseconds since Unix epoch (i64)
        Literal::Moment(nanos) => format!("LogosMoment({})", nanos),
        // Span stored as (months, days) - separate because they're incommensurable
        Literal::Span { months, days } => format!("LogosSpan::new({}, {})", months, days),
        // Time-of-day stored as nanoseconds from midnight
        Literal::Time(nanos) => format!("LogosTime({})", nanos),
    }
}

/// Converts a LogicExpr to a Rust boolean expression for debug_assert!().
/// Uses RustFormatter to unify all logic-to-Rust translation.
pub fn codegen_assertion(expr: &LogicExpr, interner: &Interner) -> String {
    let mut registry = SymbolRegistry::new();
    let formatter = RustFormatter;
    let mut buf = String::new();

    match expr.write_logic(&mut buf, &mut registry, interner, &formatter) {
        Ok(_) => buf,
        Err(_) => "/* error generating assertion */ false".to_string(),
    }
}

pub fn codegen_term(term: &Term, interner: &Interner) -> String {
    match term {
        Term::Constant(sym) => interner.resolve(*sym).to_string(),
        Term::Variable(sym) => interner.resolve(*sym).to_string(),
        Term::Value { kind, .. } => match kind {
            NumberKind::Integer(n) => n.to_string(),
            NumberKind::Real(f) => f.to_string(),
            NumberKind::Symbolic(sym) => interner.resolve(*sym).to_string(),
        },
        Term::Function(name, args) => {
            let args_str: Vec<String> = args.iter()
                .map(|a| codegen_term(a, interner))
                .collect();
            format!("{}({})", interner.resolve(*name), args_str.join(", "))
        }
        Term::Possessed { possessor, possessed } => {
            let poss_str = codegen_term(possessor, interner);
            format!("{}.{}", poss_str, interner.resolve(*possessed))
        }
        Term::Group(members) => {
            let members_str: Vec<String> = members.iter()
                .map(|m| codegen_term(m, interner))
                .collect();
            format!("({})", members_str.join(", "))
        }
        _ => "/* unsupported Term */".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_literal_number() {
        let interner = Interner::new();
        let synced_vars = HashSet::new();
        let expr = Expr::Literal(Literal::Number(42));
        assert_eq!(codegen_expr(&expr, &interner, &synced_vars), "42");
    }

    #[test]
    fn test_literal_boolean() {
        let interner = Interner::new();
        let synced_vars = HashSet::new();
        assert_eq!(codegen_expr(&Expr::Literal(Literal::Boolean(true)), &interner, &synced_vars), "true");
        assert_eq!(codegen_expr(&Expr::Literal(Literal::Boolean(false)), &interner, &synced_vars), "false");
    }

    #[test]
    fn test_literal_nothing() {
        let interner = Interner::new();
        let synced_vars = HashSet::new();
        assert_eq!(codegen_expr(&Expr::Literal(Literal::Nothing), &interner, &synced_vars), "()");
    }
}
