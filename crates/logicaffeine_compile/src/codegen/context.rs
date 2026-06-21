use std::collections::{HashMap, HashSet};
use std::fmt::Write;

use crate::ast::logic::LogicExpr;
use crate::ast::stmt::Stmt;
use crate::intern::{Interner, Symbol};

use super::{codegen_assertion, codegen_expr};

// =============================================================================
// Refinement Type Enforcement
// =============================================================================

/// Tracks refinement type constraints across scopes for mutation enforcement.
///
/// When a variable with a refinement type is defined, its constraint is registered
/// in the current scope. When that variable is mutated via `Set`, the assertion is
/// re-emitted to ensure the invariant is preserved.
///
/// # Scope Management
///
/// The context maintains a stack of scopes to handle nested blocks:
///
/// ```text
/// ┌─────────────────────────────┐
/// │ Global Scope               │ ← x: { it > 0 }
/// │  ┌──────────────────────┐  │
/// │  │ Zone Scope           │  │ ← y: { it < 100 }
/// │  │  ┌────────────────┐  │  │
/// │  │  │ If Block Scope │  │  │ ← z: { it != 0 }
/// │  │  └────────────────┘  │  │
/// │  └──────────────────────┘  │
/// └─────────────────────────────┘
/// ```
///
/// # Variable Type Tracking
///
/// The context also tracks variable types for capability resolution. This allows
/// statements like `Check that user can publish the document` to resolve "the document"
/// to a variable named `doc` of type `Document`.
pub struct RefinementContext<'a> {
    /// Stack of scopes. Each scope maps variable Symbol to (bound_var, predicate).
    scopes: Vec<HashMap<Symbol, (Symbol, &'a LogicExpr<'a>)>>,

    /// Maps variable name Symbol to Rust type name (for capability resolution and optimization).
    variable_types: HashMap<Symbol, String>,

    /// Stack of scopes tracking which bindings came from boxed enum fields.
    /// When these are used in expressions, they need to be dereferenced with `*`.
    boxed_binding_scopes: Vec<HashSet<Symbol>>,

    /// Tracks variables that are known to be String type.
    /// Used for proper string concatenation codegen (format! vs +).
    string_vars: HashSet<Symbol>,

    /// Maps function Symbol to its return type string.
    /// Used to infer variable types from function call results.
    fn_returns: HashMap<Symbol, String>,

    /// Variables live immediately after the current top-level statement.
    ///
    /// `None` = no liveness information available → OPT-1C must conservatively clone.
    /// `Some(set)` = liveness computed; variables NOT in `set` are dead after this statement.
    ///
    /// Set by the caller of `codegen_stmt` before each top-level statement in a function body.
    /// Consumed (cleared to `None`) at the start of `codegen_stmt` so that recursive calls for
    /// nested blocks conservatively clone.
    live_vars_after: Option<HashSet<Symbol>>,

    /// Collection variables that escape the function (passed to calls, returned).
    /// Variables NOT in this set can use Vec<T> instead of LogosSeq<T> for zero-overhead indexing.
    escaping_vars: HashSet<Symbol>,

    /// Oracle facts computed on the SAME statement slice codegen walks, so
    /// the pointer-keyed loop alias snapshots match. Borrow hoisting reads
    /// the distinctness queries from here; `None` (tests, unset) hoists
    /// nothing.
    oracle: Option<std::rc::Rc<crate::optimize::OracleFacts>>,

    /// O3 scalarization: next fill index for each `[T; N]`-scalarized Seq.
    /// Incremented as the init pushes are emitted (`x[0] = …`, `x[1] = …`).
    array_fill_pos: HashMap<Symbol, usize>,


    /// O2 de-Rc: Seq variables proven to never need reference semantics, so
    /// they are emitted as a plain `Vec<T>` (no Rc/RefCell). The Let arm flips
    /// the declaration; access sites then dispatch on the `Vec<T>` type.
    de_rc_vars: HashSet<Symbol>,
    /// Phase 4: the function being codegen'd returns an owned `Vec<T>` (not
    /// `LogosSeq<T>`), so a `Return` of a borrowed-slice param emits `.to_vec()`
    /// without the `LogosSeq::from_vec(...)` wrapper.
    returns_vec: bool,

    /// Wave-2 A1 buffer-fill conversion: reused buffers (the inner partner of a
    /// `Set outer to inner` ping-pong swap) that are refilled by a COUNTED push
    /// loop. They are emitted as a SIZED `Vec` (`.resize` per round, not
    /// `Vec::new()`+`.clear()`), and the loop's `Push v to buf` becomes an
    /// INDEXED WRITE `buf[counter] = v` — removing the per-iteration `len`
    /// mutation that blocks vectorization of the DP scan.
    buffer_reuse_fill: HashSet<Symbol>,
    /// Loop-local scratch buffers whose per-iteration allocation has been
    /// hoisted out of the enclosing `while`: the buffer is declared once
    /// (`Vec::new()`) before the loop, and the per-iteration full-copy fill
    /// (`buf = src[..].to_vec()`) is lowered to `buf.clear(); buf.extend_from_slice(&src[..])`
    /// — reusing the allocation instead of mallocing/freeing each iteration.
    /// Populated by the `Stmt::While` handler after `detect_scratch_hoist_in_body`
    /// proves the buffer is de-Rc'd (uniquely owned), declared in the body, and
    /// non-escaping; cleared when the loop closes.
    scratch_hoist: HashSet<Symbol>,
    /// Active fill loop: `(buffer, counter_name)` while emitting the body of a
    /// counted loop that refills a `buffer_reuse_fill` buffer. The `Push`
    /// codegen reads this to emit `buffer[counter] = v` instead of `.push(v)`.
    fill_loop: Option<(Symbol, String)>,
    /// Strings built by cursor-lockstep appends in a counted loop (string_search's
    /// `text`): declared as a pre-zeroed byte buffer and written at the cursor
    /// (`*text.as_mut_vec().get_unchecked_mut(cursor) = b`) instead of `push`,
    /// then truncated to the cursor after the loop. Keyed by the string symbol →
    /// the cursor variable name. See `codegen/peephole.rs::try_emit_indexed_string_build`.
    indexed_string_builds: HashMap<Symbol, String>,
    /// Append-only worklists (BFS/DFS frontiers) proven bounded by a monotone
    /// visited-set guard: emitted as a pre-sized buffer + a register tail
    /// (`q[tail]=x; tail+=1`) instead of `Vec::push` — C's exact frontier code.
    /// Keyed by the worklist symbol; see `codegen/worklist.rs`.
    worklists: HashMap<Symbol, super::worklist::WorklistInfo>,
    /// Affine read-only arrays (CSR offset arrays): a Seq built `push f(i) to A`
    /// with affine `f`, IV from 0 step 1, never mutated after. Deleted entirely;
    /// every `item k of A` becomes `coeff*(k-1)+offset` and `length of A` the
    /// trip count. Keyed by the array symbol; see `codegen/affine_array.rs`.
    affine_arrays: HashMap<Symbol, super::affine_array::AffineArrayInfo>,
    /// `Seq of Int` sequences proven to hold only `i32`-range values → stored as
    /// `Vec<i32>` (half the footprint). Keyed by the sequence symbol; carries any
    /// runtime guard (`% m` divisor bound). See `codegen/narrow.rs`. Gated by
    /// `LOGOS_NARROW`, so empty unless that flag is set.
    narrowed: HashMap<Symbol, super::narrow::NarrowInfo>,

    /// Non-aliased local `Map of Int to Int` variables proven safe to lower to
    /// the specialized open-addressing `LogosI64Map` (no `Rc<RefCell>`, no
    /// clone) instead of `LogosMap<i64, i64>`; see `codegen/i64_map.rs`.
    i64_maps: HashSet<Symbol>,
    /// The subset of `i64_maps` whose value is never read (`contains` + `insert`
    /// only) — lowered to the keys-only `LogosI64Set` instead of `LogosI64Map`.
    i64_sets: HashSet<Symbol>,
    /// The subset of `i64_maps` whose key domain is PROVEN bounded within the
    /// map's `with capacity` hint → lowered to a direct-addressed flat array
    /// (`LogosDenseI64Map`/`…NoPresence`/`LogosDenseI64Set`). Carries the proven
    /// window offset `lo` and which representation to emit; see the dense gate in
    /// `codegen/i64_map.rs`. Empty unless that gate fires (and it can be forced off
    /// with `LOGOS_DENSE_MAP=0`).
    dense_i64: HashMap<Symbol, super::i64_map::DenseMapInfo>,
    /// Non-dense `Map of Int to Int` locals whose keys+values provably fit i32 →
    /// lowered to the half-width `LogosI32Map` / quarter-width `LogosI32Set`. A
    /// memory-traffic fallback for hash maps the dense gate cannot capture; empty
    /// unless the narrowing gate fires (forced off with `LOGOS_NARROW_MAP=0`).
    i32_maps: HashSet<Symbol>,
    i32_sets: HashSet<Symbol>,
    /// Push-built de-Rc Vecs that should be declared `Vec::with_capacity(cap)`
    /// rather than `Vec::new()` — keyed `sym -> capacity expr` — because a later
    /// counted loop index-reads them up to a proven bound (so growth reallocs
    /// are pure overhead C avoids by sizing the buffer exactly).
    vec_presize: HashMap<Symbol, String>,
    /// Loop-invariant positive divisors lowered to a precomputed `LogosDivU64`
    /// magic multiply — keyed `divisor sym -> helper variable name`. The `% n` /
    /// `/ n` sites read this to emit `helper.rem(..)` / `helper.div(..)` instead
    /// of a hardware division (O9 libdivide).
    fast_div: HashMap<Symbol, String>,
}

impl<'a> RefinementContext<'a> {
    pub fn new() -> Self {
        Self {
            scopes: vec![HashMap::new()],
            variable_types: HashMap::new(),
            boxed_binding_scopes: vec![HashSet::new()],
            string_vars: HashSet::new(),
            live_vars_after: None,
            escaping_vars: HashSet::new(),
            fn_returns: HashMap::new(),
            oracle: None,
            array_fill_pos: HashMap::new(),
            de_rc_vars: HashSet::new(),
            returns_vec: false,
            buffer_reuse_fill: HashSet::new(),
            scratch_hoist: HashSet::new(),
            fill_loop: None,
            indexed_string_builds: HashMap::new(),
            worklists: HashMap::new(),
            affine_arrays: HashMap::new(),
            narrowed: HashMap::new(),
            i64_maps: HashSet::new(),
            i64_sets: HashSet::new(),
            dense_i64: HashMap::new(),
            i32_maps: HashSet::new(),
            i32_sets: HashSet::new(),
            vec_presize: HashMap::new(),
            fast_div: HashMap::new(),
        }
    }

    /// Create a RefinementContext seeded from a TypeEnv.
    pub fn from_type_env(type_env: &crate::analysis::types::TypeEnv) -> Self {
        Self {
            scopes: vec![HashMap::new()],
            variable_types: type_env.to_legacy_variable_types(),
            boxed_binding_scopes: vec![HashSet::new()],
            string_vars: type_env.to_legacy_string_vars(),
            live_vars_after: None,
            escaping_vars: HashSet::new(),
            fn_returns: HashMap::new(),
            oracle: None,
            array_fill_pos: HashMap::new(),
            de_rc_vars: HashSet::new(),
            returns_vec: false,
            buffer_reuse_fill: HashSet::new(),
            scratch_hoist: HashSet::new(),
            fill_loop: None,
            indexed_string_builds: HashMap::new(),
            worklists: HashMap::new(),
            affine_arrays: HashMap::new(),
            narrowed: HashMap::new(),
            i64_maps: HashSet::new(),
            i64_sets: HashSet::new(),
            dense_i64: HashMap::new(),
            i32_maps: HashSet::new(),
            i32_sets: HashSet::new(),
            vec_presize: HashMap::new(),
            fast_div: HashMap::new(),
        }
    }

    /// Attach the oracle fact table for borrow-hoist distinctness queries.
    pub fn set_oracle(&mut self, oracle: std::rc::Rc<crate::optimize::OracleFacts>) {
        self.oracle = Some(oracle);
    }

    /// The attached oracle facts, if any.
    pub fn oracle(&self) -> Option<&crate::optimize::OracleFacts> {
        self.oracle.as_deref()
    }

    /// Begin tracking a scalarized `[T; N]` array's fill position at 0.
    pub(super) fn init_array_fill(&mut self, sym: Symbol) {
        self.array_fill_pos.insert(sym, 0);
    }

    /// Return the next fill index for a scalarized array and advance it.
    pub(super) fn next_array_fill(&mut self, sym: Symbol) -> usize {
        let slot = self.array_fill_pos.entry(sym).or_insert(0);
        let k = *slot;
        *slot += 1;
        k
    }

    /// Is this symbol a scalarized `[T; N]` array (registered with a `[` type)?
    pub(super) fn is_scalarized_array(&self, sym: Symbol) -> bool {
        self.variable_types
            .get(&sym)
            .map_or(false, |t| t.starts_with('['))
    }

    /// A1: mark a reused buffer to be filled by indexed write (not push).
    pub(super) fn register_buffer_reuse_fill(&mut self, sym: Symbol) {
        self.buffer_reuse_fill.insert(sym);
    }

    /// A1: is this buffer one whose counted push-refill becomes indexed writes?
    pub(super) fn is_buffer_reuse_fill(&self, sym: Symbol) -> bool {
        self.buffer_reuse_fill.contains(&sym)
    }

    /// Scratch-hoist: mark a loop-local buffer whose allocation was hoisted
    /// before the enclosing `while`. Its per-iteration full-copy fill is then
    /// lowered to `clear()` + `extend_from_slice` (reuse, not realloc).
    pub(super) fn register_scratch_hoist(&mut self, sym: Symbol) {
        self.scratch_hoist.insert(sym);
    }

    /// Scratch-hoist: is this buffer's allocation hoisted (refill via reuse)?
    pub(super) fn is_scratch_hoist(&self, sym: Symbol) -> bool {
        self.scratch_hoist.contains(&sym)
    }

    /// Scratch-hoist: stop treating `sym` as hoisted once its loop closes, so a
    /// same-named buffer in a sibling loop is not mis-rewritten.
    pub(super) fn clear_scratch_hoist(&mut self, sym: Symbol) {
        self.scratch_hoist.remove(&sym);
    }

    /// A1: enter a fill loop — `Push v to buffer` inside it becomes
    /// `buffer[counter] = v`. `counter` is the (0-based) loop counter name.
    pub(super) fn set_fill_loop(&mut self, buffer: Symbol, counter: String) {
        self.fill_loop = Some((buffer, counter));
    }

    /// A1: the active fill loop `(buffer, counter_name)`, if any.
    pub(super) fn fill_loop(&self) -> Option<(Symbol, &str)> {
        self.fill_loop.as_ref().map(|(b, c)| (*b, c.as_str()))
    }

    /// A1: leave the current fill loop.
    pub(super) fn clear_fill_loop(&mut self) {
        self.fill_loop = None;
    }

    /// Mark `text` as a cursor-indexed string build with the given cursor var name:
    /// its `Set text to text + X` appends become `text[cursor] = …` writes.
    pub(super) fn register_indexed_string_build(&mut self, text: Symbol, cursor: String) {
        self.indexed_string_builds.insert(text, cursor);
    }

    /// The cursor variable name for a cursor-indexed string build, if `text` is one.
    pub(super) fn indexed_string_build(&self, text: Symbol) -> Option<&str> {
        self.indexed_string_builds.get(&text).map(String::as_str)
    }

    /// Record the recognized append-only worklists for this function body.
    pub(super) fn set_worklists(
        &mut self,
        w: HashMap<Symbol, super::worklist::WorklistInfo>,
    ) {
        self.worklists = w;
    }

    /// The worklist conversion for `sym` (pre-sized buffer + register tail), if
    /// it was recognized.
    pub(super) fn worklist(&self, sym: Symbol) -> Option<&super::worklist::WorklistInfo> {
        self.worklists.get(&sym)
    }

    /// Record the recognized affine read-only arrays for this function body.
    pub(super) fn set_affine_arrays(
        &mut self,
        a: HashMap<Symbol, super::affine_array::AffineArrayInfo>,
    ) {
        self.affine_arrays = a;
    }

    /// The affine-array scalarization for `sym` (deleted array + closed-form
    /// reads), if it was recognized.
    pub(super) fn affine_array(&self, sym: Symbol) -> Option<&super::affine_array::AffineArrayInfo> {
        self.affine_arrays.get(&sym)
    }

    /// Record the `Seq of Int` sequences narrowed to `Vec<i32>`.
    pub(super) fn set_narrowed(&mut self, n: HashMap<Symbol, super::narrow::NarrowInfo>) {
        self.narrowed = n;
    }

    /// Whether `sym` is stored as `Vec<i32>` (loads sign-extend, stores truncate).
    pub(super) fn is_narrowed(&self, sym: Symbol) -> bool {
        self.narrowed.contains_key(&sym)
    }

    /// The runtime guards `sym`'s narrowing depends on (asserted at its decl).
    pub(super) fn narrow_guards(&self, sym: Symbol) -> &[String] {
        self.narrowed.get(&sym).map(|i| i.guards.as_slice()).unwrap_or(&[])
    }

    /// Record the `Map of Int to Int` locals lowered to `LogosI64Map`.
    pub(super) fn set_i64_maps(&mut self, m: HashSet<Symbol>) {
        self.i64_maps = m;
    }

    /// Record the subset lowered to the keys-only `LogosI64Set` (value unread).
    pub(super) fn set_i64_sets(&mut self, s: HashSet<Symbol>) {
        self.i64_sets = s;
    }

    /// Is this Map variable lowered to the specialized `LogosI64Map`
    /// (open-addressing, no Rc/RefCell)?
    pub(super) fn is_i64_map(&self, sym: Symbol) -> bool {
        self.i64_maps.contains(&sym)
    }

    /// Is this Map variable lowered to the keys-only `LogosI64Set`?
    pub(super) fn is_i64_set(&self, sym: Symbol) -> bool {
        self.i64_sets.contains(&sym)
    }

    /// Record the `Map of Int to Int` locals proven dense (direct-addressed
    /// array), keyed `sym -> {lo, kind}`. These are a subset of `i64_maps`.
    pub(super) fn set_dense_i64(&mut self, m: HashMap<Symbol, super::i64_map::DenseMapInfo>) {
        self.dense_i64 = m;
    }

    /// Which dense representation this Map variable lowers to, if any.
    pub(super) fn dense_kind(&self, sym: Symbol) -> Option<super::i64_map::DenseKind> {
        self.dense_i64.get(&sym).map(|i| i.kind)
    }

    /// The proven dense window info (`lo`, `kind`) for this Map variable, if any.
    pub(super) fn dense_info(&self, sym: Symbol) -> Option<super::i64_map::DenseMapInfo> {
        self.dense_i64.get(&sym).copied()
    }

    /// Record the non-dense `Map of Int to Int` locals narrowed to i32 storage,
    /// split into value-read maps (`LogosI32Map`) and keys-only sets (`LogosI32Set`).
    pub(super) fn set_i32_maps(&mut self, maps: HashSet<Symbol>, sets: HashSet<Symbol>) {
        self.i32_maps = maps;
        self.i32_sets = sets;
    }

    /// Is this Map variable lowered to the i32-narrowed `LogosI32Map`?
    pub(super) fn is_i32_map(&self, sym: Symbol) -> bool {
        self.i32_maps.contains(&sym)
    }

    /// Is this Map variable lowered to the i32-narrowed keys-only `LogosI32Set`?
    pub(super) fn is_i32_set(&self, sym: Symbol) -> bool {
        self.i32_sets.contains(&sym)
    }

    /// Record the push-built Vecs to pre-size (`sym -> capacity expr`).
    pub(super) fn set_vec_presize(&mut self, m: HashMap<Symbol, String>) {
        self.vec_presize = m;
    }

    /// The `with_capacity` argument for a push-built de-Rc Vec, if it is
    /// index-read up to a proven bound (else `None` → plain `Vec::new()`).
    pub(super) fn get_vec_presize(&self, sym: Symbol) -> Option<&String> {
        self.vec_presize.get(&sym)
    }

    /// Record the loop-invariant positive divisors to lower to `LogosDivU64`
    /// (`divisor sym -> helper variable name`).
    pub(super) fn set_fast_div(&mut self, m: HashMap<Symbol, String>) {
        self.fast_div = m;
    }

    /// The whole `divisor sym -> helper name` map, threaded into expression
    /// codegen so each `% n` / `/ n` site can emit the magic multiply.
    pub(super) fn get_fast_div(&self) -> &HashMap<Symbol, String> {
        &self.fast_div
    }

    /// Set the de-Rc'd Seq variables (emitted as plain `Vec<T>`).
    pub fn set_de_rc_vars(&mut self, vars: HashSet<Symbol>) {
        self.de_rc_vars = vars;
    }

    /// Phase 4: mark that the current function returns an owned `Vec<T>`.
    pub fn set_returns_vec(&mut self, v: bool) {
        self.returns_vec = v;
    }

    /// Does the current function return an owned `Vec<T>` (Phase 4)?
    pub(super) fn returns_vec(&self) -> bool {
        self.returns_vec
    }

    /// Is this Seq variable de-Rc'd to a plain `Vec<T>` (no Rc/RefCell)?
    pub(super) fn is_de_rc(&self, sym: Symbol) -> bool {
        self.de_rc_vars.contains(&sym)
    }

    /// Set the escaping vars for local Vec optimization.
    pub fn set_escaping_vars(&mut self, vars: HashSet<Symbol>) {
        self.escaping_vars = vars;
    }

    /// Check if a variable escapes (and thus must remain LogosSeq).
    pub fn var_escapes(&self, sym: &Symbol) -> bool {
        self.escaping_vars.contains(sym)
    }

    /// Register a function's return type.
    pub fn register_fn_return(&mut self, fn_sym: Symbol, return_type: String) {
        self.fn_returns.insert(fn_sym, return_type);
    }

    /// Get a function's return type.
    pub fn get_fn_return(&self, fn_sym: &Symbol) -> Option<&String> {
        self.fn_returns.get(fn_sym)
    }

    /// Set the live-after set for the next statement about to be generated.
    ///
    /// Must be called before each top-level `codegen_stmt` call in a function body.
    /// `codegen_stmt` will consume this (clearing it to `None`) so recursive nested calls
    /// conservatively clone.
    pub fn set_live_vars_after(&mut self, live: HashSet<Symbol>) {
        self.live_vars_after = Some(live);
    }

    /// Take (and clear) the live-after set.  Called once at the start of `codegen_stmt`.
    ///
    /// Returns `None` when no liveness information was provided (conservative path).
    pub fn take_live_vars_after(&mut self) -> Option<HashSet<Symbol>> {
        self.live_vars_after.take()
    }

    pub(super) fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
        self.boxed_binding_scopes.push(HashSet::new());
    }

    pub(super) fn pop_scope(&mut self) {
        self.scopes.pop();
        self.boxed_binding_scopes.pop();
    }

    /// Register a binding that came from a boxed enum field.
    /// These need `*` dereferencing when used in expressions.
    pub(super) fn register_boxed_binding(&mut self, var: Symbol) {
        if let Some(scope) = self.boxed_binding_scopes.last_mut() {
            scope.insert(var);
        }
    }

    /// Check if a variable is a boxed binding (needs dereferencing).
    pub(super) fn is_boxed_binding(&self, var: Symbol) -> bool {
        for scope in self.boxed_binding_scopes.iter().rev() {
            if scope.contains(&var) {
                return true;
            }
        }
        false
    }

    /// Register a variable as having String type.
    pub(super) fn register_string_var(&mut self, var: Symbol) {
        self.string_vars.insert(var);
    }

    /// Check if a variable is known to be a String.
    pub(super) fn is_string_var(&self, var: Symbol) -> bool {
        self.string_vars.contains(&var)
    }

    /// Get a reference to the string_vars set for expression codegen.
    pub(super) fn get_string_vars(&self) -> &HashSet<Symbol> {
        &self.string_vars
    }

    pub(super) fn register(&mut self, var: Symbol, bound_var: Symbol, predicate: &'a LogicExpr<'a>) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(var, (bound_var, predicate));
        }
    }

    pub(super) fn get_constraint(&self, var: Symbol) -> Option<(Symbol, &'a LogicExpr<'a>)> {
        for scope in self.scopes.iter().rev() {
            if let Some(entry) = scope.get(&var) {
                return Some(*entry);
            }
        }
        None
    }

    /// Register a variable with its type for capability resolution.
    pub(super) fn register_variable_type(&mut self, var: Symbol, type_name: String) {
        self.variable_types.insert(var, type_name);
    }

    /// Get variable type map for expression codegen optimization.
    pub(super) fn get_variable_types(&self) -> &HashMap<Symbol, String> {
        &self.variable_types
    }

    /// Get mutable variable type map for restoring types after hoisting scope.
    pub(super) fn get_variable_types_mut(&mut self) -> &mut HashMap<Symbol, String> {
        &mut self.variable_types
    }

    /// Find a variable name by its type (for resolving "the document" to "doc").
    pub(super) fn find_variable_by_type(&self, type_name: &str, interner: &Interner) -> Option<String> {
        let type_lower = type_name.to_lowercase();
        for (var_sym, var_type) in &self.variable_types {
            if var_type.to_lowercase() == type_lower {
                return Some(interner.resolve(*var_sym).to_string());
            }
        }
        None
    }
}

/// Emits a debug_assert for a refinement predicate, substituting the bound variable.
pub(super) fn emit_refinement_check(
    var_name: &str,
    bound_var: Symbol,
    predicate: &LogicExpr,
    interner: &Interner,
    indent_str: &str,
    output: &mut String,
) {
    let assertion = codegen_assertion(predicate, interner);
    let bound = interner.resolve(bound_var);
    let check = if bound == var_name {
        assertion
    } else {
        replace_word(&assertion, bound, var_name)
    };
    writeln!(output, "{}debug_assert!({});", indent_str, check).unwrap();
}

/// Word-boundary replacement to substitute bound variable with actual variable.
pub(super) fn replace_word(text: &str, from: &str, to: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut word = String::new();
    for c in text.chars() {
        if c.is_alphanumeric() || c == '_' {
            word.push(c);
        } else {
            if !word.is_empty() {
                result.push_str(if word == from { to } else { &word });
                word.clear();
            }
            result.push(c);
        }
    }
    if !word.is_empty() {
        result.push_str(if word == from { to } else { &word });
    }
    result
}

// =============================================================================
// Mount+Sync Detection for Distributed<T>
// =============================================================================

/// Tracks which variables have Mount and/or Sync statements.
///
/// This is used to detect when a variable needs `Distributed<T>` instead of
/// separate persistence and synchronization wrappers. A variable that is both
/// mounted and synced can use the unified `Distributed<T>` type.
///
/// # Detection Flow
///
/// ```text
/// Pre-scan all statements
///       ↓
/// Found "Mount x at path"  →  x.mounted = true, x.mount_path = Some(path)
/// Found "Sync x on topic"  →  x.synced = true, x.sync_topic = Some(topic)
///       ↓
/// If x.mounted && x.synced  →  Use Distributed<T> with both
/// ```
#[derive(Debug, Default)]
pub struct VariableCapabilities {
    /// Variable has a Mount statement (persistence).
    pub(super) mounted: bool,
    /// Variable has a Sync statement (network synchronization).
    pub(super) synced: bool,
    /// Path expression for Mount (as generated code string).
    pub(super) mount_path: Option<String>,
    /// Topic expression for Sync (as generated code string).
    pub(super) sync_topic: Option<String>,
}

/// Helper to create an empty VariableCapabilities map (for tests).
pub fn empty_var_caps() -> HashMap<Symbol, VariableCapabilities> {
    HashMap::new()
}

/// Pre-scan statements to detect variables that have both Mount and Sync.
/// Returns a map from variable Symbol to its capabilities.
pub(super) fn analyze_variable_capabilities<'a>(
    stmts: &[Stmt<'a>],
    interner: &Interner,
) -> HashMap<Symbol, VariableCapabilities> {
    let mut caps: HashMap<Symbol, VariableCapabilities> = HashMap::new();
    let empty_synced = HashSet::new();

    for stmt in stmts {
        match stmt {
            Stmt::Mount { var, path } => {
                let entry = caps.entry(*var).or_default();
                entry.mounted = true;
                entry.mount_path = Some(codegen_expr(path, interner, &empty_synced));
            }
            Stmt::Sync { var, topic } => {
                let entry = caps.entry(*var).or_default();
                entry.synced = true;
                entry.sync_topic = Some(codegen_expr(topic, interner, &empty_synced));
            }
            // Recursively check nested blocks (Block<'a> is &[Stmt<'a>])
            Stmt::If { then_block, else_block, .. } => {
                let nested = analyze_variable_capabilities(then_block, interner);
                for (var, cap) in nested {
                    let entry = caps.entry(var).or_default();
                    if cap.mounted { entry.mounted = true; entry.mount_path = cap.mount_path; }
                    if cap.synced { entry.synced = true; entry.sync_topic = cap.sync_topic; }
                }
                if let Some(else_b) = else_block {
                    let nested = analyze_variable_capabilities(else_b, interner);
                    for (var, cap) in nested {
                        let entry = caps.entry(var).or_default();
                        if cap.mounted { entry.mounted = true; entry.mount_path = cap.mount_path; }
                        if cap.synced { entry.synced = true; entry.sync_topic = cap.sync_topic; }
                    }
                }
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } => {
                let nested = analyze_variable_capabilities(body, interner);
                for (var, cap) in nested {
                    let entry = caps.entry(var).or_default();
                    if cap.mounted { entry.mounted = true; entry.mount_path = cap.mount_path; }
                    if cap.synced { entry.synced = true; entry.sync_topic = cap.sync_topic; }
                }
            }
            _ => {}
        }
    }

    caps
}
