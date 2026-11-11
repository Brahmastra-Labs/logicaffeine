use std::collections::{HashMap, HashSet};

use crate::analysis::registry::TypeRegistry;
use crate::analysis::types::RustNames;
use crate::ast::logic::{LogicExpr, NumberKind, Term};
use crate::ast::stmt::{BinaryOpKind, Expr, Literal, Stmt};
use crate::formatter::RustFormatter;
use crate::intern::{Interner, Symbol};
use crate::registry::SymbolRegistry;

use super::context::RefinementContext;
use super::detection::{collect_mutable_vars, expr_debug_prefix, parse_aos_tag};
use super::i64_map::is_logos_map_type;
use super::types::{codegen_type_expr, infer_logos_type, infer_numeric_type, rust_type_ident};
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
    /// True while codegen'ing a closure ARGUMENT to a scalar exact higher-order function
    /// (`exact_hof_fns` — `apply(f, x)`). The closure's integer params are typed
    /// `LogosInt` and registered as promoted (`__bigint`), so an overflowing body
    /// (`(n) -> n * 2`) routes through the exact helper and yields a bignum == the VM,
    /// rather than the raw `i64` that wraps/panics. Set ONLY on the closure arg's own
    /// codegen and reset inside the body — see the `Expr::Closure` arm and the call-site
    /// arg loop. `false` for every other closure (a `map`/`fold` closure stays `i64`).
    pub exact_closure: bool,
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
            exact_closure: false,
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

thread_local! {
    /// Opt::StrengthReducePow2 — the current function body's variables proven ALWAYS `>= 0` (their
    /// init and every assignment preserve non-negativity, e.g. a `0`-start `+1` loop counter). Set
    /// once per function body by [`set_nonneg_vars`]; read by [`is_nonneg_expr`] at a `% 2^k`/`/ 2^k`
    /// site so the mask/shift reduction fires only where signed and masked/shifted agree. Codegen is
    /// single-threaded per program and each body overwrites the set, so nothing leaks across bodies.
    static NONNEG_VARS: std::cell::RefCell<HashSet<Symbol>> = std::cell::RefCell::new(HashSet::new());
}

/// Record the current function body's always-non-negative variables (see [`NONNEG_VARS`]).
pub(crate) fn set_nonneg_vars(vars: HashSet<Symbol>) {
    NONNEG_VARS.with(|v| *v.borrow_mut() = vars);
}

thread_local! {
    /// Opt::WideAccumulator — the current body's variables widened to `i128` (a bounded counting
    /// accumulator). Set once per body; read at the `Let`/`Set`/branchless-increment/`Show` sites so
    /// the accumulator is declared `i128`, added plainly (no checked-add branch), and narrowed `as i64`
    /// at the `Show`. Per-body (symbols are name-keyed), single-threaded per program.
    static WIDE_ACC: std::cell::RefCell<HashSet<Symbol>> = std::cell::RefCell::new(HashSet::new());
}

/// Record the current function body's i128-widened accumulators (see [`WIDE_ACC`]).
pub(crate) fn set_wide_acc(vars: HashSet<Symbol>) {
    WIDE_ACC.with(|v| *v.borrow_mut() = vars);
}

/// Is `x` an i128-widened counting accumulator in the current body?
pub(crate) fn is_wide_acc(x: Symbol) -> bool {
    WIDE_ACC.with(|v| v.borrow().contains(&x))
}

/// Is `e` provably `>= 0`? Syntactic over the non-negative-closed operations: non-negative literals,
/// `length of …`, `nonneg % positive`, `nonneg + nonneg`, `nonneg * nonneg`, `nonneg / positive`, and
/// variables the body-level analysis proved always non-negative ([`NONNEG_VARS`]).
fn is_nonneg_expr(e: &Expr) -> bool {
    NONNEG_VARS.with(|vars| expr_is_nonnegative(e, &vars.borrow()))
}

/// Backend-independent spelling of the non-negativity query. Rust keeps its
/// current-body set thread-local; direct C carries the same set in `CContext`.
pub(crate) fn expr_is_nonnegative(e: &Expr, vars: &HashSet<Symbol>) -> bool {
    match e {
        Expr::Literal(Literal::Number(n)) => *n >= 0,
        Expr::Identifier(s) => vars.contains(s),
        Expr::Length { .. } => true,
        Expr::BinaryOp { op, left, right } => match op {
            // A remainder keeps its dividend's sign (`-7 % 8` is `-7`), as a quotient does.
            BinaryOpKind::Modulo | BinaryOpKind::Divide => {
                matches!(right, Expr::Literal(Literal::Number(d)) if *d > 0)
                    && expr_is_nonnegative(left, vars)
            }
            BinaryOpKind::Add | BinaryOpKind::Multiply => {
                expr_is_nonnegative(left, vars) && expr_is_nonnegative(right, vars)
            }
            _ => false,
        },
        _ => false,
    }
}

/// The variables provably `>= 0` throughout `stmts`: every `Let`/`Set` to the variable assigns a
/// non-negativity-preserving expression (a fixpoint, since one such var may reference another). A
/// `0`-start `+1` loop counter qualifies; a var ever assigned a possibly-negative value does not.
/// Sound because it collects EVERY assignment across nested blocks before deciding.
pub(crate) fn collect_always_nonneg(stmts: &[Stmt]) -> HashSet<Symbol> {
    fn walk<'a>(stmts: &'a [Stmt<'a>], out: &mut Vec<(Symbol, &'a Expr<'a>)>) {
        for s in stmts {
            match s {
                Stmt::Let { var, value, .. } => out.push((*var, value)),
                Stmt::Set { target, value } => out.push((*target, value)),
                Stmt::If {
                    then_block,
                    else_block,
                    ..
                } => {
                    walk(then_block, out);
                    if let Some(eb) = else_block {
                        walk(eb, out);
                    }
                }
                Stmt::While { body, .. } | Stmt::Repeat { body, .. } | Stmt::Zone { body, .. } => {
                    walk(body, out)
                }
                Stmt::Inspect { arms, .. } => {
                    for arm in arms.iter() {
                        walk(arm.body, out);
                    }
                }
                _ => {}
            }
        }
    }
    let mut assigns: Vec<(Symbol, &Expr)> = Vec::new();
    walk(stmts, &mut assigns);
    let mut cand: HashSet<Symbol> = assigns.iter().map(|(v, _)| *v).collect();
    loop {
        let remove: Vec<Symbol> = cand
            .iter()
            .filter(|v| {
                assigns
                    .iter()
                    .any(|(t, rhs)| *t == **v && !expr_is_nonnegative(rhs, &cand))
            })
            .copied()
            .collect();
        if remove.is_empty() {
            break;
        }
        for v in remove {
            cand.remove(&v);
        }
    }
    cand
}

/// Record the program's bignum-returning function set for the current codegen (see
/// [`BIGINT_RETURNING_FNS`]). Called once at the top of `generate_rust_code`.
/// Start a fresh program's emission: zero the bignum-temp counter.
///
/// ★ CODEGEN MUST NOT DEPEND ON WHAT WAS COMPILED BEFORE IT. `BIGNUM_TMP` is monotonic across a
/// program, which is all uniqueness needs — but it was never reset, so compiling the same source
/// TWICE IN ONE PROCESS named its temps `__lbop0` the first time and `__lbop27` the second.
/// `phase_codegen_determinism` and `phase_opt_trace` both compile twice and compare, and both
/// caught it the moment a program started using the bignum path at all: the output was a function
/// of the compiler's history rather than of the source.
pub(crate) fn begin_program_emission() {
    BIGNUM_TMP.with(|c| c.set(0));
    // SIMD_REDUCE_ID (codegen::simd_reduce) names emitted `__logos_simd_chunks_N` kernels, so it
    // must reset per program for deterministic output — ResetAtEntry in the emitter-state catalogue.
    crate::codegen::simd_reduce::reset_ids();
    // PROBE_ID (codegen::probe) numbers the `__probeN` bindings a fused map test makes: ResetAtEntry, as above.
    crate::codegen::probe::reset_names();
}

thread_local! {
    /// Every `## Route` this program cannot lower, keyed by `(method, path)`, with the reason —
    /// installed once at codegen entry, where the whole program is visible.
    ///
    /// A `-> View` route's lowerability depends on the COMPONENTS beside it, and the statement
    /// emitter reaches a route with no program in hand. Asking there with an empty component list
    /// refused every route that placed a component — which is the shape every page is built out
    /// of — so the answer is computed where it can be right and merely read here.
    static ROUTE_REFUSALS: std::cell::RefCell<HashMap<(Symbol, Symbol), String>> =
        std::cell::RefCell::new(HashMap::new());
}

pub(crate) fn set_route_refusals(rs: &[(Symbol, Symbol, String)]) {
    ROUTE_REFUSALS.with(|c| {
        *c.borrow_mut() = rs.iter().map(|(m, p, w)| ((*m, *p), w.clone())).collect();
    });
}

/// Why this route cannot lower, if it cannot.
pub(crate) fn route_refusal(method: Symbol, path: Symbol) -> Option<String> {
    ROUTE_REFUSALS.with(|c| c.borrow().get(&(method, path)).cloned())
}

pub(crate) fn set_bigint_returning_fns(fns: &HashSet<Symbol>) {
    BIGINT_RETURNING_FNS.with(|c| *c.borrow_mut() = fns.clone());
}

fn is_bigint_returning_call(function: Symbol) -> bool {
    BIGINT_RETURNING_FNS.with(|c| c.borrow().contains(&function))
}

/// The program's bignum fields, keyed `(struct_type or variant, field)`.
#[derive(Default)]
struct BignumFields {
    /// Every field some construction feeds a bignum-promoted value
    /// (`bigint_promote::bigint_struct_fields`) — what a read of the field is taken to hold.
    tracked: HashSet<(Symbol, Symbol)>,
    /// The tracked fields the emitter writes down as `LogosInt`: every one except a variant field of a
    /// GENERIC enum. `Result<T, E>`'s `Ok { value: T }` carries whatever the call site put there — a
    /// record, a list, an Int — so rewriting it to `LogosInt` would erase the parameter and break every
    /// other use of the enum; the analysis may track such a payload, the emitter does not store it so.
    stored: HashSet<(Symbol, Symbol)>,
}

thread_local! {
    /// The program's [`BignumFields`], matching the interpreter's arbitrary-precision `Int`. Set once
    /// per program at codegen entry by [`set_bigint_struct_fields`]; read at struct-def emission,
    /// construction, and field access so the field's Rust type, its stored value, and every read agree.
    static BIGINT_STRUCT_FIELDS: std::cell::RefCell<BignumFields> =
        std::cell::RefCell::new(BignumFields::default());
}

/// Record the program's bignum fields for the current codegen (see [`BIGINT_STRUCT_FIELDS`]): all of
/// `fields` tracked, and stored as `LogosInt` unless the key is a variant of one of `registry`'s generic
/// enums.
pub(crate) fn set_bigint_struct_fields(
    fields: &HashSet<(Symbol, Symbol)>,
    registry: &TypeRegistry,
) {
    let generic_variants: HashSet<Symbol> = registry
        .iter_types()
        .filter_map(|(_, def)| match def {
            crate::analysis::registry::TypeDef::Enum {
                variants, generics, ..
            } if !generics.is_empty() => Some(variants.iter().map(|v| v.name)),
            _ => None,
        })
        .flatten()
        .collect();
    let stored = fields
        .iter()
        .copied()
        .filter(|(ty, _)| !generic_variants.contains(ty))
        .collect();
    BIGINT_STRUCT_FIELDS.with(|c| {
        *c.borrow_mut() = BignumFields {
            tracked: fields.clone(),
            stored,
        }
    });
}

/// True when `(struct_type or variant, field)` is written down as `LogosInt` — the field's Rust type
/// and every construction of it ask here.
pub(crate) fn stores_logos_int(struct_type: Symbol, field: Symbol) -> bool {
    BIGINT_STRUCT_FIELDS.with(|c| c.borrow().stored.contains(&(struct_type, field)))
}

/// The value a field stored as `LogosInt` is built with: the TOLERANT (un-narrowed) form of `value`
/// — the context a promoted `Let` binding uses — widened to a `LogosInt` (`From` is the identity on
/// one already, and promotes a machine word). A narrowed form would trap on the way in a bignum the
/// field exists to hold, and a bare machine word is not a `LogosInt` at all.
fn exact_field_value(value: &Expr, ecx: &ExprCtx) -> String {
    let tolerant = codegen_expr_ctx(
        value,
        &ExprCtx {
            int_exact_tolerant: true,
            ..*ecx
        },
    );
    format!("logicaffeine_data::LogosInt::from({tolerant})")
}

/// The whole promoted-field set, for an analysis that needs to ask about many fields at once
/// (`bigint_promote::promotable_int_vars`, deciding whether a local assigned from a field read
/// must hold a `LogosInt`). It is installed ONCE per program at codegen entry, so reading it here
/// is reading the same answer the struct definitions were emitted from — which is precisely what
/// used to differ.
pub(crate) fn bigint_struct_fields_snapshot() -> HashSet<(Symbol, Symbol)> {
    BIGINT_STRUCT_FIELDS.with(|c| c.borrow().tracked.clone())
}

/// Is a field of this NAME stored as `LogosInt`? Asked at the EMIT, through the same rule the
/// analysis used — see `bigint_promote::field_name_is_exact`.
pub(crate) fn is_exact_field_name(field: Symbol) -> bool {
    BIGINT_STRUCT_FIELDS
        .with(|c| super::bigint_promote::field_name_is_exact(&c.borrow().tracked, field))
}

/// True when `value` is an identifier of a non-Copy VALUE type — a user struct/enum or a `String` —
/// where storing it (into a struct field, an enum variant field, or a collection slot) COPIES it under
/// LOGOS value semantics, so the source stays usable. The default emit MOVES such an identifier (the
/// Identifier arm bare-moves a non-bignum non-Copy value), which fails E0382 when the source is used
/// again (json's `en`, built once and both stored and pushed). Cloning at the store is output-preserving
/// — a currently-working move was a last-use, and clone-then-drop is equivalent. EXCLUDED: Copy
/// primitives (a clone is pointless), references, `__bigint` (the Identifier arm already clones it), and
/// COLLECTIONS (`LogosSeq`/`LogosMap`/`Vec`/`HashMap`) — those could be large, and an O(n) clone per
/// store risks turning an O(n) loop into O(n²); their reuse is a separate, rarer concern.
/// The DECLARED type of `obj's field` when the object is a variable of a known record type, OR
/// itself another field read (`o's inner's value` — the object of the OUTER access is the INNER
/// `FieldAccess`, resolved by the same rule one level down).
///
/// `infer_logos_type` cannot answer this: it builds its environment from the Rust-type strings of
/// variables and has no type REGISTRY, so a field read comes back `Unknown`. Two AOT-Rust bugs
/// both reduced to that one blind spot — a `Text` field moved out of a row it was only read from,
/// and `p's left + p's right` emitted as `String + String`, which does not compile. Both need the
/// same fact: what is this field?
pub(crate) fn record_field_rust_type(
    expr: &Expr,
    interner: &Interner,
    variable_types: &HashMap<Symbol, String>,
    registry: &TypeRegistry,
) -> Option<String> {
    let Expr::FieldAccess { object, field } = expr else {
        return None;
    };
    let object = match object {
        Expr::Copy { expr } => *expr,
        other => other,
    };
    let rust_ty = match object {
        Expr::Identifier(var) => variable_types.get(var)?.clone(),
        Expr::FieldAccess { .. } => {
            record_field_rust_type(object, interner, variable_types, registry)?
        }
        _ => return None,
    };
    let base = rust_ty
        .split("|__hl:")
        .next()
        .unwrap_or(&rust_ty)
        .trim_start_matches('&');
    let declared = registry.iter_types().find_map(|(sym, def)| match def {
        crate::analysis::registry::TypeDef::Struct { fields, .. }
            if rust_type_ident(interner.resolve(*sym)) == base =>
        {
            Some(fields.iter().find(|f| f.name == *field).map(|fd| {
                super::absent::field(*sym, fd.name, super::types::codegen_field_type(&fd.ty, interner))
            }))
        }
        _ => None,
    });
    match declared {
        Some(found) => found,
        None => super::types::program_record_field_type(base, *field),
    }
}

/// The Rust type of the list a slice reads, as it is read: a binding's or a field's, and the list a value-or-nothing
/// holds when an enclosing operation has read it in place.
fn sliced_type(collection: &Expr, ecx: &ExprCtx) -> Option<String> {
    let ty = match collection {
        Expr::Identifier(sym) => ecx.variable_types.get(sym).cloned(),
        Expr::FieldAccess { .. } => record_field_rust_type(collection, ecx.interner, ecx.variable_types, ecx.registry),
        _ => value_rust_type(collection, ecx.variable_types, ecx.registry, ecx.interner),
    }?;
    let ty = ty.split("|__hl:").next().unwrap_or(&ty).to_string();
    match super::absent::read(collection).and(super::absent::payload(&ty)) {
        Some(held) => Some(held.to_string()),
        None => Some(ty),
    }
}

/// The items `items start through end of collection` names, as a borrowed view of the collection's buffer (`&[T]`):
/// what a reader that only reads them in place takes — a length, a membership, a borrowed parameter, a loop's
/// snapshot, a further slice, a copy.
pub(crate) fn slice_view(collection: &Expr, start: &Expr, end: &Expr, ecx: &ExprCtx) -> String {
    let coll_str = codegen_expr_ctx(collection, ecx);
    let bound = |e: &Expr| {
        super::rational::integer_operand(e, crate::semantics::collections::IntegerPlace::Slice, ecx)
            .unwrap_or_else(|| codegen_expr_ctx(e, ecx))
    };
    let start_str = bound(start);
    let end_str = bound(end);
    if matches!(sliced_type(collection, ecx).as_deref(), Some(t) if t.starts_with("LogosSeq")) {
        format!("{coll_str}.borrow().logos_slice({start_str}, {end_str})")
    } else {
        format!("{coll_str}.logos_slice({start_str}, {end_str})")
    }
}

/// Whether `expr` reads a counter CRDT — a `Tally` or `ConvergentCount` field or binding, held as a
/// `PNCounter`/`GCounter`. Comparison and equality see a counter as its Int total, as the
/// interpreter projects it (`semantics::compare::counter_as_int`), so such an operand reads
/// `.value()`. Everywhere else — an argument, a field initializer, a `Set`, a `Merge` — the counter
/// stays a counter.
fn is_counter_read(expr: &Expr, ecx: &ExprCtx) -> bool {
    let rust_ty = match expr {
        Expr::FieldAccess { .. } => {
            record_field_rust_type(expr, ecx.interner, ecx.variable_types, ecx.registry)
        }
        Expr::Identifier(sym) => ecx.variable_types.get(sym).cloned(),
        _ => None,
    };
    rust_ty.is_some_and(|t| {
        let t = t.trim_start_matches('&');
        let name = t.rsplit("::").next().unwrap_or(t);
        matches!(name, "PNCounter" | "GCounter")
    })
}

/// Argument `i` of a call to `function`, lowered for the callee's parameter: a read-only borrow,
/// an element `&mut` borrow, a value-semantics `mutable` collection, an exactified `LogosInt`, or an
/// owned value (a copy of anything a move would consume; a Real parameter's integer converted). The
/// one rule every call lowers its arguments with; [`call_arguments`] applies it to a whole call.
pub(super) fn call_argument(function: Symbol, i: usize, a: &Expr, ecx: &ExprCtx) -> String {
    call_argument_read(function, i, a, ecx, false)
}

/// Every argument of a call to `function`, lowered for the callee's parameters (see
/// [`call_argument`]). Arguments are values taken left to right, so an argument that borrows a
/// list an argument to its RIGHT lends to a `mutable` parameter reads a copy of its own: the lend
/// writes the list before the callee reads the borrow. Using a list again in the very call that
/// lends it never reaches here — the law of exclusivity refuses it.
pub(super) fn call_arguments(function: Symbol, args: &[&Expr], ecx: &ExprCtx) -> Vec<String> {
    let lent_later = lent_by_later_arguments(args);
    args.iter()
        .enumerate()
        .map(|(i, a)| {
            if super::tower::param_at(function, i) {
                return super::tower::value(a, ecx);
            }
            if let Some(code) = super::absent::argument(function, i, a, ecx) {
                return code;
            }
            let lent_after = matches!(a, Expr::Identifier(s) if lent_later[i].contains(s));
            call_argument_read(function, i, a, ecx, lent_after)
        })
        .collect()
}

/// `function(args)` as written with `rendered` arguments, sequenced so Rust can borrow-check it.
///
/// ★ A LEND IS A `&mut` FOR THE WHOLE CALL. When one argument lends a binding `&mut` — the program
/// record a lifted function takes, a record lent to a `mutable` parameter — and another argument
/// reads that binding (a field of the record, a nested call lending it too), Rust sees a shared
/// borrow while the exclusive one is live. The author's own overlapping lends are refused before
/// this (the law of exclusivity); these are the compiler's, made by the program-bindings lift, and
/// are sequenced instead: every argument that is not a lend is evaluated first, left to right, into
/// a temporary, and the call then lends. LOGOS evaluates a call's arguments before it runs the
/// callee, so evaluating them before the lend changes nothing a program can observe.
pub(super) fn sequenced_call(
    path: &str,
    function: Symbol,
    args: &[&Expr],
    mut rendered: Vec<String>,
    wait: Wait,
) -> String {
    let lent = super::fn_role_indices(function, super::FnRole::ValueMutable);
    let lends: Vec<(usize, Symbol)> = lent
        .iter()
        .filter(|i| rendered.get(**i).is_some_and(|r| r.starts_with("&mut ")))
        .filter_map(|i| {
            args.get(*i)
                .and_then(|a| lent_place_root(a))
                .map(|root| (*i, root))
        })
        .collect();
    let overlaps = lends.iter().any(|(at, root)| {
        args.iter()
            .enumerate()
            .any(|(j, a)| j != *at && mentions_binding(a, *root))
    });
    if !overlaps {
        return wait.around(format!("{path}({})", rendered.join(", ")));
    }
    let mut before = String::new();
    for (j, arg) in rendered.iter_mut().enumerate() {
        if lends.iter().any(|(at, _)| *at == j) {
            continue;
        }
        before.push_str(&format!("let __arg{j} = {arg}; "));
        *arg = format!("__arg{j}");
    }
    format!("{{ {before}{} }}", wait.around(format!("{path}({})", rendered.join(", "))))
}

thread_local! {
    /// The program's async functions that lie on a cycle of its call graph, installed at codegen entry
    /// ([`set_recursive_async`]).
    static RECURSIVE_ASYNC: std::cell::RefCell<HashSet<Symbol>> = std::cell::RefCell::new(HashSet::new());
}

/// Install the program's async functions that lie on a cycle of its call graph. Called once at codegen entry.
pub(crate) fn set_recursive_async(recursive: HashSet<Symbol>) {
    RECURSIVE_ASYNC.with(|c| *c.borrow_mut() = recursive);
}

/// How a call waits for the function it calls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Wait {
    /// The function does not await: the call is its value.
    No,
    /// The function awaits: the call is awaited.
    Await,
    /// The function awaits and lies on a cycle of the call graph: its future would hold itself, so the call's
    /// future is boxed before it is awaited.
    Boxed,
}

impl Wait {
    /// How a call of `function` waits, in a program whose async functions are `async_functions`.
    pub(crate) fn of(function: Symbol, async_functions: &HashSet<Symbol>) -> Wait {
        if !async_functions.contains(&function) {
            Wait::No
        } else if RECURSIVE_ASYNC.with(|c| c.borrow().contains(&function)) {
            Wait::Boxed
        } else {
            Wait::Await
        }
    }

    /// `call` waited for this way.
    pub(crate) fn around(self, call: String) -> String {
        match self {
            Wait::No => call,
            Wait::Await => format!("{call}.await"),
            Wait::Boxed => format!("::std::boxed::Box::pin({call}).await"),
        }
    }
}

/// The binding a lent place is rooted at: the binding, or the record a field of it belongs to.
fn lent_place_root(place: &Expr) -> Option<Symbol> {
    match place {
        Expr::Identifier(sym) => Some(*sym),
        Expr::FieldAccess { object, .. } => lent_place_root(object),
        _ => None,
    }
}

/// Whether `expr` names `binding` anywhere — read, lent, or as the root of a place.
fn mentions_binding(expr: &Expr, binding: Symbol) -> bool {
    let mut ids = HashSet::new();
    super::detection::collect_expr_identifiers(expr, &mut ids);
    ids.contains(&binding)
}

/// Whether a call anywhere in `expr` lends `binding` to a `mutable` parameter. A statement that
/// writes into `binding` evaluates such an expression before it touches its target — as every
/// tier's statement order has it — rather than inside the write.
pub(super) fn lends_binding(expr: &Expr, binding: Symbol) -> bool {
    let mut lends = false;
    crate::analysis::callgraph::for_each_call_site_in_expr(expr, &mut |function, inner| {
        for at in super::fn_role_indices(function, super::FnRole::ValueMutable) {
            if matches!(inner.get(at), Some(Expr::Identifier(s)) if *s == binding) {
                lends = true;
            }
        }
    });
    lends
}

/// `format!("{fmt}", values…)` over `operands`, taken left to right. `format!` borrows every
/// operand for the whole call, so when an operand's call lends a binding to a `mutable` parameter
/// the text is built from values taken first, in order — a field or binding read copied out
/// before the lend writes it, as every tier reads it.
fn format_in_order(fmt: &str, operands: &[&Expr], values: Vec<String>) -> String {
    let lends = operands.iter().any(|operand| {
        let mut lends = false;
        crate::analysis::callgraph::for_each_call_site_in_expr(operand, &mut |function, _| {
            lends |= !super::fn_role_indices(function, super::FnRole::ValueMutable).is_empty();
        });
        lends
    });
    if !lends {
        return format!("format!(\"{fmt}\", {})", values.join(", "));
    }
    let mut taken = String::from("{ ");
    for (i, (operand, value)) in operands.iter().zip(&values).enumerate() {
        let value = match operand {
            Expr::Identifier(_) | Expr::FieldAccess { .. } => format!("({value}).clone()"),
            _ => value.clone(),
        };
        taken.push_str(&format!("let __text_{i} = {value}; "));
    }
    let names: Vec<String> = (0..values.len()).map(|i| format!("__text_{i}")).collect();
    taken.push_str(&format!("format!(\"{fmt}\", {}) }}", names.join(", ")));
    taken
}

/// For each argument, the bindings that an argument to its right lends to a `mutable` parameter,
/// in a call anywhere inside it.
fn lent_by_later_arguments(args: &[&Expr]) -> Vec<HashSet<Symbol>> {
    let mut later = vec![HashSet::new(); args.len()];
    let mut lent: HashSet<Symbol> = HashSet::new();
    for (i, a) in args.iter().enumerate().rev() {
        later[i] = lent.clone();
        crate::analysis::callgraph::for_each_call_site_in_expr(a, &mut |function, inner| {
            for at in super::fn_role_indices(function, super::FnRole::ValueMutable) {
                if let Some(Expr::Identifier(s)) = inner.get(at) {
                    lent.insert(*s);
                }
            }
        });
    }
    later
}

fn call_argument_read(
    function: Symbol,
    i: usize,
    a: &Expr,
    ecx: &ExprCtx,
    lent_after: bool,
) -> String {
    let variable_types = ecx.variable_types;
    let names = RustNames::new(ecx.interner);
    let raw_name = names.raw(function);
    // The callee's calling roles: a readonly borrow (`&[T]`) and a value-semantics `mutable`
    // collection param can BOTH be present, so each role is read on its own.
    let callee_borrow_indices: HashSet<usize> = super::fn_role_indices(function, super::FnRole::Borrow);
    // `mutable` collection params (value semantics): pass the caller's
    // collection by shared reference (no clone) so the callee mutates it
    // in place — the explicit propagation escape hatch.
    let callee_value_mutable: HashSet<usize> = super::fn_role_indices(function, super::FnRole::ValueMutable);
    // Element `&mut` borrows: the callee writes a sequence's elements in place.
    let callee_mut_borrow_indices: HashSet<usize> = super::fn_role_indices(function, super::FnRole::MutBorrow);
    // An integer handed to a parameter that holds rationals is the rational it equals, a list of them item by item.
    if let Some(param) = fn_param_rust_type(function, i).filter(|t| t.contains(super::rational::RATIONAL)) {
        if !callee_value_mutable.contains(&i) && !callee_mut_borrow_indices.contains(&i) {
            if let Some(code) = super::rational::value_as(a, &super::rational::named(&param), ecx) {
                return if callee_borrow_indices.contains(&i) {
                    format!("&*({code}).borrow()")
                } else {
                    code
                };
            }
        }
    }
    // Recursively codegen args with full context.
    // Borrow params: pass &name (or pass through if already a slice).
    // Non-borrow params: clone non-Copy identifiers to avoid move-after-use.
    // A `wordN(...)` argument is codegen'd in `int_wrapping` mode (its Int arithmetic is mod 2^N);
    // every OTHER call resets the flag — a callee's argument is a full-width value, not truncated,
    // so `wordN(f(a * b))` must NOT wrap `a * b`.
    let arg_wrapping = matches!(raw_name, "word8" | "word16" | "word32" | "word64");
    if arg_wrapping {
        let value = codegen_expr_ctx(
            a,
            &ExprCtx {
                int_wrapping: true,
                int_exact_tolerant: true,
                ..*ecx
            },
        );
        return format!("(logicaffeine_data::LogosInt::from({value}).wrapping_u64() as i64)");
    }
    // Selective higher-order-overflow analysis (call-site half): a scalar exact-HOF
    // (`apply(f, x)`) exactifies its fn-param + int-params to `LogosInt`, and a `fn`-typed
    // param CALLED inside a HOF body (`f(x)`) is `impl Fn(LogosInt) -> LogosInt`. So a closure
    // argument renders exact, and an integer argument promotes to `LogosInt` — matching the VM,
    // where an overflowing closure produces a bignum. Seq-HOFs are NOT in `EXACT_HOFS`, so
    // `map`/`fold` closures stay `i64` (no regression).
    let callee_is_exact_hof = is_exact_hof_call(function);
    let callee_is_exact_fnptr = variable_types.get(&function).map_or(false, |t| t.contains(EXACT_FNPTR_MARKER));
    let exact_closure_arg = callee_is_exact_hof
        && fn_param_rust_type(function, i).map_or(false, |t| t.starts_with("impl Fn"));
    let exact_int_parameter = callee_is_exact_fnptr
        || (callee_is_exact_hof
            && fn_param_rust_type(function, i).as_deref() == Some("logicaffeine_data::LogosInt"));
    // A parameter declared as a machine word is a storage sink: the argument narrows into it, whatever exact chain
    // the call sits in.
    let word_parameter = fn_param_rust_type(function, i).as_deref() == Some("i64");
    let s = codegen_expr_ctx(
        a,
        &ExprCtx {
            int_wrapping: arg_wrapping,
            exact_closure: exact_closure_arg,
            int_exact_tolerant: exact_int_parameter || (ecx.int_exact_tolerant && !word_parameter),
            ..*ecx
        },
    );
    // Promote an integer argument to `LogosInt` when the callee's parameter is exactified
    // (an exact-HOF's `LogosInt` param, or any arg of an exact `fn`-typed param). An
    // already-promoted var passes by clone (never the `.expect_i64` narrow below); a raw
    // `i64` value/literal wraps in `LogosInt::from` (identity for an existing `LogosInt`).
    let promote_int = !matches!(a, Expr::Closure { .. })
        && (callee_is_exact_fnptr
            || (callee_is_exact_hof
                && fn_param_rust_type(function, i).as_deref()
                    == Some("logicaffeine_data::LogosInt")));
    if promote_int {
        if let Expr::Identifier(sym) = a {
            if variable_types
                .get(sym)
                .map_or(false, |t| t.contains("__bigint"))
            {
                return format!("{}.clone()", names.ident(*sym));
            }
        }
        return format!("logicaffeine_data::LogosInt::from({})", s);
    }
    if callee_value_mutable.contains(&i) {
        // Pass the caller's collection by shared reference (no clone)
        // so the callee mutates it in place. If the arg is already a
        // reference (a `mutable` param threaded through a nested call),
        // pass it as-is rather than re-referencing.
        if let Expr::Identifier(sym) = a {
            if variable_types.get(sym).is_some_and(|t| t.starts_with('&')) {
                return s;
            }
            // A binding whose type this emission did not record is lent as the callee's parameter says
            // it is: the parameter's type is the authority on what the call borrows.
            let held = variable_types.get(sym).cloned().or_else(|| fn_param_rust_type(function, i));
            // A record, a scalar, a `Set`: lent as `&mut` to the binding itself — or, passing on
            // a parameter lent the same way, to the place behind it.
            if !held.as_deref().is_some_and(lent_by_shared_reference) {
                return format!("&mut {}", s);
            }
            if !variable_types.contains_key(sym) && crate::semantics::collections::value_semantics_enabled() {
                return format!("{{ {s}.cow(); &{s} }}");
            }
        }
        // The callee writes the binding's buffer in place, so it must be the binding's own: a
        // value copied from it before the call keeps what it had.
        if let Some(own) = cow_handle(a, variable_types, &names) {
            return format!("{{ {own}.cow(); &{own} }}");
        }
        // A value that is not a binding — a field, a call's result — is lent as the callee's
        // parameter takes it: a record or a scalar as `&mut`.
        if fn_param_rust_type(function, i).is_some_and(|t| !lent_by_shared_reference(&t)) {
            return format!("&mut {}", s);
        }
        return format!("&{}", s);
    }
    if callee_mut_borrow_indices.contains(&i) {
        // An element `&mut` borrow: the callee writes the caller's elements in place.
        if let Expr::Identifier(sym) = a {
            let held = variable_types
                .get(sym)
                .map(|t| t.split("|__hl:").next().unwrap_or(t).to_string());
            return match held.as_deref() {
                Some(t) if t.starts_with("&mut [") => s,
                Some(t) if t.starts_with("Vec<") => format!("&mut {}", s),
                _ => format!("&mut *{}.borrow_mut()", s),
            };
        }
        return format!("&mut {}", s);
    }
    if callee_borrow_indices.contains(&i)
        && fn_param_rust_type(function, i).is_some_and(|t| lent_record_type(&t))
    {
        // A lent enum value (`codegen::lent`): a borrow the caller already holds passes through, an owned value — a
        // binding or a call's result — is lent for the call.
        if let Expr::Identifier(sym) = a {
            if variable_types.get(sym).is_some_and(|t| t.starts_with('&')) {
                return s;
            }
        }
        return format!("&({})", s);
    }
    if callee_borrow_indices.contains(&i) {
        // A slice handed to a borrowed parameter is the view of the items, never a copy of them.
        if let Expr::Slice { collection, start, end } = a {
            return slice_view(collection, start, end, ecx);
        }
        // Borrow param: pass &[T] reference instead of cloning
        if let Expr::Identifier(sym) = a {
            if let Some(ty) = variable_types.get(sym) {
                let ty = ty.split("|__hl:").next().unwrap_or(ty.as_str());
                if ty.starts_with("&[") || ty.starts_with("&mut [") {
                    return s; // Already a slice — pass through
                }
                if ty.starts_with("&str") {
                    return s; // Already a borrowed &str — pass through
                }
                if ty == "String" {
                    return format!("&{}", s); // &String coerces to &str (Text borrow)
                }
                if ty.starts_with("Vec<") {
                    return format!("&{}", s); // Vec<T> derefs to &[T]
                }
                if ty.starts_with('[') {
                    return format!("&{}", s); // fixed array [T; N] coerces to &[T]
                }
                // A list a later argument lends is written before this callee reads it, so the
                // borrow is of a copy taken now (see `call_arguments`).
                if ty.starts_with("LogosSeq") && lent_after {
                    return format!("&{}.to_vec()", s);
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
        // Most such expressions evaluate to LogosSeq<T> and borrow the temporary,
        // but a Text literal is a `String` and a slice-return call is already a
        // borrow — see `borrow_position_non_identifier`.
        if let Some(marshalled) = borrow_position_non_identifier(a, &s) {
            return marshalled;
        }
        format!("&*{}.borrow()", s)
    } else {
        // Regular param: clone non-Copy identifiers
        if let Expr::Identifier(sym) = a {
            // A `LogosInt` (promoted) var passed to a scalar `i64` param narrows to
            // the param's declared width — a loud panic if it exceeds i64 (the
            // i64-param contract). Functions do not yet take `LogosInt` params.
            if variable_types
                .get(sym)
                .map_or(false, |t| t.contains("__bigint"))
            {
                return format!("{}.expect_i64(\"Int\")", names.ident(*sym));
            }
            if let Some(ty) = variable_types.get(sym) {
                // A sequence this function holds BORROWED (`&[T]`, its own
                // read-only parameter) handed to a parameter that OWNS its
                // sequence: the callee gets the owned form it declares. A
                // specialized clone of a recursive function borrows what the
                // original, which it calls, owns.
                let held = ty.split("|__hl:").next().unwrap_or(ty.as_str());
                if held.starts_with("&[") || held.starts_with("&mut [") {
                    return match fn_param_rust_type(function, i) {
                        Some(p) if p.starts_with("Vec<") => format!("({}).to_vec()", s),
                        _ => format!("LogosSeq::from_vec(({}).to_vec())", s),
                    };
                }
                if !is_copy_type(ty) {
                    return format!("{}.clone()", s);
                }
            } else {
                // Unknown type (e.g. pattern-bound variable from Inspect):
                // clone defensively to avoid move-after-use in loops
                return format!("{}.clone()", s);
            }
        }
        // ★ A FIELD READ IS A READ, NOT A HANDOVER. Only `Expr::Identifier` was
        // considered here, so `shout(r's name)` MOVED the field and every later
        // mention of `r` failed to build — on a program the tree-walker and the VM
        // both run, so only a BUILD ever saw it.
        if field_arg_needs_clone(a, fn_param_rust_type(function, i)) {
            return format!("{}.clone()", s);
        }
        // An integer handed to a `Real` parameter is converted at the call, as the interpreter and
        // the VM convert it at the call boundary (`sin(0)`, `cConj(3, 4)`).
        if fn_param_rust_type(function, i).as_deref() == Some("f64")
            && !produces_float(a, variable_types, ecx.interner)
            && !mentions_bigint_var(a, variable_types)
        {
            return format!("(({}) as f64)", s);
        }
        s
    }
}

/// Whether a parameter declared with Rust type `ty` is lent as `&ty` when it sits at a borrow position: a value of the
/// program's own types. A sequence borrows as a slice and a text as `&str`, each with its own call-site conversion.
pub(super) fn lent_record_type(ty: &str) -> bool {
    let held = ty.split("|__hl:").next().unwrap_or(ty);
    !(held.starts_with("Vec<")
        || held.starts_with("LogosSeq<")
        || held.starts_with('[')
        || held.starts_with('&')
        || held == "String")
}

/// Whether a call to `function` lends the value it is given at argument `index` — a borrowed enum parameter
/// (`codegen::lent`), read from the function's calling roles.
pub(super) fn lends_record_at(function: Symbol, index: usize) -> bool {
    super::fn_role_indices(function, super::FnRole::Borrow).contains(&index)
        && fn_param_rust_type(function, index).is_some_and(|t| lent_record_type(&t))
}

/// Whether a binding of Rust type `ty` holds a lent record — `&T` for one of the program's own types.
pub(super) fn is_lent_record(ty: &str) -> bool {
    ty.strip_prefix('&')
        .is_some_and(|t| !t.starts_with("mut ") && lent_record_type(t))
}

/// The Rust path a call to the program's function `function` names: its own name, or `crate::name`
/// where a local value of that name is in scope. A value and a function may share a name and a call
/// always means the function (the loader's hygiene rule), while a Rust local would shadow it.
pub(super) fn called_function_path(
    function: Symbol,
    name: String,
    variable_types: &HashMap<Symbol, String>,
) -> String {
    let shadowed = program_defines_function(function) && variable_types.contains_key(&function);
    if shadowed {
        format!("crate::{name}")
    } else {
        name
    }
}

/// Whether the program declares a function named `function` itself — the one question a
/// builtin's name-keyed emission asks before it claims a call (declarer wins).
fn program_defines_function(function: Symbol) -> bool {
    FN_RETURN_TYPES.with(|c| c.borrow().contains_key(&function))
        || FN_PARAM_TYPES.with(|c| c.borrow().contains_key(&function))
}

/// True when that field's Rust type is one a MOVE would consume — a `String`, a collection, a
/// nested record. The Copy scalars are safe to read as-is.
fn field_read_moves(ty: &str) -> bool {
    !matches!(
        ty,
        "i64"
            | "i32"
            | "i16"
            | "i8"
            | "i128"
            | "isize"
            | "u64"
            | "u32"
            | "u16"
            | "u8"
            | "u128"
            | "usize"
            | "f64"
            | "f32"
            | "bool"
            | "char"
            | "()"
    )
}

thread_local! {
    /// While an outermost enum-variant construction is emitted: for each variable named inside it, how many
    /// times it is named in all and how many of those are still to be emitted (see the `NewVariant` arm). `None`
    /// outside any construction.
    static CONSTRUCTION_USES: std::cell::RefCell<Option<HashMap<Symbol, (usize, usize)>>> =
        const { std::cell::RefCell::new(None) };
}

thread_local! {
    /// The type parameters of the generic function whose body is being emitted. A value typed by
    /// one is never `Copy` as far as the body knows; storing it copies it (the parameter is bounded
    /// `Clone`). Installed for one function's emission by [`GenericParamsInScope`].
    static GENERIC_PARAMS: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The type parameters of one generic function, in scope while its signature and body are
/// emitted; the previous set comes back when this is dropped.
pub(crate) struct GenericParamsInScope(Vec<String>);

impl GenericParamsInScope {
    pub(crate) fn enter(params: Vec<String>) -> Self {
        Self(GENERIC_PARAMS.with(|g| std::mem::replace(&mut *g.borrow_mut(), params)))
    }
}

impl Drop for GenericParamsInScope {
    fn drop(&mut self) {
        GENERIC_PARAMS.with(|g| *g.borrow_mut() = std::mem::take(&mut self.0));
    }
}

thread_local! {
    /// The `mutable` collection parameters of the function whose body is being emitted. Each is
    /// the caller's own list or map, lent as a `&LogosSeq`/`&LogosMap`: its writes are meant to
    /// reach the caller, so it is never copied on write (and a `&` handle cannot give up its
    /// buffer). Installed for one function's emission by [`LentParamsInScope`].
    static LENT_PARAMS: std::cell::RefCell<HashSet<Symbol>> = std::cell::RefCell::new(HashSet::new());
}

/// The `mutable` collection parameters of one function, in scope while its body is emitted; the
/// previous set comes back when this is dropped.
pub(crate) struct LentParamsInScope(HashSet<Symbol>);

impl LentParamsInScope {
    pub(crate) fn enter(params: HashSet<Symbol>) -> Self {
        Self(LENT_PARAMS.with(|l| std::mem::replace(&mut *l.borrow_mut(), params)))
    }
}

impl Drop for LentParamsInScope {
    fn drop(&mut self) {
        LENT_PARAMS.with(|l| *l.borrow_mut() = std::mem::take(&mut self.0));
    }
}

/// Whether a `mutable` parameter of Rust type `rust_ty` is lent as a shared `&` reference: a
/// `LogosSeq`/`LogosMap` writes through its own `RefCell`, so `&` is all a callee needs, and a
/// shared reference can be passed on and read beside other borrows. Every other type — a record,
/// a scalar, a `Set` — is lent as `&mut`.
pub(crate) fn lent_by_shared_reference(rust_ty: &str) -> bool {
    rust_ty.starts_with("LogosSeq") || rust_ty.starts_with("LogosMap")
}

/// Whether `expr` reads a `mutable` parameter lent as `&mut` whole, by a value that is not `Copy`:
/// the value handed out is a copy, since the place behind the reference stays the caller's.
pub(crate) fn copies_out_of_a_lend(expr: &Expr, variable_types: &HashMap<Symbol, String>) -> bool {
    match expr {
        Expr::Identifier(sym) => {
            lent_in_place(*sym, variable_types) && identifier_read_moves(*sym, variable_types)
        }
        Expr::FieldAccess { .. } => {
            lent_field_type(expr, variable_types).is_some_and(|ty| field_read_moves(&ty))
        }
        _ => false,
    }
}

/// The Rust type of a field read out of a record lent in place (`(*b).f`, `(*b).f.g`) — what the
/// read would move out of the caller's record unless it copies. `None` for anything else.
fn lent_field_type(expr: &Expr, variable_types: &HashMap<Symbol, String>) -> Option<String> {
    match expr {
        Expr::Identifier(sym) if lent_in_place(*sym, variable_types) => {
            let ty = variable_types.get(sym)?;
            Some(ty.split("|__hl:").next().unwrap_or(ty).to_string())
        }
        Expr::FieldAccess { object, field } => {
            let record = lent_field_type(object, variable_types)?;
            super::types::program_record_field_type(&record, *field)
        }
        _ => None,
    }
}

/// Whether `sym` holds a borrowed slice (`&[T]`), a list parameter the function only reads: a record or a variant
/// owns its fields, so one storing it copies its items into a list of its own.
pub(crate) fn holds_a_borrowed_slice(sym: Symbol, variable_types: &HashMap<Symbol, String>) -> bool {
    variable_types.get(&sym).is_some_and(|t| t.starts_with("&["))
}

/// Whether `sym` is a `mutable` list or map parameter of the function being emitted, lent as the caller's own
/// buffer ([`lent_by_shared_reference`]): every write the body makes lands in the caller's list, so a value the body
/// copies out of it must own its items, never share that buffer.
pub(crate) fn lent_by_sharing(sym: Symbol, variable_types: &HashMap<Symbol, String>) -> bool {
    LENT_PARAMS.with(|l| l.borrow().contains(&sym))
        && variable_types.get(&sym).is_some_and(|t| lent_by_shared_reference(t.split("|__hl:").next().unwrap_or(t)))
}

/// Whether `sym` is a `mutable` parameter of the function being emitted that is lent as `&mut`
/// ([`lent_by_shared_reference`]): the body reads and writes it as the place `(*sym)`, so it is the
/// caller's own value every write lands on.
pub(crate) fn lent_in_place(sym: Symbol, variable_types: &HashMap<Symbol, String>) -> bool {
    LENT_PARAMS.with(|l| l.borrow().contains(&sym))
        && !variable_types
            .get(&sym)
            .is_some_and(|t| lent_by_shared_reference(t))
}

/// The Rust identifier of `collection` when writing it in place — or lending it to a `mutable`
/// parameter, which writes it in place — must first copy it on write: value semantics is on and
/// `collection` names an owned `LogosSeq`/`LogosMap` binding, whose shared `Rc` buffer a value
/// copied from it may still hold. `None` for every other handle:
/// - a de-Rc'd plain `Vec`/`HashMap` local, and the specialized `LogosI64Map`/`LogosI32Map`/
///   `LogosDense…`/set variants — none shares a buffer;
/// - a `mutable` parameter, the caller's own list lent as a `&LogosSeq`/`&LogosMap` (see
///   [`LentParamsInScope`]);
/// - an expression that is not a binding (a field, a temporary).
///
/// Under reference semantics (`LOGOS_VALUE_SEMANTICS=0`, or the compile-time PE reference scope) a
/// write is meant to reach every alias, so nothing is copied.
pub(crate) fn cow_handle(
    collection: &Expr,
    variable_types: &HashMap<Symbol, String>,
    names: &RustNames,
) -> Option<String> {
    if !crate::semantics::collections::value_semantics_enabled() {
        return None;
    }
    let (place, ty) = collection_place(collection, variable_types, names)?;
    lent_by_shared_reference(&ty).then_some(place)
}

/// The Rust place `collection` names and its Rust type, when it is a binding or a field of one
/// (`b's tape`, `(*b).tape` for a record lent as `&mut`). A copied record shares its lists with
/// the original until one of them writes, so a field is copied on write exactly as a binding is.
/// `None` for a `mutable` collection parameter — the caller's own list, lent to be written — and
/// for anything that is not a place.
fn collection_place(
    collection: &Expr,
    variable_types: &HashMap<Symbol, String>,
    names: &RustNames,
) -> Option<(String, String)> {
    match collection {
        Expr::Identifier(sym) => {
            if LENT_PARAMS.with(|l| l.borrow().contains(sym))
                && !lent_in_place(*sym, variable_types)
            {
                return None;
            }
            let ty = variable_types.get(sym)?;
            let ty = ty.split("|__hl:").next().unwrap_or(ty.as_str()).to_string();
            let place = if lent_in_place(*sym, variable_types) {
                format!("(*{})", names.ident(*sym))
            } else {
                names.ident(*sym)
            };
            Some((place, ty))
        }
        Expr::FieldAccess { object, field } => {
            let (place, record) = collection_place(object, variable_types, names)?;
            let ty = super::types::program_record_field_type(&record, *field)?;
            Some((
                format!(
                    "{place}.{}",
                    crate::codegen::escape_rust_ident(names.raw(*field))
                ),
                ty,
            ))
        }
        _ => None,
    }
}

/// Whether the Rust type `ty` names one of the type parameters in scope (`T`, `LogosSeq<T>`).
pub(crate) fn mentions_generic_param(ty: &str) -> bool {
    GENERIC_PARAMS.with(|g| {
        g.borrow().iter().any(|p| {
            ty.match_indices(p.as_str()).any(|(at, _)| {
                let before = ty[..at].chars().next_back();
                let after = ty[at + p.len()..].chars().next();
                let ident = |c: char| c.is_alphanumeric() || c == '_';
                !before.is_some_and(ident) && !after.is_some_and(ident)
            })
        })
    })
}

/// Does reading variable `sym` by value consume its binding? Only a `Copy` scalar leaves the binding
/// usable. An overflow-promoting `LogosInt` is already cloned by the identifier arm itself, and an
/// untracked variable may hold a record, so it counts as moving.
fn identifier_read_moves(sym: Symbol, types: &HashMap<Symbol, String>) -> bool {
    match types.get(&sym) {
        Some(t) => {
            let base = t.split("|__hl:").next().unwrap_or(t);
            !base.contains("__bigint") && field_read_moves(base)
        }
        None => true,
    }
}

/// Does passing this argument to a parameter of Rust type `param_ty` MOVE a field out of its
/// record? Only then is a clone owed.
///
/// ★ KEYED ON THE CALLEE'S PARAMETER, NOT THE RECORD'S FIELD — which is the whole reason this
/// works. `record_field_rust_type` needs `variable_types` to name the record, and that map holds
/// typed PARAMETERS and almost nothing else, so a rule built on it fires for `## To f (r: Row)`
/// and stays silent one line below for `Let r be a new Row`. A callee's signature, by contrast, is
/// always known: it is a declared function.
///
/// ★ AND IT STAYS NARROW, which the partial evaluator requires. `phase_futamura` ratchets on the
/// SIZE of a residual program, so a clone emitted where no move occurs is paid for by every
/// specialization. A borrowed parameter (`&…`) takes nothing; a `Copy` scalar parameter takes a
/// copy; only an owned non-`Copy` parameter consumes what it is given.
fn field_arg_needs_clone(arg: &Expr, param_ty: Option<String>) -> bool {
    let bare = match arg {
        Expr::Copy { expr } => *expr,
        other => other,
    };
    if !matches!(bare, Expr::FieldAccess { .. }) {
        return false;
    }
    param_ty.is_some_and(|t| !t.starts_with('&') && field_read_moves(&t))
}

/// Whether `value` is a bare variable holding a collection (a `LogosSeq`, `LogosMap`, `Vec` or `HashMap`): storing
/// it anywhere but at its last use needs a copy.
fn holds_collection(value: &Expr, types: &HashMap<Symbol, String>) -> bool {
    let Expr::Identifier(s) = value else {
        return false;
    };
    types.get(s).is_some_and(|t| {
        let base = t.split("|__hl:").next().unwrap_or(t);
        base.starts_with("LogosSeq")
            || base.starts_with("LogosMap")
            || base.starts_with("Vec<")
            || base.starts_with("HashMap")
    })
}

pub(crate) fn is_value_struct_identifier(value: &Expr, types: &HashMap<Symbol, String>) -> bool {
    // A value read can arrive wrapped in a `Copy` marker (`Expr::Copy { expr: Identifier(..) }`) — the
    // same unwrap the struct-New slice check does — so see through it to the underlying identifier.
    let value = match value {
        Expr::Copy { expr } => expr,
        other => other,
    };
    if let Expr::Identifier(s) = value {
        match types.get(s) {
            Some(t) => {
                let base = t.split("|__hl:").next().unwrap_or(t);
                if base == "String" {
                    return true;
                }
                let copy_or_collection = matches!(
                    base,
                    "i64"
                        | "i32"
                        | "i16"
                        | "i8"
                        | "i128"
                        | "isize"
                        | "u64"
                        | "u32"
                        | "u16"
                        | "u8"
                        | "u128"
                        | "usize"
                        | "f64"
                        | "f32"
                        | "bool"
                        | "char"
                        | "()"
                ) || base.starts_with('&')
                    || base.starts_with("LogosSeq")
                    || base.starts_with("LogosMap")
                    || base.starts_with("Vec<")
                    || base.starts_with("HashMap")
                    || base.contains("__bigint");
                return !copy_or_collection;
            }
            // Untracked type: the value may be a non-Copy struct/enum, so a value-semantics store must
            // copy it — clone rather than risk an E0382 move. Cloning a Copy value is a harmless no-op,
            // and large values are collections, which are TYPED above and excluded. (json's `en` is
            // untracked in the SetIndex handler's variable-type view.)
            None => return true,
        }
    }
    false
}

thread_local! {
    /// The whole-program set of SCALAR exact higher-order functions (`bigint_promote::exact_hof_fns`
    /// — an `Int`/`Nat` return + a `fn`-typed param + no seq param, e.g. `apply(f, x)`). Set once per
    /// program at codegen entry by [`set_exact_hofs`]; read at the call emit so a closure argument is
    /// rendered exact (`LogosInt` params) and an integer argument is promoted to `LogosInt` — the
    /// call-site half of the selective higher-order-overflow analysis. Overwritten per program, so no
    /// state leaks (codegen is single-threaded per program).
    static EXACT_HOFS: std::cell::RefCell<HashSet<Symbol>> =
        std::cell::RefCell::new(HashSet::new());
}

/// Record the program's scalar exact-HOF set for the current codegen (see [`EXACT_HOFS`]). Called
/// once at codegen entry.
pub(crate) fn set_exact_hofs(fns: &HashSet<Symbol>) {
    EXACT_HOFS.with(|c| *c.borrow_mut() = fns.clone());
}

/// True iff `function` is a scalar exact higher-order function (its fn-param + int-params + return
/// are exactified to `LogosInt`, so a closure argument must render exact and an integer argument
/// must promote).
fn is_exact_hof_call(function: Symbol) -> bool {
    EXACT_HOFS.with(|c| c.borrow().contains(&function))
}

/// The `|__exact_fnptr` sentinel a `fn`-typed parameter of an exact-HOF is registered with (its Rust
/// type is `impl Fn(LogosInt) -> LogosInt`). When such a param is CALLED (`f(x)` inside the HOF
/// body), its integer arguments promote to `LogosInt` — the callee-is-a-closure-param half of the
/// promotion.
const EXACT_FNPTR_MARKER: &str = "|__exact_fnptr";

thread_local! {
    /// The range oracle of the compile in progress ([`install_range_oracle`]):
    /// whether an Int op stays in i64 (`oracle_proves_int_op_in_range`) and
    /// whether an access names an item on every run ([`range_proves_index`]),
    /// including what the emitted program's own guards establish
    /// ([`mark_index_in_range`]). Computed on the SAME statement slice codegen
    /// walks, so expression addresses match. It does NOT drive borrow-hoisting /
    /// BCE / fast-div (those read the `ExprCtx` oracle). `None` outside a compile,
    /// so a freed arena's reused addresses never meet it.
    static RANGE_ORACLE: std::cell::RefCell<Option<std::rc::Rc<crate::optimize::OracleFacts>>> =
        const { std::cell::RefCell::new(None) };
}

/// Install the range oracle for one compile ([`RANGE_ORACLE`]); it is removed when the returned scope ends.
/// Every fact it holds is keyed by the addresses of that program's nodes, so nothing it proves may outlive
/// the compile — a later compile, or an emitter run outside one, sees no oracle at all.
pub(crate) fn install_range_oracle(
    o: Option<std::rc::Rc<crate::optimize::OracleFacts>>,
) -> RangeOracleScope {
    RANGE_ORACLE.with(|c| *c.borrow_mut() = o);
    RangeOracleScope(())
}

/// The lifetime of one compile's range oracle ([`install_range_oracle`]).
pub(crate) struct RangeOracleScope(());

impl Drop for RangeOracleScope {
    fn drop(&mut self) {
        RANGE_ORACLE.with(|c| *c.borrow_mut() = None);
    }
}

/// Whether a range oracle is installed now.
pub(crate) fn range_oracle_installed() -> bool {
    RANGE_ORACLE.with(|c| c.borrow().is_some())
}

thread_local! {
    /// Whole-program map: user function name → its parameters' Rust types (as `codegen_type_expr`
    /// spells them, e.g. `"f64"` for a `Real` parameter). Set once per program at codegen entry by
    /// [`set_fn_param_types`]; read at the user-call emit so an integer argument passed to an `f64`
    /// (`Real`) parameter is coerced with `as f64` — matching the interpreter/VM, which coerce
    /// int→Real at the call boundary, so `sin(0)` / `cConj(3, 4)` type-check on the AOT tier too.
    static FN_PARAM_TYPES: std::cell::RefCell<HashMap<Symbol, Vec<String>>> =
        std::cell::RefCell::new(HashMap::new());
}

/// Install the whole-program function → parameter-Rust-types map for this program. Called once at
/// codegen entry (overwrites any prior program's map, so it never goes stale).
pub(crate) fn set_fn_param_types(m: &HashMap<Symbol, Vec<String>>) {
    FN_PARAM_TYPES.with(|c| *c.borrow_mut() = m.clone());
}

/// The declared Rust type of `function`'s parameter at `index`, if that user function is known.
fn fn_param_rust_type(function: Symbol, index: usize) -> Option<String> {
    FN_PARAM_TYPES.with(|c| {
        c.borrow()
            .get(&function)
            .and_then(|v| v.get(index).cloned())
    })
}

/// The argument `Show x to f` hands `f`. Showing LENDS: the caller keeps `x`. A recipient taking a
/// reference (the built-in `show`, or a parameter lowered to `&str`/`&[T]`) gets `&x`; one whose
/// parameter owns its value (`(data: Text)` lowered to `String`) gets a copy.
pub(super) fn lend_to(recipient: &Expr, value: &str) -> String {
    let owned = match recipient {
        Expr::Identifier(f) => fn_param_rust_type(*f, 0).is_some_and(|t| !t.starts_with('&')),
        _ => false,
    };
    if owned {
        format!("({value}).clone()")
    } else {
        format!("&{value}")
    }
}

thread_local! {
    /// Whole-program map: user function name → its declared Rust RETURN type. The sibling of
    /// [`FN_PARAM_TYPES`], set at the same codegen entry, and read where a call's result type is
    /// inferred — so a program that declares its OWN function with a name math.lg also uses is typed
    /// by its own declaration rather than the stdlib's.
    static FN_RETURN_TYPES: std::cell::RefCell<HashMap<Symbol, String>> =
        std::cell::RefCell::new(HashMap::new());
}

/// Install the whole-program function → return-Rust-type map. Called once at codegen entry.
pub(crate) fn set_fn_return_types(m: &HashMap<Symbol, String>) {
    FN_RETURN_TYPES.with(|c| *c.borrow_mut() = m.clone());
}

/// `f` of the whole-program function → declared-return-Rust-type map.
pub(crate) fn with_declared_returns<R>(f: impl FnOnce(&HashMap<Symbol, String>) -> R) -> R {
    FN_RETURN_TYPES.with(|c| f(&c.borrow()))
}

/// True when `function`'s declared return type is an owned `String` or a borrowed `&str` — i.e. a
/// call whose result is a text value for the purposes of the `+` concat fold below, exactly like a
/// string literal or a `Text`-typed field read. Without this, `f() + g()` where BOTH `f` and `g`
/// return `Text` falls through to a bare `+`, and `String + String` has no operator impl (`rustc`
/// wants `&str` on the right) — the shape `hexByteText`'s own body hit (`charAtText(...) +
/// charAtText(...)`).
fn fn_call_returns_text(function: Symbol) -> bool {
    FN_RETURN_TYPES.with(|c| {
        c.borrow().get(&function).is_some_and(|t| {
            let t = t.split("|__hl:").next().unwrap_or(t);
            t == "String" || t == "&str"
        })
    })
}

thread_local! {
    /// Functions whose result is ALREADY a borrowed slice — the `Opt::TextSliceReturn` (`&'a str`)
    /// and `Opt::ByteSliceReturn` (`&'a [T]`) families. Kept apart from [`FN_RETURN_TYPES`] on
    /// purpose: this answers one question at one emit site (how to marshal the value at a borrow
    /// position) and must not perturb whole-program result-type inference.
    static BORROWED_RETURN_FNS: std::cell::RefCell<std::collections::HashSet<Symbol>> =
        std::cell::RefCell::new(std::collections::HashSet::new());
}

/// Install the borrowed-slice-returning set. Called once at codegen entry, after the two slice
/// analyses have run their caller-gates.
pub(crate) fn set_borrowed_return_fns(m: &std::collections::HashSet<Symbol>) {
    BORROWED_RETURN_FNS.with(|c| *c.borrow_mut() = m.clone());
}

/// How a NON-IDENTIFIER argument should reach a borrow-position parameter.
///
/// The identifier path above reads the variable's tracked Rust type and marshals accordingly; this
/// path had no type to read, so it assumed `LogosSeq<T>` and emitted `&*{expr}.borrow()` for
/// everything. Two shapes are not `LogosSeq` and the generated Rust did not compile:
///
/// * a Text literal is a `String` — `&*String::from("xx").borrow()` is
///   `E0599: no method named `borrow` found for struct `String``, which failed
///   `parity_str_trim_left_func` / `parity_str_trim_right_func`;
/// * a call to a slice-return fn is ALREADY `&[T]` — `&*sliceReal(..).borrow()` is
///   `E0599 … for reference `&[f64]``, which failed `convert_to_real_functions_compile_on_aot_rust`.
///
/// Returns the marshalled expression, or `None` to keep the `LogosSeq` default.
fn borrow_position_non_identifier(a: &Expr, rendered: &str) -> Option<String> {
    match a {
        // Already a borrow — hand it straight through.
        Expr::Call { function, .. }
            if BORROWED_RETURN_FNS.with(|c| c.borrow().contains(function)) =>
        {
            Some(rendered.to_string())
        }
        // `String`/`&str`-shaped values: `&String` coerces to `&str`, the same move the identifier
        // path makes for a `String` variable.
        Expr::Literal(Literal::Text(_)) | Expr::InterpolatedString(_) => {
            Some(format!("&{rendered}"))
        }
        Expr::Call { function, .. } => {
            let declared = FN_RETURN_TYPES.with(|c| c.borrow().get(function).cloned())?;
            let declared = declared
                .split("|__hl:")
                .next()
                .unwrap_or(&declared)
                .to_string();
            if declared == "String" || declared == "&str" || declared.starts_with("&[") {
                Some(if declared.starts_with('&') {
                    rendered.to_string()
                } else {
                    format!("&{rendered}")
                })
            } else if declared.starts_with("Vec<") || declared.starts_with('[') {
                Some(format!("&{rendered}"))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Does a call to `function` produce a float? A math.lg `-> Real` name does — UNLESS this program
/// declares a function of its own by that name, in which case the program's declaration wins.
///
/// The name list exists because the `TypeEnv` at these sites carries no function signatures; it is a
/// list of NAMES, so it cannot tell `math.lg`'s `gamma (z: Real) -> Real` from a program's own
/// `## To gamma (n: Int) -> Int`. It typed the latter's calls as `f64` and emitted
/// `gamma(n - 1) + ((1) as f64)` against an `i64` — Rust that does not compile. Same family as a
/// stdlib name shadowing a user's: whoever declared it in THIS program owns it.
pub(crate) fn is_float_returning_math_call(function: Symbol, interner: &Interner) -> bool {
    if let Some(declared) = FN_RETURN_TYPES.with(|c| c.borrow().get(&function).cloned()) {
        return declared == "f64";
    }
    is_float_returning_math_fn(interner.resolve(function))
}

/// The machine number a call to one of this program's functions returns, by its declaration: `i64` for an `-> Int`
/// lowered to a word, `f64` for a `-> Real`. A call typed `unknown` took a raw `+` beside an Int, which a word
/// overflows silently where every other tier answers exactly.
pub(crate) fn declared_numeric_return(function: Symbol) -> Option<&'static str> {
    FN_RETURN_TYPES.with(|c| {
        c.borrow()
            .get(&function)
            .and_then(|t| match t.split("|__hl:").next().unwrap_or(t) {
                "i64" => Some("i64"),
                "f64" => Some("f64"),
                _ => None,
            })
    })
}

thread_local! {
    /// A transient hint for the ONE promoted-`LogosInt` accumulator read that is a
    /// dead last-use: the accumulator in `Set p to <expr mentioning p exactly once>`,
    /// or the bare `Return p` of a bignum function. While set, the first value-position
    /// read of that symbol is emitted as a MOVE instead of `.clone()` — the read is
    /// immediately followed by a reassignment (or a return), so the old binding is
    /// dead. Set around a single statement's RHS codegen and consumed on the matching
    /// read (both the emitter and the read clear it), so exactly one move is ever
    /// licensed and no hint leaks to another statement. Codegen is single-threaded per
    /// program, so this never races.
    static MOVABLE_LAST_USE: std::cell::RefCell<Option<Symbol>> =
        const { std::cell::RefCell::new(None) };
}

/// Mark `sym` as movable at its next value-position read (see [`MOVABLE_LAST_USE`]);
/// `None` clears the mark. The caller sets this only when it has proven the read is a
/// dead last-use (the accumulator appears exactly once in the RHS it is reassigned by).
pub(crate) fn set_movable_last_use(sym: Option<Symbol>) {
    MOVABLE_LAST_USE.with(|c| *c.borrow_mut() = sym);
}

/// Whether `sym` is currently marked movable (a peek — the mark is cleared by the
/// emitter that set it, once the whole RHS is rendered). Peeking rather than
/// consuming is deliberate: the `BinaryOp` arm may render an operand more than once
/// (an early `left_str` the promoted-exact path then discards), and the mark is only
/// ever set when `sym` occurs exactly once in the AST — so at most one rendered read
/// reaches the emitted output, and every read of it is a safe move.
fn is_movable_last_use(sym: Symbol) -> bool {
    MOVABLE_LAST_USE.with(|c| *c.borrow() == Some(sym))
}

/// Does `e` PRODUCE a value stored as the overflow-promoting `LogosInt`? — a variable typed with
/// the `|__bigint` sentinel, or an inline call to a bignum-returning function. Such an operand
/// forces the exact-helper path so the result stays `LogosInt` rather than a raw `i64` operator
/// (or `as u64` cast) that `LogosInt` has no impl for.
fn mentions_bigint_var(e: &Expr, variable_types: &HashMap<Symbol, String>) -> bool {
    match e {
        Expr::Identifier(s) => variable_types
            .get(s)
            .map_or(false, |t| t.contains("__bigint")),
        // ★ A FIELD STORED AS `LogosInt` IS A BIGNUM OPERAND. `c's v * 2` emitted a plain `*`
        // between a `LogosInt` and an integer literal — "cannot multiply `LogosInt` by
        // `{integer}`" — because only a promoted VARIABLE counted. Reading a promoted field is the
        // same fact arriving by a different route.
        Expr::FieldAccess { field, .. } => is_exact_field_name(*field),
        Expr::Call { function, .. } => is_bigint_returning_call(*function),
        Expr::BinaryOp { left, right, .. } => {
            mentions_bigint_var(left, variable_types) || mentions_bigint_var(right, variable_types)
        }
        Expr::Not { operand } => mentions_bigint_var(operand, variable_types),
        _ => false,
    }
}

thread_local! {
    /// Fresh-temp counter for the by-ref bignum operand bindings (see
    /// [`bignum_operand_place`]). Monotonic across a program — each `let __lbop{n}`
    /// lands in its own nested block, so uniqueness only needs to avoid a same-name
    /// collision between two temps that share a scope; a global counter suffices.
    static BIGNUM_TMP: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// A borrowable `LogosInt` place for one operand of a by-reference bignum `+`/`-`/`*`
/// (the clone-eliding lowering at the exact-op emit). A promoted (`__bigint`) VARIABLE is
/// returned as its bare place (`acc`, or `(*x)` when boxed) so the enclosing method BORROWS it
/// — the entire point, since `logos_*_exact(a, b)` takes its operands by value and thus
/// `.clone()`s a reused bignum's heap buffer. Anything else — an i64 leaf, a numeric literal, a
/// nested arithmetic op, a bignum-returning call — is codegen'd in the exact-tolerant context and
/// bound once to a fresh `LogosInt` temp via `.into()` (a no-op move for a value already
/// `LogosInt`; a cheap `Small` wrap for an i64), whose name is returned. The result is usable
/// both as a receiver (`place.mul(..)`, which auto-borrows) and, prefixed with `&`, as the by-ref
/// argument (`&place`).
fn bignum_operand_place(
    operand: &Expr,
    ecx: &ExprCtx,
    names: &RustNames,
    prelude: &mut String,
) -> String {
    if let Expr::Identifier(sym) = operand {
        if ecx
            .variable_types
            .get(sym)
            .map_or(false, |t| t.contains("__bigint"))
        {
            let name = names.ident(*sym);
            return if ecx.boxed_bindings.contains(sym) {
                format!("(*{})", name)
            } else {
                name
            };
        }
    }
    let tol = ExprCtx {
        int_exact_tolerant: true,
        ..*ecx
    };
    let emitted = codegen_expr_ctx(operand, &tol);
    let n = BIGNUM_TMP.with(|c| {
        let v = c.get();
        c.set(v + 1);
        v
    });
    let tmp = format!("__lbop{}", n);
    prelude.push_str(&format!(
        "let {}: logicaffeine_data::LogosInt = ({}).into(); ",
        tmp, emitted
    ));
    tmp
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
    if crate::optimize::facts::holds_expr(crate::optimize::facts::Fact::RawIntOp, whole) {
        return true;
    }
    // Prefer the `ExprCtx` oracle (present on the borrow-hoisting path); else the
    // codegen-wide range oracle, computed for arithmetic guard-elimination even
    // when borrow-hoisting is off (the default, since `Scalarize` preempts
    // `HoistBorrows`). Same range analysis — only the availability differs.
    if let Some(o) = ecx.oracle {
        return oracle_facts_prove_int_op(o, whole, op, left, right);
    }
    RANGE_ORACLE.with(|r| match r.borrow().as_deref() {
        Some(o) => oracle_facts_prove_int_op(o, whole, op, left, right),
        None => false,
    })
}

/// The range check over a specific [`OracleFacts`]: is the exact result of
/// `left op right` provably inside i64? Add/Sub are monotone in each operand;
/// Mul takes the corner products. Div/Mod never qualify here (their rims are
/// divisor-shaped, not range-shaped).
fn oracle_facts_prove_int_op(
    o: &crate::optimize::OracleFacts,
    whole: &Expr,
    op: BinaryOpKind,
    left: &Expr,
    right: &Expr,
) -> bool {
    // If the oracle bounds the WHOLE expression to an i64 range (the affine
    // index proof `prev[w - wi]` gives `w - wi ∈ [0, len)`), it cannot overflow.
    if o.expr_int_range(whole).is_some() {
        return true;
    }
    let (Some((la, lb)), Some((ra, rb))) = (o.expr_int_range(left), o.expr_int_range(right)) else {
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

/// If `index` is a **coefficient-1 affine** expression in a `__zero_based_i64`
/// loop counter — `j`, `j + k`, `k + j`, or `j - k` (literal `k`) — return
/// `(counter_sym, delta)`. The 0-based array index is then `counter + delta`
/// with **no** 1-based `- 1` (the counter's rebase already absorbed it): original
/// `item (j + k) of arr` = `arr[(j₁ + k) - 1]` and with `j₀ = j₁ - 1` that is
/// `arr[j₀ + k]`. Only coefficient 1 is a clean `-1` cancellation (a `2*j` index
/// is not), so `2*j` etc. return `None` — and the loop-rebase eligibility gate
/// (`counter_only_used_for_indexing`) never zero-based-registers a counter used
/// in a non-coeff-1 index, so this stays sound.
pub(crate) fn zero_based_affine_index(
    index: &Expr,
    variable_types: &HashMap<Symbol, String>,
) -> Option<(Symbol, i64)> {
    let is_zb = |s: Symbol| {
        variable_types
            .get(&s)
            .map_or(false, |t| t == "__zero_based_i64")
    };
    match index {
        Expr::Identifier(s) if is_zb(*s) => Some((*s, 0)),
        Expr::BinaryOp {
            op: BinaryOpKind::Add,
            left,
            right,
        } => {
            if let (Expr::Identifier(s), Expr::Literal(Literal::Number(n))) = (left, right) {
                if is_zb(*s) {
                    return Some((*s, *n));
                }
            }
            if let (Expr::Literal(Literal::Number(n)), Expr::Identifier(s)) = (left, right) {
                if is_zb(*s) {
                    return Some((*s, *n));
                }
            }
            None
        }
        Expr::BinaryOp {
            op: BinaryOpKind::Subtract,
            left,
            right,
        } => {
            if let (Expr::Identifier(s), Expr::Literal(Literal::Number(n))) = (left, right) {
                if is_zb(*s) {
                    return Some((*s, -*n));
                }
            }
            None
        }
        _ => None,
    }
}

/// The 0-based **i64** index string for a coeff-1 affine zero-based index (`j`,
/// `(j + k)`, `(j - k)`), or `None` if `index` is not one. Shared by every indexed
/// read/write site; callers append `as usize` for indexing, or use it as-is (i64)
/// for the BCE `debug_assert!` bound.
pub(crate) fn zero_based_index_str(
    index: &Expr,
    variable_types: &HashMap<Symbol, String>,
    interner: &Interner,
) -> Option<String> {
    let (sym, delta) = zero_based_affine_index(index, variable_types)?;
    let name = RustNames::new(interner).ident(sym);
    Some(if delta == 0 {
        name.to_string()
    } else if delta > 0 {
        format!("({} + {})", name, delta)
    } else {
        format!("({} - {})", name, -delta)
    })
}

/// The 1-based index `index` names, as an i64 expression for `LogosItems::logos_item`: a loop counter the
/// codegen rebased to 0 gets its 1 back, and anything else is the index as written. The item accessor holds
/// the whole rule, so no index is folded here: `(i - 1) as usize` means something else once `i` is negative.
pub(crate) fn logos_index_str(
    index: &Expr,
    ecx: &ExprCtx,
    variable_types: &HashMap<Symbol, String>,
    interner: &Interner,
) -> String {
    rebased_logos_index(index, variable_types, interner).unwrap_or_else(|| {
        codegen_expr_ctx(
            index,
            &ExprCtx {
                int_index_context: true,
                ..*ecx
            },
        )
    })
}

/// The 1-based index a coefficient-1 affine index in a rebased loop counter names (`j`, `j + k`,
/// `j - k`, the counter 0-based): the counter with its 1 given back. `None` for any other index.
pub(crate) fn rebased_logos_index(
    index: &Expr,
    variable_types: &HashMap<Symbol, String>,
    interner: &Interner,
) -> Option<String> {
    let (sym, delta) = zero_based_affine_index(index, variable_types)?;
    let name = RustNames::new(interner).ident(sym);
    let k = delta + 1;
    Some(if k == 0 {
        name.to_string()
    } else if k > 0 {
        format!("({} + {})", name, k)
    } else {
        format!("({} - {})", name, -k)
    })
}

/// The 0-based offset of an index the oracle proved names an item (so it is at least 1), for an unchecked
/// access: a rebased loop counter as it is, and `X + K` with its `- 1` folded into `K`.
pub(crate) fn proven_offset_str(
    index: &Expr,
    ecx: &ExprCtx,
    variable_types: &HashMap<Symbol, String>,
    interner: &Interner,
) -> String {
    let i = |e: &Expr| {
        codegen_expr_ctx(
            e,
            &ExprCtx {
                int_index_context: true,
                ..*ecx
            },
        )
    };
    if let Some(zb) = zero_based_index_str(index, variable_types, interner) {
        return format!("{} as usize", zb);
    }
    match index {
        Expr::Literal(Literal::Number(n)) => format!("{}", n - 1),
        Expr::BinaryOp {
            op: BinaryOpKind::Add,
            left,
            right,
        } => match (left, right) {
            (_, Expr::Literal(Literal::Number(1))) => match left {
                Expr::Identifier(_) => format!("{} as usize", i(left)),
                _ => format!("({}) as usize", i(left)),
            },
            (Expr::Literal(Literal::Number(1)), _) => match right {
                Expr::Identifier(_) => format!("{} as usize", i(right)),
                _ => format!("({}) as usize", i(right)),
            },
            (_, Expr::Literal(Literal::Number(k))) if *k > 1 => {
                format!("({} + {}) as usize", i(left), k - 1)
            }
            (Expr::Literal(Literal::Number(k)), _) if *k > 1 => {
                format!("({} + {}) as usize", i(right), k - 1)
            }
            _ => format!("({} - 1) as usize", i(index)),
        },
        _ => format!("({} - 1) as usize", i(index)),
    }
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

/// Predicts whether `codegen_expr_ctx(e, ecx)` yields a pure-`i64` Rust expression rather than a
/// promoted `LogosInt`. An exactable Int `BinaryOp` promotes to `logos_*_exact` (→ `LogosInt`) UNLESS
/// it const-folds in range, is a safe literal `/`/`%`, or the Oracle proves it in i64 range — and it
/// is pure-i64 only when EVERY exactable descendant is too. A raw `left op right` composed with a
/// promoted operand is a type error (`LogosInt + i64`, E0369): the Oracle can prove an OUTER sum in
/// range while an INNER sub-sum promotes (its operand ranges unknown), so the raw fast-paths gate on
/// this and fall through to the Into-coercing exact helper when a child would promote. Conservative:
/// a node whose rawness it can't confirm (e.g. an i128/dual narrowed chain) reads as promoting, which
/// only over-routes to the exact helper — never emits the type-broken mix.
fn emits_pure_i64(e: &Expr, ecx: &ExprCtx) -> bool {
    let Expr::BinaryOp { left, right, .. } = e else {
        return true;
    };
    if !is_exact_arith_node(e) {
        return true;
    }
    int_op_stays_raw(e, ecx) && emits_pure_i64(left, ecx) && emits_pure_i64(right, ecx)
}

/// Whether the exactable Int node `e` ITSELF emits raw i64, whatever its operands emit: it const-folds within i64,
/// is a `/` or `%` by a safe literal, or the Oracle proves it within i64 — so its exact value is its i64 value.
fn int_op_stays_raw(e: &Expr, ecx: &ExprCtx) -> bool {
    let Expr::BinaryOp { op, left, right } = e else {
        return false;
    };
    if mentions_bigint_var(e, ecx.variable_types) {
        return false;
    }
    if let (Expr::Literal(Literal::Number(a)), Expr::Literal(Literal::Number(b))) = (left, right) {
        matches!(const_exact_int(*op, *a, *b), Some(ConstExact::InRange))
    } else {
        let safe_lit_div_mod = matches!(right, Expr::Literal(Literal::Number(d))
            if (matches!(op, BinaryOpKind::Divide) && *d != 0 && *d != -1)
                || (matches!(op, BinaryOpKind::Modulo) && *d != 0));
        safe_lit_div_mod || oracle_proves_int_op_in_range(ecx, e, *op, left, right)
    }
}

/// What the Rust emitter proves about the integers of one expression context, for [`super::lowbits`].
struct RustWords<'c, 'a> {
    ecx: &'c ExprCtx<'a>,
}

impl super::lowbits::WordProof for RustWords<'_, '_> {
    fn stays_in_word(&self, node: &Expr) -> bool {
        int_op_stays_raw(node, self.ecx)
    }
    fn is_word(&self, operand: &Expr) -> bool {
        // An item of a sequence held at a narrower integer width (`Vec<i32>`, a byte view) is that width's value,
        // and a cast to `u32` reads its low bits as well as an `i64`'s.
        let narrow_item = matches!(operand, Expr::Index { collection, .. }
            if seq_element_rust_type(collection, self.ecx.variable_types)
                .is_some_and(|t| matches!(t, "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32")));
        !mentions_bigint_var(operand, self.ecx.variable_types)
            && !is_counter_read(operand, self.ecx)
            && (narrow_item
                || numeric_type_through_arith(operand, self.ecx.interner, self.ecx.variable_types)
                    == "i64")
            && emits_pure_i64(operand, self.ecx)
    }
    fn is_nonnegative(&self, value: &Expr) -> bool {
        is_nonneg_expr(value)
    }
}

/// The range the oracle proves for `e`: the `ExprCtx` oracle where the borrow-hoisting path carries one, else the
/// codegen-wide range oracle.
fn oracle_int_range(ecx: &ExprCtx, e: &Expr) -> Option<(i64, i64)> {
    if let Some(o) = ecx.oracle {
        return o.expr_int_range(e);
    }
    RANGE_ORACLE.with(|r| r.borrow().as_deref().and_then(|o| o.expr_int_range(e)))
}

/// Opt::ModWrap: `x % m` for a literal `m > 0` and an `x` the range oracle bounds to `[0, 2m)`, as one conditional
/// subtraction, or as `x` itself below `m` ([`crate::optimize::modwrap`]). The oracle's bound on `x` is the same proof that
/// emits it as a raw machine word.
fn try_mod_wrap(expr: &Expr, ecx: &ExprCtx) -> Option<String> {
    let Expr::BinaryOp {
        op: BinaryOpKind::Modulo,
        left,
        right,
    } = expr
    else {
        return None;
    };
    let Expr::Literal(Literal::Number(m)) = right else {
        return None;
    };
    if !crate::optimize::active_config().is_on(crate::optimization::Opt::ModWrap)
        || mentions_bigint_var(left, ecx.variable_types)
        || infer_numeric_type(left, ecx.interner, ecx.variable_types) != "i64"
    {
        return None;
    }
    let range = oracle_int_range(ecx, left).map(|(lo, hi)| (i128::from(lo), i128::from(hi)));
    let wrap = crate::optimize::modwrap::mod_wrap(*m, range)?;
    crate::optimize::mark_fired(crate::optimization::Opt::ModWrap);
    let word = ExprCtx {
        int_exact_tolerant: false,
        ..*ecx
    };
    let x = codegen_expr_ctx(left, &word);
    Some(spell_mod_wrap(wrap, &x, &m.to_string(), "i64"))
}

/// The Rust a [`crate::optimize::modwrap::ModWrap`] reduction of `l % r` reads as, on words of Rust type `ty`.
fn spell_mod_wrap(wrap: crate::optimize::modwrap::ModWrap, l: &str, r: &str, ty: &str) -> String {
    match wrap {
        crate::optimize::modwrap::ModWrap::Identity => format!("({l})"),
        crate::optimize::modwrap::ModWrap::Once => format!(
            "{{ let __wrap: {ty} = {l}; if __wrap >= {r} {{ __wrap - {r} }} else {{ __wrap }} }}"
        ),
    }
}

/// The range the codegen-wide range oracle proves for `e`, for an emitter that carries no `ExprCtx`.
fn codegen_oracle_range(e: &Expr) -> Option<(i64, i64)> {
    RANGE_ORACLE.with(|r| r.borrow().as_deref().and_then(|o| o.expr_int_range(e)))
}

/// `l % r` on raw words of Rust type `ty` (`i64` or `i128`), where `r` is the nonzero literal `divisor` and `l` the
/// emitted `dividend`: under Opt::ModWrap, one conditional subtraction, or `l` itself, where `range` (the oracle's
/// bound on `dividend`) allows it ([`crate::optimize::modwrap`]); the machine remainder otherwise. Every raw emitter spells
/// its literal remainder through this one function.
fn raw_literal_rem(divisor: i64, range: Option<(i64, i64)>, l: &str, r: &str, ty: &str) -> String {
    if crate::optimize::active_config().is_on(crate::optimization::Opt::ModWrap) {
        let range = range.map(|(lo, hi)| (i128::from(lo), i128::from(hi)));
        if let Some(wrap) = crate::optimize::modwrap::mod_wrap(divisor, range) {
            crate::optimize::mark_fired(crate::optimization::Opt::ModWrap);
            return spell_mod_wrap(wrap, l, r, ty);
        }
    }
    format!("({l} % {r})")
}

/// Opt::MaskNarrow: `expr` computed in `u32` wrapping arithmetic under its low mask, when it is an integer chain
/// under one (see [`super::lowbits`]). Every operand is emitted as the i64 it is (never a tolerant `LogosInt`) and
/// read as its low 32 bits.
fn try_mask_narrow(expr: &Expr, ecx: &ExprCtx) -> Option<String> {
    if !crate::optimize::active_config().is_on(crate::optimization::Opt::MaskNarrow) {
        return None;
    }
    let masked = super::lowbits::masked_chain(expr, &RustWords { ecx })?;
    crate::optimize::mark_fired(crate::optimization::Opt::MaskNarrow);
    let operands = ExprCtx {
        int_exact_tolerant: false,
        ..*ecx
    };
    Some(format!(
        "((({}) & {}u32) as i64)",
        rust_low_bits(&masked.chain, &operands),
        masked.mask
    ))
}

fn rust_low_bits(chain: &super::lowbits::LowBits, ecx: &ExprCtx) -> String {
    use super::lowbits::{LowBits, LowOp};
    match chain {
        LowBits::Word(e) => format!("(({}) as u32)", codegen_expr_ctx(e, ecx)),
        LowBits::Const(c) => format!("{c}u32"),
        LowBits::Shl(c, s) => format!("({} << {s})", rust_low_bits(c, ecx)),
        LowBits::Op(op, l, r) => {
            let (l, r) = (rust_low_bits(l, ecx), rust_low_bits(r, ecx));
            match op {
                LowOp::Add => format!("{l}.wrapping_add({r})"),
                LowOp::Sub => format!("{l}.wrapping_sub({r})"),
                LowOp::Mul => format!("{l}.wrapping_mul({r})"),
                LowOp::And => format!("({l} & {r})"),
                LowOp::Or => format!("({l} | {r})"),
                LowOp::Xor => format!("({l} ^ {r})"),
            }
        }
    }
}

/// `infer_numeric_type` resolved THROUGH exactable arithmetic. `infer_numeric_type` reads a direct
/// `item k of arr` as its scalar element type (its Index special-case), but its TypeEnv can't peel a
/// scalarized `[i64; N]` element back out through an arithmetic node, so `(item i of arr) + (item j of
/// arr)` reports `unknown`. That inconsistency made a leaf index-pair enter the exact-int block (and
/// promote to `logos_*_exact` → `LogosInt`) while the enclosing add — seeing an `unknown` operand —
/// skipped the block and emitted a raw `i64 +`, composing `LogosInt + i64` (E0369). Resolving arith
/// nodes by their operands (float dominates) makes the whole chain enter the block as one consistent
/// unit. Only the BinaryOp handler's `left_type`/`right_type` use this — the blast radius is this one
/// lowering, not the global `infer_numeric_type`.
fn numeric_type_through_arith(
    e: &Expr,
    interner: &Interner,
    variable_types: &HashMap<Symbol, String>,
) -> &'static str {
    if is_exact_arith_node(e) {
        if let Expr::BinaryOp { left, right, .. } = e {
            let lt = numeric_type_through_arith(left, interner, variable_types);
            let rt = numeric_type_through_arith(right, interner, variable_types);
            if lt == "f64" || rt == "f64" {
                return "f64";
            }
            if lt == "i64" && rt == "i64" {
                return "i64";
            }
        }
    }
    infer_numeric_type(e, interner, variable_types)
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
                BinaryOpKind::Modulo if nonzero_lit => {
                    let Expr::Literal(Literal::Number(m)) = &**right else { unreachable!() };
                    raw_literal_rem(*m, oracle_int_range(ecx, left), &l, &r, "i128")
                }
                BinaryOpKind::Modulo => format!("logos_rem_i128({}, {})", l, r),
                _ => unreachable!("is_exact_arith_node covers exactly these five"),
            }
        }
        _ => {
            let plain = ExprCtx {
                int_exact_tolerant: false,
                ..*ecx
            };
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
    let plain = ExprCtx {
        int_exact_tolerant: false,
        ..*ecx
    };
    if !is_exact_arith_node(left) && !is_exact_arith_node(right) {
        // Division/modulo by a non-zero constant literal never traps — no div-by-zero, and no
        // `i64::MIN / -1` overflow when the divisor is not `-1` — so the checked `logos_div_i64`/
        // `logos_rem_i64` helper is dead weight. Emit the raw op (LLVM turns `/ C` into a
        // multiply-shift). Mirrors the i128 chain's `safe_lit_div_mod`.
        let safe_lit_div_mod = matches!(right, Expr::Literal(Literal::Number(d))
            if (matches!(op, BinaryOpKind::Divide) && *d != 0 && *d != -1)
                || (matches!(op, BinaryOpKind::Modulo) && *d != 0));
        if safe_lit_div_mod {
            let l = codegen_expr_ctx(left, &plain);
            let r = codegen_expr_ctx(right, &plain);
            let sym = if matches!(op, BinaryOpKind::Divide) {
                "/"
            } else {
                "%"
            };
            return Some(format!("({} {} {})", l, sym, r));
        }
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
fn try_i128_chain(op: BinaryOpKind, left: &Expr, right: &Expr, ecx: &ExprCtx) -> Option<String> {
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
        BinaryOpKind::Modulo if nonzero_lit => {
            let Expr::Literal(Literal::Number(m)) = right else { unreachable!() };
            raw_literal_rem(*m, oracle_int_range(ecx, left), &le, &re, "i128")
        }
        BinaryOpKind::Modulo => format!("logos_rem_i128({}, {})", le, re),
        _ => unreachable!("root op matched above"),
    };
    Some(format!("logos_narrow_i128({})", chain))
}

/// The key a guarded dual knows a leaf by: a name's symbol (every occurrence in one expression holds the one value
/// the guard tests), any other leaf its node.
fn dual_leaf_key(e: &Expr) -> usize {
    match e {
        Expr::Identifier(s) => usize::MAX - s.index(),
        _ => e as *const Expr as usize,
    }
}

/// Chain magnitude with every non-literal leaf capped at `bound`, or at its own magnitude when the Oracle proves its
/// range (`proven`, by [`dual_leaf_key`]) — the solver's evaluation function for the guarded-dual tier.
fn chain_magnitude_with_leaf_bound(
    e: &Expr,
    bound: u128,
    proven: &HashMap<usize, u128>,
) -> Option<u128> {
    match e {
        Expr::Literal(Literal::Number(n)) => Some(n.unsigned_abs() as u128),
        Expr::BinaryOp { op, left, right } if is_exact_arith_node(e) => {
            let l = chain_magnitude_with_leaf_bound(left, bound, proven)?;
            let r = chain_magnitude_with_leaf_bound(right, bound, proven)?;
            match op {
                BinaryOpKind::Add | BinaryOpKind::Subtract => l.checked_add(r),
                BinaryOpKind::Multiply => l.checked_mul(r),
                BinaryOpKind::Divide => Some(l),
                BinaryOpKind::Modulo => Some(r.min(l)),
                _ => unreachable!("is_exact_arith_node covers exactly these five"),
            }
        }
        _ => Some(proven.get(&dual_leaf_key(e)).copied().unwrap_or(bound)),
    }
}

/// Emit a chain over PRE-BOUND leaf names. `raw` picks plain i64 ops (the
/// guarded fast branch — proven in range by the leaf-bound guard); otherwise
/// the promoting `LogosInt` helpers (the exact slow branch). `/`/`%` keep
/// their canonical zero-divisor errors in both forms.
fn emit_chain_with_bound_leaves(e: &Expr, names: &HashMap<usize, String>, raw: bool) -> String {
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
                    BinaryOpKind::Modulo if nonzero_lit => {
                        let Expr::Literal(Literal::Number(m)) = &**right else { unreachable!() };
                        raw_literal_rem(*m, codegen_oracle_range(left), &l, &r, "i64")
                    }
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
/// register-shaped, vectorizable hot path behind one predictable branch. The
/// slow branch runs out of line (`logos_cold_path`): inlined, its big-integer
/// code crowds the loop it guards. Its closure takes its machine words by
/// value: one borrowing a loop counter into an out-of-line call would pin the
/// counter to memory for the whole loop.
fn try_guarded_dual_chain<'e>(
    op: BinaryOpKind,
    left: &'e Expr<'e>,
    right: &'e Expr<'e>,
    whole_ecx: &ExprCtx,
) -> Option<String> {
    let dual = guarded_dual_parts(op, left, right, whole_ecx, "__chain_l")?;
    Some(match dual.guard {
        Some(guard) => format!(
            "{{ {}if {guard} {{ {} }} else {{ logos_cold_path(move || {}.expect_i64(\"Int\")) }} }}",
            dual.binds, dual.fast, dual.slow
        ),
        None => format!("{{ {}{} }}", dual.binds, dual.fast),
    })
}

/// An exact Int chain as checked machine-word arithmetic: its leaves bound once (`binds`), `checked` — an
/// `Option<i64>` that is the chain's value when every step fits a word and `None` the moment one does not (a `/` or
/// `%` by zero included) — and `exact`, the chain's exact `LogosInt` over the same names, for the branch `None` takes.
pub(crate) struct CheckedChain {
    pub(crate) binds: String,
    pub(crate) checked: String,
    pub(crate) exact: String,
}

fn checked_chain_parts<'e>(e: &'e Expr<'e>, ecx: &ExprCtx, prefix: &str) -> Option<CheckedChain> {
    fn emit(e: &Expr, names: &HashMap<usize, String>) -> String {
        match e {
            Expr::Literal(Literal::Number(n)) => format!("({n}i64)"),
            Expr::BinaryOp { op, left, right } if is_exact_arith_node(e) => {
                let step = match op {
                    BinaryOpKind::Add => "checked_add",
                    BinaryOpKind::Subtract => "checked_sub",
                    BinaryOpKind::Multiply => "checked_mul",
                    BinaryOpKind::Divide => "checked_div",
                    _ => "checked_rem",
                };
                format!("{}.{step}({})?", emit(left, names), emit(right, names))
            }
            _ => names[&dual_leaf_key(e)].clone(),
        }
    }
    if !is_exact_arith_node(e) {
        return None;
    }
    let (leaves, exprs) = chain_leaves(&[e], ecx)?;
    if leaves.len() > 6 {
        return None;
    }
    let (binds, names) = bind_chain_leaves(&leaves, &exprs, ecx, prefix);
    Some(CheckedChain {
        binds,
        checked: format!("(|| -> Option<i64> {{ Some({}) }})()", emit(e, &names)),
        exact: emit_chain_with_bound_leaves(e, &names, false),
    })
}

/// A bit operation (`&`, `|`, `^`, `<<`, `>>`) over an Int operand the Oracle cannot keep within a machine word — an
/// exactable chain like `0 - available` — on machine words: each such operand computed as a [`CheckedChain`], the raw
/// operation when every step fit a word, and otherwise the exact form (`logos_band_exact` with the operand's exact
/// value, or `logos_word_operand`'s refusal of a big integer), out of line in `logos_cold_path` so its big-integer code
/// stays out of the hot function. One overflow check per step, exact at the very edge of the word. A plain operand
/// evaluated anywhere but first is bound before the chains' leaves, so evaluation stays left to right. Value- and
/// refusal-identical to the exact form; `None` when an operand that may promote is not such a chain (a promoted
/// binding, a call returning a big integer), which keeps the exact form.
fn try_guarded_bitwise<'e>(
    op: BinaryOpKind,
    op_str: &str,
    left: &'e Expr<'e>,
    right: &'e Expr<'e>,
    ecx: &ExprCtx,
    promotes: impl Fn(&Expr) -> bool,
) -> Option<String> {
    let plain = ExprCtx {
        int_exact_tolerant: false,
        ..*ecx
    };
    let mut binds = String::new();
    let mut checked = Vec::new();
    let mut words = Vec::new();
    let mut slow = Vec::new();
    for (side, e) in [("l", left), ("r", right)] {
        if promotes(e) {
            if mentions_bigint_var(e, ecx.variable_types) {
                return None;
            }
            let chain = checked_chain_parts(e, ecx, &format!("__bw{side}"))?;
            binds.push_str(&chain.binds);
            let word = format!("__bw{side}v");
            checked.push((word.clone(), chain.checked));
            words.push(word);
            slow.push(Some(chain.exact));
        } else {
            let code = codegen_expr_ctx(e, &plain);
            if let Expr::Literal(Literal::Number(n)) = e {
                words.push(format!("{n}i64"));
            } else if matches!(e, Expr::Identifier(_)) {
                words.push(code);
            } else {
                let name = format!("__bw{side}");
                binds.push_str(&format!("let {name}: i64 = {code}; "));
                words.push(name);
            }
            slow.push(None);
        }
    }
    let operand = |k: usize| -> String {
        match &slow[k] {
            Some(exact) if matches!(op, BinaryOpKind::BitAnd) => exact.clone(),
            Some(exact) => {
                let refusal = match op {
                    BinaryOpKind::BitOr => "BITOR_OPERANDS",
                    BinaryOpKind::BitXor => "BITXOR_OPERANDS",
                    BinaryOpKind::Shl => "SHL_OPERANDS",
                    _ => "SHR_OPERANDS",
                };
                format!("logos_word_operand({exact}, logicaffeine_data::failure::{refusal})")
            }
            None => words[k].clone(),
        }
    };
    let exact = if matches!(op, BinaryOpKind::BitAnd) {
        format!("logos_band_exact({}, {})", operand(0), operand(1))
    } else {
        format!("({} {op_str} {})", operand(0), operand(1))
    };
    let (scrutinee, pattern) = match checked.as_slice() {
        [(word, value)] => (value.clone(), format!("Some({word})")),
        _ => (
            format!("({})", checked.iter().map(|(_, v)| v.as_str()).collect::<Vec<_>>().join(", ")),
            format!(
                "({})",
                checked.iter().map(|(w, _)| format!("Some({w})")).collect::<Vec<_>>().join(", ")
            ),
        ),
    };
    Some(format!(
        "{{ {binds}match {scrutinee} {{ {pattern} => ({} {op_str} {}), _ => logos_cold_path(move || {exact}) }} }}",
        words[0], words[1]
    ))
}

/// The guarded dual of [`try_guarded_dual_chain`], in its parts: the leaf bindings (named `prefix` and the leaf's
/// number, so two duals in one block keep their own), the guard over the leaves the Oracle does not bound (`None` when
/// it bounds them all, so the raw chain always holds), the raw chain the guard proves within a machine word, and the
/// exact chain (a `LogosInt`, not narrowed) that runs otherwise.
pub(crate) struct GuardedDual {
    pub(crate) binds: String,
    pub(crate) guard: Option<String>,
    pub(crate) fast: String,
    pub(crate) slow: String,
}

/// The leaves of an exact Int chain over `roots`, in evaluation order and once each (a name by its symbol, any other
/// leaf by its node — [`dual_leaf_key`]): `None` when the chain mentions a promoted binding or a leaf that is not a
/// machine-word Int.
fn chain_leaves<'e>(
    roots: &[&'e Expr<'e>],
    ecx: &ExprCtx,
) -> Option<(Vec<usize>, Vec<&'e Expr<'e>>)> {
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
                let addr = dual_leaf_key(e);
                if seen.insert(addr) {
                    addrs.push(addr);
                    exprs.push(e);
                }
                Some(())
            }
        }
    }
    let mut leaves = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut exprs = Vec::new();
    for root in roots {
        walk(root, ecx, &mut leaves, &mut exprs, &mut seen)?;
    }
    Some((leaves, exprs))
}

/// Bind each leaf of a chain that is not a bare name once, as `prefix` and its number, in evaluation order and typed
/// as the machine word every leaf is (a `1 << k` leaf is otherwise an `{integer}` no checked method can be called
/// on): the bindings, and the name every leaf is emitted by.
fn bind_chain_leaves(
    leaves: &[usize],
    exprs: &[&Expr],
    ecx: &ExprCtx,
    prefix: &str,
) -> (String, HashMap<usize, String>) {
    let plain = ExprCtx {
        int_exact_tolerant: false,
        ..*ecx
    };
    let mut names: HashMap<usize, String> = HashMap::new();
    let mut binds = String::new();
    for (k, (addr, le)) in leaves.iter().zip(exprs.iter()).enumerate() {
        // A bare identifier needs no binding — its emitted name is pure and
        // the guard/chain reference it directly (one guard term per SYMBOL).
        if matches!(le, Expr::Identifier(_)) {
            names.insert(*addr, codegen_expr_ctx(le, &plain));
            continue;
        }
        let name = format!("{prefix}{k}");
        binds.push_str(&format!(
            "let {}: i64 = {}; ",
            name,
            codegen_expr_ctx(le, &plain)
        ));
        names.insert(*addr, name);
    }
    (binds, names)
}

fn guarded_dual_parts<'e>(
    op: BinaryOpKind,
    left: &'e Expr<'e>,
    right: &'e Expr<'e>,
    whole_ecx: &ExprCtx,
    prefix: &str,
) -> Option<GuardedDual> {
    let (leaves, leaf_exprs) = chain_leaves(&[left, right], whole_ecx)?;
    if leaves.is_empty() || leaves.len() > 6 {
        return None;
    }
    // A leaf the Oracle bounds costs its own magnitude and needs no guard term.
    let proven: HashMap<usize, u128> = leaves
        .iter()
        .zip(leaf_exprs.iter())
        .filter_map(|(addr, le)| {
            let (lo, hi) = whole_ecx.oracle?.expr_int_range(le)?;
            Some((*addr, lo.unsigned_abs().max(hi.unsigned_abs()) as u128))
        })
        .collect();
    // Solve for the largest power-of-two leaf bound that keeps the chain in
    // i64 (descending scan; the chain magnitude is monotone in the bound).
    let root_mag = |b: u128| -> Option<u128> {
        let l = chain_magnitude_with_leaf_bound(left, b, &proven)?;
        let r = chain_magnitude_with_leaf_bound(right, b, &proven)?;
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
    let (binds, names) = bind_chain_leaves(&leaves, &leaf_exprs, whole_ecx, prefix);
    // Guard terms in LEAF ORDER (the deduped evaluation order) — HashMap
    // iteration would make the emitted text nondeterministic across compiles.
    // A leaf the oracle proves non-negative (whatever its ceiling) needs only
    // the UPPER bound: its `>= -bound` term is vacuous, so we drop it. This halves the per-iteration
    // guard on hot multiply chains whose operand is an inductive `>= 0` (e.g.
    // collatz's `3*k+1`, where `k`'s loop range fixes to `[0, ∞)`).
    let terms = leaves
        .iter()
        .zip(leaf_exprs.iter())
        .filter(|(addr, _)| !proven.contains_key(addr))
        .map(|(addr, le)| {
            let n = &names[addr];
            let b = bound as i64;
            let nonneg = whole_ecx.oracle.is_some_and(|o| o.expr_proven_nonneg(le));
            if nonneg {
                format!("({n} <= {b}i64)")
            } else {
                format!("({n} >= -{b}i64 && {n} <= {b}i64)")
            }
        })
        .collect::<Vec<_>>();
    let guard = (!terms.is_empty()).then(|| terms.join(" && "));
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
                BinaryOpKind::Modulo if nonzero_lit => {
                    let Expr::Literal(Literal::Number(m)) = right else { unreachable!() };
                    raw_literal_rem(*m, codegen_oracle_range(left), &l, &r, "i64")
                }
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
            format!("{}({}, {})", helper, l, r)
        }
    };
    Some(GuardedDual {
        fast: root(true),
        slow: root(false),
        binds,
        guard,
    })
}

/// One arm of a two-armed assignment (`If c: Set x to a. Otherwise: Set x to b.`) as a machine word computed ahead
/// of the choice: the guard under which `fast` is the arm's exact value, `None` when it always is.
pub(crate) struct SelectArm {
    pub(crate) guard: Option<String>,
    pub(crate) fast: String,
}

/// `value` as a [`SelectArm`] (`Opt::BranchlessSelect`), when computing it on a pass that does not choose it is
/// harmless: its syntax cannot trap (integer names and literals under `+ - * & | ^`, and `/` or `%` by a literal that
/// neither divides by zero nor overflows), and its machine-word form is either raw ([`emits_pure_i64`]) or the fast
/// side of a [`GuardedDual`] whose leaves are names, so nothing is evaluated but the arithmetic.
pub(crate) fn select_arm<'a, 'e>(
    value: &'e Expr<'e>,
    interner: &'a Interner,
    synced_vars: &'a HashSet<Symbol>,
    async_functions: &'a HashSet<Symbol>,
    variable_types: &'a HashMap<Symbol, String>,
    oracle: Option<&'a crate::optimize::OracleFacts>,
) -> Option<SelectArm> {
    let ecx = ExprCtx {
        async_functions,
        variable_types,
        oracle,
        int_exact_tolerant: false,
        ..ExprCtx::bare(interner, synced_vars)
    };
    fn speculable(e: &Expr, ecx: &ExprCtx) -> bool {
        match e {
            Expr::Literal(Literal::Number(_)) => true,
            Expr::Identifier(_) => {
                !mentions_bigint_var(e, ecx.variable_types)
                    && infer_numeric_type(e, ecx.interner, ecx.variable_types) == "i64"
            }
            Expr::BinaryOp { op, left, right } => match op {
                BinaryOpKind::Add
                | BinaryOpKind::Subtract
                | BinaryOpKind::Multiply
                | BinaryOpKind::BitAnd
                | BinaryOpKind::BitOr
                | BinaryOpKind::BitXor => speculable(left, ecx) && speculable(right, ecx),
                BinaryOpKind::Divide => {
                    matches!(&**right, Expr::Literal(Literal::Number(d)) if *d != 0 && *d != -1)
                        && speculable(left, ecx)
                }
                BinaryOpKind::Modulo => {
                    matches!(&**right, Expr::Literal(Literal::Number(d)) if *d != 0)
                        && speculable(left, ecx)
                }
                _ => false,
            },
            _ => false,
        }
    }
    if !speculable(value, &ecx) {
        return None;
    }
    if emits_pure_i64(value, &ecx) {
        return Some(SelectArm {
            guard: None,
            fast: codegen_expr_ctx(value, &ecx),
        });
    }
    let Expr::BinaryOp { op, left, right } = value else {
        return None;
    };
    if !is_exact_arith_node(value) {
        return None;
    }
    let dual = guarded_dual_parts(*op, left, right, &ecx, "__chain_l")?;
    if !dual.binds.is_empty() {
        return None;
    }
    Some(SelectArm {
        guard: dual.guard,
        fast: dual.fast,
    })
}

fn oracle_proves_index(ecx: &ExprCtx, collection: &Expr, index: &Expr) -> bool {
    if !crate::optimize::active_config().is_on(crate::optimization::Opt::Unchecked) {
        return false;
    }
    let proven = ecx
        .oracle
        .map_or(false, |o| o.index_provably_in_bounds(collection, index));
    if proven {
        crate::optimize::mark_fired(crate::optimization::Opt::Unchecked);
    }
    proven
}

/// The emitted bounds of `start to end`. Two type-silent bounds leave the counter's width to rustc's `i32`
/// default, so the end states `i64`.
pub(crate) fn range_bound_strs(
    start: &Expr,
    end: &Expr,
    emit: impl Fn(&Expr) -> String,
) -> (String, String) {
    let start_str = emit(start);
    let end_str = match super::types::type_silent_int(start) {
        Some(_) => super::types::type_silent_int_i64(end).unwrap_or_else(|| emit(end)),
        None => emit(end),
    };
    (start_str, end_str)
}

/// The Rust range a counted `Repeat for c from start to end` walks. The half-open `start..end + 1` is the
/// form LLVM optimizes (an inclusive `RangeInclusive` carries its exhausted flag through every iteration),
/// and it is exact wherever `end + 1` exists: a constant `end` below `i64::MAX`, or an `end` the range oracle
/// bounds below it. Any other `end` keeps the inclusive range, which reaches `i64::MAX` itself.
pub(crate) fn counted_loop_range(
    start: &Expr,
    end: &Expr,
    start_str: &str,
    end_str: &str,
    oracle: Option<&crate::optimize::OracleFacts>,
) -> String {
    if let Some(last) = super::types::type_silent_int(end) {
        return match last.checked_add(1) {
            Some(past) if super::types::type_silent_int(start).is_some() => {
                format!("({start_str}..{past}_i64)")
            }
            Some(past) => format!("({start_str}..{past})"),
            None => format!("({start_str}..={end_str})"),
        };
    }
    if oracle
        .and_then(|o| o.expr_int_range(end))
        .is_some_and(|(_, hi)| hi < i64::MAX)
    {
        return format!("({start_str}..({end_str} + 1))");
    }
    format!("({start_str}..={end_str})")
}

/// Record, in this compile's facts ([`crate::optimize::facts`]), that the access whose index is `index` names
/// an item wherever it is evaluated; the construct calling this is what proves it. Outside a compile's facts
/// scope nothing is recorded and the access stays checked.
pub(crate) fn mark_index_in_range(index: &Expr) {
    crate::optimize::facts::mark_expr(crate::optimize::facts::Fact::InBounds, index);
}

/// Whether `item index of collection` names an item on every run, by this compile's range oracle
/// ([`crate::optimize::OracleFacts::index_always_in_bounds`]). A plain indexed access there never meets its
/// own bounds check.
pub(crate) fn range_proves_index(collection: &Expr, index: &Expr) -> bool {
    RANGE_ORACLE.with(|r| {
        r.borrow()
            .as_deref()
            .is_some_and(|o| o.index_always_in_bounds(collection, index))
    })
}

/// Whether the 1-based `index` is at least 1 wherever it is evaluated, so the item it names, if any, counts from the
/// front and [`LogosItems::logos_item_front`](logicaffeine_data::LogosItems::logos_item_front) holds the whole
/// index rule: a literal of at least 1, a non-negative value plus a literal of at least 1, or an index the range
/// oracle bounds below by 1. The plain accessor's path for an index counted from the back is a second way to an
/// item, which keeps a proven-in-range loop from compiling to plain loads.
pub(crate) fn counts_from_the_front(
    index: &Expr,
    oracle: Option<&crate::optimize::OracleFacts>,
) -> bool {
    let at_least_one = |e: &Expr| matches!(e, Expr::Literal(Literal::Number(n)) if *n >= 1);
    match index {
        Expr::Literal(Literal::Number(n)) => return *n >= 1,
        Expr::BinaryOp {
            op: BinaryOpKind::Add,
            left,
            right,
        } if (at_least_one(right) && is_nonneg_expr(left))
            || (at_least_one(left) && is_nonneg_expr(right)) =>
        {
            return true
        }
        _ => {}
    }
    let proven = |o: &crate::optimize::OracleFacts| o.expr_proven_at_least(index, 1);
    oracle.is_some_and(proven) || RANGE_ORACLE.with(|r| r.borrow().as_deref().is_some_and(proven))
}

/// The accessor an emitted `item index of …` goes through: [`counts_from_the_front`]'s, or the whole index rule.
pub(crate) fn item_accessor(
    index: &Expr,
    oracle: Option<&crate::optimize::OracleFacts>,
    write: bool,
) -> &'static str {
    match (counts_from_the_front(index, oracle), write) {
        (true, false) => "logos_item_front",
        (true, true) => "logos_item_front_mut",
        (false, false) => "logos_item",
        (false, true) => "logos_item_mut",
    }
}

/// The read of `item index of collection` from `owner` (the collection's Rust place): unchecked where the
/// statement licensed the oracle and it proves the index in range, a plain indexed load where the range oracle
/// proves it (the load's own check never fires), and through the one index rule otherwise. `copy` reads the
/// element by value; any other element is cloned.
fn indexed_read(owner: &str, collection: &Expr, index: &Expr, copy: bool, ecx: &ExprCtx) -> String {
    let (variable_types, interner) = (ecx.variable_types, ecx.interner);
    let (place, by_ref) = if oracle_proves_index(ecx, collection, index) {
        let offset = proven_offset_str(index, ecx, variable_types, interner);
        return if copy {
            format!("unsafe {{ *{}.get_unchecked({}) }}", owner, offset)
        } else {
            format!("unsafe {{ {}.get_unchecked({}).clone() }}", owner, offset)
        };
    } else if range_proves_index(collection, index) {
        (
            format!(
                "{}[{}]",
                owner,
                proven_offset_str(index, ecx, variable_types, interner)
            ),
            false,
        )
    } else {
        (
            format!(
                "{}.{}({})",
                owner,
                item_accessor(index, ecx.oracle, false),
                logos_index_str(index, ecx, variable_types, interner)
            ),
            true,
        )
    };
    match (copy, by_ref) {
        (true, true) => format!("(*{})", place),
        (true, false) => place,
        (false, _) => format!("{}.clone()", place),
    }
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
        &ExprCtx {
            async_functions,
            variable_types,
            oracle,
            ..ExprCtx::bare(interner, synced_vars)
        },
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
        &ExprCtx {
            async_functions,
            variable_types,
            ..ExprCtx::bare(interner, synced_vars)
        },
    )
}

/// Codegen expression with async support, string variable tracking, the loop-invariant libdivide
/// map (`fast_div`), and the type REGISTRY. Every caller is a `Show`/`Observe` sink whose object
/// may concatenate a struct FIELD read (`p's label + p's label`): the concat-fold decision
/// (`is_string_concat`, above) recognises a Text-typed field only through `record_field_rust_type`,
/// which needs the registry to resolve the field's owning struct — `ExprCtx::bare`'s empty
/// registry made every such field invisible to it, so a field-only concat fell through to a raw
/// `+`, and `String: Add<String>` wants `&str` on the right, so it did not build.
pub(crate) fn codegen_expr_with_async_and_strings(
    expr: &Expr,
    interner: &Interner,
    synced_vars: &HashSet<Symbol>,
    async_functions: &HashSet<Symbol>,
    string_vars: &HashSet<Symbol>,
    variable_types: &HashMap<Symbol, String>,
    fast_div: &HashMap<Symbol, String>,
    registry: &TypeRegistry,
) -> String {
    codegen_expr_ctx(
        expr,
        &ExprCtx {
            async_functions,
            string_vars,
            variable_types,
            fast_div,
            registry,
            // Every caller is a `Show` sink (plain object or an interpolated
            // part): a display accepts the promoting `LogosInt`, so overflow
            // prints the EXACT value instead of narrowing.
            int_exact_tolerant: true,
            ..ExprCtx::bare(interner, synced_vars)
        },
    )
}

/// Check if an expression is definitely a string (needs format! for concatenation).
/// Takes a set of known string variable symbols for identifier lookup.
pub(crate) fn is_definitely_string_expr_with_vars(
    expr: &Expr,
    string_vars: &HashSet<Symbol>,
) -> bool {
    match expr {
        // String literals are definitely strings
        Expr::Literal(Literal::Text(_)) => true,
        // Variables known to be strings
        Expr::Identifier(sym) => string_vars.contains(sym),
        // Concat always produces strings
        Expr::BinaryOp {
            op: BinaryOpKind::Concat,
            ..
        } => true,
        // Add with a string operand produces a string
        Expr::BinaryOp {
            op: BinaryOpKind::Add,
            left,
            right,
        } => {
            is_definitely_string_expr_with_vars(left, string_vars)
                || is_definitely_string_expr_with_vars(right, string_vars)
        }
        // WithCapacity wrapping a string value is a string
        Expr::WithCapacity { value, .. } => is_definitely_string_expr_with_vars(value, string_vars),
        // Interpolated strings always produce strings
        Expr::InterpolatedString(_) => true,
        // A call to a Text-returning function is a string operand, exactly like a Text-typed
        // field read (below) — the callee's OWN declared return type, not the argument shapes.
        Expr::Call { function, .. } => fn_call_returns_text(*function),
        _ => false,
    }
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
        Expr::BinaryOp {
            op: BinaryOpKind::SeqConcat,
            ..
        } => true,
        Expr::BinaryOp {
            op: BinaryOpKind::Add | BinaryOpKind::Multiply,
            left,
            right,
        } => {
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
/// [`collect_string_concat_operands`] with the "is this a string operand?" test supplied by the
/// caller, so the DECISION to fold into a `format!` and the FLATTENING of its operands ask exactly
/// the same question. When they disagreed, a chain whose stringness was only visible to the
/// decision was pushed back through the ordinary `+` emission it was folded to avoid.
pub(crate) fn collect_string_concat_operands_with<'a, 'b>(
    expr: &'b Expr<'a>,
    is_string: &dyn Fn(&Expr<'a>) -> bool,
    operands: &mut Vec<&'b Expr<'a>>,
) {
    match expr {
        Expr::BinaryOp {
            op: BinaryOpKind::Concat,
            left,
            right,
        } => {
            collect_string_concat_operands_with(left, is_string, operands);
            collect_string_concat_operands_with(right, is_string, operands);
        }
        Expr::BinaryOp {
            op: BinaryOpKind::Add,
            left,
            right,
        } if is_string(left) || is_string(right) => {
            collect_string_concat_operands_with(left, is_string, operands);
            collect_string_concat_operands_with(right, is_string, operands);
        }
        _ => operands.push(expr),
    }
}

pub(crate) fn collect_string_concat_operands<'a, 'b>(
    expr: &'b Expr<'a>,
    string_vars: &HashSet<Symbol>,
    operands: &mut Vec<&'b Expr<'a>>,
) {
    match expr {
        Expr::BinaryOp {
            op: BinaryOpKind::Concat,
            left,
            right,
        } => {
            collect_string_concat_operands(left, string_vars, operands);
            collect_string_concat_operands(right, string_vars, operands);
        }
        Expr::BinaryOp {
            op: BinaryOpKind::Add,
            left,
            right,
        } => {
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
        Expr::BinaryOp {
            op: BinaryOpKind::ExactDivide,
            ..
        } => true,
        Expr::BinaryOp {
            op: BinaryOpKind::Add | BinaryOpKind::Subtract | BinaryOpKind::Multiply,
            left,
            right,
        } => is_rational_expr(left, variable_types) || is_rational_expr(right, variable_types),
        Expr::Identifier(sym) => variable_types
            .get(sym)
            .map_or(false, |t| t == "LogosRational"),
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

/// The exact value type `expr` produces, if it produces one — most specific first: a float
/// complex, an exact complex, a decimal, a modular, a quantity, a money. The ONE answer a binding's
/// type and a sequence literal's element type both take, so neither can drift from the other.
pub(super) fn exact_value_type_of(
    expr: &Expr,
    variable_types: &HashMap<Symbol, String>,
    interner: &Interner,
) -> Option<&'static str> {
    if is_complexf_expr(expr, variable_types, interner) {
        Some("LogosComplexF")
    } else if is_complex_expr(expr, variable_types, interner) {
        Some("LogosComplex")
    } else if is_decimal_expr(expr, variable_types, interner) {
        Some("LogosDecimal")
    } else if is_modular_expr(expr, variable_types, interner) {
        Some("LogosModular")
    } else if is_quantity_expr(expr, variable_types, interner) {
        Some("LogosQuantity")
    } else if is_money_expr(expr, variable_types, interner) {
        Some("LogosMoney")
    } else {
        None
    }
}

/// The Rust a clock read ([`ClockRead`](crate::semantics::temporal::ClockRead)) emits, and the Rust type of what it
/// reads.
pub(crate) fn clock_read(name: &str) -> Option<(&'static str, &'static str)> {
    use crate::semantics::temporal::ClockRead;
    ClockRead::named(name).map(|read| match read {
        ClockRead::Today => ("logicaffeine_system::temporal::today()", "LogosDate"),
        ClockRead::Now => ("logicaffeine_system::temporal::now()", "LogosMoment"),
    })
}

/// The Rust type a value expression evidently produces, read from its shape: an exact value, a
/// literal, a variable of a plain registered type, a record or variant of the program's being built,
/// a record field, a call to a program function, a list of such, or a tuple of such. `None` when the
/// shape does not say, or when the registered type is not the value's plain Rust type (a borrow, an
/// owned `Vec`, a hoisting or bignum annotation, a placeholder). The element type a tuple or list
/// literal records for its binding.
pub(super) fn value_rust_type(
    expr: &Expr,
    variable_types: &HashMap<Symbol, String>,
    registry: &TypeRegistry,
    interner: &Interner,
) -> Option<String> {
    if let Some(exact) = exact_value_type_of(expr, variable_types, interner) {
        return Some(exact.to_string());
    }
    let plain = |t: String| -> Option<String> {
        let placeholder = t
            .split(|c: char| matches!(c, '<' | '>' | ',' | ' '))
            .any(|p| p == "_");
        let annotated = t.contains('|') || t.starts_with('&') || t.starts_with("Vec<");
        (!t.is_empty() && !placeholder && !annotated).then_some(t)
    };
    let declared = |name: Symbol| -> Option<String> {
        let generic = match registry.get(name)? {
            crate::analysis::registry::TypeDef::Struct { generics, .. }
            | crate::analysis::registry::TypeDef::Enum { generics, .. } => !generics.is_empty(),
            _ => return None,
        };
        (!generic).then(|| rust_type_ident(interner.resolve(name)))
    };
    match expr {
        Expr::Literal(lit) => {
            plain(crate::analysis::types::LogosType::from_literal(lit).to_rust_type())
        }
        Expr::Identifier(sym) => match clock_read(interner.resolve(*sym)) {
            Some((_, ty)) => Some(ty.to_string()),
            None => plain(variable_types.get(sym)?.clone()),
        },
        Expr::New {
            type_name,
            type_args,
            ..
        } if type_args.is_empty() => declared(*type_name),
        Expr::NewVariant { enum_name, .. } => declared(*enum_name),
        Expr::FieldAccess { .. } => plain(record_field_rust_type(
            expr,
            interner,
            variable_types,
            registry,
        )?),
        Expr::Call { function, .. } => {
            plain(FN_RETURN_TYPES.with(|c| c.borrow().get(function).cloned())?)
        }
        Expr::List(items) => {
            let held = super::rational::type_of(expr, super::rational::Typing { interner, variable_types, registry });
            if held.holds_rational() {
                return Some(held.to_rust_type());
            }
            Some(format!(
                "LogosSeq<{}>",
                value_rust_type(items.first()?, variable_types, registry, interner)?
            ))
        }
        Expr::Tuple(items) => {
            let elems = items
                .iter()
                .map(|i| value_rust_type(i, variable_types, registry, interner))
                .collect::<Option<Vec<_>>>()?;
            Some(super::types::tuple_rust_type(&elems))
        }
        _ => None,
    }
}

/// One interpolation hole — the `format!` piece and the argument it formats — shared by the `Show`
/// statement and an interpolated-string expression, so a spec means one thing in both. A currency
/// (`$`) or precision (`.N`) hole renders through the runtime's `format_currency`/`format_fixed`,
/// which format each value type exactly as the interpreter's `apply_format_spec` does (an exact
/// decimal keeps its own digits rather than taking a lossy `f64` round trip). A plain hole renders
/// through `Showable`, like `Show`; an alignment or width spec is Rust's own.
pub(super) fn interpolation_hole(spec: Option<&str>, arg: &str) -> (String, String) {
    match spec {
        None => (
            "{}".to_string(),
            format!("logicaffeine_system::io::show_string(&({arg}))"),
        ),
        Some("$") => (
            "{}".to_string(),
            format!("logicaffeine_system::io::format_currency(&({arg}))"),
        ),
        Some(s) => match s.strip_prefix('.').and_then(|p| p.parse::<usize>().ok()) {
            Some(precision) => (
                "{}".to_string(),
                format!("logicaffeine_system::io::format_fixed(&({arg}), {precision})"),
            ),
            None => (format!("{{:{s}}}"), arg.to_string()),
        },
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
        Expr::Identifier(sym) => variable_types
            .get(sym)
            .map_or(false, |t| t == "LogosDecimal"),
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

/// The Rust element type of `collection` when it is an identifier bound to a sequence — a
/// `LogosSeq<T>`, an owned `Vec<T>`, or a borrowed `&[T]` / `&mut [T]` / fixed `&[T; N]` param.
/// `&[LogosComplexF]` yields `Some("LogosComplexF")`. Used to type `item i of xs` so a
/// `Seq of ComplexF` element's `+ − × ÷` take the ComplexF method path (also across borrow params).
/// Whether an item read from a collection of Rust type `t` is emitted widened to `i64` (`(x as i64)`): a narrowed
/// `Vec<i32>`, an `i32` slice or fixed array, and a text's byte view. Such a read is an Int word wherever it appears.
pub(super) fn item_read_widens(t: &str) -> bool {
    let t = t.split("|__hl:").next().unwrap_or(t);
    if t == "TextBytes" || t == "Vec<i32>" {
        return true;
    }
    let body = t.strip_prefix("&mut ").or_else(|| t.strip_prefix('&')).unwrap_or(t);
    body.strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .is_some_and(|inner| inner.split(';').next().unwrap_or(inner).trim() == "i32")
}

pub(super) fn seq_element_rust_type<'a>(
    collection: &Expr,
    variable_types: &'a HashMap<Symbol, String>,
) -> Option<&'a str> {
    let Expr::Identifier(sym) = collection else {
        return None;
    };
    let t = variable_types.get(sym)?.as_str();
    if let Some(inner) = t
        .strip_prefix("LogosSeq<")
        .and_then(|s| s.strip_suffix('>'))
    {
        return Some(inner);
    }
    if let Some(inner) = t.strip_prefix("Vec<").and_then(|s| s.strip_suffix('>')) {
        return Some(inner);
    }
    // Borrowed slices: `&[T]`, `&mut [T]`, or a fixed `&[T; N]` (element is the part before `;`).
    let body = t
        .strip_prefix("&mut ")
        .or_else(|| t.strip_prefix('&'))
        .unwrap_or(t);
    if let Some(inner) = body.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        return Some(inner.split(';').next().unwrap_or(inner).trim());
    }
    None
}

/// The Rust VALUE type of `collection` when it is an identifier bound to a `LogosMap<K, V>` —
/// splits on the top-level comma (bracket-depth tracked) so a generic value type
/// (`LogosMap<String, LogosSeq<Int>>`) is not cut at an inner comma.
fn logos_map_value_rust_type<'a>(
    collection: &Expr,
    variable_types: &'a HashMap<Symbol, String>,
) -> Option<&'a str> {
    let Expr::Identifier(sym) = collection else {
        return None;
    };
    let t = variable_types.get(sym)?.as_str();
    match super::types::top_level_parts(t.strip_prefix("LogosMap<")?.strip_suffix('>')?).as_slice() {
        [_, value] => Some(*value),
        _ => None,
    }
}

/// True when `e` is a bare identifier whose DECLARED Rust type is `String` — a Text-typed
/// function PARAMETER is never entered into `string_vars` (only a `Let`-bound local is, at its
/// own binding site in `codegen_let_stmt`), so `is_definitely_string_expr_with_vars`'s
/// `string_vars`-only lookup cannot see one. `variable_types` is the parameter's own source of
/// truth: every parameter is registered there by name at codegen entry, `Text` mapping to
/// `"String"` the same way every other Text-typed binding does.
fn is_text_typed_identifier(e: &Expr, variable_types: &HashMap<Symbol, String>) -> bool {
    matches!(e, Expr::Identifier(sym)
        if variable_types.get(sym)
            .map(|t| t.split("|__hl:").next().unwrap_or(t))
            == Some("String"))
}

/// True when `e` reads a `Seq of Text` or a `Map of _ to Text` by index (`item i of xs`, `item k
/// of m`) — a Text-typed element or value is a string operand exactly like a Text-typed field,
/// and for the same reason: only the DECLARED type tells an Int-valued index apart from a
/// Text-valued one, and `is_definitely_string_expr_with_vars` has no `Expr::Index` arm at all.
fn is_text_typed_index(e: &Expr, variable_types: &HashMap<Symbol, String>) -> bool {
    let Expr::Index { collection, .. } = e else {
        return false;
    };
    seq_element_rust_type(collection, variable_types) == Some("String")
        || logos_map_value_rust_type(collection, variable_types) == Some("String")
}

/// Does `expr` evaluate to a `LogosComplex`? True for a `complex(..)` call, for `+ − × ÷`
/// over a Complex operand, and for an identifier bound to a `LogosComplex`.
pub(super) fn is_complex_expr(
    expr: &Expr,
    variable_types: &HashMap<Symbol, String>,
    interner: &Interner,
) -> bool {
    match expr {
        // `complex(a, b)` builds one; `conjugate(z)` of an exact complex stays exact.
        Expr::Call { function, args } => {
            let name = interner.resolve(*function);
            name == "complex"
                || (name == "conjugate"
                    && args.len() == 1
                    && is_complex_expr(args[0], variable_types, interner))
        }
        Expr::BinaryOp {
            op:
                BinaryOpKind::Add
                | BinaryOpKind::Subtract
                | BinaryOpKind::Multiply
                | BinaryOpKind::Divide,
            left,
            right,
        } => {
            is_complex_expr(left, variable_types, interner)
                || is_complex_expr(right, variable_types, interner)
        }
        // `item i of xs` where `xs : Seq of ComplexF-exact` (`LogosSeq<LogosComplex>`).
        Expr::Index { collection, .. } => {
            seq_element_rust_type(collection, variable_types) == Some("LogosComplex")
        }
        Expr::Identifier(sym) => variable_types
            .get(sym)
            .map_or(false, |t| t == "LogosComplex"),
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

/// Does `expr` statically produce an `f64`? A float literal, a float-typed binding, float `+ − × ÷`,
/// or a real `sqrt`. Used to route `complex(re, im)` to the FLOATING-POINT complex when a part is a
/// float (mirroring the interpreter's `complex()` dispatch).
pub(super) fn produces_float(
    expr: &Expr,
    variable_types: &HashMap<Symbol, String>,
    interner: &Interner,
) -> bool {
    match expr {
        Expr::Literal(Literal::Float(_)) => true,
        Expr::Identifier(sym) => variable_types.get(sym).map_or(false, |t| t == "f64"),
        Expr::BinaryOp {
            op:
                BinaryOpKind::Add
                | BinaryOpKind::Subtract
                | BinaryOpKind::Multiply
                | BinaryOpKind::Divide,
            left,
            right,
        } => {
            produces_float(left, variable_types, interner)
                || produces_float(right, variable_types, interner)
        }
        // `abs` always yields an f64 magnitude; `sqrt` yields an f64 UNLESS its argument is a
        // float-complex (in which case it is a `ComplexF`, not a float). A call to one of the
        // math.lg `-> Real` transcendentals (`sin`/`cos`/`exp`/`ln`/`hypot`/…) likewise yields an
        // f64 — WITHOUT this, `complex(sin(x), cos(y))` mis-selects the exact `LogosComplex`
        // representation and lossily truncates each float part through `as i64`.
        Expr::Call { function, args } => {
            let name = interner.resolve(*function);
            name == "abs"
                || name == "re"
                || name == "im"
                || is_float_returning_math_call(*function, interner)
                || (name == "sqrt"
                    && !args.is_empty()
                    && !is_complexf_expr(args[0], variable_types, interner))
        }
        // `item i of xs` over a `Seq of Float` (a local, a literal, or a borrow `&[f64]` param).
        Expr::Index { collection, .. } => {
            seq_element_rust_type(collection, variable_types) == Some("f64")
        }
        _ => false,
    }
}

/// The math.lg functions whose declared return type is `Real` (compiles to `f64`). MUST mirror the
/// `-> Real` surface of `assets/std/math.lg` exactly — `float_returning_list_mirrors_math_lg`
/// (below) parses the module and fails the build if the two drift, so a newly-added transcendental
/// cannot silently fall off this list.
const FLOAT_RETURNING_MATH_FNS: &[&str] = &[
    "absR",
    "absReal",
    "acos",
    "acosh",
    "asin",
    "asinh",
    "atan",
    "atan2",
    "atanOffset",
    "atanPos",
    "atanReduce",
    "atanSign",
    "atanh",
    "cAsinhIm",
    "cAsinhRe",
    "cAtanhIm",
    "cAtanhRe",
    "cbrt",
    "copysign",
    "cos",
    "cosCore",
    "cosCoreP",
    "cosh",
    "cosP",
    "cPhase",
    "cSqrtIm",
    "cSqrtRe",
    "degrees",
    "dimReal",
    "distReal",
    "divEuclid",
    "eReal",
    "erf",
    "erfc",
    "erfcinv",
    "erfinv",
    "exp",
    "exp2",
    "expm1",
    "expP",
    "floorR",
    "fma",
    "fmax",
    "fmin",
    "fmodReal",
    "fract",
    "frexpFrac",
    "gamma",
    "hypot",
    "ldexp",
    "lgamma",
    "ln",
    "lnP",
    "log10",
    "log1p",
    "log2",
    "logb",
    "logBase",
    "mathInf",
    "mathNan",
    "maxReal",
    "modfFrac",
    "modfInt",
    "piReal",
    "pow2int",
    "powi",
    "radians",
    "ratAtan",
    "recip",
    "remainderReal",
    "remEuclid",
    "rint",
    "roundEvenReal",
    "roundR",
    "sin",
    "sinCore",
    "sinCoreP",
    "sinh",
    "sinP",
    "tan",
    "tanh",
    "tauReal",
    "nextafter",
    "ulp",
];

/// True when a call to `name` produces a float — needed so `complex(sin(x), cos(y))` and friends
/// select the floating-point `LogosComplexF` representation instead of the exact `LogosComplex`
/// (which would mismatch a `-> ComplexF` return type and truncate the float parts through `as i64`).
pub(crate) fn is_float_returning_math_fn(name: &str) -> bool {
    FLOAT_RETURNING_MATH_FNS.contains(&name)
}

/// The math.lg functions whose declared return type is `ComplexF` (compiles to `LogosComplexF`).
/// MUST mirror the `-> ComplexF` surface of `assets/std/math.lg` exactly — a call to one of these
/// IS a floating-point complex, so `re`/`im` extraction, complex arithmetic, and a `-> ComplexF`
/// return must take the `LogosComplexF` path, not cast the value through `as f64`. The
/// `complexf_returning_list_mirrors_math_lg` test fails the build if the two drift.
const COMPLEXF_RETURNING_MATH_FNS: &[&str] = &[
    "cAcos", "cAcosh", "cAsin", "cAsinh", "cAtan", "cAtanh", "cConj", "cCos", "cCosh", "cCot",
    "cExp", "cInf", "cInfj", "cLog", "cLog10", "cNan", "cNanj", "cPow", "cRect", "cSin", "cSinh",
    "cSqrt", "cTan", "cTanh",
];

/// True when `name` is a math.lg function whose return type is `ComplexF`.
pub(crate) fn is_complexf_returning_math_fn(name: &str) -> bool {
    COMPLEXF_RETURNING_MATH_FNS.contains(&name)
}

/// True when a CALL to `function` yields a `ComplexF` — the declarer-wins analogue of
/// [`is_float_returning_math_call`]. A `## To cExp …` in THIS program shadows math.lg's `cExp`:
/// the `COMPLEXF_RETURNING_MATH_FNS` list keys on bare NAMES, so without this a user's own
/// `## To cExp (n: Int) -> Int` would be typed `ComplexF` and mis-lowered (re/im/arith on a plain
/// Int). Whoever declared the name in this program owns its return type; the name list is only the
/// fallback for a name the program does not declare.
pub(crate) fn is_complexf_returning_math_call(function: Symbol, interner: &Interner) -> bool {
    if let Some(declared) = FN_RETURN_TYPES.with(|c| c.borrow().get(&function).cloned()) {
        return declared == "LogosComplexF";
    }
    is_complexf_returning_math_fn(interner.resolve(function))
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen/expr/float_returning_math_fn_tests.rs"]
mod float_returning_math_fn_tests;

/// True when `complex(a, b)` should build a floating-point complex — a part is an `f64`.
pub(super) fn complex_call_is_float(
    args: &[&Expr],
    variable_types: &HashMap<Symbol, String>,
    interner: &Interner,
) -> bool {
    args.len() == 2
        && args
            .iter()
            .any(|&a| produces_float(a, variable_types, interner))
}

/// Does `expr` evaluate to a `LogosComplexF` (floating-point complex)? A float-argument `complex(..)`,
/// `+ − × ÷` where a side is `ComplexF`, or an identifier bound to `LogosComplexF`.
pub(super) fn is_complexf_expr(
    expr: &Expr,
    variable_types: &HashMap<Symbol, String>,
    interner: &Interner,
) -> bool {
    match expr {
        // A float-argument `complex(..)`, `sqrt` of a float-complex (`√z` is a `ComplexF`), or
        // `conjugate` of a float-complex (conjugation preserves the `ComplexF` type).
        Expr::Call { function, args } => {
            let name = interner.resolve(*function);
            is_complexf_returning_math_call(*function, interner)
                || (name == "complex" && complex_call_is_float(args, variable_types, interner))
                || (name == "sqrt"
                    && args.len() == 1
                    && is_complexf_expr(args[0], variable_types, interner))
                || (name == "conjugate"
                    && args.len() == 1
                    && is_complexf_expr(args[0], variable_types, interner))
        }
        Expr::BinaryOp {
            op:
                BinaryOpKind::Add
                | BinaryOpKind::Subtract
                | BinaryOpKind::Multiply
                | BinaryOpKind::Divide,
            left,
            right,
        } => {
            is_complexf_expr(left, variable_types, interner)
                || is_complexf_expr(right, variable_types, interner)
        }
        // `item i of xs` where `xs : Seq of ComplexF` (`LogosSeq<LogosComplexF>`).
        Expr::Index { collection, .. } => {
            seq_element_rust_type(collection, variable_types) == Some("LogosComplexF")
        }
        Expr::Identifier(sym) => variable_types
            .get(sym)
            .map_or(false, |t| t == "LogosComplexF"),
        _ => false,
    }
}

/// A `LogosComplexF`-producing operand: an already-`ComplexF` value is used verbatim; anything else
/// (an int, a float, or an exact `Complex`) embeds as `LogosComplexF::from_f64((..) as f64)`.
fn complexf_operand(
    expr: &Expr,
    code: &str,
    variable_types: &HashMap<Symbol, String>,
    interner: &Interner,
) -> String {
    if is_complexf_expr(expr, variable_types, interner) {
        code.to_string()
    } else {
        format!("LogosComplexF::from_f64(({}) as f64)", code)
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
            op:
                BinaryOpKind::Add
                | BinaryOpKind::Subtract
                | BinaryOpKind::Multiply
                | BinaryOpKind::Divide,
            left,
            right,
        } => {
            is_modular_expr(left, variable_types, interner)
                && is_modular_expr(right, variable_types, interner)
        }
        Expr::Identifier(sym) => variable_types
            .get(sym)
            .map_or(false, |t| t == "LogosModular"),
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
        Expr::BinaryOp {
            op: BinaryOpKind::Add | BinaryOpKind::Subtract,
            left,
            right,
        } => {
            is_quantity_expr(left, variable_types, interner)
                && is_quantity_expr(right, variable_types, interner)
        }
        Expr::BinaryOp {
            op: BinaryOpKind::Multiply | BinaryOpKind::Divide,
            left,
            right,
        } => {
            is_quantity_expr(left, variable_types, interner)
                || is_quantity_expr(right, variable_types, interner)
        }
        Expr::Identifier(sym) => variable_types
            .get(sym)
            .map_or(false, |t| t == "LogosQuantity"),
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
        Expr::BinaryOp {
            op: BinaryOpKind::Add | BinaryOpKind::Subtract,
            left,
            right,
        } => {
            is_money_expr(left, variable_types, interner)
                && is_money_expr(right, variable_types, interner)
        }
        Expr::BinaryOp {
            op: BinaryOpKind::Multiply | BinaryOpKind::Divide,
            left,
            right,
        } => {
            is_money_expr(left, variable_types, interner)
                || is_money_expr(right, variable_types, interner)
        }
        Expr::Identifier(sym) => variable_types.get(sym).map_or(false, |t| t == "LogosMoney"),
        _ => false,
    }
}

/// The native word width whose all-ones mask is `m` (`2^w - 1`), or `None`. Width 64 is
/// excluded — its mask exceeds `i64` so it never appears as a positive literal here.
fn native_width_of_mask(m: i64) -> Option<i64> {
    match m {
        255 => Some(8),
        65535 => Some(16),
        4294967295 => Some(32),
        _ => None,
    }
}

fn as_num_lit(e: &Expr) -> Option<i64> {
    if let Expr::Literal(Literal::Number(n)) = e {
        Some(*n)
    } else {
        None
    }
}

/// Match the low half of a rotate idiom, `(x << s) & M` (either `&` ordering), returning the
/// rotated value, the shift `s`, and the mask `M`. `x` may be any expression.
fn match_rotate_low<'a>(e: &'a Expr<'a>) -> Option<(&'a Expr<'a>, i64, i64)> {
    let Expr::BinaryOp {
        op: BinaryOpKind::BitAnd,
        left,
        right,
    } = e
    else {
        return None;
    };
    let (left, right): (&'a Expr<'a>, &'a Expr<'a>) = (*left, *right);
    for (shl, mask) in [(left, right), (right, left)] {
        if let Expr::BinaryOp {
            op: BinaryOpKind::Shl,
            left: x,
            right: s,
        } = shl
        {
            let (x, s): (&'a Expr<'a>, &'a Expr<'a>) = (*x, *s);
            if let (Some(s), Some(m)) = (as_num_lit(s), as_num_lit(mask)) {
                return Some((x, s, m));
            }
        }
    }
    None
}

/// Match the high half of a rotate idiom, `(x & M) >> t` (either `&` ordering), returning the
/// rotated value, the shift `t`, and the mask `M`.
fn match_rotate_high<'a>(e: &'a Expr<'a>) -> Option<(&'a Expr<'a>, i64, i64)> {
    let Expr::BinaryOp {
        op: BinaryOpKind::Shr,
        left,
        right,
    } = e
    else {
        return None;
    };
    let (left, right): (&'a Expr<'a>, &'a Expr<'a>) = (*left, *right);
    let t = as_num_lit(right)?;
    let Expr::BinaryOp {
        op: BinaryOpKind::BitAnd,
        left: a,
        right: b,
    } = left
    else {
        return None;
    };
    let (a, b): (&'a Expr<'a>, &'a Expr<'a>) = (*a, *b);
    if let Some(m) = as_num_lit(b) {
        return Some((a, t, m));
    }
    if let Some(m) = as_num_lit(a) {
        return Some((b, t, m));
    }
    None
}

/// Recognize the rotate idiom `((x << s) & M) | ((x & M) >> t)` — the shape a const-specialized
/// `rotateLeft(x, s, w)` folds to — with `M = 2^w - 1` (w a native width) and `s + t = w` over
/// the SAME value `x`, and emit the single-instruction intrinsic `(x as uW).rotate_left(s) as
/// i64`. Sound: complementary shifts summing to a native mask width admit no meaning but a
/// w-bit rotate (a rotate-right by `t` is the same idiom with the halves swapped).
fn try_rotate_intrinsic<'a>(
    left: &'a Expr<'a>,
    right: &'a Expr<'a>,
    codegen: &dyn Fn(&Expr) -> String,
) -> Option<String> {
    let go = |lo: &'a Expr<'a>, hi: &'a Expr<'a>| -> Option<String> {
        let (xl, s, m1) = match_rotate_low(lo)?;
        let (xh, t, m2) = match_rotate_high(hi)?;
        if m1 != m2 || !super::peephole::exprs_equal(xl, xh) {
            return None;
        }
        let w = native_width_of_mask(m1)?;
        if s >= 1 && t >= 1 && s + t == w {
            return Some(format!(
                "(({} as u{w}).rotate_left({}u32) as i64)",
                codegen(xl),
                s
            ));
        }
        None
    };
    go(left, right).or_else(|| go(right, left))
}

pub(crate) fn codegen_expr_ctx(expr: &Expr, ecx: &ExprCtx) -> String {
    // An operand an enclosing operation has read already, and an expression reading a value-or-nothing.
    if let Some(code) = super::absent::read(expr) {
        return code;
    }
    if let Some(code) = super::absent::emit(expr, ecx) {
        return code;
    }
    // The exact extension's values are `LogosValue`s, emitted on that model ahead of every
    // representation choice the rest of this function makes for standard numbers.
    if super::tower::active() {
        if let Some(code) = super::tower::emit_logic(expr, ecx) {
            return code;
        }
        if let Some(code) = super::tower::emit_collection(expr, ecx) {
            return code;
        }
        if super::tower::is_tower_expr(expr, ecx.variable_types, ecx.interner) {
            if let Some(code) = super::tower::emit(expr, ecx) {
                return code;
            }
        }
    }
    // A rational takes the rules of the exact number it is, ahead of every rewrite this function makes for
    // machine integers.
    if let Some(code) = super::rational::emit(expr, ecx) {
        return code;
    }
    // A versioner-marked node is emitted inside `std::hint::black_box(...)` — an identity
    // that hides a value's proven range from LLVM's over-unroller. Lift the mark while
    // emitting the inner form so it wraps exactly once, then restore it.
    if crate::optimize::facts::unmark_expr(crate::optimize::facts::Fact::BlackBox, expr) {
        let inner = codegen_expr_ctx(expr, ecx);
        crate::optimize::facts::mark_expr(crate::optimize::facts::Fact::BlackBox, expr);
        return format!("std::hint::black_box({})", inner);
    }
    if let Some(code) = try_mask_narrow(expr, ecx) {
        return code;
    }
    // Opt::StrengthReducePow2: `x % 2^k` -> `x & (2^k-1)`, `x / 2^k` -> `x >> k`, when `x >= 0`.
    // Intercepted HERE (before exact-chain / narrowed-op routing) so it fires uniformly on every
    // path. Byte-identical for `x >= 0`; the value is in `[0, x]` so it never overflows i64 — the
    // surrounding overflow guard is unaffected. Hands LLVM the single AND/SHR it can't derive from a
    // signed i64 `%`/`/`, which also unblocks auto-vectorization.
    if let Expr::BinaryOp { op, left, right } = expr {
        if matches!(op, BinaryOpKind::Modulo | BinaryOpKind::Divide)
            && crate::optimize::active_config().is_on(crate::optimization::Opt::StrengthReducePow2)
        {
            if let Expr::Literal(Literal::Number(d)) = right {
                // Skip a bignum-promoted operand: `LogosInt` has no `Shr`/`BitAnd` with an integer, so
                // `len >> 3` / `len & 7` would be an E0369 — the division/modulo must stay on the exact
                // bignum path. (Only fire on a raw i64 `x >= 0`, where the mask/shift is valid + faster.)
                if *d > 0
                    && (*d as u64).is_power_of_two()
                    && is_nonneg_expr(left)
                    && !mentions_bigint_var(left, ecx.variable_types)
                {
                    crate::optimize::mark_fired(crate::optimization::Opt::StrengthReducePow2);
                    let l = codegen_expr_ctx(left, ecx);
                    return match op {
                        BinaryOpKind::Modulo => format!("({} & {})", l, d - 1),
                        _ => format!("({} >> {})", l, d.trailing_zeros()),
                    };
                }
            }
        }
    }
    if let Some(code) = try_mod_wrap(expr, ecx) {
        return code;
    }
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
            codegen_expr_ctx(
                $e,
                &ExprCtx {
                    int_index_context: true,
                    ..*ecx
                },
            )
        };
    }

    match expr {
        // Server rendering of a View lands with `## Route … -> View`; until then the AOT tier
        // refuses at compile time rather than emitting a value it cannot represent.
        Expr::View(_) => {
            return "compile_error!(\"LGX: a View value has no AOT-Rust representation yet\")"
                .to_string()
        }
        Expr::Dom(_) => {
            return "compile_error!(\"LGX: a page query has no AOT-Rust representation yet\")"
                .to_string()
        }
        Expr::Literal(lit) => codegen_literal(lit, interner),

        Expr::Identifier(sym) => {
            if let Some((read, _)) = clock_read(interner.resolve(*sym)) {
                return read.to_string();
            }
            let name = names.ident(*sym);
            // Dereference boxed bindings from enum destructuring, and a `mutable` parameter lent
            // as `&mut` — both are read and written as the place behind the reference.
            let base = if boxed_bindings.contains(sym) || lent_in_place(*sym, variable_types) {
                format!("(*{})", name)
            } else {
                name
            };
            // An overflow-promoting `LogosInt` variable: in an INDEX position it must narrow to
            // an i64 (`.expect_i64`, then `- 1 as usize` downstream); in any other value position
            // it is read by value, so `.clone()` avoids a move (LogosInt is not Copy).
            if variable_types
                .get(sym)
                .map_or(false, |t| t.contains("__bigint"))
            {
                if ecx.int_index_context {
                    format!("{}.expect_i64(\"Int\")", base)
                } else if is_movable_last_use(*sym) {
                    // Dead last-use (reassigned/returned right after): move, not clone.
                    base
                } else {
                    format!("{}.clone()", base)
                }
            } else {
                base
            }
        }

        Expr::BinaryOp { op, left, right } => {
            // A comparison or equality reads a counter operand as its total (`is_counter_read`).
            let compares = matches!(
                op,
                BinaryOpKind::Lt
                    | BinaryOpKind::Gt
                    | BinaryOpKind::LtEq
                    | BinaryOpKind::GtEq
                    | BinaryOpKind::Eq
                    | BinaryOpKind::NotEq
            );
            macro_rules! recurse {
                ($e:expr) => {{
                    let operand: &Expr = $e;
                    let code = codegen_expr_ctx(operand, ecx);
                    if compares && is_counter_read(operand, ecx) {
                        format!("{}.value()", code)
                    } else {
                        code
                    }
                }};
            }
            // Rotate-idiom recognition: an inlined/const-specialized `rotateLeft(x,s,w)` folds to
            // `((x << s) & M) | ((x & M) >> (w-s))` — a masked shift LLVM will not turn into a
            // rotate. That shape IS a w-bit rotate of x's low w bits, so emit the single-`ROL`
            // intrinsic. (The clean call-site form is handled in the `Call` arm; this catches the
            // post-specialization form the optimizer produces when the args are constant.)
            if matches!(op, BinaryOpKind::BitOr)
                && crate::optimize::active_config().is_on(crate::optimization::Opt::BitIntrinsics)
            {
                if let Some(code) = try_rotate_intrinsic(*left, *right, &|e| recurse!(e)) {
                    crate::optimize::mark_fired(crate::optimization::Opt::BitIntrinsics);
                    return code;
                }
            }
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
                        let method = if matches!(op, BinaryOpKind::Modulo) {
                            "rem"
                        } else {
                            "div"
                        };
                        return format!("({}.{}(({}) as u64) as i64)", helper, method, dividend);
                    }
                }
            }

            // Flatten chained string concat/add into a single format! call.
            // Turns O(n^2) nested format! into O(n) single-allocation.
            // ★ A TEXT-TYPED FIELD READ, PARAMETER, OR INDEXED READ IS A STRING OPERAND. `p's left
            // + p's right` was not recognised as concatenation, so it emitted `p.left + p.right` —
            // and Rust's `+` on `String` wants `&str` on the right, so it did not build. The type
            // is the only thing that can answer this: a FieldAccess alone cannot be assumed to be
            // text, because an Int field under `+` is ADDITION and folding it into a `format!`
            // would turn `5 + 1` into "51". The same blind spot hits a Text-typed function
            // PARAMETER (never entered into `string_vars`, only a `Let` local is) and an indexed
            // read of a `Seq of Text` / `Map of _ to Text` (`is_definitely_string_expr_with_vars`
            // has no `Expr::Index` arm at all) — `is_text_typed_identifier` and
            // `is_text_typed_index` close both the same way `record_field_rust_type` closes the
            // field case: by consulting the DECLARED type instead of guessing from shape alone.
            let string_operand = |e: &Expr| {
                is_definitely_string_expr_with_vars(e, string_vars)
                    || super::absent::holds_text(e, ecx)
                    || is_text_typed_identifier(e, variable_types)
                    || is_text_typed_index(e, variable_types)
                    || record_field_rust_type(e, interner, variable_types, ecx.registry)
                        .is_some_and(|t| t == "String")
            };
            let is_string_concat = matches!(op, BinaryOpKind::Concat)
                || (matches!(op, BinaryOpKind::Add)
                    && (string_operand(left) || string_operand(right)));

            if is_string_concat {
                let mut operands = Vec::new();
                collect_string_concat_operands_with(expr, &string_operand, &mut operands);
                let placeholders: String =
                    operands.iter().map(|_| "{}").collect::<Vec<_>>().join("");
                let values: Vec<String> = operands
                    .iter()
                    .map(|e| {
                        // String literals can be &str inside format!() — no heap allocation needed
                        if let Expr::Literal(Literal::Text(sym)) = e {
                            format!(
                                "\"{}\"",
                                crate::codegen::stmt::rust_string_escape(interner.resolve(*sym))
                            )
                        } else if matches!(e, Expr::Literal(Literal::Nothing) | Expr::OptionNone) {
                            "\"nothing\"".to_string()
                        } else if super::absent::is_held(e, ecx) {
                            // A value-or-nothing joins as it shows: its value, or `nothing`.
                            format!("logicaffeine_system::io::show_string(&({}))", recurse!(e))
                        } else {
                            recurse!(e)
                        }
                    })
                    .collect();
                return format_in_order(&placeholders, &operands, values);
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
            if matches!(
                op,
                BinaryOpKind::BitAnd
                    | BinaryOpKind::BitOr
                    | BinaryOpKind::BitXor
                    | BinaryOpKind::Subtract
            ) {
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
                    let (seq_e, n_e) = if seq_first {
                        (left, right)
                    } else {
                        (right, left)
                    };
                    let s = recurse!(seq_e);
                    let n = recurse!(n_e);
                    return format!(
                        "{{ let __rp_src = ({}).to_vec(); let __rp_n = (({}) as i64).max(0) as usize; \
                         let mut __rp = Vec::with_capacity(__rp_src.len() * __rp_n); \
                         for _ in 0..__rp_n {{ __rp.extend(__rp_src.iter().map(|__e| logicaffeine_data::deep_copy!(*__e))); }} \
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
                                return format!(
                                    "({}.get(&({})) {} Some({}))",
                                    coll_str, key_str, cmp, val_str
                                );
                            } else if t.starts_with("std::collections::HashMap")
                                || t.starts_with("HashMap")
                                || t.starts_with("rustc_hash::FxHashMap")
                                || t.starts_with("FxHashMap")
                            {
                                let coll_str = recurse!(collection);
                                let key_str = irecurse!(index);
                                let val_str = recurse!(right);
                                let cmp = if neg { "!=" } else { "==" };
                                if has_copy_value_type(t) {
                                    return format!(
                                        "({}.get(&({})).copied() {} Some({}))",
                                        coll_str, key_str, cmp, val_str
                                    );
                                } else {
                                    return format!(
                                        "({}.get(&({})) {} Some(&({})))",
                                        coll_str, key_str, cmp, val_str
                                    );
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
                                return format!(
                                    "(Some({}) {} {}.get(&({})))",
                                    val_str, cmp, coll_str, key_str
                                );
                            } else if t.starts_with("std::collections::HashMap")
                                || t.starts_with("HashMap")
                                || t.starts_with("rustc_hash::FxHashMap")
                                || t.starts_with("FxHashMap")
                            {
                                let coll_str = recurse!(collection);
                                let key_str = irecurse!(index);
                                let val_str = recurse!(left);
                                let cmp = if neg { "!=" } else { "==" };
                                if has_copy_value_type(t) {
                                    return format!(
                                        "(Some({}) {} {}.get(&({})).copied())",
                                        val_str, cmp, coll_str, key_str
                                    );
                                } else {
                                    return format!(
                                        "(Some(&({})) {} {}.get(&({})))",
                                        val_str, cmp, coll_str, key_str
                                    );
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
                if let (
                    Expr::Index {
                        collection: left_coll,
                        index: left_idx,
                    },
                    Expr::Index {
                        collection: right_coll,
                        index: right_idx,
                    },
                ) = (left, right)
                {
                    let left_is_string = if let Expr::Identifier(sym) = left_coll {
                        string_vars.contains(sym)
                            || variable_types.get(sym).map_or(false, |t| t == "String")
                    } else {
                        false
                    };
                    let right_is_string = if let Expr::Identifier(sym) = right_coll {
                        string_vars.contains(sym)
                            || variable_types.get(sym).map_or(false, |t| t == "String")
                    } else {
                        false
                    };
                    if left_is_string && right_is_string {
                        let cmp = if neg { "!=" } else { "==" };
                        let left_coll_str = recurse!(left_coll);
                        let right_coll_str = recurse!(right_coll);
                        let left_idx_simplified = super::peephole::simplify_1based_index(
                            left_idx,
                            interner,
                            true,
                            variable_types,
                        );
                        let right_idx_simplified = super::peephole::simplify_1based_index(
                            right_idx,
                            interner,
                            true,
                            variable_types,
                        );
                        return format!(
                            "({}.as_bytes()[{}] {} {}.as_bytes()[{}])",
                            left_coll_str,
                            left_idx_simplified,
                            cmp,
                            right_coll_str,
                            right_idx_simplified
                        );
                    }
                }

                // Optimize string-index-vs-single-char-literal comparison to use
                // logos_get_char() == 'c' instead of LogosIndex::logos_get() == String::from("c").
                // Avoids two heap allocations per comparison in hot loops.
                let is_string_index = |expr: &Expr| -> bool {
                    if let Expr::Index { collection, .. } = expr {
                        if let Expr::Identifier(sym) = collection {
                            return string_vars.contains(sym)
                                || variable_types.get(sym).map_or(false, |t| t == "String");
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
                            return format!(
                                "({}.logos_get_char({}) {} '{}')",
                                coll_str, idx_str, cmp, ch_escaped
                            );
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
                            return format!(
                                "('{}' {} {}.logos_get_char({}))",
                                ch_escaped, cmp, coll_str, idx_str
                            );
                        }
                    }
                }
            }

            // OPT-8b: Zero-based counter in comparison.
            // When a __zero_based_i64 counter appears as a bare operand in a comparison,
            // emit (counter + 1) to compensate for the 0-based range shift.
            // E.g., `If i > 3` with 0-based `i` becomes `if (i + 1) > 3`.
            if matches!(
                op,
                BinaryOpKind::Lt
                    | BinaryOpKind::LtEq
                    | BinaryOpKind::Gt
                    | BinaryOpKind::GtEq
                    | BinaryOpKind::Eq
                    | BinaryOpKind::NotEq
            ) {
                let left_zb = if let Expr::Identifier(sym) = left {
                    variable_types
                        .get(sym)
                        .map_or(false, |t| t == "__zero_based_i64")
                } else {
                    false
                };
                let right_zb = if let Expr::Identifier(sym) = right {
                    variable_types
                        .get(sym)
                        .map_or(false, |t| t == "__zero_based_i64")
                } else {
                    false
                };
                if left_zb || right_zb {
                    let left_str = if left_zb {
                        format!("({} + 1)", recurse!(left))
                    } else {
                        recurse!(left)
                    };
                    let right_str = if right_zb {
                        format!("({} + 1)", recurse!(right))
                    } else {
                        recurse!(right)
                    };
                    let op_str = match op {
                        BinaryOpKind::Lt => "<",
                        BinaryOpKind::LtEq => "<=",
                        BinaryOpKind::Gt => ">",
                        BinaryOpKind::GtEq => ">=",
                        BinaryOpKind::Eq => "==",
                        BinaryOpKind::NotEq => "!=",
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
                BinaryOpKind::Add
                    | BinaryOpKind::Subtract
                    | BinaryOpKind::Multiply
                    | BinaryOpKind::Divide
                    | BinaryOpKind::Lt
                    | BinaryOpKind::Gt
                    | BinaryOpKind::LtEq
                    | BinaryOpKind::GtEq
                    | BinaryOpKind::Eq
                    | BinaryOpKind::NotEq
            ) && (is_quantity_expr(left, variable_types, interner)
                || is_quantity_expr(right, variable_types, interner))
            {
                let l_q = is_quantity_expr(left, variable_types, interner);
                let r_q = is_quantity_expr(right, variable_types, interner);
                let code: Option<std::string::String> = match (op, l_q, r_q) {
                    (BinaryOpKind::Add, true, true) => {
                        Some(format!("{}.add(&{})", left_str, right_str))
                    }
                    (BinaryOpKind::Subtract, true, true) => {
                        Some(format!("{}.sub(&{})", left_str, right_str))
                    }
                    (BinaryOpKind::Multiply, true, true) => {
                        Some(format!("{}.mul(&{})", left_str, right_str))
                    }
                    (BinaryOpKind::Divide, true, true) => {
                        Some(format!("{}.div_exact(&{})", left_str, right_str))
                    }
                    // Scale a quantity by an integer (the common case), commuting for ×.
                    (BinaryOpKind::Multiply, true, false) => {
                        Some(format!("{}.scale_int({})", left_str, right_str))
                    }
                    (BinaryOpKind::Multiply, false, true) => {
                        Some(format!("{}.scale_int({})", right_str, left_str))
                    }
                    (BinaryOpKind::Divide, true, false) => {
                        Some(format!("{}.div_int({})", left_str, right_str))
                    }
                    // Comparison / equality over two quantities (PartialOrd / physical PartialEq).
                    (BinaryOpKind::Lt, true, true) => {
                        Some(format!("({} < {})", left_str, right_str))
                    }
                    (BinaryOpKind::Gt, true, true) => {
                        Some(format!("({} > {})", left_str, right_str))
                    }
                    (BinaryOpKind::LtEq, true, true) => {
                        Some(format!("({} <= {})", left_str, right_str))
                    }
                    (BinaryOpKind::GtEq, true, true) => {
                        Some(format!("({} >= {})", left_str, right_str))
                    }
                    (BinaryOpKind::Eq, true, true) => {
                        Some(format!("({} == {})", left_str, right_str))
                    }
                    (BinaryOpKind::NotEq, true, true) => {
                        Some(format!("({} != {})", left_str, right_str))
                    }
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
                BinaryOpKind::Add
                    | BinaryOpKind::Subtract
                    | BinaryOpKind::Multiply
                    | BinaryOpKind::Divide
                    | BinaryOpKind::Lt
                    | BinaryOpKind::Gt
                    | BinaryOpKind::LtEq
                    | BinaryOpKind::GtEq
                    | BinaryOpKind::Eq
                    | BinaryOpKind::NotEq
            ) && (is_money_expr(left, variable_types, interner)
                || is_money_expr(right, variable_types, interner))
            {
                let l_m = is_money_expr(left, variable_types, interner);
                let r_m = is_money_expr(right, variable_types, interner);
                let code: Option<std::string::String> = match (op, l_m, r_m) {
                    (BinaryOpKind::Add, true, true) => {
                        Some(format!("{}.add(&{})", left_str, right_str))
                    }
                    (BinaryOpKind::Subtract, true, true) => {
                        Some(format!("{}.sub(&{})", left_str, right_str))
                    }
                    // Same-currency ratio → an exact Rational.
                    (BinaryOpKind::Divide, true, true) => {
                        Some(format!("{}.ratio(&{})", left_str, right_str))
                    }
                    // Scale money by an integer (commutes for ×); split money by an integer.
                    (BinaryOpKind::Multiply, true, false) => {
                        Some(format!("{}.scale_int({})", left_str, right_str))
                    }
                    (BinaryOpKind::Multiply, false, true) => {
                        Some(format!("{}.scale_int({})", right_str, left_str))
                    }
                    (BinaryOpKind::Divide, true, false) => {
                        Some(format!("{}.div_int({})", left_str, right_str))
                    }
                    (BinaryOpKind::Lt, true, true) => {
                        Some(format!("({} < {})", left_str, right_str))
                    }
                    (BinaryOpKind::Gt, true, true) => {
                        Some(format!("({} > {})", left_str, right_str))
                    }
                    (BinaryOpKind::LtEq, true, true) => {
                        Some(format!("({} <= {})", left_str, right_str))
                    }
                    (BinaryOpKind::GtEq, true, true) => {
                        Some(format!("({} >= {})", left_str, right_str))
                    }
                    (BinaryOpKind::Eq, true, true) => {
                        Some(format!("({} == {})", left_str, right_str))
                    }
                    (BinaryOpKind::NotEq, true, true) => {
                        Some(format!("({} != {})", left_str, right_str))
                    }
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
                BinaryOpKind::Add
                    | BinaryOpKind::Subtract
                    | BinaryOpKind::Multiply
                    | BinaryOpKind::Divide
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

            // Floating-point complex arithmetic. `+ − × ÷` stay a `LogosComplexF` the moment one
            // operand is `ComplexF` (a real/exact operand embeds as `re + 0i`). Checked BEFORE the
            // exact-Complex path, so two exact complexes still stay exact.
            if matches!(
                op,
                BinaryOpKind::Add
                    | BinaryOpKind::Subtract
                    | BinaryOpKind::Multiply
                    | BinaryOpKind::Divide
            ) && (is_complexf_expr(left, variable_types, interner)
                || is_complexf_expr(right, variable_types, interner))
            {
                let l = complexf_operand(left, &left_str, variable_types, interner);
                let r = complexf_operand(right, &right_str, variable_types, interner);
                let method = match op {
                    BinaryOpKind::Add => "add",
                    BinaryOpKind::Subtract => "sub",
                    BinaryOpKind::Multiply => "mul",
                    BinaryOpKind::Divide => "div_exact",
                    _ => unreachable!(),
                };
                return format!("{}.{}(&{})", l, method, r);
            }

            // Exact Complex arithmetic. `+ − × ÷` stay an exact `LogosComplex` (the field is
            // closed) the moment one operand is Complex; an integer operand embeds as `re + 0i`.
            if matches!(
                op,
                BinaryOpKind::Add
                    | BinaryOpKind::Subtract
                    | BinaryOpKind::Multiply
                    | BinaryOpKind::Divide
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
            if matches!(
                op,
                BinaryOpKind::Add | BinaryOpKind::Subtract | BinaryOpKind::Multiply
            ) && (is_decimal_expr(left, variable_types, interner)
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
                BinaryOpKind::Lt
                    | BinaryOpKind::Gt
                    | BinaryOpKind::LtEq
                    | BinaryOpKind::GtEq
                    | BinaryOpKind::Eq
                    | BinaryOpKind::NotEq
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
                let op_str = if matches!(op, BinaryOpKind::Eq) {
                    "=="
                } else {
                    "!="
                };
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
                    let l = if lt == "f64" {
                        left_str.clone()
                    } else {
                        format!("(({}) as f64)", left_str)
                    };
                    let r = if rt == "f64" {
                        right_str.clone()
                    } else {
                        format!("(({}) as f64)", right_str)
                    };
                    return format!("({}).powf({})", l, r);
                }
                // Both operands constant: fold at compile time. The i64 fast path
                // (`checked_pow`) emits a plain literal; overflow or a negative/oversized
                // exponent falls through to the runtime helper (BigInt promotion, or the
                // loud negative-exponent error), never a giant compile-time BigInt.
                if let (Expr::Literal(Literal::Number(a)), Expr::Literal(Literal::Number(b))) =
                    (left, right)
                {
                    if let Ok(e) = u32::try_from(*b) {
                        if let Some(v) = a.checked_pow(e) {
                            return format!("{}i64", v);
                        }
                    }
                }
                let tol = ExprCtx {
                    int_exact_tolerant: true,
                    ..*ecx
                };
                let l = codegen_expr_ctx(left, &tol);
                let r = codegen_expr_ctx(right, &tol);
                // A bare integer-literal operand needs its `i64` suffix: only `i64: Into<LogosInt>`,
                // so a bare `{integer}` (default i32) would fail the `logos_pow_exact` bound.
                let l = if matches!(left, Expr::Literal(Literal::Number(_))) {
                    format!("{}i64", l)
                } else {
                    l
                };
                let r = if matches!(right, Expr::Literal(Literal::Number(_))) {
                    format!("{}i64", r)
                } else {
                    r
                };
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
                    let l = if lt == "f64" {
                        left_str.clone()
                    } else {
                        format!("(({}) as f64)", left_str)
                    };
                    let r = if rt == "f64" {
                        right_str.clone()
                    } else {
                        format!("(({}) as f64)", right_str)
                    };
                    return format!("({} / {}).floor()", l, r);
                }
                // Both operands constant: fold the floored quotient at compile time when
                // it fits i64 (a zero divisor or the `i64::MIN // -1` overflow falls
                // through to the promoting runtime helper).
                if let (Expr::Literal(Literal::Number(a)), Expr::Literal(Literal::Number(b))) =
                    (left, right)
                {
                    if *b != 0 {
                        if let (Some(q), Some(r)) = (a.checked_div(*b), a.checked_rem(*b)) {
                            let floored = if r != 0 && (r < 0) != (*b < 0) {
                                q - 1
                            } else {
                                q
                            };
                            return format!("{}i64", floored);
                        }
                    }
                }
                let tol = ExprCtx {
                    int_exact_tolerant: true,
                    ..*ecx
                };
                let l = codegen_expr_ctx(left, &tol);
                let r = codegen_expr_ctx(right, &tol);
                let l = if matches!(left, Expr::Literal(Literal::Number(_))) {
                    format!("{}i64", l)
                } else {
                    l
                };
                let r = if matches!(right, Expr::Literal(Literal::Number(_))) {
                    format!("{}i64", r)
                } else {
                    r
                };
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
                BinaryOpKind::Concat => unreachable!(),                 // handled above
                BinaryOpKind::SeqConcat => unreachable!(),              // handled above
                BinaryOpKind::ApproxEq => unreachable!(),               // handled above
                BinaryOpKind::Pow => unreachable!(),                    // handled above
                BinaryOpKind::FloorDivide => unreachable!(),            // handled above
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
                let left_type = numeric_type_through_arith(left, interner, variable_types);
                let right_type = numeric_type_through_arith(right, interner, variable_types);
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
                        (BinaryOpKind::Lt, false) | (BinaryOpKind::Gt, true) => {
                            "Some(core::cmp::Ordering::Less)"
                        }
                        (BinaryOpKind::Gt, false) | (BinaryOpKind::Lt, true) => {
                            "Some(core::cmp::Ordering::Greater)"
                        }
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
                    return format!(
                        "matches!(logos_cmp_i64_f64({}, {}), {})",
                        int_e, float_e, pat
                    );
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
                    && matches!(
                        op,
                        BinaryOpKind::Add | BinaryOpKind::Subtract | BinaryOpKind::Multiply
                    )
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

                // ★ BIT OPERATIONS ACT ON MACHINE WORDS. `semantics::arith::binary_op` defines `|`, `^`,
                // `<<` and `>>` on two `Int`s alone, and `&` also on a big integer that is not negative
                // beside a low mask `2^k - 1`, taking its low `k` bits; any other big operand is refused in
                // the operator's words. An Int operand that may be a big integer here — a promoted binding
                // or field, a bignum call, or arithmetic the Oracle cannot keep within i64 — is computed
                // exactly and handed to the runtime's rule (`logos_band_exact`, `logos_word_operand`), so
                // the value, or the refusal, is the reference's. A raw `LogosInt & i64` has no operator.
                let is_bitwise = matches!(
                    op,
                    BinaryOpKind::Shl
                        | BinaryOpKind::Shr
                        | BinaryOpKind::BitAnd
                        | BinaryOpKind::BitOr
                        | BinaryOpKind::BitXor
                );
                let promotes = |e: &Expr| {
                    mentions_bigint_var(e, variable_types) || !emits_pure_i64(e, ecx)
                };
                let int_operand = |e: &Expr, ty: &str| {
                    ty == "i64" || mentions_bigint_var(e, variable_types)
                };
                if is_bitwise
                    && int_operand(left, left_type)
                    && int_operand(right, right_type)
                    && (promotes(left) || promotes(right))
                {
                    if let Some(dual) =
                        try_guarded_bitwise(*op, op_str, left, right, ecx, &promotes)
                    {
                        return dual;
                    }
                    let exact = ExprCtx {
                        int_exact_tolerant: true,
                        ..*ecx
                    };
                    let l = codegen_expr_ctx(left, &exact);
                    let r = codegen_expr_ctx(right, &exact);
                    if matches!(op, BinaryOpKind::BitAnd) {
                        return format!("logos_band_exact({l}, {r})");
                    }
                    let refusal = match op {
                        BinaryOpKind::BitOr => "BITOR_OPERANDS",
                        BinaryOpKind::BitXor => "BITXOR_OPERANDS",
                        BinaryOpKind::Shl => "SHL_OPERANDS",
                        _ => "SHR_OPERANDS",
                    };
                    let word = |e: &Expr, code: String| {
                        if promotes(e) {
                            format!(
                                "logos_word_operand({code}, logicaffeine_data::failure::{refusal})"
                            )
                        } else {
                            code
                        }
                    };
                    return format!("({} {} {})", word(left, l), op_str, word(right, r));
                }
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
                        // i64, so it must use the exact helper below rather than this raw fast-path —
                        // including a dividend whose own inner sub-op promotes (`emits_pure_i64`).
                        if (safe_div || safe_mod)
                            && !has_bigint_operand
                            && emits_pure_i64(left, ecx)
                        {
                            // The raw i64 `/`/`%` needs an i64 DIVIDEND. In a tolerant context `left_str` is
                            // a promoted `LogosInt` (no `Div<i64>`/`Rem<i64>`) — e.g. `(a + b) / 64` inside
                            // an exact op — so recompute the dividend as i64; the i64 result flows back into
                            // the enclosing exact op via `Into`. (The divisor `d` is a literal, always i64.)
                            let l = if ecx.int_exact_tolerant {
                                codegen_expr_ctx(
                                    left,
                                    &ExprCtx {
                                        int_exact_tolerant: false,
                                        ..*ecx
                                    },
                                )
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
                    } else if !has_bigint_operand
                        && oracle_proves_int_op_in_range(ecx, expr, *op, left, right)
                        && emits_pure_i64(left, ecx)
                        && emits_pure_i64(right, ecx)
                    {
                        // A raw `left op right` needs both sides to be i64. With a promoted operand
                        // (`LogosInt`, no arithmetic operators) the exact helper below is the only
                        // valid lowering, even when the result provably fits i64 — the Oracle can
                        // prove THIS sum in range while an inner sub-sum promotes (E0369 otherwise).
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
                    // By-reference bignum arithmetic (clone elision). A promoted (`LogosInt`)
                    // operand forces the exact helper — but `logos_*_exact(a, b)` takes its
                    // operands BY VALUE, so a reused bignum variable `.clone()`s its heap buffer
                    // every time purely to be passed. `LogosInt::{add,sub,mul}` are already
                    // `(&self, &LogosInt) -> LogosInt`, and `logos_mul_exact(a, b)` is DEFINED as
                    // `a.into().mul(&b.into())` — so the by-ref method is byte-identical (same
                    // overflow-promotion) minus the clone. A reused bignum var is BORROWED; an
                    // i64/temporary operand is bound once to a `LogosInt` and borrowed. `/`/`%`
                    // keep the helper (its loud zero-divisor panic has no by-ref method form).
                    if has_bigint_operand
                        && matches!(
                            op,
                            BinaryOpKind::Add | BinaryOpKind::Subtract | BinaryOpKind::Multiply
                        )
                        && crate::optimize::active_config()
                            .is_on(crate::optimization::Opt::BignumByRef)
                    {
                        crate::optimize::mark_fired(crate::optimization::Opt::BignumByRef);
                        let method = match op {
                            BinaryOpKind::Add => "add",
                            BinaryOpKind::Subtract => "sub",
                            BinaryOpKind::Multiply => "mul",
                            _ => unreachable!("guarded to Add/Subtract/Multiply"),
                        };
                        let mut prelude = String::new();
                        let l_place = bignum_operand_place(left, ecx, &names, &mut prelude);
                        let r_place = bignum_operand_place(right, ecx, &names, &mut prelude);
                        let core = format!("{}.{}(&{})", l_place, method, r_place);
                        let body = if prelude.is_empty() {
                            core
                        } else {
                            // Parenthesize a block so `.expect_i64(..)` (the non-tolerant narrow)
                            // never mis-parses it at statement position.
                            format!("({{ {}{} }})", prelude, core)
                        };
                        return narrow(body);
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
                    let tol = ExprCtx {
                        int_exact_tolerant: true,
                        ..*ecx
                    };
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
            // ★ THE TEXT `contains(haystack, needle)` BUILTIN. `xs contains v` — the OPERATOR form —
            // has always emitted `logos_contains`; the CALL form had no arm at all, so it fell
            // through to an ordinary call and emitted `contains(a, b)`, a free function that does
            // not exist in the generated crate. Any program asking whether a text holds something
            // produced Rust that would not build — including `web.lg`'s `isIntText`, which is what
            // every typed route parameter is validated with. One meaning, one emission.
            if raw_name == "contains" && args.len() == 2 {
                return format!(
                    "{}.logos_contains(&{})",
                    recurse!(&args[0]),
                    recurse!(&args[1])
                );
            }
            // ★ THE `format` BUILTIN renders a value exactly as `Show` prints it — the same
            // `show_string` interpolation holes use. It had no arm at all, so every `format(x)` the
            // guide teaches emitted a call to Rust's `format!` MACRO as a function and the program
            // did not build. A program that defines its own `format` keeps it (declarer wins).
            if raw_name == "format" && args.len() == 1 && !program_defines_function(*function) {
                return format!(
                    "logicaffeine_system::io::show_string(&({}))",
                    recurse!(&args[0])
                );
            }
            // `length(x)` and `copy(x)` are the call spellings of `length of x` and `copy of x`: each
            // emits exactly what its dedicated node emits. A program's own `length`/`copy` wins.
            if args.len() == 1 && !program_defines_function(*function) {
                let canonical = match raw_name {
                    "length" => Some(Expr::Length {
                        collection: args[0],
                    }),
                    "copy" => Some(Expr::Copy { expr: args[0] }),
                    _ => None,
                };
                if let Some(canonical) = canonical {
                    return recurse!(&canonical);
                }
            }
            // Native-width BIT INTRINSICS (the stdlib `bits` ops). The stdlib `rotateLeft`/
            // `rotateRight`/`reverseBits`/`reverseBytes` are width-parameterized and compile to a
            // masked LOGOS loop that LLVM will NOT recognize as a rotate/reverse. But with a
            // COMPILE-TIME native word width (8/16/32/64) the body is EXACTLY one Rust intrinsic —
            // `(x as uW).rotate_left(n)` / `.reverse_bits()` / `.swap_bytes()` — which LLVM lowers
            // to a single CPU instruction (ROL/ROR, bit-reverse, BSWAP). The rewrite is exact for
            // any correct rotate/reverse (their meaning at a fixed native width is universal), and
            // is matched by stdlib name + arity + native-width literal, like the other name-keyed
            // builtins (`contains`, `wordN`). A non-native or non-literal width falls through to
            // the general stdlib call unchanged.
            if crate::optimize::active_config().is_on(crate::optimization::Opt::BitIntrinsics)
                && crate::loader::is_library_function(raw_name)
            {
                let width_of = |a: &Expr| -> Option<i64> {
                    if let Expr::Literal(Literal::Number(w)) = a {
                        if matches!(*w, 8 | 16 | 32 | 64) {
                            return Some(*w);
                        }
                    }
                    None
                };
                let fired = || crate::optimize::mark_fired(crate::optimization::Opt::BitIntrinsics);
                match (raw_name, args.len()) {
                    ("rotateLeft", 3) | ("rotateRight", 3) => {
                        if let Some(w) = width_of(args[2]) {
                            fired();
                            let m = if raw_name == "rotateLeft" {
                                "rotate_left"
                            } else {
                                "rotate_right"
                            };
                            return format!(
                                "(({} as u{w}).{}(({}) as u32) as i64)",
                                recurse!(&args[0]),
                                m,
                                recurse!(&args[1])
                            );
                        }
                    }
                    ("reverseBits", 2) => {
                        if let Some(w) = width_of(args[1]) {
                            fired();
                            return format!(
                                "(({} as u{w}).reverse_bits() as i64)",
                                recurse!(&args[0])
                            );
                        }
                    }
                    ("reverseBytes", 2) => {
                        if let Some(w) = width_of(args[1]) {
                            fired();
                            return format!(
                                "(({} as u{w}).swap_bytes() as i64)",
                                recurse!(&args[0])
                            );
                        }
                    }
                    _ => {}
                }
            }
            let args_str: Vec<String> = call_arguments(*function, args, ecx);
            // Builtin math functions → Rust method call syntax
            match raw_name {
                // stdlib `indexOf`/`countSub` (Text substring search) → the
                // `memchr` crate's SIMD `memmem` (a vectorized skip-loop that
                // beats glibc `strstr` and Rust's std two-way `str::find`). The
                // byte-indexed 1-based result and non-overlapping count are
                // identical to the pure-LOGOS body (so the tier differential
                // holds); empty needle is guarded to match its `0` return, and a
                // String identifier is borrowed (not cloned) so a large haystack
                // is not copied on every call. (`build.rs` injects the `memchr`
                // dep when the generated code names it.)
                "indexOf" | "countSub" if args_str.len() == 2 && crate::loader::is_library_function(raw_name) => {
                    let mut decl = String::new();
                    let hv = if let Expr::Identifier(sym) = &args[0] {
                        format!("&{}", names.ident(*sym))
                    } else {
                        decl.push_str(&format!("let __ho = ({}).to_string(); ", args_str[0]));
                        "&__ho".to_string()
                    };
                    let nv = if let Expr::Identifier(sym) = &args[1] {
                        format!("&{}", names.ident(*sym))
                    } else {
                        decl.push_str(&format!("let __no = ({}).to_string(); ", args_str[1]));
                        "&__no".to_string()
                    };
                    let op = if raw_name == "indexOf" {
                        // glibc `strstr`'s strategy — a single-byte SIMD prefilter
                        // on the needle's first byte + a `starts_with` verify — but
                        // with the `memchr` crate's scanner (the one behind ripgrep,
                        // which beats glibc's own memchr). One memory sweep, no
                        // two-byte-prefilter overhead of `memmem::find`.
                        "if __n.is_empty() { 0i64 } else { let __hb = __h.as_bytes(); let __nb = __n.as_bytes(); let __m = __nb.len(); if __m > __hb.len() { 0i64 } else { let __f = __nb[0]; let __last = __hb.len() - __m; let mut __o = 0usize; let mut __r = 0i64; while __o <= __last { match memchr::memchr(__f, &__hb[__o..=__last]) { Some(__p) => { let __i = __o + __p; if __hb[__i..].starts_with(__nb) { __r = __i as i64 + 1; break; } __o = __i + 1; } None => break, } } __r } }"
                    } else {
                        // Non-overlapping count: `memmem::find_iter` builds ONE
                        // Finder and iterates it (no per-match setup), which a
                        // hand-rolled per-match `memchr` cannot match when the
                        // needle is frequent (a match every few bytes).
                        "if __n.is_empty() { 0i64 } else { memchr::memmem::find_iter(__h.as_bytes(), __n.as_bytes()).count() as i64 }"
                    };
                    format!("{{ {}let __h: &str = {}; let __n: &str = {}; {} }}", decl, hv, nv, op)
                }
                "sqrt" if args_str.len() == 1 && is_complexf_expr(&args[0], variable_types, interner) => {
                    // Complex sqrt (`√(3+4i)=2+i`) — a `LogosComplexF`, via the real-sqrt formula.
                    format!("({}).sqrt()", args_str[0])
                }
                "sqrt" if args_str.len() == 1 => {
                    format!("(({}) as f64).sqrt()", args_str[0])
                }
                "conjugate"
                    if args_str.len() == 1
                        && (is_complexf_expr(&args[0], variable_types, interner)
                            || is_complex_expr(&args[0], variable_types, interner)) =>
                {
                    // Conjugate of a complex (exact `LogosComplex` or float `LogosComplexF`) — the same
                    // `.conjugate()` method negates the imaginary part on both.
                    format!("({}).conjugate()", args_str[0])
                }
                "conjugate" if args_str.len() == 1 => {
                    // A real number is its own conjugate — emit it unchanged.
                    format!("({})", args_str[0])
                }
                "re" if args_str.len() == 1
                    && (is_complexf_expr(&args[0], variable_types, interner)
                        || is_complex_expr(&args[0], variable_types, interner)) =>
                {
                    format!("({}).re()", args_str[0])
                }
                "re" if args_str.len() == 1 => {
                    // A real number IS its real part.
                    format!("(({}) as f64)", args_str[0])
                }
                "im" if args_str.len() == 1
                    && (is_complexf_expr(&args[0], variable_types, interner)
                        || is_complex_expr(&args[0], variable_types, interner)) =>
                {
                    format!("({}).im()", args_str[0])
                }
                "im" if args_str.len() == 1 => {
                    // A real number has zero imaginary part.
                    let _ = &args_str[0];
                    "0f64".to_string()
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
                         for _ in 0..__rp_n {{ __rp.push(logicaffeine_data::deep_copy!(__rp_x)); }} \
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
                // Exact base-10: `decimal_of` takes the argument by its Rust type (Text is read, an Int
                // widens, a Decimal is itself, any other kind fails with the engines' sentence).
                "decimal" if args_str.len() == 1 => {
                    format!("logicaffeine_data::decimal_of({})", args_str[0])
                }
                // Complex `re + im·i`: a float part makes it a floating-point `LogosComplexF` (has a
                // `sqrt`); two exact reals stay an exact `LogosComplex` (integers embed as `n/1`).
                "complex" if args_str.len() == 2 => {
                    if complex_call_is_float(args, variable_types, interner) {
                        format!("LogosComplexF::new(({}) as f64, ({}) as f64)", args_str[0], args_str[1])
                    } else {
                        let re = rational_operand(&args[0], &args_str[0], variable_types);
                        let im = rational_operand(&args[1], &args_str[1], variable_types);
                        format!("LogosComplex::new({}, {})", re, im)
                    }
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
                // Both builtins only BORROW their argument (`&…`), so a bare identifier needs no
                // `.clone()` — the identifier emitter adds one for non-Copy values, which here is a
                // full O(n) copy of the source text / byte-list on every call for nothing.
                "text_bytes" if args_str.len() == 1 => match args[0] {
                    Expr::Identifier(sym) => format!("text_bytes(&{})", names.ident(*sym)),
                    _ => format!("text_bytes(&({}))", args_str[0]),
                },
                "text_from_bytes" if args_str.len() == 1 => match args[0] {
                    // Write-side byte builder (Opt::TextFromBytesBuilder): `out` is already a
                    // `Vec<u8>`, so validate once and move it into the String — no `Vec<i64>`
                    // detour, no `i64`->`u8` rebuild. Same trap as the runtime on invalid UTF-8.
                    Expr::Identifier(sym) if variable_types.get(sym).map_or(false, |t| t == "ByteBuilder") =>
                        format!("String::from_utf8({}).unwrap_or_else(|e| panic!(\"{{}}\", logicaffeine_data::failure::invalid_utf8(e)))", names.ident(*sym)),
                    Expr::Identifier(sym) => format!("text_from_bytes(&{})", names.ident(*sym)),
                    _ => format!("text_from_bytes(&({}))", args_str[0]),
                },
                // A file's raw bytes as a `Seq of Int` (the `readBytes` builtin — serving a
                // `.wasm`/image from a handler). Mirrors the interpreter: a failed read is a
                // LOGOS runtime error carrying the same `readBytes: cannot read …` message,
                // never a silent empty Seq.
                // The total byte reader: a `Result` VALUE rather than a `panic!`. A LOGOS
                // `Result` is Rust's own `Result` on this tier — `file::read` has always answered
                // that way — so an `Inspect` over it lowers with the machinery already there.
                //
                // ★ IT CALLS THE SAME FUNCTION THE INTERPRETER DOES, which is the point: the read
                // AND its error wording live once, in `logicaffeine_system::file::read_bytes`. Two
                // `std::fs::read` calls with two hand-written format strings is how a message comes
                // to differ between the tier an author tests on and the tier they ship.
                "tryReadBytes" if args_str.len() == 1 => {
                    format!("match logicaffeine_system::file::read_bytes(({}).to_string()) {{ Ok(__b) => Ok(LogosSeq::from_vec(__b.iter().map(|&__x| __x as i64).collect())), Err(__e) => Err(__e) }}", args_str[0])
                }
                // The read and its refusals live once, in `logicaffeine_system::file::read_reals`, which the
                // interpreter calls too.
                "readReals" if args_str.len() == 1 => {
                    format!("match logicaffeine_system::file::read_reals(({}).to_string()) {{ Ok(__r) => LogosSeq::from_vec(__r), Err(__e) => panic!(\"{{}}\", __e) }}", args_str[0])
                }
                "readBytes" if args_str.len() == 1 => {
                    format!("{{ let __p = ({}).to_string(); match std::fs::read(&__p) {{ Ok(__b) => LogosSeq::from_vec(__b.iter().map(|&__x| __x as i64).collect()), Err(__e) => panic!(\"{{}}\", logicaffeine_data::failure::cannot_read_bytes(&__p, __e)) }} }}", args_str[0])
                }
                "readWireProgram" if args_str.is_empty() => {
                    "{ use std::io::Read as _; logicaffeine_system::io::flush_stdout(); let mut __len = [0u8; 4]; if std::io::stdin().read_exact(&mut __len).is_err() { std::process::exit(0); } let __n = u32::from_le_bytes(__len) as usize; let mut __wb = vec![0u8; __n]; std::io::stdin().read_exact(&mut __wb).expect(\"readWireProgram: frame\"); <CProgram as logicaffeine_data::wire::WireDecode>::wire_decode(&__wb, &mut 0usize).expect(\"readWireProgram: decode\") }".to_string()
                }
                "writeWireResidual" if args_str.len() == 1 => {
                    format!("{{ use std::io::Write as _; logicaffeine_system::io::flush_stdout(); let __s: String = ({}).into(); let __b = __s.as_bytes(); let __o = std::io::stdout(); let mut __h = __o.lock(); __h.write_all(&(__b.len() as u32).to_le_bytes()).unwrap(); __h.write_all(__b).unwrap(); __h.flush().unwrap(); __b.len() as i64 }}", args_str[0])
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
                // `parseInt`/`parseFloat(text)` → i64/f64. Like `chr`, they live in the `text`
                // submodule (not the crate-root glob), so the call MUST be fully qualified — a bare
                // `parseFloat(..)` fails to resolve (E0425) and poisons any spliced stdlib module
                // that uses it. `.to_string()` normalizes the argument to the `String` the runtime
                // takes, whether it arrives as a `&str`, a `String`, or an interpolated `format!`.
                "parseInt" if args_str.len() == 1 => {
                    format!("logicaffeine_system::text::parseInt(({}).to_string())", args_str[0])
                }
                "parseFloat" if args_str.len() == 1 => {
                    format!("logicaffeine_system::text::parseFloat(({}).to_string())", args_str[0])
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
                // Construct a `LogosDate`/`LogosMoment` from civil components (base `days_from_civil`).
                "makeDate" if args_str.len() == 3 => format!(
                    "LogosDate::from_ymd({}, {}, {})",
                    args_str[0], args_str[1], args_str[2]
                ),
                "makeMoment" if args_str.len() == 6 => format!(
                    "LogosMoment::from_ymd({}, {}, {}, {}, {}, {})",
                    args_str[0], args_str[1], args_str[2], args_str[3], args_str[4], args_str[5]
                ),
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
                "word8" if args_str.len() == 1 => {
                    format!("word8({})", args_str[0])
                }
                "intOfWord8" if args_str.len() == 1 => {
                    format!("int_of_word8({})", args_str[0])
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
                // |z| of an EXACT complex (`LogosComplex`, e.g. `complex(3, 4)`) is its `f64` modulus.
                // `LogosComplex` has no `abs()`; it delegates to the wrapped `Complex::abs_f64`, which is
                // exactly what the interpreter's `abs(Complex)` returns (`Float(c.abs_f64())`).
                "abs" if args_str.len() == 1 && is_complex_expr(&args[0], variable_types, interner) => {
                    format!("({}).abs_f64()", args_str[0])
                }
                // |z| of a float-complex is its `f64` modulus.
                "abs" if args_str.len() == 1 && is_complexf_expr(&args[0], variable_types, interner) => {
                    format!("({}).abs()", args_str[0])
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
                method @ ("min" | "max") if args_str.len() == 2 => {
                    // A Real on either side makes the whole call Real, as the VM promotes it: both
                    // operands widen to f64, and the language's own `min`/`max` of reals decides the
                    // sign of equal zeros and a NaN as the interpreter does.
                    let real = |e: &Expr| infer_numeric_type(e, interner, variable_types) == "f64";
                    if real(&args[0]) || real(&args[1]) {
                        format!(
                            "logicaffeine_data::logos_{}_f64(({}) as f64, ({}) as f64)",
                            method, args_str[0], args_str[1]
                        )
                    } else {
                        format!("({}).{}({})", args_str[0], method, args_str[1])
                    }
                }
                _ => {
                    let fish = super::instantiation::for_expr(expr);
                    let path = format!("{}{}", called_function_path(*function, func_name, variable_types), fish);
                    sequenced_call(&path, *function, args, args_str, Wait::of(*function, async_functions))
                }
            }
        }

        // Affine read-only array (deleted CSR offset array): `item k of A` is the
        // closed form `coeff * (k-1) + offset` — no load, no array. The 1-based→
        // 0-based `-1` cancels a `+1` in the index, so `item (v+1) of A` with
        // `A[p]=5*p` becomes `v * 5` (C's `v*MAX_EDGES`).
        Expr::Index {
            collection: Expr::Identifier(sym),
            index,
        } if affine_array_coeff_offset(variable_types.get(sym)).is_some() => {
            let (coeff, offset) = affine_array_coeff_offset(variable_types.get(sym)).unwrap();
            enum Pos {
                Const(i64),
                Expr(String),
            }
            let pos = match index {
                Expr::Literal(Literal::Number(n)) => Pos::Const(n - 1),
                Expr::BinaryOp {
                    op: BinaryOpKind::Add,
                    left,
                    right,
                } => match (left, right) {
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
                    let base = if coeff == 1 {
                        p
                    } else {
                        format!("(({}) * {})", p, coeff)
                    };
                    if offset == 0 {
                        base
                    } else {
                        format!("({} + {})", base, offset)
                    }
                }
            }
        }

        // AoS interleaving: `item i of member` reads column `col` of the fused
        // backing's row `i` names. Adjacent columns are memory-adjacent, so LLVM
        // packs them (C's struct-array load).
        Expr::Index {
            collection: member @ Expr::Identifier(sym),
            index,
        } if parse_aos_tag(variable_types.get(sym)).is_some() => {
            let tag = parse_aos_tag(variable_types.get(sym)).unwrap();
            if range_proves_index(member, index) {
                let offset = proven_offset_str(index, ecx, variable_types, interner);
                format!("{}[{}][{}]", tag.backing, offset, tag.col)
            } else {
                let at = logos_index_str(index, ecx, variable_types, interner);
                let item = item_accessor(index, ecx.oracle, false);
                format!("{}.{}({})[{}]", tag.backing, item, at, tag.col)
            }
        }

        Expr::Index { collection, index } => {
            let coll_str = recurse!(collection);
            // Direct indexing for known collection types (avoids trait dispatch)
            // Strip |__hl: hoisting suffix so type parsing (strip_suffix, etc.) works correctly.
            // A collection no binding names — a call's result — is a map when the program declares it one, and is
            // read by key as a named map is.
            let returned_map = match collection {
                Expr::Identifier(_) => None,
                _ => value_rust_type(collection, variable_types, ecx.registry, interner).filter(|t| is_logos_map_type(t)),
            };
            let known_type = if let Expr::Identifier(sym) = collection {
                variable_types
                    .get(sym)
                    .map(|s| s.split("|__hl:").next().unwrap_or(s.as_str()))
            } else {
                returned_map.as_deref()
            };
            // A map entry, `(key, value)`, read at a constant index: the key or the value itself, of its own type.
            if let (Some(t), Expr::Literal(Literal::Number(n))) = (known_type, index) {
                if let Some((component, ty)) = super::types::entry_component(t, *n) {
                    return if is_copy_type(ty) {
                        format!("{}.{}", coll_str, component)
                    } else {
                        format!("{}.{}.clone()", coll_str, component)
                    };
                }
            }
            // HETEROGENEOUS-TUPLE element: a CONSTANT `item N of t`, where `t` is a marked tuple variable
            // (`Vec<Value> /* logos_tuple: … */`), reads the boxed `Value` at position N and UNBOXES it to
            // that position's scalar type — so `upper(item 2 of t)` gets a `String`, not a `Value`. Only
            // fires for a positive constant index into a marked tuple; every other collection/index is
            // unchanged (a `Value` result still works for `Show`, which takes any `Display`).
            if let Expr::Literal(Literal::Number(n)) = index {
                if *n >= 1 {
                    let tuple_type = known_type.map(str::to_string).or_else(|| {
                        record_field_rust_type(collection, interner, variable_types, ecx.registry)
                    });
                    if let Some(unbox) = tuple_type
                        .as_deref()
                        .and_then(|t| super::types::logos_tuple_elem_type(t, *n))
                        .and_then(super::types::logos_value_unbox_call)
                    {
                        return format!(
                            "logicaffeine_data::LogosIndex::logos_get(&{}, {}i64).{}",
                            coll_str, n, unbox
                        );
                    }
                }
            }
            match known_type {
                // A bit-packed `Seq of Bool` (`Opt::BitPack`): the bit an index names, read unchecked where the
                // oracle proves the index in range.
                Some("LogosBits") => {
                    if oracle_proves_index(ecx, collection, index) {
                        let offset = proven_offset_str(index, ecx, variable_types, interner);
                        format!("unsafe {{ {}.get_unchecked({}) }}", coll_str, offset)
                    } else {
                        let at = logos_index_str(index, ecx, variable_types, interner);
                        format!("{}.item({})", coll_str, at)
                    }
                }
                Some(t)
                    if t.starts_with("LogosSeq") || t.starts_with("Vec") || t == "TextBytes" =>
                {
                    // `TextBytes` is the zero-copy `&[u8]` view of a text's bytes (Opt::TextBytesView).
                    // `item i of sb` reads a `u8` from the borrowed slice and casts it back to the
                    // `i64` the rest of the program expects — no `Vec<i64>`, no `.borrow()`.
                    let owner = if t.starts_with("LogosSeq") {
                        format!("{}.borrow()", coll_str)
                    } else {
                        coll_str.clone()
                    };
                    let copy = t == "TextBytes" || has_copy_element_type(t);
                    let read = indexed_read(&owner, collection, index, copy, ecx);
                    // A byte view and a narrowed (`Vec<i32>`) sequence widen to the i64 the rest of the
                    // program expects (lossless).
                    if item_read_widens(t) {
                        format!("(({}) as i64)", read)
                    } else {
                        read
                    }
                }
                Some(t) if t.starts_with("&[") || t.starts_with("&mut [") || t.starts_with('[') => {
                    // A slice (&[T] / &mut [T]) or an O3 fixed array [T; N].
                    let elem = if t.starts_with('[') {
                        t.trim_start_matches('[').split("; ").next().unwrap_or("_")
                    } else {
                        t.strip_prefix("&mut [")
                            .or_else(|| t.strip_prefix("&["))
                            .and_then(|s| s.strip_suffix(']'))
                            .unwrap_or("_")
                    };
                    let copy = is_copy_type(elem);
                    let read = indexed_read(&coll_str, collection, index, copy, ecx);
                    // A narrowed slice (`&[i32]`/`&mut [i32]`, from a hoisted
                    // `Vec<i32>`) sign-extends to i64 (lossless).
                    if item_read_widens(t) {
                        format!("(({}) as i64)", read)
                    } else {
                        read
                    }
                }
                // A key the map does not hold fails with the one sentence naming it. `.get` borrows the
                // key, so a Text key is not moved.
                Some(t) if is_logos_map_type(t) => {
                    if let Some(probe) = super::probe::read(collection, index) {
                        return probe;
                    }
                    let index_str = irecurse!(index);
                    format!(
                        "{{ let __key = &({}); logicaffeine_data::failure::found({}.get(__key), __key) }}",
                        index_str, coll_str
                    )
                }
                Some(t)
                    if t.starts_with("std::collections::HashMap")
                        || t.starts_with("HashMap")
                        || t.starts_with("rustc_hash::FxHashMap")
                        || t.starts_with("FxHashMap") =>
                {
                    let index_str = irecurse!(index);
                    format!(
                        "{{ let __key = &({}); logicaffeine_data::failure::found({}.get(__key).cloned(), __key) }}",
                        index_str, coll_str
                    )
                }
                Some("String") => {
                    let index_str = irecurse!(index);
                    format!("LogosIndex::logos_get(&{}, {})", coll_str, index_str)
                }
                // A collection no type reaches takes its key by value: a key that is not `Copy` goes in as a copy, so
                // the program still holds the one it reads again.
                _ => {
                    let index_str = irecurse!(index);
                    let key = match index {
                        Expr::Identifier(sym) if variable_types.get(sym).is_some_and(|t| !is_copy_type(t)) => {
                            format!("{index_str}.clone()")
                        }
                        _ => index_str,
                    };
                    format!("LogosIndex::logos_get(&{}, {})", coll_str, key)
                }
            }
        }

        // A slice is a new list of the items it names: held, returned, passed or stored, it is one. A reader that
        // only reads the items in place takes the view instead (`slice_view`).
        Expr::Slice {
            collection,
            start,
            end,
        } => {
            let view = slice_view(collection, start, end, ecx);
            if sliced_type(collection, ecx).as_deref() == Some("String") {
                view
            } else {
                format!("LogosSeq::from_vec({view}.to_vec())")
            }
        }

        Expr::Copy { expr: inner } => {
            // Special case: Copy of Slice → the slice's items `.to_vec()`, wrapped in LogosSeq
            if let Expr::Slice {
                collection,
                start,
                end,
            } = inner
            {
                let coll_str = recurse!(collection);
                let bound = |e: &Expr| {
                    super::rational::integer_operand(e, crate::semantics::collections::IntegerPlace::Slice, ecx)
                        .unwrap_or_else(|| recurse!(e))
                };
                let start_str = bound(start);
                let end_str = bound(end);
                let known_type = sliced_type(collection, ecx);
                let reference = !crate::semantics::collections::value_semantics_enabled();
                if reference && matches!(known_type.as_deref(), Some(t) if t.starts_with("LogosSeq")) {
                    format!(
                        "LogosSeq::from_vec(logicaffeine_data::deep_copy!({}.borrow().logos_slice({}, {}).to_vec()))",
                        coll_str, start_str, end_str
                    )
                } else if reference
                    && matches!(known_type.as_deref(), Some(t) if t.starts_with("&[") || t.starts_with("Vec<"))
                {
                    format!(
                        "LogosSeq::from_vec(logicaffeine_data::deep_copy!({}.logos_slice({}, {}).to_vec()))",
                        coll_str, start_str, end_str
                    )
                } else if reference {
                    format!(
                        "logicaffeine_data::deep_copy!({}.logos_slice({}, {}).to_vec())",
                        coll_str, start_str, end_str
                    )
                } else if matches!(known_type.as_deref(), Some(t) if t.starts_with("LogosSeq")) {
                    format!(
                        "LogosSeq::from_vec({}.borrow().logos_slice({}, {}).to_vec())",
                        coll_str, start_str, end_str
                    )
                } else if matches!(known_type.as_deref(), Some(t) if t.starts_with("&[") || t.starts_with("Vec<"))
                {
                    format!(
                        "LogosSeq::from_vec({}.logos_slice({}, {}).to_vec())",
                        coll_str, start_str, end_str
                    )
                } else {
                    format!(
                        "{}.logos_slice({}, {}).to_vec()",
                        coll_str, start_str, end_str
                    )
                }
            } else {
                // Check if the inner expression is a LogosSeq/LogosMap → deep_clone()
                let known_type = if let Expr::Identifier(sym) = inner {
                    variable_types
                        .get(sym)
                        .map(|s| s.split("|__hl:").next().unwrap_or(s.as_str()))
                } else {
                    None
                };
                let expr_str = recurse!(inner);
                // Under reference semantics no write copies on write, so the copy itself must share
                // nothing with what it copies, at any depth — as the tree-walker's `deep_clone` does.
                // `to_owned` first reads the value through a lent parameter's reference.
                let reference = !crate::semantics::collections::value_semantics_enabled();
                if reference && matches!(known_type, Some(t) if t.starts_with("Vec<") || t.starts_with("&[")) {
                    format!("LogosSeq::from_vec(logicaffeine_data::deep_copy!({}.to_vec()))", expr_str)
                } else if reference {
                    format!("logicaffeine_data::deep_copy!(({}).to_owned())", expr_str)
                } else if matches!(known_type, Some(t) if t.starts_with("Vec<")) {
                    // Slice variable stored as Vec<T> — wrap with LogosSeq::from_vec.
                    // One copy (.clone()) + zero-cost wrap, same as old pre-ref-semantics .to_vec().
                    format!("LogosSeq::from_vec({}.clone())", expr_str)
                } else if matches!(known_type, Some(t) if t.starts_with("&[")) {
                    format!("LogosSeq::from_vec({}.to_vec())", expr_str)
                } else if matches!(known_type, Some(t) if t.starts_with("LogosSeq") || t.starts_with("LogosMap"))
                {
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
        Expr::Length {
            collection: Expr::Identifier(sym),
        } if affine_array_trip(variable_types.get(sym)).is_some() => {
            format!(
                "(({}) as i64)",
                affine_array_trip(variable_types.get(sym)).unwrap()
            )
        }

        // AoS-interleaved member: its length is the backing's fixed row count.
        Expr::Length {
            collection: Expr::Identifier(sym),
        } if parse_aos_tag(variable_types.get(sym)).is_some() => {
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
            let coll_str = match collection {
                Expr::Slice { collection, start, end } => slice_view(collection, start, end, ecx),
                _ => recurse!(collection),
            };
            // Phase 43D: Collection length - cast to i64 for LOGOS integer semantics
            format!("({}.len() as i64)", coll_str)
        }

        Expr::Contains { collection, value } => {
            let coll_str = match collection {
                Expr::Slice { collection, start, end } => slice_view(collection, start, end, ecx),
                _ => recurse!(collection),
            };
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
            format!(
                "{}.union(&{}).cloned().collect::<Set<_>>()",
                left_str, right_str
            )
        }

        Expr::Intersection { left, right } => {
            let left_str = recurse!(left);
            let right_str = recurse!(right);
            format!(
                "{}.intersection(&{}).cloned().collect::<Set<_>>()",
                left_str, right_str
            )
        }

        // Phase 48: Sipping Protocol expressions
        Expr::ManifestOf { zone } => {
            let zone_str = recurse!(zone);
            format!(
                "logicaffeine_system::network::FileSipper::from_zone(&{}).manifest()",
                zone_str
            )
        }

        Expr::ChunkAt { index, zone } => {
            let zone_str = recurse!(zone);
            let index_str = irecurse!(index);
            // LOGOS uses 1-indexed, Rust uses 0-indexed
            format!("logicaffeine_system::network::FileSipper::from_zone(&{}).get_chunk(({} - 1) as usize)", zone_str, index_str)
        }

        Expr::List(ref items) => {
            // ★ A SEQUENCE LITERAL COPIES ITS ELEMENTS, and `vec![…]` MOVES whatever it names — so
            // a field read among its elements moves that field out of its record (`[c's env,
            // c's env]` could not build twice), and a variable named twice, or read again after
            // the literal, is used after its move. There is no callee signature to consult here,
            // so the test is the element's own type; when that is unknown the clone is owed
            // anyway, because ownership is not in doubt at this site the way it is at a call.
            // `give x` is the explicit move and stays one.
            //
            // `analysis/ownership.rs` has no `Expr::List` arm at all, so nothing upstream sees it.
            let item_strs: Vec<String> = items
                .iter()
                .map(|i| {
                    let emitted = recurse!(i);
                    let bare = match i {
                        Expr::Copy { expr } => *expr,
                        other => other,
                    };
                    match bare {
                        Expr::FieldAccess { .. } => {
                            match record_field_rust_type(
                                bare,
                                ecx.interner,
                                ecx.variable_types,
                                ecx.registry,
                            ) {
                                Some(ty) if !field_read_moves(&ty) => emitted,
                                _ => format!("{emitted}.clone()"),
                            }
                        }
                        Expr::Identifier(sym)
                            if identifier_read_moves(*sym, ecx.variable_types) =>
                        {
                            format!("{emitted}.clone()")
                        }
                        _ => emitted,
                    }
                })
                .collect();
            format!("LogosSeq::from_vec(vec![{}])", item_strs.join(", "))
        }

        Expr::Tuple(ref items) => {
            let item_strs: Vec<String> = items
                .iter()
                .map(|i| format!("Value::from({})", recurse!(i)))
                .collect();
            // A tuple is a `LogosTuple` (newtype over `Vec<Value>`, Derefs to it so indexing /
            // destructuring / returns are unchanged) — distinct from a Seq so `Show` renders parens.
            format!(
                "logicaffeine_data::LogosTuple(vec![{}])",
                item_strs.join(", ")
            )
        }

        Expr::Range { start, end } => {
            let (start_str, end_str) = range_bound_strs(start, end, |e| {
                super::rational::integer_operand(e, crate::semantics::collections::IntegerPlace::Range, ecx)
                    .unwrap_or_else(|| recurse!(e))
            });
            format!("({}..={})", start_str, end_str)
        }

        Expr::FieldAccess { object, field } => {
            let field_name = interner.resolve(*field);

            // Phase 52: Check if root object is synced - use .get().await
            let root_sym = get_root_identifier(object);
            if let Some(sym) = root_sym {
                if synced_vars.contains(&sym) {
                    let obj_name = interner.resolve(sym);
                    return format!(
                        "{}.get().await.{}",
                        obj_name,
                        crate::codegen::escape_rust_ident(field_name)
                    );
                }
            }

            // NAMED-record field access: a record is a positional `Vec<Value>` (no named Rust fields), so
            // `r's quotient` resolves the field NAME to its 1-based `Vec` position via the record's type
            // marker and reads it with `LogosIndex` — the same path `item N of r` takes — then UNBOXES the
            // boxed `Value` to the field's scalar type so `r's ratio * 2.0` (a `Real` field) type-checks.
            // Only fires when the object is a marked record variable, so a real struct still emits `.field`.
            if let Expr::Identifier(sym) = object {
                if let Some(ty) = variable_types.get(sym) {
                    if let Some(pos) = super::types::logos_record_field_index(ty, field_name) {
                        let unbox = super::types::logos_record_field_type(ty, field_name)
                            .and_then(super::types::logos_value_unbox_call);
                        let obj_str = recurse!(object);
                        let get = format!("LogosIndex::logos_get(&{}, {}i64)", obj_str, pos);
                        return match unbox {
                            Some(unbox) => format!("({}).{}", get, unbox),
                            None => get,
                        };
                    }
                }
            }

            let obj_str = recurse!(object);
            format!(
                "{}.{}",
                obj_str,
                crate::codegen::escape_rust_ident(field_name)
            )
        }

        Expr::New {
            type_name,
            type_args,
            init_fields,
        } => {
            // Sanitize a cross-module reference (`Geometry::Point`) into the same Rust
            // identifier the struct declaration uses (`Geometry__Point`); plain names
            // are unchanged, so single-module output stays byte-identical.
            let type_str = rust_type_ident(interner.resolve(*type_name));
            // The language holds nothing in a rational field a record is built without, which a compiled rational
            // cannot hold.
            if let Some(field) = super::rational::unset_rational_field(*type_name, init_fields, ecx) {
                return format!(
                    "compile_error!({:?})",
                    format!(
                        "a `{}` built without its field `{}`, a rational: the language holds nothing there, and a \
                         compiled rational holds no nothing",
                        interner.resolve(*type_name),
                        interner.resolve(field)
                    )
                );
            }
            if !init_fields.is_empty() {
                // Struct initialization with fields: Point { x: 10, y: 20, ..Default::default() }
                // Always add ..Default::default() to handle partial initialization (e.g., CRDT fields)
                let fields_str = init_fields.iter()
                    .map(|(name, value)| {
                        let field_name = crate::codegen::escape_rust_ident(interner.resolve(*name));
                        // A field of the exact extension holds the value model's value of what fills it.
                        if super::types::program_record_field_type(&type_str, *name)
                            .is_some_and(|t| t == super::tower::VALUE)
                        {
                            return format!("{}: {}", field_name, super::tower::value(value, ecx));
                        }
                        // A field that holds a value-or-nothing stores `nothing`, another whole, or a value present.
                        if let Some(code) = super::types::program_record_field_type(&type_str, *name)
                            .and_then(|t| super::absent::stored(value, &t, ecx))
                        {
                            return format!("{}: {}", field_name, code);
                        }
                        // An integer stored in a field that holds rationals is the rational it equals.
                        if let Some(code) = super::types::program_record_field_type(&type_str, *name)
                            .filter(|t| t.contains(super::rational::RATIONAL))
                            .and_then(|t| super::rational::value_as(value, &super::rational::named(&t), ecx))
                        {
                            return format!("{}: {}", field_name, code);
                        }
                        let exact_field = stores_logos_int(*type_name, *name);
                        let value_str = if exact_field {
                            exact_field_value(value, ecx)
                        } else {
                            recurse!(value)
                        };
                        // A struct OWNS its fields, so a borrowed-slice param (`&[T]`) assigned
                        // to an owned `LogosSeq<T>` field must be materialized — mirrors the
                        // Return-of-a-slice coercion in stmt.rs. Without it the generated Rust
                        // fails E0308 (expected `LogosSeq<T>`, found `&[T]`).
                        let slice_sym = match value {
                            Expr::Identifier(sym) => Some(*sym),
                            Expr::Copy { expr: inner } => {
                                if let Expr::Identifier(sym) = inner { Some(*sym) } else { None }
                            }
                            _ => None,
                        };
                        let borrowed_slice = slice_sym.is_some_and(|sym| holds_a_borrowed_slice(sym, variable_types));
                        // A struct OWNS its fields; a non-Copy VALUE identifier (struct/enum/String)
                        // moved in may still be read after the construction (json's `JEntry { key:
                        // kname }`, where kname is later used in the dup-key message). The Identifier
                        // arm bare-moves it and the statement-level clone does not reach inside a
                        // construction, so clone here — output-preserving (value semantics: a store
                        // copies; a currently-working move was a last-use, equivalent to clone+drop).
                        // ★ AND A FIELD READ IS NOT A CONSUMPTION. `a new Todo with … text t's text …`
                        // inside a loop moved `t.text` out of the row, so a sibling branch pushing
                        // `t` itself failed to build ("use of partially moved value"). That is how
                        // every "change one row, keep the others" loop is written — a toggle, an
                        // edit — and the two branches are mutually exclusive at RUN time, which
                        // rustc cannot know. Reading a field must leave the row whole.
                        // ★ A VARIABLE STORED IN A FIELD IS MOVED ONLY AT ITS LAST USE. The record owns
                        // what it holds, so a variable named here that the program reads again — a list
                        // pushed to after it was stored, a row the next iteration builds from — is
                        // copied in; one this statement uses for the last time, and names once, is moved
                        // for free. Liveness says which (`flow::moves_at_last_use`); where it does not
                        // reach, the store copies.
                        let field_read_of_non_copy =
                            record_field_rust_type(value, interner, variable_types, ecx.registry)
                                .is_some_and(|t| field_read_moves(&t));
                        let last_use = matches!(value, Expr::Identifier(s) if super::flow::moves_at_last_use(*s, 1));
                        let owned_binding = is_value_struct_identifier(value, variable_types)
                            || holds_collection(value, variable_types);
                        // ★ A `mutable` PARAMETER IS THE CALLER'S VALUE, LENT. Stored in a field it is copied, at its
                        // last use too: a list or map lent by sharing deeply (`lent_by_sharing`), since every later
                        // write the body makes lands in the caller's buffer; a value lent as `&mut` by value.
                        let lent_by_sharing_binding = matches!(value, Expr::Identifier(s) if lent_by_sharing(*s, variable_types));
                        let value_semantics_copy = field_read_of_non_copy
                            || (owned_binding && !last_use)
                            || copies_out_of_a_lend(value, variable_types);
                        // ★ WIDEN AT THE BOUNDARY. A field stored as `LogosInt` may be filled from
                        // an `i64` — a literal, a counter, a function that cannot overflow — and
                        // requiring both sides to have been PREDICTED the same is what made this
                        // whole area fragile: two analyses meeting on one value, one saying `i64`
                        // and the other `LogosInt`, and rustc rejecting well-formed generated code
                        // on a type nobody wrote.
                        //
                        // `LogosInt::from` is total in both directions of the only conversion that
                        // is safe: identity when the value is already a `LogosInt` (Rust's
                        // reflexive `From`), a widening when it is an `i64`. NARROWING is never
                        // emitted anywhere — `to_i64` answers an `Option`, so a value that
                        // genuinely exceeded `i64` would be silently replaced, trading a loud
                        // build failure for a wrong number.
                        if exact_field {
                            format!("{}: logicaffeine_data::LogosInt::from({})", field_name, value_str)
                        } else if lent_by_sharing_binding {
                            format!("{}: {}.deep_clone()", field_name, value_str)
                        } else if borrowed_slice {
                            format!("{}: LogosSeq::from_vec(({}).to_vec())", field_name, value_str)
                        } else if value_semantics_copy {
                            format!("{}: {}.clone()", field_name, value_str)
                        } else {
                            format!("{}: {}", field_name, value_str)
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                // The rest of the record — fields the construction leaves out (a CRDT field, a
                // defaulted one) — comes from `Default`. A construction naming every field needs
                // no `Default`, which a generic record (`Box<T>`) has only when `T` has one.
                let names_every_field = match ecx.registry.get(*type_name) {
                    Some(crate::analysis::registry::TypeDef::Struct { fields, .. }) => fields
                        .iter()
                        .all(|f| init_fields.iter().any(|(given, _)| *given == f.name)),
                    _ => false,
                };
                if names_every_field {
                    format!("{} {{ {} }}", type_str, fields_str)
                } else {
                    format!("{} {{ {}, ..Default::default() }}", type_str, fields_str)
                }
            } else if type_args.is_empty() {
                format!("{}::default()", type_str)
            } else {
                // Phase 34: Turbofish syntax for generic instantiation
                // Bug fix: Use codegen_type_expr to support nested types like Seq of (Seq of Int)
                let args_str = type_args
                    .iter()
                    .map(|t| codegen_type_expr(t, interner))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{}::<{}>::default()", type_str, args_str)
            }
        }

        Expr::NewVariant {
            enum_name,
            variant,
            fields,
        } => {
            // The RAW interned name keys the `boxed_fields` lookup (its keys are built
            // from the same raw name in detection.rs); the emitted `EnumName::Variant`
            // PATH must be the SANITIZED ident so a cross-module enum (`Colors::Filter`)
            // constructs as `Colors__Filter::Variant`. Plain names are unchanged.
            let enum_name_raw = interner.resolve(*enum_name);
            let enum_str = rust_type_ident(enum_name_raw);
            let enum_str = enum_str.as_str();
            let variant_str = interner.resolve(*variant);
            let fish = super::instantiation::for_expr(expr);
            let enum_str = format!("{enum_str}{fish}");
            let enum_str = enum_str.as_str();
            let exact = |field: Symbol| stores_logos_int(*variant, field);
            if fields.is_empty() {
                // Unit variant: Shape::Point
                format!("{}::{}", enum_str, variant_str)
            } else {
                // A variable named in a construction is cloned at every mention but its last, which
                // moves it when the statement allows (`flow::moves_at_last_use`): the statement names it
                // nowhere else, and either reassigns it — a list built by `Set l to a new Cons with …
                // tail l` never deep-copies `l` — or is its last use. Otherwise the last mention clones
                // too. The count spans the OUTERMOST construction's whole subtree, nested constructions
                // included, so an inner `Node` taking `leaf` twice and an outer one taking it again do
                // not both move it. Only a direct field decrements the count; a mention some other arm
                // emits (inside a list, a call) never does, so a later direct field keeps cloning — the
                // safe side.
                let outermost = CONSTRUCTION_USES.with(|u| u.borrow().is_none());
                if outermost {
                    let mut counts: HashMap<Symbol, (usize, usize)> = HashMap::new();
                    super::worklist::visit_idents(expr, &mut |s| {
                        let c = counts.entry(s).or_insert((0, 0));
                        c.0 += 1;
                        c.1 += 1;
                    });
                    CONSTRUCTION_USES.with(|u| *u.borrow_mut() = Some(counts));
                }

                // Struct variant: Shape::Circle { radius: 10 }. The variant is the one the program declares: the
                // registry this context carries answers, or the program's own declarations where it carries none (a
                // call statement's arguments), so a construction takes its declared shape wherever it is written.
                let declared_variant = match ecx.registry.variant_in(*enum_name, *variant) {
                    Some(_) => None,
                    None => super::types::program_variant(enum_name_raw, variant_str),
                };
                let variant_def = ecx.registry.variant_in(*enum_name, *variant).or(declared_variant.as_ref());
                let fields_str: Vec<(String, String)> = fields
                    .iter()
                    .map(|(field_name, value)| {
                        let name = interner.resolve(*field_name);

                        let binding = match value {
                            Expr::Identifier(sym) if clock_read(interner.resolve(*sym)).is_none() => Some(sym),
                            _ => None,
                        };
                        let val = if let Some(sym) = binding {
                            let base_name = if boxed_bindings.contains(sym) {
                                format!("(*{})", interner.resolve(*sym))
                            } else {
                                interner.resolve(*sym).to_string()
                            };
                            let movable = CONSTRUCTION_USES.with(|u| {
                                let mut uses = u.borrow_mut();
                                match uses.as_mut().and_then(|m| m.get_mut(sym)) {
                                    Some((named, left)) => {
                                        *left = left.saturating_sub(1);
                                        *left == 0 && super::flow::moves_at_last_use(*sym, *named)
                                    }
                                    None => false,
                                }
                            });
                            // A `mutable` parameter is the caller's value, lent: the variant stores a copy of it — a
                            // deep one of a list or map lent by sharing, whose buffer the caller keeps writing. A list
                            // the function only reads is a borrowed slice, copied into a list the variant owns.
                            let read = if lent_by_sharing(*sym, variable_types) {
                                format!("{}.deep_clone()", base_name)
                            } else if lent_in_place(*sym, variable_types) {
                                format!("(*{}).clone()", base_name)
                            } else if holds_a_borrowed_slice(*sym, variable_types) {
                                format!("LogosSeq::from_vec({}.to_vec())", base_name)
                            } else if movable {
                                base_name
                            } else {
                                format!("{}.clone()", base_name)
                            };
                            if exact(*field_name) {
                                format!("logicaffeine_data::LogosInt::from({read})")
                            } else {
                                read
                            }
                        } else if exact(*field_name) {
                            exact_field_value(value, ecx)
                        } else {
                            recurse!(value)
                        };

                        // A field stored in a box of its own is boxed here. Keyed by the RAW enum name —
                        // `boxed_fields` is collected from the same raw interned name. The enum's own
                        // declaration answers too, so a construction reached through an entry that carries
                        // the registry but not the collected set (a `Show`'s argument) still boxes what the
                        // declaration boxed.
                        let key = (
                            enum_name_raw.to_string(),
                            variant_str.to_string(),
                            name.to_string(),
                        );
                        let declared_boxed = variant_def
                            .and_then(|v| v.fields.iter().find(|f| f.name == *field_name))
                            .is_some_and(|f| {
                                super::types::field_is_boxed(&f.ty, enum_name_raw, interner)
                            });
                        let field = crate::codegen::escape_rust_ident(name);
                        if boxed_fields.contains(&key) || declared_boxed {
                            (field, format!("Box::new({})", val))
                        } else {
                            (field, val)
                        }
                    })
                    .collect();
                if outermost {
                    CONSTRUCTION_USES.with(|u| *u.borrow_mut() = None);
                }
                let shape = variant_def.map_or(super::types::VariantShape::InPlace, |v| {
                    super::types::VariantShape::of(v, enum_name_raw, interner)
                });
                shape.build(&format!("{}::{}", enum_str, variant_str), &fields_str)
            }
        }

        Expr::OptionSome { value } => {
            format!("Some({})", recurse!(value))
        }

        Expr::OptionNone => "None".to_string(),

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
                    let text = crate::codegen::stmt::rust_string_escape(interner.resolve(*sym));
                    format!("{{ let mut __s = String::with_capacity(({}) as usize); __s.push_str(\"{}\"); __s }}", cap_str, text)
                }
                // Collection Expr::New → Type::with_capacity(cap)
                Expr::New {
                    type_name,
                    type_args,
                    ..
                } => {
                    let type_str = interner.resolve(*type_name);
                    match type_str {
                        "Seq" | "List" | "Vec" => {
                            let elem = if !type_args.is_empty() {
                                codegen_type_expr(&type_args[0], interner)
                            } else {
                                "()".to_string()
                            };
                            format!(
                                "LogosSeq::<{}>::with_capacity(({}) as usize)",
                                elem, cap_str
                            )
                        }
                        "Map" | "HashMap" => {
                            let (k, v) = if type_args.len() >= 2 {
                                (
                                    codegen_type_expr(&type_args[0], interner),
                                    codegen_type_expr(&type_args[1], interner),
                                )
                            } else {
                                ("String".to_string(), "String".to_string())
                            };
                            format!(
                                "LogosMap::<{}, {}>::with_capacity(({}) as usize)",
                                k, v, cap_str
                            )
                        }
                        "Set" | "HashSet" => {
                            let elem = if !type_args.is_empty() {
                                codegen_type_expr(&type_args[0], interner)
                            } else {
                                "()".to_string()
                            };
                            format!("{{ let __s: Set<{}> = Set::with_capacity_and_hasher(({}) as usize, Default::default()); __s }}", elem, cap_str)
                        }
                        _ => recurse!(value), // Unknown type — ignore capacity
                    }
                }
                // Other expressions — ignore capacity hint
                _ => recurse!(value),
            }
        }

        Expr::Closure { params, body, .. } => {
            use crate::ast::stmt::ClosureBody;
            let _body_moves_nothing = super::flow::suspend();
            let _control = super::control::function();
            // A closure passed to a scalar exact-HOF (`apply(f, x)`) has its integer params typed
            // `LogosInt` so an overflowing body promotes to a bignum == the VM. `exact_closure` is set
            // ONLY on this closure's own codegen by the call-site arg loop; a `map`/`fold` closure
            // keeps its `i64` params (its HOF is seq-shaped, hence not exact).
            let exact = ecx.exact_closure;
            let params_str: Vec<String> = params
                .iter()
                .map(|(name, ty)| {
                    let param_name = names.ident(*name);
                    let param_type = codegen_type_expr(ty, interner);
                    let param_type = if exact && param_type == "i64" {
                        "logicaffeine_data::LogosInt".to_string()
                    } else {
                        param_type
                    };
                    format!("{}: {}", param_name, param_type)
                })
                .collect();
            // A closure's parameters SHADOW whatever the enclosing scope knew by the same names:
            // the scope's facts are keyed by symbol, and two closures in one function commonly
            // reuse `a`/`x` — a `(a: Text, x: Text)` closure must not make another closure's
            // `(a: Int, x: Int) -> a + x` read as text concatenation.
            let mut scope_strings = ecx.string_vars.clone();
            let mut scope_types = ecx.variable_types.clone();
            for (name, ty) in params.iter() {
                let declared = codegen_type_expr(ty, interner);
                scope_strings.remove(name);
                if declared == "String" {
                    scope_strings.insert(*name);
                }
                if scope_types.contains_key(name) {
                    scope_types.insert(*name, declared);
                }
            }
            let ecx = &ExprCtx {
                string_vars: &scope_strings,
                variable_types: &scope_types,
                ..*ecx
            };
            // The exactified (integer) params, registered as promoted (`__bigint`) so the body's
            // arithmetic (`n * 2`) routes through the exact helper and stays `LogosInt`.
            let exact_param_types: HashMap<Symbol, String> = if exact {
                let mut vt = ecx.variable_types.clone();
                for (name, ty) in params.iter() {
                    if codegen_type_expr(ty, interner) == "i64" {
                        vt.insert(*name, "i64|__bigint".to_string());
                    }
                }
                vt
            } else {
                HashMap::new()
            };

            match body {
                ClosureBody::Expression(expr) => {
                    let body_str = if exact {
                        // Exact + tolerant: the promoted operand chain stays `LogosInt` (no narrow),
                        // matching the closure's `-> LogosInt` return; reset `exact_closure` so a
                        // nested closure does not wrongly inherit it.
                        let bctx = ExprCtx {
                            variable_types: &exact_param_types,
                            int_exact_tolerant: true,
                            exact_closure: false,
                            ..*ecx
                        };
                        codegen_expr_ctx(expr, &bctx)
                    } else {
                        // The body is a fresh expression root behind an `i64`-shaped boundary:
                        // an enclosing tolerant chain must not leak in, or the body would return
                        // an un-narrowed `LogosInt` from an `i64` closure.
                        let bctx = ExprCtx {
                            int_exact_tolerant: false,
                            exact_closure: false,
                            ..*ecx
                        };
                        codegen_expr_ctx(expr, &bctx)
                    };
                    format!("move |{}| {{ {} }}", params_str.join(", "), body_str)
                }
                ClosureBody::Block(stmts) => {
                    let mut body_str = String::new();
                    let mut ctx = RefinementContext::new();
                    // Exact-HOF closure: register the integer params as promoted (`__bigint`) so a
                    // statement-body's arithmetic is exact, matching the Expression path.
                    if exact {
                        ctx.set_returns_bigint(true);
                        for (name, ty) in params.iter() {
                            if codegen_type_expr(ty, interner) == "i64" {
                                ctx.register_variable_type(*name, "i64|__bigint".to_string());
                            }
                        }
                    }
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
                            stmt,
                            interner,
                            2,
                            &empty_mutable,
                            &mut ctx,
                            &empty_lww,
                            &empty_mv,
                            &mut empty_synced,
                            &empty_caps,
                            async_functions,
                            &empty_pipes,
                            &empty_boxed,
                            &empty_registry,
                            &type_env,
                        ));
                    }
                    format!(
                        "move |{}| {{\n{}{}}}",
                        params_str.join(", "),
                        body_str,
                        "    "
                    )
                }
            }
        }

        Expr::CallExpr { callee, args } => {
            let callee_str = recurse!(callee);
            let args_str: Vec<String> = args.iter().map(|a| recurse!(a)).collect();
            format!("({})({})", callee_str, args_str.join(", "))
        }

        Expr::InterpolatedString(parts) => codegen_interpolated_string(parts, ecx),

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
    let mut holes: Vec<&Expr> = Vec::new();

    for part in parts {
        match part {
            StringPart::Literal(sym) => {
                let text = interner.resolve(*sym);
                // Escape braces and special chars in the format string. `"` and `\` MUST be
                // escaped too — the literal segments sit inside the `format!("…")` string literal,
                // so an unescaped quote (e.g. from `"key \"{k}\""`) closes the Rust string early.
                // Mirrors the no-holes `String::from` path below.
                for ch in text.chars() {
                    match ch {
                        '{' => fmt_str.push_str("{{"),
                        '}' => fmt_str.push_str("}}"),
                        '\n' => fmt_str.push_str("\\n"),
                        '\t' => fmt_str.push_str("\\t"),
                        '\r' => fmt_str.push_str("\\r"),
                        '"' => fmt_str.push_str("\\\""),
                        '\\' => fmt_str.push_str("\\\\"),
                        _ => fmt_str.push(ch),
                    }
                }
            }
            StringPart::Expr {
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
                let arg_str = codegen_expr_ctx(value, ecx);
                let (piece, arg) =
                    interpolation_hole(format_spec.map(|s| interner.resolve(s)), &arg_str);
                fmt_str.push_str(&piece);
                args.push(arg);
                holes.push(value);
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
        format!("::std::string::String::from(\"{}\")", raw)
    } else {
        format_in_order(&fmt_str, &holes, args)
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
            format!(
                "::std::string::String::from(\"{}\")",
                crate::codegen::stmt::rust_string_escape(interner.resolve(*sym))
            )
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
        Literal::Duration(nanos) => format!("LogosDuration({}_i64)", nanos),
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
            let args_str: Vec<String> = args.iter().map(|a| codegen_term(a, interner)).collect();
            format!("{}({})", interner.resolve(*name), args_str.join(", "))
        }
        Term::Possessed {
            possessor,
            possessed,
        } => {
            let poss_str = codegen_term(possessor, interner);
            format!("{}.{}", poss_str, interner.resolve(*possessed))
        }
        Term::Group(members) => {
            let members_str: Vec<String> =
                members.iter().map(|m| codegen_term(m, interner)).collect();
            format!("({})", members_str.join(", "))
        }
        _ => "/* unsupported Term */".to_string(),
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen/expr/tests.rs"]
mod tests;
