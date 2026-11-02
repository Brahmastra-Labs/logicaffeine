//! Positive conversion for closed computations with a first-order data result.
//! A lazy closure machine executes beta/delta/iota/zeta and guarded Fix. Shared
//! term IDs retain original syntax; environments carry substitutions without
//! copying annotations or proof terms. Unsupported rules are inconclusive.
use super::{const_step, count_type_params, ConstStep};
use crate::term_store::{Node, NodeShape, TermId, TermStore};
use crate::{Context, Literal, Term};
use logicaffeine_base::Symbol;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::rc::Rc;

#[path = "certified_arithmetic.rs"]
mod certified_arithmetic;
use certified_arithmetic::Arithmetic;
#[path = "certified_division.rs"]
mod certified_division;
#[path = "certified_equality.rs"]
mod certified_equality;
use certified_division::Division;
#[path = "constructor_disagreement.rs"]
mod constructor_disagreement;
pub(crate) use constructor_disagreement::constructors_disagree;
#[path = "certified_index.rs"]
mod certified_index;
use certified_index::Lookup;

type Env = Option<Rc<Binding>>;
type Lazy = Rc<ThunkCell>;
#[derive(Clone, Copy)]
struct KeyObservation {
    revision: u64,
    depth: usize,
    budget: usize,
    key: usize,
    spent: usize,
    dependencies: u64,
    depth_used: usize,
    depth_limited: bool,
}
// Buckets are conservative invalidation filters, not semantic identities.
// Collisions can only cause an unnecessary recomputation of an exact key.
fn dependency_bucket(id: usize) -> usize {
    ((id as u64).wrapping_mul(0x9e3779b97f4a7c15) >> 58) as usize
}
fn dependency_mask(id: usize) -> u64 {
    1u64 << dependency_bucket(id)
}

struct ThunkCell {
    id: usize,
    key: Cell<Option<usize>>,
    observation: Cell<Option<KeyObservation>>,
    state: RefCell<Thunk>,
}
// Persistent compressed binary trie over the interner's exact 32-bit identity.
// Branch bits decrease along every path, bounding lookup and path copying.
enum Binding {
    Leaf { name: Symbol, value: Option<Lazy> },
    Branch { bit: usize, left: Env, right: Env },
}
enum Thunk {
    Suspended(TermId, Env),
    Running,
    // A speculative input: observing it aborts the entire positive attempt.
    Unknown,
    Done(Value),
}
// A last Rc may own a very deep graph even though evaluation itself is
// iterative. Detach outgoing edges before dropping each unique node; shared
// nodes retain their other owners and are visited when their last owner dies.
enum Edge {
    Thunk(Lazy),
    Environment(Rc<Binding>),
}
fn take_state_edges(state: Thunk, pending: &mut Vec<Edge>) {
    let (env, args) = match state {
        Thunk::Suspended(_, env) | Thunk::Done(Value::Lambda { env, .. }) => (env, Vec::new()),
        Thunk::Done(Value::Fix { env, args, .. }) => (env, args),
        Thunk::Done(Value::Constructor { args, .. }) => (None, args),
        Thunk::Done(Value::UnaryRun { tail, .. }) => (None, vec![tail]),
        _ => return,
    };
    pending.extend(args.into_iter().map(Edge::Thunk));
    if let Some(env) = env {
        pending.push(Edge::Environment(env));
    }
}
fn take_binding_edges(binding: &mut Binding, pending: &mut Vec<Edge>) {
    match binding {
        Binding::Leaf { value, .. } => {
            if let Some(value) = value.take() {
                pending.push(Edge::Thunk(value));
            }
        }
        Binding::Branch { left, right, .. } => {
            if let Some(left) = left.take() {
                pending.push(Edge::Environment(left));
            }
            if let Some(right) = right.take() {
                pending.push(Edge::Environment(right));
            }
        }
    }
}
fn release_edges(mut pending: Vec<Edge>) {
    while let Some(edge) = pending.pop() {
        match edge {
            Edge::Thunk(thunk) => {
                if let Ok(mut cell) = Rc::try_unwrap(thunk) {
                    let state = std::mem::replace(cell.state.get_mut(), Thunk::Running);
                    take_state_edges(state, &mut pending);
                    // cell's destructor now has no outgoing edges.
                }
            }
            Edge::Environment(env) => {
                if let Ok(mut binding) = Rc::try_unwrap(env) {
                    take_binding_edges(&mut binding, &mut pending);
                }
            }
        }
    }
}
impl Drop for ThunkCell {
    fn drop(&mut self) {
        let state = std::mem::replace(self.state.get_mut(), Thunk::Running);
        let mut pending = Vec::new();
        take_state_edges(state, &mut pending);
        release_edges(pending);
    }
}
impl Drop for Binding {
    fn drop(&mut self) {
        let mut pending = Vec::new();
        take_binding_edges(self, &mut pending);
        release_edges(pending);
    }
}
#[derive(Clone)]
enum Value {
    Lambda {
        param: Symbol,
        param_type: TermId,
        body: TermId,
        env: Env,
    },
    Fix {
        id: TermId,
        name: Symbol,
        body: TermId,
        env: Env,
        args: Vec<Lazy>,
    },
    Constructor {
        head: TermId,
        args: Vec<Lazy>,
    },
    // Exact repeated application of one full unary constructor. The final
    // argument remains a thunk, including when it is Unknown or unsupported.
    UnaryRun {
        head: TermId,
        count: usize,
        tail: Lazy,
    },
    Nat(crate::BigInt),
    Neutral,
}
#[derive(Clone)]
struct Constructor {
    index: usize,
    inventory: usize,
    parameters: usize,
    arity: usize,
    domains: Rc<[TermId]>,
}

// A comparison-only view: original values and exact readback keys are unchanged.
enum NatCursor {
    Pending(Lazy),
    Prefix {
        count: crate::BigInt,
        tail: Option<Lazy>,
    },
}

enum Control {
    Eval(TermId, Env),
    Compute(TermId, Env),
    Force(Lazy),
    Return(Value),
    NatOrder {
        left: NatCursor,
        right: NatCursor,
        answers: [TermId; 2],
    },
}
enum Argument {
    Expression(TermId, Env),
    Ready(Lazy),
}

enum Frame {
    IndexNumber {
        lookup: Lookup,
        total: crate::BigInt,
    },
    IndexCell {
        lookup: Lookup,
        index: crate::BigInt,
    },
    NatEqualityReverse {
        left: Lazy,
        right: Lazy,
        answers: [TermId; 2],
    },
    ArithmeticLeft {
        op: Arithmetic,
        right: Lazy,
        total: crate::BigInt,
    },
    DivisionPair {
        division: Division,
    },
    ArithmeticRight {
        op: Arithmetic,
        left: crate::BigInt,
        total: crate::BigInt,
    },
    NatOrderLeft {
        right: NatCursor,
        answers: [TermId; 2],
    },
    NatOrderRight {
        left: NatCursor,
        answers: [TermId; 2],
    },
    Update {
        thunk: Lazy,
        term: TermId,
        env: Env,
    },
    FinishUnary(Value),
    Apply(Argument),
    Match {
        cases: Box<[TermId]>,
        env: Env,
    },
    Unroll {
        id: TermId,
        name: Symbol,
        body: TermId,
        env: Env,
        args: Vec<Lazy>,
    },
}

#[derive(Clone, Hash, PartialEq, Eq)]
enum ValueKey {
    Atom(TermId),
    Constructor(TermId, Vec<usize>),
    UnaryRun(TermId, usize, usize),
    Suspended(TermId),
    Lambda(Symbol, TermId, Vec<usize>),
    Fix(TermId, Vec<usize>, Vec<usize>),
}

// Eviction is amortized over a generation instead of hashing/removing every old
// entry on every insertion. Both generations use exact keys. A hit in the older
// generation is retained in the newer one; updates cannot resurrect stale values.
struct Generations<K, V, const N: usize = 50_000> {
    recent: HashMap<K, V>,
    previous: HashMap<K, V>,
}
impl<K: Eq + Hash + Clone, V: Clone, const N: usize> Generations<K, V, N> {
    fn new() -> Self {
        assert!(N > 0);
        Self {
            recent: HashMap::new(),
            previous: HashMap::new(),
        }
    }
    fn get(&mut self, key: &K) -> Option<V> {
        if let Some(value) = self.recent.get(key) {
            return Some(value.clone());
        }
        let value = self.previous.get(key)?.clone();
        self.insert(key.clone(), value.clone());
        Some(value)
    }
    fn insert(&mut self, key: K, value: V) {
        if self.recent.len() == N {
            std::mem::swap(&mut self.recent, &mut self.previous);
            self.recent.clear();
        }
        self.recent.insert(key, value);
    }
    #[cfg(test)]
    fn len(&self) -> usize {
        self.recent.len() + self.previous.len()
    }
}

type HeadReducer<'a> = HeadReducerWithWork<'a, false>;

/// The evaluator's store bound: past it every run gives up as its allowance would.
const STORE_BOUND: usize = 500_000;

struct HeadReducerWithWork<'a, const TRACE: bool> {
    ctx: &'a Context,
    store: TermStore,
    globals: HashMap<TermId, Lazy>,
    constructors: HashMap<TermId, Option<Constructor>>,
    columns: HashMap<TermId, Option<usize>>,
    fuel: u64,
    work: Option<super::DagAtomWork>,
    canonical_nat: bool,
    preserve_nat_syntax: bool,
    nat_order_attestation: [Option<bool>; 2],
    certified_arithmetic: HashMap<TermId, Option<Arithmetic>>,
    arithmetic_attempts: usize,
    certified_divisions: HashMap<TermId, Option<Division>>,
    division_attempts: usize,
    certified_equalities: HashMap<TermId, Option<[TermId; 2]>>,
    equality_attempts: usize,
    certified_orders: HashMap<TermId, Option<[TermId; 2]>>,
    certified_indices: HashMap<TermId, Option<TermId>>,
    index_attempts: usize,
    index_spines: Generations<(TermId, usize), certified_index::Spine, 128>,
    normalize_unary_spines: bool,
    unknown: Option<Lazy>,
    proof_domains: HashMap<TermId, bool>,
    next_thunk: usize,
    key_revision: u64,
    key_mutations: [u64; 64],
    key_dependencies: u64,
    key_depth_used: usize,
    key_depth_limited: bool,
    closures: Generations<(TermId, Vec<usize>), Lazy>,
    values: Generations<ValueKey, usize>,
}

// This optimization uses only the existing Peano/literal bridge. A name alone
// cannot license it: alternate datatypes or constructor signatures stay opaque.
fn canonical_nat(ctx: &Context) -> bool {
    crate::int_inductive::canonical_nat_environment(ctx)
}

fn bind(env: Env, name: Symbol, value: Lazy) -> Env {
    let leaf = || {
        Rc::new(Binding::Leaf {
            name,
            value: Some(value.clone()),
        })
    };
    let Some(root) = env else {
        return Some(leaf());
    };
    let key = name.index();
    let mut current = root.clone();
    let previous = loop {
        match current.as_ref() {
            Binding::Leaf { name, .. } => break name.index(),
            Binding::Branch { bit, left, right } => {
                current = if key & bit == 0 { left } else { right }
                    .as_ref()
                    .expect("live environment branch")
                    .clone();
            }
        }
    };
    let difference = key ^ previous;
    // A zero bit means replacement of this exact key, preserving old roots.
    let split = if difference == 0 {
        0
    } else {
        1usize << (usize::BITS - 1 - difference.leading_zeros())
    };
    let mut path = Vec::new();
    let mut current = root;
    loop {
        match current.as_ref() {
            Binding::Branch { bit, left, right } if *bit > split => {
                let turn_right = key & bit != 0;
                let (child, sibling) = if turn_right {
                    (right, left)
                } else {
                    (left, right)
                };
                path.push((*bit, sibling.clone(), turn_right));
                current = child.as_ref().expect("live environment branch").clone();
            }
            _ => break,
        }
    }
    debug_assert!(path.len() <= 32);
    let mut result = leaf();
    if split != 0 {
        result = if key & split == 0 {
            Rc::new(Binding::Branch {
                bit: split,
                left: Some(result),
                right: Some(current),
            })
        } else {
            Rc::new(Binding::Branch {
                bit: split,
                left: Some(current),
                right: Some(result),
            })
        };
    }
    while let Some((bit, sibling, turn_right)) = path.pop() {
        result = if turn_right {
            Rc::new(Binding::Branch {
                bit,
                left: sibling,
                right: Some(result),
            })
        } else {
            Rc::new(Binding::Branch {
                bit,
                left: Some(result),
                right: sibling,
            })
        };
    }
    Some(result)
}
#[cfg(test)]
thread_local! { static ENV_LOOKUP_STEPS: Cell<usize> = const { Cell::new(0) }; }

fn lookup(env: &Env, name: Symbol) -> Option<Lazy> {
    let mut current = env.as_ref();
    while let Some(binding) = current {
        #[cfg(test)]
        ENV_LOOKUP_STEPS.with(|steps| steps.set(steps.get() + 1));
        match binding.as_ref() {
            Binding::Leaf { name: found, value } => {
                return (*found == name).then(|| value.clone()).flatten();
            }
            Binding::Branch { bit, left, right } => {
                current = if name.index() & bit == 0 {
                    left.as_ref()
                } else {
                    right.as_ref()
                };
            }
        }
    }
    None
}

impl<'a, const TRACE: bool> HeadReducerWithWork<'a, TRACE> {
    fn new(ctx: &'a Context, fuel: u64) -> Self {
        Self::with_nat_syntax(ctx, fuel, false)
    }

    fn with_nat_syntax(ctx: &'a Context, fuel: u64, preserve_nat_syntax: bool) -> Self {
        Self {
            ctx,
            store: TermStore::new(),
            globals: HashMap::new(),
            constructors: HashMap::new(),
            columns: HashMap::new(),
            fuel,
            work: TRACE.then(Default::default),
            canonical_nat: (!preserve_nat_syntax
                || ctx.constructor_inventory_upper_bound("Nat") <= 4096)
                && canonical_nat(ctx),
            preserve_nat_syntax,
            nat_order_attestation: [None, None],
            certified_orders: HashMap::new(),
            certified_indices: HashMap::new(),
            index_attempts: 64,
            index_spines: Generations::new(),
            certified_arithmetic: HashMap::new(),
            arithmetic_attempts: 64,
            certified_divisions: HashMap::new(),
            division_attempts: 16,
            certified_equalities: HashMap::new(),
            equality_attempts: 64,
            normalize_unary_spines: false,
            unknown: None,
            proof_domains: HashMap::new(),
            next_thunk: 0,
            key_revision: 0,
            key_mutations: [0; 64],
            key_dependencies: 0,
            key_depth_used: 0,
            key_depth_limited: false,
            closures: Generations::new(),
            values: Generations::new(),
        }
    }

    /// The allowance or the store bound is spent: every further run gives no answer.
    fn spent(&self) -> bool {
        self.fuel == 0 || self.store.len() > STORE_BOUND
    }

    fn ready_nat(&mut self, n: crate::BigInt) -> Option<Lazy> {
        if n < crate::BigInt::from_i64(0) {
            return None;
        }
        let id = self.store.mk_lit(&Literal::Nat(n.clone()));
        if let Some(value) = self.globals.get(&id) {
            return Some(value.clone());
        }
        let value = self.thunk(Thunk::Done(Value::Nat(n)))?;
        self.globals.insert(id, value.clone());
        Some(value)
    }

    fn compact_run(&self, head: TermId, mut count: usize, mut tail: Lazy) -> Value {
        // Never force a tail to compress it. Each merged head has already been
        // observed; the final thunk and its full meaning remain intact. Bound
        // each opportunistic pass just like the existing behavioral-key walk.
        for _ in 0..32 {
            let next = match &*tail.state.borrow() {
                Thunk::Done(Value::UnaryRun {
                    head: inner,
                    count: n,
                    tail: t,
                }) if *inner == head => count.checked_add(*n).map(|n| (n, t.clone())),
                Thunk::Done(Value::Constructor { head: inner, args })
                    if *inner == head && args.len() == 1 =>
                {
                    count.checked_add(1).map(|n| (n, args[0].clone()))
                }
                _ => None,
            };
            let Some((next_count, next_tail)) = next else {
                break;
            };
            count = next_count;
            tail = next_tail;
        }
        Value::UnaryRun { head, count, tail }
    }

    fn compact_value(&self, value: Value) -> Value {
        if let Value::UnaryRun { head, count, tail } = value {
            return self.compact_run(head, count, tail);
        }
        let Value::Constructor { head, args } = value else {
            return value;
        };
        let name = match self.store.node(head) {
            Node::Global(name) | Node::Const { name, .. } => self.store.name(*name),
            _ => return Value::Constructor { head, args },
        };
        if self.canonical_nat && !self.preserve_nat_syntax {
            if name == "Zero" && args.is_empty() {
                return Value::Nat(crate::BigInt::from_i64(0));
            }
            if name == "Succ" && args.len() == 1 {
                let number = match &*args[0].state.borrow() {
                    Thunk::Done(Value::Nat(n)) => Some(n.add(&crate::BigInt::from_i64(1))),
                    _ => None,
                };
                return match number {
                    Some(n) => Value::Nat(n),
                    None => Value::Constructor { head, args },
                };
            }
        }
        // Full arity includes every parameter. Restrict this representation to
        // constructors with exactly one field and no erased/type parameters.
        if args.len() == 1
            && self
                .constructors
                .get(&head)
                .and_then(Option::as_ref)
                .is_some_and(|c| c.arity == 1 && c.parameters == 0)
        {
            return self.compact_run(head, 1, args[0].clone());
        }
        Value::Constructor { head, args }
    }

    fn run_predecessor(&mut self, head: TermId, count: usize, tail: &Lazy) -> Option<Lazy> {
        let info = self.constructor(head)?;
        if info.arity != 1 || info.parameters != 0 || count == 0 {
            return None;
        }
        if count == 1 {
            Some(tail.clone())
        } else {
            self.thunk(Thunk::Done(Value::UnaryRun {
                head,
                count: count - 1,
                tail: tail.clone(),
            }))
        }
    }

    fn intern_value_key(&mut self, key: ValueKey, fallback: usize) -> usize {
        if let Some(id) = self.values.get(&key) {
            return id;
        }
        let id = self.next_thunk;
        let Some(next) = id.checked_add(1) else {
            return fallback;
        };
        self.next_thunk = next;
        // Semantic keys and thunk identities share a monotonic namespace.
        // Eviction never recycles an identity retained by a live closure.
        self.values.insert(key, id);
        id
    }

    fn thunk(&mut self, state: Thunk) -> Option<Lazy> {
        let id = self.next_thunk;
        self.next_thunk = id.checked_add(1)?;
        if TRACE {
            if let Some(work) = &mut self.work {
                work.thunk_allocations += 1;
            }
        }
        if TRACE {
            if let Some(work) = &mut self.work {
                if matches!(&state, Thunk::Suspended(..)) {
                    work.suspended_constructions += 1;
                }
                if let Thunk::Suspended(term, _) = &state {
                    if matches!(self.store.node(*term), Node::Lambda { .. }) {
                        work.suspended_lambda_constructions += 1;
                    }
                }
            }
        }
        Some(Rc::new(ThunkCell {
            id,
            key: Cell::new(None),
            observation: Cell::new(None),
            state: RefCell::new(state),
        }))
    }

    // The key includes exactly the lexical bindings this machine can observe.
    // IDs are monotonic for this immutable-context call, never recycled pointer
    // addresses. A bound thunk retains its own defining environment.
    fn delay(&mut self, term: TermId, env: &Env) -> Option<Lazy> {
        self.prepare_delay::<false>(term, env)
            .map(|(value, _)| value)
    }

    // Only a fresh cache miss may start Running. Existing Suspended, Done and
    // Running entries preserve their identity and follow the ordinary Force path.
    fn prepare_delay<const IMMEDIATE: bool>(
        &mut self,
        term: TermId,
        env: &Env,
    ) -> Option<(Lazy, bool)> {
        if let Node::Var(name) = self.store.node(term) {
            return lookup(env, *name).map(|value| (value, false));
        }
        if matches!(self.store.node(term), Node::Lit(_)) {
            if let Some(value) = self.globals.get(&term) {
                return Some((value.clone(), false));
            }
            if let Term::Lit(Literal::Nat(n)) = self.store.extract(term) {
                return self.ready_nat(n).map(|value| (value, false));
            }
        }
        if TRACE {
            if let Some(work) = &mut self.work {
                work.capture_key_entries += self.store.head_reduction_symbols(term).len() as u64;
            }
        }
        let values = self
            .store
            .head_reduction_symbols(term)
            .iter()
            .map(|name| lookup(env, *name))
            .collect::<Option<Vec<_>>>()?;
        // A captured thunk can move from an identity key to a compact
        // behavioral key as it finishes. Retain its physical-environment alias
        // so that this key improvement cannot discard the original computation.
        // Both aliases use the existing bounded cache; IDs are never recycled.
        let identity = (term, values.iter().map(|value| value.id).collect());
        // Captured thunk IDs are immutable and unique within this reducer.
        // Exact hits need no traversal of unsettled capture graphs. A running
        // entry still allows lookup of a completed semantic alias below.
        if let Some(value) = self.closures.get(&identity) {
            if !matches!(&*value.state.borrow(), Thunk::Running) {
                return Some((value, false));
            }
        }
        let mut remaining = 256;
        let bindings = values
            .iter()
            .map(|value| self.binding_key(value, 32, &mut remaining))
            .collect();
        let key = (term, bindings);
        if let Some(value) = self.closures.get(&key) {
            if identity != key {
                self.closures.insert(identity, value.clone());
            }
            return Some((value, false));
        }
        if identity != key {
            if let Some(value) = self.closures.get(&identity) {
                self.closures.insert(key, value.clone());
                return Some((value, false));
            }
        }
        let (state, fresh) = match self.store.node(term) {
            // A lambda is already a value. Capture its exact syntax and lexical
            // environment without observing its body or constructing a future
            // Compute/Update job. Memo admission above remains unchanged.
            Node::Lambda {
                param,
                param_type,
                body,
            } => (
                Thunk::Done(Value::Lambda {
                    param: *param,
                    param_type: *param_type,
                    body: *body,
                    env: env.clone(),
                }),
                false,
            ),
            _ if IMMEDIATE => (Thunk::Running, true),
            _ => (Thunk::Suspended(term, env.clone()), false),
        };
        let value = self.thunk(state)?;
        // Active computations own their thunks independently of cache eviction.
        if identity != key {
            self.closures.insert(identity, value.clone());
        }
        self.closures.insert(key, value.clone());
        Some((value, fresh))
    }

    // Exact semantic keys never erase a constructor field. Speculative unknowns
    // are represented explicitly and abort if demanded. A bounded traversal may
    // miss sharing, but cannot invent equality.
    fn environment_key(
        &mut self,
        term: TermId,
        env: &Env,
        bound: Option<Symbol>,
        depth: usize,
        remaining: &mut usize,
    ) -> Option<(Vec<usize>, bool)> {
        if TRACE {
            if let Some(work) = &mut self.work {
                work.capture_key_entries += self
                    .store
                    .head_reduction_symbols(term)
                    .iter()
                    .filter(|name| Some(**name) != bound)
                    .count() as u64;
            }
        }
        let values = self
            .store
            .head_reduction_symbols(term)
            .iter()
            .filter(|name| Some(**name) != bound)
            .map(|name| lookup(env, *name))
            .collect::<Option<Vec<_>>>()?;
        Some(self.argument_keys(&values, depth, remaining))
    }

    fn argument_keys(
        &mut self,
        values: &[Lazy],
        depth: usize,
        remaining: &mut usize,
    ) -> (Vec<usize>, bool) {
        let mut stable = true;
        let keys = values
            .iter()
            .map(|v| {
                let (key, done) = self.binding_key_state(v, depth, remaining);
                stable &= done;
                key
            })
            .collect();
        (keys, stable)
    }

    fn binding_key(&mut self, value: &Lazy, depth: usize, remaining: &mut usize) -> usize {
        self.binding_key_state(value, depth, remaining).0
    }

    fn key_changed(&mut self, value: &Lazy) {
        self.key_revision = self.key_revision.saturating_add(1);
        self.key_mutations[dependency_bucket(value.id)] = self.key_revision;
    }

    fn observation_current(&self, observation: KeyObservation) -> bool {
        if observation.revision == self.key_revision {
            return true;
        }
        if observation.revision > self.key_revision {
            return false;
        }
        let mut dependencies = observation.dependencies;
        while dependencies != 0 {
            let bucket = dependencies.trailing_zeros() as usize;
            if self.key_mutations[bucket] > observation.revision {
                return false;
            }
            dependencies &= dependencies - 1;
        }
        true
    }

    fn binding_key_state(
        &mut self,
        value: &Lazy,
        depth: usize,
        remaining: &mut usize,
    ) -> (usize, bool) {
        // Each nested observation owns its dependency set. Its caller inherits
        // that set even when the child is served directly from the cache.
        let outer = std::mem::replace(&mut self.key_dependencies, 0);
        let outer_depth = std::mem::replace(&mut self.key_depth_used, 0);
        let outer_limited = std::mem::replace(&mut self.key_depth_limited, false);
        let result = self.binding_key_observation(value, depth, remaining);
        self.key_dependencies |= outer;
        // A caller reaches this child through one edge. An already stable key
        // needs no traversal depth; a cached walk reports its actual height.
        self.key_depth_used = outer_depth.max(self.key_depth_used.saturating_add(1));
        self.key_depth_limited |= outer_limited;
        result
    }

    fn binding_key_observation(
        &mut self,
        value: &Lazy,
        depth: usize,
        remaining: &mut usize,
    ) -> (usize, bool) {
        // A stable key was derived only after all observed dependencies settled.
        // Recompacting that immutable value cannot improve its behavior; doing
        // so would discard the cache entry and repeat the completed traversal.
        if let Some(key) = value.key.get() {
            return (key, true);
        }
        let budget = *remaining;
        let revision = self.key_revision;
        if depth > 0 && budget > 0 && revision != u64::MAX {
            if let Some(observed) = value.observation.get().filter(|o| {
                self.observation_current(*o)
                    && (o.depth == depth || (!o.depth_limited && depth >= o.depth_used))
                    && (o.budget == budget || (o.spent < o.budget && budget >= o.spent))
            }) {
                self.key_dependencies |= observed.dependencies;
                self.key_depth_used = observed.depth_used;
                self.key_depth_limited |= observed.depth_limited;
                *remaining -= observed.spent;
                return (observed.key, false);
            }
        }
        // As delayed fields finish, already-observed constructor prefixes can
        // acquire a compact key. This remains behavioral sharing, not equality.
        let compact = match &*value.state.borrow() {
            Thunk::Done(v @ Value::UnaryRun { .. }) => Some(self.compact_value(v.clone())),
            Thunk::Done(v @ Value::Constructor { args, .. }) if args.len() == 1 => {
                Some(self.compact_value(v.clone()))
            }
            _ => None,
        };
        if let Some(compact) = compact {
            *value.state.borrow_mut() = Thunk::Done(compact);
            value.key.set(None);
        }
        if depth == 0 || *remaining == 0 {
            self.key_depth_limited |= depth == 0;
            // A later independently derived stable key can refine this cutoff.
            self.key_dependencies |= dependency_mask(value.id);
            return (value.id, false);
        }
        *remaining -= 1;
        self.key_depth_used = 1;
        if matches!(
            &*value.state.borrow(),
            Thunk::Suspended(..) | Thunk::Running
        ) {
            self.key_dependencies |= dependency_mask(value.id);
        }
        let (key, stable) = match &*value.state.borrow() {
            Thunk::Unknown | Thunk::Done(Value::Neutral) => (None, true),
            Thunk::Suspended(term, _) if self.store.head_reduction_symbols(*term).is_empty() => {
                (Some(ValueKey::Suspended(*term)), false)
            }
            Thunk::Done(Value::Nat(n)) => (
                Some(ValueKey::Atom(self.store.mk_lit(&Literal::Nat(n.clone())))),
                true,
            ),
            Thunk::Done(Value::Constructor { head, args }) if args.is_empty() => {
                (Some(ValueKey::Atom(*head)), true)
            }
            Thunk::Done(Value::Constructor { head, args }) => {
                let (keys, stable) = self.argument_keys(args, depth - 1, remaining);
                (Some(ValueKey::Constructor(*head, keys)), stable)
            }
            Thunk::Done(Value::UnaryRun { head, count, tail }) => {
                let (key, stable) = self.binding_key_state(tail, depth - 1, remaining);
                (Some(ValueKey::UnaryRun(*head, *count, key)), stable)
            }
            Thunk::Done(Value::Lambda {
                param, body, env, ..
            }) => match self.environment_key(*body, env, Some(*param), depth - 1, remaining) {
                Some((keys, stable)) => (Some(ValueKey::Lambda(*param, *body, keys)), stable),
                None => (None, false),
            },
            Thunk::Done(Value::Fix { id, env, args, .. }) => {
                match self.environment_key(*id, env, None, depth - 1, remaining) {
                    Some((keys, env_stable)) => {
                        let (args, args_stable) = self.argument_keys(args, depth - 1, remaining);
                        (
                            Some(ValueKey::Fix(*id, keys, args)),
                            env_stable && args_stable,
                        )
                    }
                    None => (None, false),
                }
            }
            _ => (None, false),
        };
        let key = key
            .map(|k| self.intern_value_key(k, value.id))
            .unwrap_or(value.id);
        // An immutable outer value may still contain delayed captures. Until
        // their keys settle, recomputing allows equal results to converge.
        if stable {
            value.key.set(Some(key));
            self.key_changed(value);
        } else if revision == self.key_revision && revision != u64::MAX {
            // Retain the exact bounded observation and charge. If its allowance
            // was not exhausted, any allowance covering the same work can reuse
            // it while none of its dependencies has advanced. A traversal
            // that reached no depth cutoff also records the depth it needed;
            // larger depth allowances cannot change that completed walk.
            // Exhausted bounds remain specific to their original allowance.
            value.observation.set(Some(KeyObservation {
                revision,
                depth,
                budget,
                key,
                spent: budget - *remaining,
                dependencies: self.key_dependencies,
                depth_used: self.key_depth_used,
                depth_limited: self.key_depth_limited,
            }));
        }
        (key, stable)
    }

    fn constructor(&mut self, head: TermId) -> Option<Constructor> {
        if let Some(info) = self.constructors.get(&head) {
            return info.clone();
        }
        let name = match self.store.node(head) {
            Node::Global(name) | Node::Const { name, .. } => self.store.name(*name),
            _ => return None,
        };
        let ind = self.ctx.constructor_inductive(name)?;
        if !self.ctx.is_inductive(ind) {
            return None;
        }
        if self.preserve_nat_syntax && self.ctx.constructor_inventory_upper_bound(ind) > 4096 {
            return None;
        }
        let ctors = self.ctx.get_constructors(ind);
        let index = ctors.iter().position(|(ctor, _)| *ctor == name)?;
        let ty = ctors[index].1;
        if self.preserve_nat_syntax && !natural_readback_admits(ty) {
            return None;
        }
        let mut arity = 0;
        let mut current = ty;
        while let Term::Pi { body_type, .. } = current {
            arity += 1;
            current = body_type;
        }
        let parameters = self
            .ctx
            .inductive_declared_params(ind)
            .unwrap_or_else(|| count_type_params(ty));
        let declared = if self.preserve_nat_syntax {
            // Monomorphic Global inference is exactly this visible lookup.
            if !matches!(self.store.node(head), Node::Global(_)) {
                return None;
            }
            let header = self.ctx.get_global(name)?;
            if !natural_readback_admits(header) {
                return None;
            }
            header.clone()
        } else {
            crate::infer_type(self.ctx, &self.store.extract(head)).ok()?
        };
        let mut ty = &declared;
        let mut domains = Vec::new();
        while let Term::Pi {
            param_type,
            body_type,
            ..
        } = ty
        {
            domains.push(self.store.intern(param_type));
            ty = body_type;
        }
        if domains.len() != arity {
            return None;
        }
        let info = Constructor {
            index,
            inventory: ctors.len(),
            parameters,
            arity,
            domains: domains.into(),
        };
        self.constructors.insert(head, Some(info.clone()));
        Some(info)
    }

    fn global(&mut self, id: TermId) -> Option<Lazy> {
        if let Some(value) = self.globals.get(&id) {
            if TRACE {
                if let Some(work) = &mut self.work {
                    work.global_cache_hits += 1;
                }
            }
            return Some(value.clone());
        }
        if self.preserve_nat_syntax {
            let Node::Global(name) = self.store.node(id) else {
                return None;
            };
            let name = self.store.name(*name);
            if let Some(body) = self.ctx.get_definition_body(name) {
                if !natural_readback_admits(body) {
                    return None;
                }
            }
        }
        let body = match self.store.extract(id) {
            Term::Global(name) => self.ctx.get_definition_body(&name).cloned(),
            Term::Const { name, levels } => match const_step(self.ctx, &name, &levels) {
                ConstStep::Illformed | ConstStep::Unresolved => return None,
                ConstStep::Delta(subst, body) => {
                    Some(crate::term::instantiate_universes(&body, &subst))
                }
                _ => None,
            },
            _ => return None,
        };
        let value = if let Some(body) = body {
            if TRACE {
                if let Some(work) = &mut self.work {
                    work.delta += 1;
                }
            }
            let body = self.store.intern(&body);
            if !self.store.free_names_are_empty(body)
                || self.store.shapes(body).contains(NodeShape::Hole)
                || !self.store.eq_faithful(body)
            {
                return None;
            }
            // Global definitions never inherit the caller's local environment.
            self.delay(body, &None)?
        } else if self.constructor(id).is_some() {
            let value = self.compact_value(Value::Constructor {
                head: id,
                args: Vec::new(),
            });
            match value {
                Value::Nat(n) => self.ready_nat(n)?,
                value => self.thunk(Thunk::Done(value))?,
            }
        } else {
            self.thunk(Thunk::Done(Value::Neutral))?
        };
        self.globals.insert(id, value.clone());
        Some(value)
    }

    // Reuse the exact checked-program gate in both observation modes. Chunk
    // cursors borrow numeric meaning without changing values or their keys.
    // The immutable Context keeps cached answers scoped to this registry.
    fn readback_nat_order(&mut self, head: TermId) -> Option<bool> {
        if !self.ctx.simp_rules().is_empty() {
            return None;
        }
        let Node::Global(name) = self.store.node(head) else {
            return None;
        };
        let operation = match self.store.name(*name) {
            "natLe" => 0,
            "natLt" => 1,
            _ => return None,
        };
        let valid = if let Some(valid) = self.nat_order_attestation[operation] {
            valid
        } else {
            let names: &[&str] = if operation == 0 {
                &["natLe"]
            } else {
                &["natLe", "natLt"]
            };
            let valid = self.ctx.constructor_inventory_upper_bound("Nat") <= 4096
                && self.ctx.constructor_inventory_upper_bound("Bool") <= 4096
                && names.iter().all(|name| {
                    self.ctx.poly_declaration(name).is_none()
                        && !self.ctx.is_constructor(name)
                        && !self.ctx.is_inductive(name)
                        && self.ctx.get_global(name) == self.ctx.get_definition_type(name)
                        && self
                            .ctx
                            .get_definition_type(name)
                            .is_some_and(natural_readback_admits)
                        && self
                            .ctx
                            .get_definition_body(name)
                            .is_some_and(natural_readback_admits)
                })
                && super::accelerates_nat_order(
                    self.ctx,
                    if operation == 0 { "natLe" } else { "natLt" },
                );
            self.nat_order_attestation[operation] = Some(valid);
            valid
        };
        valid.then_some(operation == 1)
    }

    fn nat_cursor(&self, value: Value) -> Option<NatCursor> {
        let zero = crate::BigInt::zero();
        let (count, tail) = match value {
            Value::Nat(n) if !n.is_negative() => (n, None),
            Value::Constructor { head, mut args } => {
                let Node::Global(name) = self.store.node(head) else {
                    return None;
                };
                match (self.store.name(*name), args.len()) {
                    ("Zero", 0) => (zero, None),
                    ("Succ", 1) => (crate::BigInt::from_i64(1), args.pop()),
                    _ => return None,
                }
            }
            Value::UnaryRun { head, count, tail } if count > 0 => {
                let Node::Global(name) = self.store.node(head) else {
                    return None;
                };
                if self.store.name(*name) != "Succ" {
                    return None;
                }
                (
                    crate::BigInt::from_u64(u64::try_from(count).ok()?),
                    Some(tail),
                )
            }
            _ => return None,
        };
        Some(NatCursor::Prefix { count, tail })
    }

    fn order_answer(&mut self, answer: bool, answers: [TermId; 2]) -> Control {
        Control::Eval(answers[usize::from(answer)], None)
    }

    // Certify a min-recursion program by its three universally quantified
    // computation equations. Induction on the first natural then proves that
    // chunk subtraction implements its ordinary reduction for every input.
    // Names only identify the program being examined; they confer no semantics.
    fn certified_order(&mut self, head: TermId) -> Option<[TermId; 2]> {
        if !self.canonical_nat
            || self.ctx.checking_order_recurrence
            || !self.ctx.simp_rules().is_empty()
        {
            return None;
        }
        if let Some(answer) = self.certified_orders.get(&head) {
            return *answer;
        }
        let answer = self.certify_order_cached(head);
        self.certified_orders.insert(head, answer);
        answer
    }

    fn certify_order_cached(&mut self, head: TermId) -> Option<[TermId; 2]> {
        let Node::Global(symbol) = self.store.node(head) else {
            return None;
        };
        let name = self.store.name(*symbol);
        // The cache admits bounded symbol payloads; larger names retain the
        // existing certification path without allocating an extra cache key.
        if name.len() > 256 {
            return self.certify_order(head);
        }
        let name = name.to_owned();
        let ctx = self.ctx;
        let outcomes = ctx.cached_order_recurrence(&name, || {
            let outcomes = self.certify_order(head)?;
            let named = |id| match self.store.node(id) {
                Node::Global(symbol) => Some(self.store.name(*symbol).to_owned()),
                _ => None,
            };
            Some([named(outcomes[0])?, named(outcomes[1])?])
        })?;
        Some(outcomes.map(|name| self.store.intern(&Term::Global(name))))
    }

    fn certify_order(&mut self, head: TermId) -> Option<[TermId; 2]> {
        let Node::Global(symbol) = self.store.node(head) else {
            return None;
        };
        let name = self.store.name(*symbol);
        if self.ctx.poly_declaration(name).is_some()
            || self.ctx.is_constructor(name)
            || self.ctx.is_inductive(name)
        {
            return None;
        }
        let ty = self.ctx.get_definition_type(name)?;
        if self.ctx.get_global(name) != Some(ty) {
            return None;
        }
        let mut result = ty;
        for _ in 0..2 {
            let Term::Pi {
                param_type,
                body_type,
                ..
            } = result
            else {
                return None;
            };
            if !matches!(&**param_type, Term::Global(n) if n=="Nat") {
                return None;
            }
            result = body_type;
        }
        let Term::Global(carrier) = result else {
            return None;
        };
        if !self.ctx.is_inductive(carrier)
            || self.ctx.get_global(carrier) != Some(&Term::Sort(crate::Universe::Type(0)))
            || self.ctx.get_definition_body(carrier).is_some()
            || self.ctx.poly_declaration(carrier).is_some()
            || self.ctx.inductive_num_params(carrier) != 0
            || self.ctx.constructor_inventory_upper_bound(carrier) != 2
        {
            return None;
        }
        let constructors = self.ctx.get_constructors(carrier);
        if constructors.len() != 2
            || constructors.iter().any(|(name, ty)| {
                *ty != result
                    || self.ctx.get_global(name) != Some(result)
                    || self.ctx.constructor_inductive(name) != Some(carrier.as_str())
                    || self.ctx.get_definition_body(name).is_some()
                    || self.ctx.poly_declaration(name).is_some()
            })
        {
            return None;
        }
        let outcomes = constructors
            .iter()
            .map(|(name, _)| Term::Global((*name).into()))
            .collect::<Vec<_>>();
        let body = self.ctx.get_definition_body(name)?;
        if !recurrence_body_admitted(body) {
            return None;
        }
        let body = self.store.intern(body);
        if !self.store.free_names_are_empty(body)
            || self.store.shapes(body).contains(NodeShape::Hole)
            || !self.store.eq_faithful(body)
        {
            return None;
        }
        #[cfg(test)]
        ORDER_CERTIFICATIONS.with(|n| n.set(n.get() + 1));
        let op = self.store.extract(head);
        let apply = |f: Term, a: Term| Term::App(Box::new(f), Box::new(a));
        let order = |a: Term, b: Term| apply(apply(op.clone(), a), b);
        let succ = |a: Term| apply(Term::Global("Succ".into()), a);
        let nat = Term::Global("Nat".into());
        let context = self
            .ctx
            .order_recurrence_context()
            .extend("order.certificate.m", nat.clone())
            .extend("order.certificate.n", nat);
        let m = Term::Var("order.certificate.m".into());
        let n = Term::Var("order.certificate.n".into());
        let zero = Term::Global("Zero".into());
        let base = order(zero.clone(), n.clone());
        // `f 0 n = yes` for every n. A program that matches on both arguments (Lean's `Nat.ble`)
        // proves it by its two cases instead: `f 0 0 = yes` and `f 0 (n+1) = yes`.
        let yes = match outcomes
            .iter()
            .position(|outcome| crate::type_checker::def_eq(&context, &base, outcome))
        {
            Some(yes) => yes,
            None => {
                let at_zero = order(zero.clone(), zero.clone());
                let at_successor = order(zero.clone(), succ(n.clone()));
                outcomes.iter().position(|outcome| {
                    crate::type_checker::def_eq(&context, &at_zero, outcome)
                        && crate::type_checker::def_eq(&context, &at_successor, outcome)
                })?
            }
        };
        let no = 1 - yes;
        if !crate::type_checker::def_eq(&context, &order(succ(m.clone()), zero), &outcomes[no])
            || !crate::type_checker::def_eq(
                &context,
                &order(succ(m.clone()), succ(n.clone())),
                &order(m, n),
            )
        {
            return None;
        }
        Some([
            self.store.intern(&outcomes[no]),
            self.store.intern(&outcomes[yes]),
        ])
    }

    fn structural_column(&mut self, fix: TermId, body: TermId) -> Option<usize> {
        if let Some(answer) = self.columns.get(&fix) {
            return *answer;
        }
        // The kernel's guard sees the original body once per Fix. Dynamic
        // environments never specialize/copy that body or alter its recursion.
        if self.store.depth(body) > crate::type_checker::MAX_TERM_DEPTH as u32
            || self.store.tree_size(body) > 65_536
        {
            return None;
        }
        let term = self.store.extract(body);
        let Node::Fix { name, .. } = self.store.node(fix) else {
            return None;
        };
        let answer = crate::termination::check_termination(self.ctx, self.store.name(*name), &term)
            .ok()
            .and_then(|()| {
                crate::termination::structural_columns(self.ctx, &term)
                    .first()
                    .copied()
            });
        self.columns.insert(fix, answer);
        answer
    }

    fn constructor_value(&mut self, value: &Value) -> Option<(usize, usize, Vec<Lazy>)> {
        match value {
            Value::Constructor { head, args } => {
                let info = self.constructor(*head)?;
                if args.len() != info.arity {
                    return None;
                }
                Some((
                    info.index,
                    info.inventory,
                    args.get(info.parameters..)?.to_vec(),
                ))
            }
            Value::UnaryRun { head, count, tail } => {
                let info = self.constructor(*head)?;
                let predecessor = self.run_predecessor(*head, *count, tail)?;
                Some((info.index, info.inventory, vec![predecessor]))
            }
            Value::Nat(n) => {
                if !self.canonical_nat {
                    return None;
                }
                let zero = crate::BigInt::from_i64(0);
                if *n < zero {
                    return None;
                }
                let ctors = self.ctx.get_constructors("Nat");
                let which = if *n == zero { "Zero" } else { "Succ" };
                let index = ctors.iter().position(|(name, _)| *name == which)?;
                let fields = if *n == zero {
                    Vec::new()
                } else {
                    vec![self.ready_nat(n.sub(&crate::BigInt::from_i64(1)))?]
                };
                Some((index, ctors.len(), fields))
            }
            _ => None,
        }
    }

    fn unknown_argument(&mut self) -> Option<Lazy> {
        if let Some(value) = &self.unknown {
            return Some(value.clone());
        }
        let value = self.thunk(Thunk::Unknown)?;
        self.unknown = Some(value.clone());
        Some(value)
    }

    // A cheap admission heuristic for inputs commonly forwarded without being
    // observed. This inspects only the declared result sort, never proof bodies.
    // Unknown is a bottom value: no reduction rule can inspect or manufacture
    // information from it. Only observed data independent of every unknown can
    // certify equality; otherwise the caller uses the original terms.
    fn proposition_domain(&mut self, term: TermId) -> bool {
        if let Some(answer) = self.proof_domains.get(&term) {
            return *answer;
        }
        let mut head = term;
        while let Node::Pi { body_type, .. } = self.store.node(head) {
            head = *body_type;
        }
        let mut arguments = 0;
        while let Node::App(f, _) = self.store.node(head) {
            arguments += 1;
            head = *f;
        }
        if self.preserve_nat_syntax {
            let answer = (|| {
                let Node::Global(name) = self.store.node(head) else {
                    return None;
                };
                let mut ty = self.ctx.get_global(self.store.name(*name))?;
                if !natural_readback_admits(ty) {
                    return None;
                }
                for _ in 0..arguments {
                    let Term::Pi { body_type, .. } = ty else {
                        return None;
                    };
                    ty = body_type;
                }
                Some(matches!(ty, Term::Sort(crate::Universe::Prop)))
            })()
            .unwrap_or(false);
            self.proof_domains.insert(term, answer);
            return answer;
        }
        let declared = match self.store.extract(head) {
            Term::Global(name) => self.ctx.get_global(&name).cloned(),
            term @ Term::Const { .. } => crate::type_checker::infer_type(self.ctx, &term).ok(),
            _ => None,
        };
        let answer = declared.is_some_and(|ty| {
            let mut result = &ty;
            for _ in 0..arguments {
                let Term::Pi { body_type, .. } = result else {
                    return false;
                };
                result = body_type;
            }
            matches!(result, Term::Sort(crate::Universe::Prop))
        });
        self.proof_domains.insert(term, answer);
        answer
    }

    fn argument(&mut self, arg: Argument) -> Option<Lazy> {
        match arg {
            Argument::Expression(term, env) => self.delay(term, &env),
            Argument::Ready(value) => Some(value),
        }
    }

    fn apply_lambda(
        &mut self,
        param: Symbol,
        param_type: TermId,
        body: TermId,
        env: Env,
        arg: Argument,
    ) -> Option<Control> {
        if TRACE {
            if let Some(work) = &mut self.work {
                work.beta += 1;
            }
        }
        let used = self.store.head_reduction_symbols(body).contains(&param);
        let env = if used {
            // This is the same demand speculation used by returned lambdas:
            // observing Unknown aborts, even if the domain heuristic errs.
            let arg = if self.proposition_domain(param_type) {
                self.unknown_argument()?
            } else {
                self.argument(arg)?
            };
            bind(env, param, arg)
        } else {
            env
        };
        Some(Control::Eval(body, env))
    }

    fn whnf(&mut self, root: TermId) -> Option<Value> {
        self.run(Control::Eval(root, None))
    }

    fn force(&mut self, thunk: Lazy) -> Option<Value> {
        self.run(Control::Force(thunk))
    }

    fn run(&mut self, mut control: Control) -> Option<Value> {
        let mut stack = Vec::new();
        loop {
            if self.spent() {
                return None;
            }
            self.fuel -= 1;
            if TRACE {
                if let Some(work) = &mut self.work {
                    match &control {
                        Control::Eval(term, _) => {
                            work.eval += 1;
                            if matches!(
                                self.store.node(*term),
                                Node::Global(_) | Node::Const { .. }
                            ) {
                                work.eval_global += 1;
                            }
                        }
                        Control::Compute(..) => work.compute += 1,
                        Control::Force(..) => work.force += 1,
                        Control::Return(..) => work.returned += 1,
                        Control::NatOrder { .. } => {}
                    }
                }
            }
            control = match control {
                Control::NatOrder {
                    left,
                    right,
                    answers,
                } => match left {
                    NatCursor::Pending(left) => {
                        stack.push(Frame::NatOrderLeft { right, answers });
                        Control::Force(left)
                    }
                    NatCursor::Prefix { count, tail } if count.is_zero() => match tail {
                        Some(left) => Control::NatOrder {
                            left: NatCursor::Pending(left),
                            right,
                            answers,
                        },
                        None => self.order_answer(true, answers),
                    },
                    left @ NatCursor::Prefix { .. } => match right {
                        NatCursor::Pending(right) => {
                            stack.push(Frame::NatOrderRight { left, answers });
                            Control::Force(right)
                        }
                        NatCursor::Prefix { count, tail } if count.is_zero() => match tail {
                            Some(right) => Control::NatOrder {
                                left,
                                right: NatCursor::Pending(right),
                                answers,
                            },
                            None => self.order_answer(false, answers),
                        },
                        NatCursor::Prefix { count: b, tail: bt } => {
                            let NatCursor::Prefix { count: a, tail: at } = left else {
                                unreachable!()
                            };
                            // Each cancellation is charged by the surrounding machine loop.
                            // Do not mutate either original value, thunk, or behavioral key.
                            let common = if a <= b { &a } else { &b };
                            Control::NatOrder {
                                answers,
                                left: NatCursor::Prefix {
                                    count: a.sub(common),
                                    tail: at,
                                },
                                right: NatCursor::Prefix {
                                    count: b.sub(common),
                                    tail: bt,
                                },
                            }
                        }
                    },
                },
                Control::Eval(term, env) => match self.store.node(term) {
                    // A lambda is already a value. Avoid allocating and forcing a
                    // thunk merely to discover the binder at an application head.
                    Node::Lambda {
                        param,
                        param_type,
                        body,
                    } => {
                        let (param, param_type, body) = (*param, *param_type, *body);
                        if matches!(stack.last(), Some(Frame::Apply(_))) {
                            let Some(Frame::Apply(arg)) = stack.pop() else {
                                unreachable!()
                            };
                            // The pending application consumes this binder immediately.
                            // No returned closure value or mutable thunk is needed.
                            self.apply_lambda(param, param_type, body, env, arg)?
                        } else {
                            if TRACE {
                                if let Some(work) = &mut self.work {
                                    work.lambda_values += 1;
                                }
                            }
                            Control::Return(Value::Lambda {
                                param,
                                param_type,
                                body,
                                env,
                            })
                        }
                    }
                    _ => {
                        let before = self.next_thunk;
                        let (delayed, fresh) = self.prepare_delay::<true>(term, &env)?;
                        if TRACE {
                            if let Some(work) = &mut self.work {
                                if fresh {
                                    work.demanded_fresh_running += 1;
                                }
                                match &*delayed.state.borrow() {
                                    Thunk::Suspended(..) if delayed.id >= before => {
                                        work.demanded_fresh_suspended += 1
                                    }
                                    Thunk::Suspended(..) => work.demanded_cached_suspended += 1,
                                    Thunk::Done(..) => work.demanded_ready += 1,
                                    _ => {}
                                }
                            }
                        }
                        if fresh {
                            // The caller already demands this new computation.
                            // Install its exact ordinary update continuation without
                            // constructing then dismantling a Suspended state.
                            stack.push(Frame::Update {
                                thunk: delayed,
                                term,
                                env: env.clone(),
                            });
                            Control::Compute(term, env)
                        } else {
                            Control::Force(delayed)
                        }
                    }
                },
                Control::Compute(term, env) => match self.store.node(term).clone() {
                    Node::Var(name) => Control::Force(lookup(&env, name)?),
                    Node::Global(_) | Node::Const { .. } => {
                        if TRACE {
                            if let Some(work) = &mut self.work {
                                work.compute_global += 1;
                            }
                        }
                        Control::Force(self.global(term)?)
                    }
                    Node::Lambda {
                        param,
                        param_type,
                        body,
                    } => {
                        if TRACE {
                            if let Some(work) = &mut self.work {
                                work.lambda_values += 1;
                            }
                        }
                        Control::Return(Value::Lambda {
                            param,
                            param_type,
                            body,
                            env,
                        })
                    }
                    Node::Fix { name, body } => Control::Return(Value::Fix {
                        id: term,
                        name,
                        body,
                        env,
                        args: Vec::new(),
                    }),
                    Node::App(partial, right) => {
                        if matches!(self.store.node(partial), Node::Global(_)) {
                            if let Some(division) = self.certified_division(partial) {
                                let pair = self.delay(right, &env)?;
                                stack.push(Frame::DivisionPair { division });
                                control = Control::Force(pair);
                                continue;
                            }
                        }
                        if let Some(lookup) = self.begin_index(term, &env) {
                            let index = lookup.args[3].clone();
                            stack.push(Frame::IndexNumber {
                                lookup,
                                total: crate::BigInt::zero(),
                            });
                            control = Control::Force(index);
                            continue;
                        }
                        if let Node::App(head, left) = self.store.node(partial) {
                            let (head, left) = (*head, *left);
                            let program = if let Some(strict) = self.readback_nat_order(head) {
                                Some((
                                    strict,
                                    [self.store.mk_global("false"), self.store.mk_global("true")],
                                ))
                            } else {
                                self.certified_order(head).map(|answers| (false, answers))
                            };
                            if let Some((strict, answers)) = program {
                                let left = self.delay(left, &env)?;
                                let right = self.delay(right, &env)?;
                                control = Control::NatOrder {
                                    answers,
                                    left: if strict {
                                        NatCursor::Prefix {
                                            count: crate::BigInt::from_i64(1),
                                            tail: Some(left),
                                        }
                                    } else {
                                        NatCursor::Pending(left)
                                    },
                                    right: NatCursor::Pending(right),
                                };
                                continue;
                            }
                            if let Some(answers) = self.certified_equality(head) {
                                let left = self.delay(left, &env)?;
                                let right = self.delay(right, &env)?;
                                // Equality is the conjunction of both natural orders.
                                // Reuse the existing lazy prefix-cancellation machine;
                                // the second direction runs only when the first succeeds.
                                stack.push(Frame::NatEqualityReverse {
                                    left: left.clone(),
                                    right: right.clone(),
                                    answers,
                                });
                                control = Control::NatOrder {
                                    left: NatCursor::Pending(left),
                                    right: NatCursor::Pending(right),
                                    answers,
                                };
                                continue;
                            }
                            if let Some(op) = self.certified_arithmetic(head) {
                                let left = self.delay(left, &env)?;
                                let right = self.delay(right, &env)?;
                                stack.push(Frame::ArithmeticLeft {
                                    op,
                                    right,
                                    total: crate::BigInt::zero(),
                                });
                                control = Control::Force(left);
                                continue;
                            }
                        }
                        // Keep a whole application spine on the control stack.
                        // Intermediate partial applications need no thunks or
                        // memo entries; arguments remain delayed in their own
                        // lexical environment and are applied from left to right.
                        let mut head = term;
                        while let Node::App(f, a) = self.store.node(head) {
                            stack.push(Frame::Apply(Argument::Expression(*a, env.clone())));
                            head = *f;
                        }
                        Control::Eval(head, env)
                    }
                    Node::Let {
                        name, value, body, ..
                    } => {
                        if TRACE {
                            if let Some(work) = &mut self.work {
                                work.zeta += 1;
                            }
                        }
                        let value = self.delay(value, &env)?;
                        Control::Eval(body, bind(env, name, value))
                    }
                    Node::Match {
                        discriminant,
                        cases,
                        ..
                    } => {
                        stack.push(Frame::Match {
                            cases,
                            env: env.clone(),
                        });
                        Control::Eval(discriminant, env)
                    }
                    Node::Lit(_) => {
                        let Term::Lit(Literal::Nat(n)) = self.store.extract(term) else {
                            return None;
                        };
                        if n < crate::BigInt::from_i64(0) {
                            return None;
                        }
                        Control::Return(Value::Nat(n))
                    }
                    Node::Hole | Node::MutualFix { .. } => return None,
                    _ => Control::Return(Value::Neutral),
                },
                Control::Force(thunk) => {
                    if matches!(stack.last(), Some(Frame::Apply(_))) {
                        let lambda = match &*thunk.state.borrow() {
                            Thunk::Done(Value::Lambda {
                                param,
                                param_type,
                                body,
                                env,
                            }) => Some((*param, *param_type, *body, env.clone())),
                            _ => None,
                        };
                        if let Some((param, param_type, body, env)) = lambda {
                            if TRACE {
                                if let Some(work) = &mut self.work {
                                    work.force_ready += 1;
                                }
                            }
                            let Some(Frame::Apply(arg)) = stack.pop() else {
                                unreachable!()
                            };
                            // Done lambdas are immutable. Reading their fields preserves
                            // the exact captured environment and memoized thunk identity,
                            // avoiding Done -> Running -> Done and a whole-value clone.
                            control = self.apply_lambda(param, param_type, body, env, arg)?;
                            continue;
                        }
                    }
                    let state = std::mem::replace(&mut *thunk.state.borrow_mut(), Thunk::Running);
                    if matches!(&state, Thunk::Suspended(..) | Thunk::Unknown) {
                        self.key_changed(&thunk);
                    }
                    if TRACE {
                        if let Some(work) = &mut self.work {
                            match &state {
                                Thunk::Done(..) => work.force_ready += 1,
                                Thunk::Suspended(..) => work.force_suspended += 1,
                                Thunk::Running | Thunk::Unknown => work.force_blocked += 1,
                            }
                        }
                    }
                    match state {
                        Thunk::Running | Thunk::Unknown => return None,
                        Thunk::Done(value) => {
                            if TRACE {
                                if let Some(work) = &mut self.work {
                                    work.ready_value_rewrites += 1;
                                }
                            }
                            let was_constructor =
                                matches!(value, Value::Constructor { .. } | Value::UnaryRun { .. });
                            let value = self.compact_value(value);
                            if was_constructor
                                && matches!(value, Value::Nat(_) | Value::UnaryRun { .. })
                            {
                                thunk.key.set(None);
                            }
                            *thunk.state.borrow_mut() = Thunk::Done(value.clone());
                            Control::Return(value)
                        }
                        Thunk::Suspended(term, env) => {
                            if TRACE {
                                if let Some(work) = &mut self.work {
                                    if matches!(self.store.node(term), Node::Lambda { .. }) {
                                        work.forced_lambda_suspensions += 1;
                                    }
                                }
                            }
                            stack.push(Frame::Update {
                                thunk,
                                term,
                                env: env.clone(),
                            });
                            Control::Compute(term, env)
                        }
                    }
                }
                Control::Return(value) => {
                    let value = if self.normalize_unary_spines {
                        self.compact_value(value)
                    } else {
                        value
                    };
                    // Full data observation needs the complete unary spine.
                    // Finish it before publishing a recursive result, so later
                    // calls can reuse its compact key rather than a triangular
                    // chain of suspended unary tails. Binary data remains lazy.
                    // Atomic head evaluation does not enable this demand mode.
                    let demand = if self.normalize_unary_spines {
                        match &value {
                            Value::UnaryRun { tail, .. }
                                if matches!(&*tail.state.borrow(), Thunk::Suspended(..)) =>
                            {
                                Some(tail.clone())
                            }
                            _ => None,
                        }
                    } else {
                        None
                    };
                    if let Some(tail) = demand {
                        if TRACE {
                            if let Some(work) = &mut self.work {
                                work.return_unary_demand += 1;
                            }
                        }
                        stack.push(Frame::FinishUnary(value));
                        Control::Force(tail)
                    } else {
                        if TRACE {
                            if let Some(work) = &mut self.work {
                                match stack.last() {
                                    Some(Frame::Update { .. }) => work.return_update += 1,
                                    Some(Frame::Apply(..)) => work.return_apply += 1,
                                    Some(Frame::Unroll { .. }) => work.return_unroll += 1,
                                    Some(Frame::Match { .. }) => work.return_match += 1,
                                    Some(Frame::FinishUnary(..)) => work.return_finish_unary += 1,
                                    None => work.return_complete += 1,
                                    Some(
                                        Frame::NatEqualityReverse { .. }
                                        | Frame::NatOrderLeft { .. }
                                        | Frame::NatOrderRight { .. }
                                        | Frame::ArithmeticLeft { .. }
                                        | Frame::ArithmeticRight { .. }
                                        | Frame::DivisionPair { .. }
                                        | Frame::IndexNumber { .. }
                                        | Frame::IndexCell { .. },
                                    ) => {}
                                }
                            }
                        }
                        match stack.pop() {
                            Some(Frame::IndexNumber { lookup, total }) => {
                                match self.nat_cursor(value) {
                                    Some(NatCursor::Prefix { count, tail }) => {
                                        let total = self.arithmetic_result(
                                            Arithmetic::Add,
                                            &total,
                                            &count,
                                        )?;
                                        if let Some(tail) = tail {
                                            stack.push(Frame::IndexNumber { lookup, total });
                                            Control::Force(tail)
                                        } else {
                                            self.index_start(lookup, total, &mut stack)
                                        }
                                    }
                                    _ => self.index_fallback(lookup, &mut stack),
                                }
                            }
                            Some(Frame::IndexCell { mut lookup, index }) => {
                                match value {
                                    Value::Constructor { head, args }
                                        if head == lookup.cons && args.len() == 3 =>
                                    {
                                        // Only witnessed constructor fields enter the index.
                                        // Leave the cached frontier at the cap; longer lists
                                        // continue through the ordinary cell traversal.
                                        if let Some(spine) = lookup.spine.take() {
                                            let mut observed = spine.borrow_mut();
                                            if observed.fields.len() < certified_index::SPINE_LIMIT
                                            {
                                                observed.fields.push(args[1].clone());
                                                observed.frontier = args[2].clone();
                                                drop(observed);
                                                lookup.spine = Some(spine);
                                            }
                                        }
                                        if index.is_zero() {
                                            Control::Force(args[1].clone())
                                        } else {
                                            let tail = args[2].clone();
                                            stack.push(Frame::IndexCell {
                                                lookup,
                                                index: index.sub(&crate::BigInt::from_i64(1)),
                                            });
                                            Control::Force(tail)
                                        }
                                    }
                                    _ => self.index_fallback(lookup, &mut stack),
                                }
                            }
                            Some(Frame::NatEqualityReverse {
                                left,
                                right,
                                answers,
                            }) => {
                                let Value::Constructor { head, args } = value else {
                                    return None;
                                };
                                if !args.is_empty() {
                                    return None;
                                }
                                if self.store.alpha_same(head, answers[0]) {
                                    self.order_answer(false, answers)
                                } else if self.store.alpha_same(head, answers[1]) {
                                    Control::NatOrder {
                                        left: NatCursor::Pending(right),
                                        right: NatCursor::Pending(left),
                                        answers,
                                    }
                                } else {
                                    return None;
                                }
                            }
                            // The pair's two fields are the operands; any other value leaves the
                            // program unobserved. The pair constructor is universe-polymorphic in
                            // Lean (`PSigma.mk.{u,v}`): its levels place no field.
                            Some(Frame::DivisionPair { division }) => {
                                let Value::Constructor { head, args } = value else {
                                    return None;
                                };
                                let (Node::Global(symbol) | Node::Const { name: symbol, .. }) =
                                    self.store.node(head)
                                else {
                                    return None;
                                };
                                if self.store.name(*symbol) != division.pair
                                    || args.len() != division.parameters + 2
                                {
                                    return None;
                                }
                                let (left, right) = (
                                    args[division.parameters].clone(),
                                    args[division.parameters + 1].clone(),
                                );
                                stack.push(Frame::ArithmeticLeft {
                                    op: division.op,
                                    right,
                                    total: crate::BigInt::zero(),
                                });
                                Control::Force(left)
                            }
                            Some(Frame::ArithmeticLeft { op, right, total }) => {
                                let NatCursor::Prefix { count, tail } = self.nat_cursor(value)?
                                else {
                                    return None;
                                };
                                let total =
                                    self.arithmetic_result(Arithmetic::Add, &total, &count)?;
                                if let Some(tail) = tail {
                                    stack.push(Frame::ArithmeticLeft { op, right, total });
                                    Control::Force(tail)
                                } else {
                                    stack.push(Frame::ArithmeticRight {
                                        op,
                                        left: total,
                                        total: crate::BigInt::zero(),
                                    });
                                    Control::Force(right)
                                }
                            }
                            Some(Frame::ArithmeticRight { op, left, total }) => {
                                let NatCursor::Prefix { count, tail } = self.nat_cursor(value)?
                                else {
                                    return None;
                                };
                                let total =
                                    self.arithmetic_result(Arithmetic::Add, &total, &count)?;
                                if let Some(tail) = tail {
                                    stack.push(Frame::ArithmeticRight { op, left, total });
                                    Control::Force(tail)
                                } else {
                                    Control::Return(Value::Nat(
                                        self.arithmetic_result(op, &left, &total)?,
                                    ))
                                }
                            }
                            Some(Frame::NatOrderLeft { right, answers }) => Control::NatOrder {
                                left: self.nat_cursor(value)?,
                                right,
                                answers,
                            },
                            Some(Frame::NatOrderRight { left, answers }) => Control::NatOrder {
                                left,
                                right: self.nat_cursor(value)?,
                                answers,
                            },
                            None => return Some(value),
                            Some(Frame::FinishUnary(value)) => {
                                Control::Return(self.compact_value(value))
                            }
                            Some(Frame::Update { thunk, term, env }) => {
                                self.key_changed(&thunk);
                                *thunk.state.borrow_mut() = Thunk::Done(value.clone());
                                // Inputs forced during this computation can now have
                                // stronger compact behavioral keys. Publish the result
                                // under those keys as well as its original identity.
                                if !self.store.head_reduction_symbols(term).is_empty() {
                                    if TRACE {
                                        if let Some(work) = &mut self.work {
                                            work.capture_key_entries +=
                                                self.store.head_reduction_symbols(term).len()
                                                    as u64;
                                        }
                                    }
                                    let values = self
                                        .store
                                        .head_reduction_symbols(term)
                                        .iter()
                                        .map(|name| lookup(&env, *name))
                                        .collect::<Option<Vec<_>>>()?;
                                    let mut remaining = 256;
                                    let keys = values
                                        .iter()
                                        .map(|v| self.binding_key(v, 32, &mut remaining))
                                        .collect();
                                    self.closures.insert((term, keys), thunk);
                                }
                                Control::Return(value)
                            }
                            Some(Frame::Apply(arg)) => match value {
                                Value::Lambda {
                                    param,
                                    param_type,
                                    body,
                                    env,
                                } => self.apply_lambda(param, param_type, body, env, arg)?,
                                Value::Constructor { head, mut args } => {
                                    let info = self.constructor(head)?;
                                    let domain = *info.domains.get(args.len())?;
                                    // The same bottom-valued speculation applies to
                                    // constructor fields. A later projection/elimination
                                    // that observes this field aborts the whole attempt.
                                    let arg = if self.proposition_domain(domain) {
                                        self.unknown_argument()?
                                    } else {
                                        self.argument(arg)?
                                    };
                                    args.push(arg);
                                    Control::Return(
                                        self.compact_value(Value::Constructor { head, args }),
                                    )
                                }
                                Value::Fix {
                                    id,
                                    name,
                                    body,
                                    env,
                                    mut args,
                                } => {
                                    let arg = self.argument(arg)?;
                                    args.push(arg);
                                    let column = self.structural_column(id, body)?;
                                    if let Some(structural) = args.get(column).cloned() {
                                        stack.push(Frame::Unroll {
                                            id,
                                            name,
                                            body,
                                            env,
                                            args,
                                        });
                                        Control::Force(structural)
                                    } else {
                                        Control::Return(Value::Fix {
                                            id,
                                            name,
                                            body,
                                            env,
                                            args,
                                        })
                                    }
                                }
                                _ => return None,
                            },
                            Some(Frame::Unroll {
                                id,
                                name,
                                body,
                                env,
                                args,
                            }) => {
                                self.constructor_value(&value)?;
                                if TRACE {
                                    if let Some(work) = &mut self.work {
                                        work.guarded_unroll += 1;
                                    }
                                }
                                let recursive = self.delay(id, &env)?;
                                for arg in args.into_iter().rev() {
                                    stack.push(Frame::Apply(Argument::Ready(arg)));
                                }
                                Control::Eval(body, bind(env, name, recursive))
                            }
                            Some(Frame::Match { cases, env }) => {
                                let (index, inventory, fields) = self.constructor_value(&value)?;
                                // Pruned indexed matches need index conversion. The ordinary
                                // converter handles them; no exported branch rule is trusted.
                                if cases.len() != inventory {
                                    return None;
                                }
                                for field in fields.into_iter().rev() {
                                    stack.push(Frame::Apply(Argument::Ready(field)));
                                }
                                if TRACE {
                                    if let Some(work) = &mut self.work {
                                        work.iota += 1;
                                    }
                                }
                                Control::Eval(*cases.get(index)?, env)
                            }
                        }
                    }
                }
            };
        }
    }
}

// Equality certificates are keyed by physical thunk identities, never the
// behavioral ValueKeys used to share evaluation. A pair is reusable only after
// all of its fields have been observed successfully. Even identical thunks must
// be forced: an Unknown proof or a function field cannot certify data equality.
fn data_equivalent(r: &mut HeadReducer<'_>, a: TermId, b: TermId) -> Option<()> {
    enum Work {
        Compare(Lazy, Lazy),
        Complete((usize, usize)),
    }
    let a = r.delay(a, &None)?;
    let b = r.delay(b, &None)?;
    let mut work = vec![Work::Compare(a, b)];
    let mut completed = HashSet::new();
    let mut active = HashSet::new();
    while let Some(next) = work.pop() {
        r.fuel = r.fuel.checked_sub(1)?;
        match next {
            Work::Complete(pair) => {
                active.remove(&pair);
                completed.insert(pair);
            }
            Work::Compare(a, b) => {
                let pair = (a.id, b.id);
                if completed.contains(&pair) {
                    continue;
                }
                if !active.insert(pair) {
                    return None;
                }
                let av = r.force(a)?;
                let bv = r.force(b)?;
                if r.canonical_nat {
                    if let (Some(left), Some(right)) =
                        (r.nat_cursor(av.clone()), r.nat_cursor(bv.clone()))
                    {
                        natural_data_equivalent(r, left, right)?;
                        active.remove(&pair);
                        completed.insert(pair);
                        continue;
                    }
                }
                match (av, bv) {
                    (Value::Nat(a), Value::Nat(b)) if a == b => {
                        active.remove(&pair);
                        completed.insert(pair);
                    }
                    (
                        Value::UnaryRun {
                            head: ah,
                            count: an,
                            tail: at,
                        },
                        Value::UnaryRun {
                            head: bh,
                            count: bn,
                            tail: bt,
                        },
                    ) if r.store.alpha_same(ah, bh) && an > 0 && bn > 0 => {
                        // Cancel exactly the common observed prefix. The tail
                        // comparison certifies all remaining fields, even if
                        // both tails have the same physical thunk identity.
                        let (at, bt) = if an > bn {
                            (
                                r.thunk(Thunk::Done(Value::UnaryRun {
                                    head: ah,
                                    count: an - bn,
                                    tail: at,
                                }))?,
                                bt,
                            )
                        } else if bn > an {
                            (
                                at,
                                r.thunk(Thunk::Done(Value::UnaryRun {
                                    head: bh,
                                    count: bn - an,
                                    tail: bt,
                                }))?,
                            )
                        } else {
                            (at, bt)
                        };
                        work.push(Work::Complete(pair));
                        work.push(Work::Compare(at, bt));
                    }
                    (
                        Value::Constructor { head: ah, args: aa },
                        Value::Constructor { head: bh, args: ba },
                    ) if r.store.alpha_same(ah, bh)
                        && aa.len() == r.constructor(ah)?.arity
                        && ba.len() == r.constructor(bh)?.arity =>
                    {
                        work.push(Work::Complete(pair));
                        // Compare every argument, including type parameters and
                        // proof fields. Unsupported observations abort; the
                        // ordinary converter then handles the original terms.
                        work.extend(
                            aa.into_iter()
                                .zip(ba)
                                .rev()
                                .map(|(a, b)| Work::Compare(a, b)),
                        );
                    }
                    _ => return None,
                }
            }
        }
    }
    Some(())
}

// Compare compact literals and observed Peano prefixes using the canonical
// Nat bridge. Every remaining tail is forced, including physically shared tails.
fn natural_data_equivalent(
    r: &mut HeadReducer<'_>,
    mut left: NatCursor,
    mut right: NatCursor,
) -> Option<()> {
    loop {
        r.fuel = r.fuel.checked_sub(1)?;
        left = match left {
            NatCursor::Pending(tail) => {
                let value = r.force(tail)?;
                r.nat_cursor(value)?
            }
            prefix => prefix,
        };
        right = match right {
            NatCursor::Pending(tail) => {
                let value = r.force(tail)?;
                r.nat_cursor(value)?
            }
            prefix => prefix,
        };
        let NatCursor::Prefix { count: a, tail: at } = left else {
            return None;
        };
        let NatCursor::Prefix { count: b, tail: bt } = right else {
            return None;
        };
        if a == b {
            match (at, bt) {
                (None, None) => return Some(()),
                (Some(a), Some(b)) => {
                    left = NatCursor::Pending(a);
                    right = NatCursor::Pending(b);
                }
                (Some(a), None) => {
                    left = NatCursor::Pending(a);
                    right = NatCursor::Prefix {
                        count: crate::BigInt::zero(),
                        tail: None,
                    };
                }
                (None, Some(b)) => {
                    left = NatCursor::Prefix {
                        count: crate::BigInt::zero(),
                        tail: None,
                    };
                    right = NatCursor::Pending(b);
                }
            }
        } else if a > b {
            let bt = bt?;
            left = NatCursor::Prefix {
                count: a.sub(&b),
                tail: at,
            };
            right = NatCursor::Pending(bt);
        } else {
            let at = at?;
            left = NatCursor::Pending(at);
            right = NatCursor::Prefix {
                count: b.sub(&a),
                tail: bt,
            };
        }
    }
}

// Definitional equality is a congruence. Proving the arguments equal can be
// much cheaper than running a common defined function on both sides. This uses
// exact syntax for the head and full data observations for every argument.
fn congruent_data_arguments(r: &mut HeadReducer<'_>, a: TermId, b: TermId) -> Option<bool> {
    fn spine(store: &TermStore, mut term: TermId) -> (TermId, Vec<TermId>) {
        let mut args = Vec::new();
        while let Node::App(f, a) = store.node(term) {
            args.push(*a);
            term = *f;
        }
        args.reverse();
        (term, args)
    }
    let (ah, aa) = spine(&r.store, a);
    let (bh, ba) = spine(&r.store, b);
    if aa.is_empty() || aa.len() != ba.len() || !r.store.alpha_same(ah, bh) {
        return None;
    }
    let name = match r.store.node(ah) {
        Node::Global(name) | Node::Const { name, .. } => r.store.name(*name),
        _ => return None,
    };
    // Constructors retain the ordinary full-field observation path, including
    // its unknown-proof and unsupported type-parameter checks.
    if r.ctx.is_constructor(name)
        || (r.ctx.get_definition_body(name).is_none() && r.ctx.get_universe_poly(name).is_none())
    {
        return None;
    }
    // Validate universe instantiation and the stored definition before using
    // its identity as the common head. Do not execute its body.
    r.global(ah)?;
    Some(
        aa.into_iter()
            .zip(ba)
            .all(|(a, b)| data_equivalent(r, a, b).is_some()),
    )
}

pub(crate) fn constructor_equivalent(ctx: &Context, a: &Term, b: &Term, fuel: u64) -> bool {
    if fuel == 0 || !ctx.simp_rules().is_empty() {
        return false;
    }
    let atomic = |t: &Term| match t {
        Term::Global(n) | Term::Const { name: n, .. } => ctx.is_constructor(n),
        _ => false,
    };
    let atomic_candidate = atomic(a) || atomic(b);
    // General data observations get their own small speculative budget. Keep
    // the established atomic evaluator's budget for expensive closed proofs,
    // without charging that budget to every ordinary conversion attempt.
    let fuel = if atomic_candidate {
        fuel
    } else {
        fuel.min(1 << 18)
    };
    if !atomic_candidate
        && ![a, b].into_iter().any(|t| {
            matches!(
                t,
                Term::App(..) | Term::Let { .. } | Term::Match { .. } | Term::Fix { .. }
            )
        })
    {
        return false;
    }
    let mut r = HeadReducer::new(ctx, fuel);
    let a = r.store.intern(a);
    let b = r.store.intern(b);
    if [a, b].into_iter().any(|t| {
        !r.store.free_names_are_empty(t)
            || r.store.shapes(t).contains(NodeShape::Hole)
            || !r.store.eq_faithful(t)
    }) {
        return false;
    }
    if atomic_candidate
        && [a, b]
            .into_iter()
            .any(|t| r.constructor(t).is_some_and(|c| c.arity == 0))
    {
        return match (r.whnf(a), r.whnf(b)) {
            (Some(Value::Nat(a)), Some(Value::Nat(b))) => a == b,
            (
                Some(Value::Constructor { head: a, args: aa }),
                Some(Value::Constructor { head: b, args: ba }),
            ) if aa.is_empty() && ba.is_empty() => r.store.alpha_same(a, b),
            _ => false,
        };
    }
    r.fuel = r.fuel.min(1 << 18);
    r.normalize_unary_spines = true;
    match congruent_data_arguments(&mut r, a, b) {
        Some(true) => return true,
        Some(false) => {
            // An inconclusive observation may leave a Running/Unknown thunk.
            // Retry the original expressions with fresh evaluation caches and
            // the SAME remaining budget, retaining the immutable syntax store.
            // Unequal arguments need not make a function's results unequal.
            let mut fresh = HeadReducer::new(ctx, r.fuel);
            fresh.normalize_unary_spines = true;
            fresh.store = std::mem::replace(&mut r.store, TermStore::new());
            r = fresh;
        }
        None => {}
    }
    data_equivalent(&mut r, a, b).is_some()
}

/// A program the evaluator certifies, by what it computes on numerals: an arithmetic operation, a
/// natural order or equality that answers one of two nullary constructors, or a unary division
/// program on a pair.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CertifiedProgram {
    Arithmetic(Arithmetic),
    Order {
        no: String,
        yes: String,
    },
    Equality {
        no: String,
        yes: String,
    },
    Division {
        op: Arithmetic,
        pair: String,
        parameters: usize,
    },
}

/// How a program is applied to what it computes on, read from its declared type alone: two natural
/// operands, or one pair `mk ps x y` of them. A scheduling filter only; it certifies nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ProgramShape {
    Binary,
    Pair { pair: String, parameters: usize },
}

pub(crate) fn program_shape(ctx: &Context, name: &str, arguments: usize) -> Option<ProgramShape> {
    if !canonical_nat(ctx)
        || !ctx.simp_rules().is_empty()
        || ctx.checking_order_recurrence
        || ctx.poly_declaration(name).is_some()
        || ctx.get_definition_body(name).is_none()
    {
        return None;
    }
    match arguments {
        1 => certified_division::division_pair(ctx, name)
            .map(|(pair, parameters)| ProgramShape::Pair { pair, parameters }),
        2 => {
            let ty = ctx.get_definition_type(name)?;
            if ctx.get_global(name) != Some(ty) {
                return None;
            }
            let mut result = ty;
            for _ in 0..2 {
                let Term::Pi {
                    param_type,
                    body_type,
                    ..
                } = result
                else {
                    return None;
                };
                if !matches!(param_type.as_ref(), Term::Global(n) if n == "Nat") {
                    return None;
                }
                result = body_type;
            }
            matches!(result, Term::Global(carrier) if ctx.is_inductive(carrier))
                .then_some(ProgramShape::Binary)
        }
        _ => None,
    }
}

/// What the evaluator certifies `name` to compute, established once per registry generation. The
/// certification is speculative work: its resource stops never reach the enclosing observation, and
/// a refusal that followed a stop is not retained.
pub(crate) fn certified_program(ctx: &Context, name: &str) -> Option<CertifiedProgram> {
    ctx.cached_certified_program(name, || {
        super::discard_resource_stops(|| {
            super::observe_resource_stops(|| {
                #[cfg(test)]
                PROGRAM_CERTIFICATIONS.with(|n| n.set(n.get() + 1));
                let mut r = HeadReducer::new(ctx, 1 << 20);
                let head = r.store.intern(&Term::Global(name.into()));
                let named = |r: &HeadReducer<'_>, id: TermId| match r.store.node(id) {
                    Node::Global(symbol) => Some(r.store.name(*symbol).to_owned()),
                    _ => None,
                };
                if let Some(op) = r.certified_arithmetic(head) {
                    return Some(CertifiedProgram::Arithmetic(op));
                }
                if let Some([no, yes]) = r.certified_order(head) {
                    return Some(CertifiedProgram::Order {
                        no: named(&r, no)?,
                        yes: named(&r, yes)?,
                    });
                }
                if let Some([no, yes]) = r.certified_equality(head) {
                    return Some(CertifiedProgram::Equality {
                        no: named(&r, no)?,
                        yes: named(&r, yes)?,
                    });
                }
                r.certified_division(head).map(
                    |Division {
                         op,
                         pair,
                         parameters,
                     }| CertifiedProgram::Division {
                        op,
                        pair,
                        parameters,
                    },
                )
            })
        })
        .into_definitive()
    })
}

#[cfg(test)]
thread_local! {
    /// The certifications this thread has run: the work the per-generation cache is held to.
    static PROGRAM_CERTIFICATIONS: Cell<usize> = const { Cell::new(0) };
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_kernel/reduction/shared_head/certified_program_controls.rs"]
mod certified_program_controls;

trait Definitive<T> {
    fn into_definitive(self) -> (Option<T>, bool);
}
impl<T> Definitive<T> for (Option<T>, Option<crate::Unreduced>) {
    fn into_definitive(self) -> (Option<T>, bool) {
        (self.0, self.1.is_none())
    }
}

/// Lean's `reduce_nat`, for the programs this evaluator certifies: `program` at the numerals `x y`
/// (the pair's fields, for a division program) computed natively. `None` past the size bounds the
/// evaluator's own arithmetic keeps.
pub(crate) fn certified_application(
    program: &CertifiedProgram,
    x: &crate::BigInt,
    y: &crate::BigInt,
) -> Option<Term> {
    if x.is_negative() || y.is_negative() {
        return None;
    }
    let limbs = |n: &crate::BigInt| n.to_le_bytes().1.len().div_ceil(8);
    let natural = |n: crate::BigInt| Some(Term::Lit(Literal::Nat(n)));
    match program.clone() {
        CertifiedProgram::Order { no, yes } => Some(Term::Global(if x <= y { yes } else { no })),
        CertifiedProgram::Equality { no, yes } => Some(Term::Global(if x == y { yes } else { no })),
        CertifiedProgram::Arithmetic(Arithmetic::Add) if limbs(x).max(limbs(y)) < 16_384 => {
            natural(x.add(y))
        }
        CertifiedProgram::Arithmetic(Arithmetic::Sub) => natural(if x >= y {
            x.sub(y)
        } else {
            crate::BigInt::zero()
        }),
        CertifiedProgram::Arithmetic(Arithmetic::Mul) if limbs(x) + limbs(y) <= 16_384 => {
            natural(x.mul(y))
        }
        CertifiedProgram::Arithmetic(Arithmetic::Pow) => {
            let one = crate::BigInt::from_u64(1);
            if y.is_zero() || *x == one {
                return natural(one);
            }
            if x.is_zero() {
                return natural(crate::BigInt::zero());
            }
            let exponent = u32::try_from(y.to_i64()?).ok()?;
            if (limbs(x) as u64).checked_mul(exponent as u64)? > 16_384 {
                return None;
            }
            let mut result = one;
            let mut base = x.clone();
            let mut remaining = exponent;
            while remaining > 0 {
                if remaining & 1 != 0 {
                    result = result.mul(&base);
                }
                remaining >>= 1;
                if remaining > 0 {
                    base = base.mul(&base);
                }
            }
            natural(result)
        }
        CertifiedProgram::Division { op, .. } if limbs(x) <= 16_384 => {
            if y.is_zero() {
                return natural(if op == Arithmetic::Div {
                    crate::BigInt::zero()
                } else {
                    x.clone()
                });
            }
            let (quotient, remainder) = x.div_rem(y)?;
            natural(if op == Arithmetic::Div {
                quotient
            } else {
                remainder
            })
        }
        _ => None,
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_kernel/reduction/shared_head/tests.rs"]
mod tests;

#[derive(Debug, PartialEq)]
pub(super) enum NatReadbackOutcome {
    Normal(Term),
    Unsupported,
    Exhausted,
    TooDeep,
}

// Eligibility is only a cheap scheduling hint. Evaluation still determines the
// complete canonical Nat result, and no header here licenses a positive answer.
pub(super) fn natural_result_candidate(
    ctx: &Context,
    term: &Term,
    tr: super::Transparency<'_>,
) -> bool {
    if !matches!(tr, super::Transparency::All) || !ctx.simp_rules().is_empty() {
        return false;
    }
    let mut visits = 256usize;
    let mut arguments = 0usize;
    let mut head = term;
    while let Term::App(f, _) = head {
        let Some(left) = visits.checked_sub(1) else {
            return false;
        };
        visits = left;
        arguments += 1;
        head = f;
    }
    let Term::Global(name) = head else {
        return false;
    };
    if ctx.get_definition_body(name).is_none() || ctx.poly_declaration(name).is_some() {
        return false;
    }
    let Some(mut result) = ctx.get_global(name) else {
        return false;
    };
    for _ in 0..arguments {
        let Some(left) = visits.checked_sub(1) else {
            return false;
        };
        visits = left;
        let Term::Pi { body_type, .. } = result else {
            return false;
        };
        result = body_type;
    }
    matches!(result, Term::Global(name) if name=="Nat")
}

fn natural_readback_admits(term: &Term) -> bool {
    matches!(super::bounded_conversion_metrics(term),
        Some((_,depth,true)) if depth<=crate::type_checker::MAX_TERM_DEPTH)
}

#[cfg(test)]
thread_local! {
    static NAT_READBACK_STORE_BUILDS: Cell<usize> = const { Cell::new(0) };
}

// Returned fuel belongs to this exact attempt, including unsuccessful reads.
pub(super) fn natural_normal_form(
    ctx: &Context,
    term: &Term,
    tr: super::Transparency<'_>,
    fuel: u64,
) -> (NatReadbackOutcome, u64) {
    if fuel == 0 {
        return (NatReadbackOutcome::Exhausted, 0);
    }
    if !matches!(tr, super::Transparency::All)
        || !ctx.simp_rules().is_empty()
        || ctx.constructor_inventory_upper_bound("Nat") > 4096
        || !natural_readback_admits(term)
        || !crate::int_inductive::canonical_nat_environment(ctx)
    {
        return (NatReadbackOutcome::Unsupported, fuel);
    }
    #[cfg(test)]
    NAT_READBACK_STORE_BUILDS.with(|builds| builds.set(builds.get() + 1));
    let mut r = HeadReducer::for_nat_readback(ctx, fuel);
    let root = r.store.intern(term);
    let answer = r.readback_nat(root, crate::type_checker::MAX_TERM_DEPTH);
    (answer, r.fuel)
}

impl<'a, const TRACE: bool> HeadReducerWithWork<'a, TRACE> {
    fn for_nat_readback(ctx: &'a Context, fuel: u64) -> Self {
        Self::with_nat_syntax(ctx, fuel, true)
    }

    fn readback_failure(&self) -> NatReadbackOutcome {
        if self.spent() {
            NatReadbackOutcome::Exhausted
        } else {
            NatReadbackOutcome::Unsupported
        }
    }

    // Read only a completely observed canonical natural-number result. Other
    // constructors, neutral tails, unknown proofs and cycles remain inconclusive.
    // Constructor heads and compact literal tails retain their original syntax.
    fn readback_nat(&mut self, root: TermId, output_depth: usize) -> NatReadbackOutcome {
        if !self.ctx.simp_rules().is_empty()
            || self.ctx.constructor_inventory_upper_bound("Nat") > 4096
            || !crate::int_inductive::canonical_nat_environment(self.ctx)
            || !self.store.free_names_are_empty(root)
            || self.store.shapes(root).contains(NodeShape::Hole)
            || !self.store.eq_faithful(root)
        {
            return NatReadbackOutcome::Unsupported;
        }
        let limit = output_depth.min(crate::type_checker::MAX_TERM_DEPTH);
        let Some(mut value) = self.whnf(root) else {
            return self.readback_failure();
        };
        let mut prefixes = Vec::<(TermId, usize)>::new();
        let mut depth = 0usize;
        let mut result = loop {
            let Some(left) = self.fuel.checked_sub(1) else {
                return NatReadbackOutcome::Exhausted;
            };
            self.fuel = left;
            let (head, count, tail) = match value {
                Value::Nat(n) => break Term::Lit(Literal::Nat(n)),
                Value::Constructor { head, mut args } => {
                    let name = match self.store.node(head) {
                        Node::Global(name) | Node::Const { name, .. } => self.store.name(*name),
                        _ => return NatReadbackOutcome::Unsupported,
                    };
                    if name == "Zero" && args.is_empty() {
                        break self.store.extract(head);
                    }
                    if name != "Succ" || args.len() != 1 {
                        return NatReadbackOutcome::Unsupported;
                    }
                    (head, 1, args.pop().unwrap())
                }
                Value::UnaryRun { head, count, tail } => {
                    let name = match self.store.node(head) {
                        Node::Global(name) | Node::Const { name, .. } => self.store.name(*name),
                        _ => return NatReadbackOutcome::Unsupported,
                    };
                    if name != "Succ" || count == 0 {
                        return NatReadbackOutcome::Unsupported;
                    }
                    (head, count, tail)
                }
                _ => return NatReadbackOutcome::Unsupported,
            };
            let Some(next_depth) = depth.checked_add(count).filter(|d| *d <= limit) else {
                return NatReadbackOutcome::TooDeep;
            };
            depth = next_depth;
            prefixes.push((head, count));
            let Some(tail) = self.force(tail) else {
                return self.readback_failure();
            };
            value = tail;
        };
        for (head, count) in prefixes.into_iter().rev() {
            for _ in 0..count {
                let Some(left) = self.fuel.checked_sub(1) else {
                    return NatReadbackOutcome::Exhausted;
                };
                self.fuel = left;
                result = Term::App(Box::new(self.store.extract(head)), Box::new(result));
            }
        }
        NatReadbackOutcome::Normal(result)
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_kernel/reduction/shared_head/natural_readback_controls.rs"]
mod natural_readback_controls;

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_kernel/reduction/shared_head/natural_readback_explicit_header_boundary.rs"]
mod natural_readback_explicit_header_boundary;

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_kernel/reduction/shared_head/natural_public_hook_controls.rs"]
mod natural_public_hook_controls;

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_kernel/reduction/shared_head/natural_total_budget_controls.rs"]
mod natural_total_budget_controls;

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_kernel/reduction/shared_head/natural_readback_cycle_gateway_boundary.rs"]
mod natural_readback_cycle_gateway_boundary;

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_kernel/reduction/shared_head/natural_order_chunk_controls.rs"]
mod natural_order_chunk_controls;

// Additive draft only: append inside reduction/shared_head.rs after source review.
// No implementation changes and no relaxation of proof-field observation.
#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_kernel/reduction/shared_head/equality_nat_order_chunk_controls.rs"]
mod equality_nat_order_chunk_controls;

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_kernel/reduction/shared_head/identity_cache_controls.rs"]
mod identity_cache_controls;

// Term metrics alone do not account for explicit universe trees. Bound both
// before interning a candidate, which recursively interns its universe levels.
fn recurrence_body_admitted(body: &Term) -> bool {
    let Some((nodes, depth, _)) = super::bounded_conversion_metrics(body) else {
        return false;
    };
    if depth > crate::type_checker::MAX_TERM_DEPTH {
        return false;
    }
    let mut remaining = 4096 - nodes;
    let mut terms = vec![(body, 0usize)];
    let mut levels = Vec::new();
    while let Some((term, depth)) = terms.pop() {
        match term {
            Term::Sort(level) => levels.push((level, depth)),
            Term::Const {
                levels: instances, ..
            } => {
                if instances.len() > remaining {
                    return false;
                }
                levels.extend(instances.iter().map(|level| (level, depth)));
            }
            Term::App(a, b) => {
                terms.push((a, depth + 1));
                terms.push((b, depth + 1));
            }
            Term::Pi {
                param_type,
                body_type,
                ..
            } => {
                terms.push((param_type, depth + 1));
                terms.push((body_type, depth + 1));
            }
            Term::Lambda {
                param_type, body, ..
            } => {
                terms.push((param_type, depth + 1));
                terms.push((body, depth + 1));
            }
            Term::Let {
                ty, value, body, ..
            } => {
                terms.push((ty, depth + 1));
                terms.push((value, depth + 1));
                terms.push((body, depth + 1));
            }
            Term::Fix { body, .. } => terms.push((body, depth + 1)),
            Term::MutualFix { defs, .. } => {
                terms.extend(defs.iter().map(|(_, body)| (body, depth + 1)))
            }
            Term::Match {
                discriminant,
                motive,
                cases,
            } => {
                terms.push((discriminant, depth + 1));
                terms.push((motive, depth + 1));
                terms.extend(cases.iter().map(|case| (case, depth + 1)));
            }
            Term::Var(_) | Term::Global(_) | Term::Lit(_) | Term::Hole => {}
            Term::Annotated { term, .. } => terms.push((term, depth + 1)),
        }
        while let Some((level, depth)) = levels.pop() {
            if remaining == 0 || depth > crate::type_checker::MAX_TERM_DEPTH {
                return false;
            }
            remaining -= 1;
            match level {
                crate::Universe::Succ(a) => levels.push((a, depth + 1)),
                crate::Universe::Max(a, b) | crate::Universe::IMax(a, b) => {
                    levels.push((a, depth + 1));
                    levels.push((b, depth + 1));
                }
                crate::Universe::SProp
                | crate::Universe::Prop
                | crate::Universe::Type(_)
                | crate::Universe::Var(_) => {}
            }
            if levels.len() > remaining {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_kernel/reduction/shared_head/certified_order_controls.rs"]
mod certified_order_controls;

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_kernel/reduction/shared_head/certified_arithmetic_controls.rs"]
mod certified_arithmetic_controls;

// A completed canonical natural computation is also evidence of inequality to
// a natural literal. Unsupported evaluation remains None, never a rejection.
pub(crate) fn natural_literal_conversion(
    ctx: &Context,
    a: &Term,
    b: &Term,
    fuel: u64,
) -> Option<bool> {
    let (term, expected) = match (a, b) {
        (Term::Lit(Literal::Nat(n)), t) | (t, Term::Lit(Literal::Nat(n))) if !n.is_negative() => {
            (t, n)
        }
        _ => return None,
    };
    if !canonical_nat(ctx)
        || !ctx.simp_rules().is_empty()
        || ctx.checking_order_recurrence
        || ctx.has_pending_dependency(term)
        || !recurrence_body_admitted(term)
    {
        return None;
    }
    let mut r = HeadReducer::new(ctx, fuel.min(1 << 18));
    let id = r.store.intern(term);
    if !r.store.free_names_are_empty(id)
        || r.store.shapes(id).contains(NodeShape::Hole)
        || !r.store.eq_faithful(id)
    {
        return None;
    }
    let value = r.whnf(id)?;
    let mut cursor = r.nat_cursor(value)?;
    let mut total = crate::BigInt::zero();
    loop {
        r.fuel = r.fuel.checked_sub(1)?;
        match cursor {
            NatCursor::Pending(tail) => {
                let value = r.force(tail)?;
                cursor = r.nat_cursor(value)?;
            }
            NatCursor::Prefix { count, tail } => {
                total = r.arithmetic_result(Arithmetic::Add, &total, &count)?;
                match tail {
                    Some(tail) => cursor = NatCursor::Pending(tail),
                    None => return Some(&total == expected),
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_kernel/reduction/shared_head/certified_equality_controls.rs"]
mod certified_equality_controls;

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_kernel/reduction/shared_head/key_observation_controls.rs"]
mod key_observation_controls;

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_kernel/reduction/shared_head/certified_index_controls.rs"]
mod certified_index_controls;

#[cfg(test)]
thread_local! { static ORDER_CERTIFICATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_kernel/reduction/shared_head/certified_order_cache_controls.rs"]
mod certified_order_cache_controls;
