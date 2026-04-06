//! SVA Semantic Model
//!
//! Provides an AST for a subset of SystemVerilog Assertions, a parser
//! for that subset, and structural equivalence checking.
//!
//! This model enables the Z3 semantic equivalence pipeline:
//! FOL (from LOGOS) ↔ SVA (from LLM) checked for structural match.

/// SVA expression AST — models a useful subset of SystemVerilog Assertions.
#[derive(Debug, Clone)]
pub enum SvaExpr {
    /// Signal reference: `req`, `ack`, `data_out`
    Signal(String),
    /// Integer constant with bit width: `8'hFF`
    Const(u64, u32),
    /// Rising edge: `$rose(sig)`
    Rose(Box<SvaExpr>),
    /// Falling edge: `$fell(sig)`
    Fell(Box<SvaExpr>),
    /// Past value: `$past(sig, n)`
    Past(Box<SvaExpr>, u32),
    /// Conjunction: `a && b`
    And(Box<SvaExpr>, Box<SvaExpr>),
    /// Disjunction: `a || b`
    Or(Box<SvaExpr>, Box<SvaExpr>),
    /// Negation: `!a`
    Not(Box<SvaExpr>),
    /// Equality: `a == b`
    Eq(Box<SvaExpr>, Box<SvaExpr>),
    /// SVA implication: `a |-> b` (overlapping) or `a |=> b` (non-overlapping)
    Implication {
        antecedent: Box<SvaExpr>,
        consequent: Box<SvaExpr>,
        overlapping: bool,
    },
    /// Delay: `##[min:max] body`
    Delay {
        body: Box<SvaExpr>,
        min: u32,
        max: Option<u32>,
    },
    /// Sequence repetition: `body[*N]` or `body[*min:max]`
    Repetition {
        body: Box<SvaExpr>,
        min: u32,
        max: Option<u32>, // None = unbounded ($)
    },
    /// Strong eventually: `s_eventually(body)`
    SEventually(Box<SvaExpr>),
    /// Strong always: `s_always(body)`
    SAlways(Box<SvaExpr>),
    /// Stable: `$stable(sig)` — signal unchanged from previous cycle
    Stable(Box<SvaExpr>),
    /// Changed: `$changed(sig)` — signal changed from previous cycle
    Changed(Box<SvaExpr>),
    /// Disable condition: `disable iff (cond) body`
    DisableIff {
        condition: Box<SvaExpr>,
        body: Box<SvaExpr>,
    },
    /// Next time: `nexttime(body)` or `nexttime[N](body)`
    Nexttime(Box<SvaExpr>, u32),
    /// Conditional property: `if (cond) P else Q`
    IfElse {
        condition: Box<SvaExpr>,
        then_expr: Box<SvaExpr>,
        else_expr: Box<SvaExpr>,
    },
    // ── IEEE 1800 Extended Constructs (Sprint 1B) ──
    /// Inequality: `a != b`
    NotEq(Box<SvaExpr>, Box<SvaExpr>),
    /// Less than: `a < b`
    LessThan(Box<SvaExpr>, Box<SvaExpr>),
    /// Greater than: `a > b`
    GreaterThan(Box<SvaExpr>, Box<SvaExpr>),
    /// Less or equal: `a <= b`
    LessEqual(Box<SvaExpr>, Box<SvaExpr>),
    /// Greater or equal: `a >= b`
    GreaterEqual(Box<SvaExpr>, Box<SvaExpr>),
    /// Ternary: `cond ? a : b`
    Ternary {
        condition: Box<SvaExpr>,
        then_expr: Box<SvaExpr>,
        else_expr: Box<SvaExpr>,
    },
    /// Throughout: `sig throughout seq` — signal holds during entire sequence
    Throughout {
        signal: Box<SvaExpr>,
        sequence: Box<SvaExpr>,
    },
    /// Within: `seq1 within seq2` — first sequence completes within second
    Within {
        inner: Box<SvaExpr>,
        outer: Box<SvaExpr>,
    },
    /// First match: `first_match(seq)` — matches at first completion
    FirstMatch(Box<SvaExpr>),
    /// Intersect: `seq1 intersect seq2` — both sequences match with same length
    Intersect {
        left: Box<SvaExpr>,
        right: Box<SvaExpr>,
    },
    // ── IEEE 1800 System Functions (Audit) ──
    /// At most one bit set: `$onehot0(sig)` — popcount ≤ 1
    OneHot0(Box<SvaExpr>),
    /// Exactly one bit set: `$onehot(sig)` — popcount = 1
    OneHot(Box<SvaExpr>),
    /// Population count: `$countones(sig)` — returns integer
    CountOnes(Box<SvaExpr>),
    /// X/Z detection: `$isunknown(sig)` — always false in 2-state formal
    IsUnknown(Box<SvaExpr>),
    /// Sampled value: `$sampled(sig)` — identity in synchronous formal
    Sampled(Box<SvaExpr>),
    /// Bit width: `$bits(sig)` — returns integer
    Bits(Box<SvaExpr>),
    /// Ceiling log2: `$clog2(val)` — minimum bits to represent val
    Clog2(Box<SvaExpr>),
    // ── IEEE 1800 System Functions (Sprint 13) ──
    /// Generalized bit counting: `$countbits(sig, '0', '1', ...)` (IEEE 20.9)
    CountBits(Box<SvaExpr>, Vec<char>),
    /// Parameter bound check: `$isunbounded(param)` (IEEE 20.9)
    IsUnbounded(Box<SvaExpr>),
    // ── Advanced Sequences (Audit) ──
    /// Goto repetition: `sig[->N]` — N non-consecutive matches
    GotoRepetition {
        body: Box<SvaExpr>,
        count: u32,
    },
    /// Non-consecutive repetition: `sig[=N]` or `sig[=min:max]`
    NonConsecRepetition {
        body: Box<SvaExpr>,
        min: u32,
        max: Option<u32>,
    },
    // ── Property Abort Operators (Audit) ──
    /// Accept on: `accept_on(cond) body` — property passes if cond true
    AcceptOn {
        condition: Box<SvaExpr>,
        body: Box<SvaExpr>,
    },
    /// Reject on: `reject_on(cond) body` — property fails if cond true
    RejectOn {
        condition: Box<SvaExpr>,
        body: Box<SvaExpr>,
    },
    // ── Property Connectives (Sprint 1, IEEE 16.12.3-8) ──
    /// Property negation: `not property_expr` (IEEE 16.12.3)
    /// Distinct from boolean `!` — negates temporal property evaluation.
    /// Flips strength: not(weak) → strong, not(strong) → weak (IEEE 16.12.15).
    PropertyNot(Box<SvaExpr>),
    /// Property implication: `p implies q` (IEEE 16.12.8)
    /// Distinct from sequence `|->` — property-level `not p or q`.
    PropertyImplies(Box<SvaExpr>, Box<SvaExpr>),
    /// Property biconditional: `p iff q` (IEEE 16.12.8)
    /// Equivalent to `(p implies q) and (q implies p)`.
    PropertyIff(Box<SvaExpr>, Box<SvaExpr>),
    // ── LTL Temporal Operators (Sprint 2, IEEE 16.12.11-13) ──
    /// Weak unbounded always: `always p` (IEEE 16.12.11)
    /// Passes if trace ends (weak semantics).
    Always(Box<SvaExpr>),
    /// Weak bounded always: `always [m:n] p` (IEEE 16.12.11)
    /// max=None means $ (weak allows $).
    AlwaysBounded { body: Box<SvaExpr>, min: u32, max: Option<u32> },
    /// Strong bounded always: `s_always [m:n] p` (IEEE 16.12.11)
    /// All ticks must exist. NO $ allowed.
    SAlwaysBounded { body: Box<SvaExpr>, min: u32, max: u32 },
    /// Weak bounded eventually: `eventually [m:n] p` (IEEE 16.12.13)
    /// Range must be bounded (no $).
    EventuallyBounded { body: Box<SvaExpr>, min: u32, max: u32 },
    /// Strong bounded eventually: `s_eventually [m:n] p` (IEEE 16.12.13)
    /// CAN use $ (max=None means $).
    SEventuallyBounded { body: Box<SvaExpr>, min: u32, max: Option<u32> },
    /// Until operator with 4 variants (IEEE 16.12.12):
    /// `p until q` (weak, non-overlapping)
    /// `p s_until q` (strong, non-overlapping)
    /// `p until_with q` (weak, overlapping)
    /// `p s_until_with q` (strong, overlapping)
    Until { lhs: Box<SvaExpr>, rhs: Box<SvaExpr>, strong: bool, inclusive: bool },
    // ── Strong/Weak, Advanced Temporal & Sync Abort (Sprint 3, IEEE 16.12.2, 16.12.9-10, 16.12.14, 16.12.16) ──
    /// Strong sequence: `strong(seq)` (IEEE 16.12.2) — match must exist within bound
    Strong(Box<SvaExpr>),
    /// Weak sequence: `weak(seq)` (IEEE 16.12.2) — no match needed if bound exhausted
    Weak(Box<SvaExpr>),
    /// Strong nexttime: `s_nexttime(body)` or `s_nexttime[N](body)` (IEEE 16.12.10)
    SNexttime(Box<SvaExpr>, u32),
    /// Followed-by: `seq #-# prop` (overlapping) or `seq #=# prop` (non-overlapping) (IEEE 16.12.9)
    FollowedBy { antecedent: Box<SvaExpr>, consequent: Box<SvaExpr>, overlapping: bool },
    /// Property case: `case(expr) val: prop; ... default: prop; endcase` (IEEE 16.12.16)
    PropertyCase { expression: Box<SvaExpr>, items: Vec<(Vec<SvaExpr>, Box<SvaExpr>)>, default: Option<Box<SvaExpr>> },
    /// Synchronous accept abort: `sync_accept_on(cond) body` (IEEE 16.12.14)
    SyncAcceptOn { condition: Box<SvaExpr>, body: Box<SvaExpr> },
    /// Synchronous reject abort: `sync_reject_on(cond) body` (IEEE 16.12.14)
    SyncRejectOn { condition: Box<SvaExpr>, body: Box<SvaExpr> },
    // ── Sequence-Level AND & OR (Sprint 5, IEEE 16.9.5, 16.9.7) ──
    /// Sequence AND: both operands match, composite ends at whichever finishes last
    SequenceAnd(Box<SvaExpr>, Box<SvaExpr>),
    /// Sequence OR: at least one matches, composite match set is union
    SequenceOr(Box<SvaExpr>, Box<SvaExpr>),
    // ── Assertion Directives (Sprint 7, IEEE 16.2-4, 16.14) ──
    /// Immediate assertion: `assert(expr)`, `assert #0(expr)`, `assert final(expr)`
    ImmediateAssert {
        expression: Box<SvaExpr>,
        deferred: Option<ImmediateDeferred>,
    },
    // ── Complex Data Types (Sprint 13, IEEE 16.6, 20.9) ──
    /// Field access: `req.addr` or `req.header.id`
    FieldAccess { signal: Box<SvaExpr>, field: String },
    /// Enum literal: `ST_READ` or `state_t::READ`
    EnumLiteral { type_name: Option<String>, value: String },
    // ── Endpoint Methods (Sprint 14, IEEE 16.9.11) ──
    /// Triggered endpoint: `seq_name.triggered`
    Triggered(String),
    /// Matched endpoint: `seq_name.matched`
    Matched(String),
    // ── Bitwise Operators (Sprint 15, IEEE 16.6) ──
    BitAnd(Box<SvaExpr>, Box<SvaExpr>),
    BitOr(Box<SvaExpr>, Box<SvaExpr>),
    BitXor(Box<SvaExpr>, Box<SvaExpr>),
    BitNot(Box<SvaExpr>),
    ReductionAnd(Box<SvaExpr>),
    ReductionOr(Box<SvaExpr>),
    ReductionXor(Box<SvaExpr>),
    BitSelect { signal: Box<SvaExpr>, index: Box<SvaExpr> },
    PartSelect { signal: Box<SvaExpr>, high: u32, low: u32 },
    Concat(Vec<SvaExpr>),
    // ── Local Variables (Sprint 10, IEEE 16.10) ──
    /// Sequence match with local variable assignments:
    /// `(expr, v = rhs, w = rhs2)` — expression matches AND variables assigned
    SequenceAction {
        expression: Box<SvaExpr>,
        assignments: Vec<(String, Box<SvaExpr>)>,
    },
    /// Local variable reference within a sequence/property
    LocalVar(String),
    // ── Let Construct (Sprint 16, IEEE 11.12) ──
    /// const' cast: `const'(expr)` — freezes value at queue time
    ConstCast(Box<SvaExpr>),
    // ── Multi-Clock (Sprint 12, IEEE 16.13) ──
    /// Clocked expression: `@(posedge clk) body` — preserves clock annotation for multi-clock
    Clocked { clock: String, edge: ClockEdge, body: Box<SvaExpr> },
    // ── IEEE 1800-2023 Additions (Sprint 23) ──
    /// Array map method: `A.map(x) with (expr)` (IEEE 7.12, 2023)
    ArrayMap { array: Box<SvaExpr>, iterator: String, with_expr: Box<SvaExpr> },
    /// Type operator: `type(this)` (IEEE 6.23, 2023)
    TypeThis,
    /// Real literal constant: `1.5`, `1.2E3` (IEEE 5.7.2, 2023)
    RealConst(f64),
}

/// Clock edge type
#[derive(Debug, Clone, PartialEq)]
pub enum ClockEdge {
    Posedge,
    Negedge,
    Edge, // any edge
}

/// Deferred assertion timing (IEEE 16.4)
#[derive(Debug, Clone, PartialEq)]
pub enum ImmediateDeferred {
    /// `#0` — observed region
    Observed,
    /// `final` — final simulation phase
    Final,
}

/// Assertion directive kind (IEEE 16.14)
#[derive(Debug, Clone, PartialEq)]
pub enum SvaDirectiveKind {
    Assert,
    Assume,
    Cover,
    CoverSequence,
    Restrict,
}

/// A concurrent assertion directive (IEEE 16.14)
#[derive(Debug, Clone)]
pub struct SvaDirective {
    pub kind: SvaDirectiveKind,
    pub property: SvaExpr,
    pub label: Option<String>,
    pub clock: Option<String>,
    pub disable_iff: Option<SvaExpr>,
    pub action_pass: Option<String>,
    pub action_fail: Option<String>,
}

/// Port type for named sequence/property declarations (IEEE 16.8, 16.12)
#[derive(Debug, Clone, PartialEq)]
pub enum SvaPortType {
    Untyped,
    Bit,
    Sequence,
    Property,
}

/// A port in a named sequence or property declaration
#[derive(Debug, Clone)]
pub struct SvaPort {
    pub name: String,
    pub port_type: SvaPortType,
    pub default: Option<SvaExpr>,
}

/// Named sequence declaration (IEEE 16.8)
#[derive(Debug, Clone)]
pub struct SequenceDecl {
    pub name: String,
    pub ports: Vec<SvaPort>,
    pub body: SvaExpr,
}

/// Named property declaration (IEEE 16.12)
#[derive(Debug, Clone)]
pub struct PropertyDecl {
    pub name: String,
    pub ports: Vec<SvaPort>,
    pub body: SvaExpr,
}

/// Let declaration (IEEE 11.12) — pure expression substitution
#[derive(Debug, Clone)]
pub struct LetDecl {
    pub name: String,
    pub ports: Vec<SvaPort>,
    pub body: SvaExpr,
}

/// Dist weight kind (IEEE 18.5.4)
#[derive(Debug, Clone, PartialEq)]
pub enum DistKind {
    /// `:=` — weight per value
    PerValue,
    /// `:/` — weight distributed across range
    PerRange,
}

/// A dist item — value or range with weight
#[derive(Debug, Clone)]
pub struct DistItem {
    pub min: u64,
    pub max: Option<u64>,
    pub weight: u64,
    pub kind: DistKind,
}

/// Checker declaration (IEEE Chapter 17)
#[derive(Debug, Clone)]
pub struct CheckerDecl {
    pub name: String,
    pub ports: Vec<SvaPort>,
    pub rand_vars: Vec<RandVar>,
    pub assertions: Vec<SvaDirective>,
}

/// Type discriminant for random variables (IEEE 17.7, extended in 2023)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RandVarType {
    /// Bitvector with width: `rand bit [N-1:0]`
    BitVec(u32),
    /// IEEE 754 double: `rand real` (IEEE 1800-2023)
    Real,
}

/// Random variable in a checker (IEEE 17.7)
#[derive(Debug, Clone)]
pub struct RandVar {
    pub name: String,
    pub var_type: RandVarType,
    pub is_const: bool,
}

/// Resolve a sequence instance by substituting actuals for formals.
pub fn resolve_sequence_instance(
    decls: &[SequenceDecl],
    name: &str,
    args: &[SvaExpr],
) -> Result<SvaExpr, SvaParseError> {
    let decl = decls.iter().find(|d| d.name == name).ok_or_else(|| SvaParseError {
        message: format!("undeclared sequence: '{}'", name),
    })?;

    let expected = decl.ports.len();
    let provided = args.len();
    // Count ports without defaults to determine minimum args
    let min_args = decl.ports.iter().filter(|p| p.default.is_none()).count();
    if provided < min_args || provided > expected {
        return Err(SvaParseError {
            message: format!(
                "sequence '{}' expects {}-{} arguments, got {}",
                name, min_args, expected, provided
            ),
        });
    }

    let mut result = decl.body.clone();
    for (i, port) in decl.ports.iter().enumerate() {
        let actual = if i < args.len() {
            args[i].clone()
        } else if let Some(ref default) = port.default {
            default.clone()
        } else {
            return Err(SvaParseError {
                message: format!("missing argument for port '{}' in sequence '{}'", port.name, name),
            });
        };
        result = substitute_signal(&result, &port.name, &actual);
    }
    Ok(result)
}

/// Substitute all occurrences of signal `name` with `replacement` in an expression.
/// Handles all 78 SvaExpr variants recursively.
fn substitute_signal(expr: &SvaExpr, name: &str, replacement: &SvaExpr) -> SvaExpr {
    let sub = |e: &SvaExpr| Box::new(substitute_signal(e, name, replacement));
    let sub_vec = |v: &[SvaExpr]| v.iter().map(|e| substitute_signal(e, name, replacement)).collect::<Vec<_>>();

    match expr {
        // ── Terminals ──
        SvaExpr::Signal(s) if s == name => replacement.clone(),
        SvaExpr::Signal(_) | SvaExpr::Const(_, _) => expr.clone(),
        SvaExpr::LocalVar(s) if s == name => replacement.clone(),
        SvaExpr::LocalVar(_) => expr.clone(),
        SvaExpr::Triggered(s) => SvaExpr::Triggered(s.clone()),
        SvaExpr::Matched(s) => SvaExpr::Matched(s.clone()),
        SvaExpr::EnumLiteral { type_name, value } => SvaExpr::EnumLiteral {
            type_name: type_name.clone(), value: value.clone(),
        },

        // ── Unary wrappers ──
        SvaExpr::Rose(inner) => SvaExpr::Rose(sub(inner)),
        SvaExpr::Fell(inner) => SvaExpr::Fell(sub(inner)),
        SvaExpr::Not(inner) => SvaExpr::Not(sub(inner)),
        SvaExpr::Stable(inner) => SvaExpr::Stable(sub(inner)),
        SvaExpr::Changed(inner) => SvaExpr::Changed(sub(inner)),
        SvaExpr::SEventually(inner) => SvaExpr::SEventually(sub(inner)),
        SvaExpr::SAlways(inner) => SvaExpr::SAlways(sub(inner)),
        SvaExpr::Always(inner) => SvaExpr::Always(sub(inner)),
        SvaExpr::FirstMatch(inner) => SvaExpr::FirstMatch(sub(inner)),
        SvaExpr::Strong(inner) => SvaExpr::Strong(sub(inner)),
        SvaExpr::Weak(inner) => SvaExpr::Weak(sub(inner)),
        SvaExpr::PropertyNot(inner) => SvaExpr::PropertyNot(sub(inner)),
        SvaExpr::OneHot0(inner) => SvaExpr::OneHot0(sub(inner)),
        SvaExpr::OneHot(inner) => SvaExpr::OneHot(sub(inner)),
        SvaExpr::CountOnes(inner) => SvaExpr::CountOnes(sub(inner)),
        SvaExpr::IsUnknown(inner) => SvaExpr::IsUnknown(sub(inner)),
        SvaExpr::Sampled(inner) => SvaExpr::Sampled(sub(inner)),
        SvaExpr::Bits(inner) => SvaExpr::Bits(sub(inner)),
        SvaExpr::Clog2(inner) => SvaExpr::Clog2(sub(inner)),
        SvaExpr::IsUnbounded(inner) => SvaExpr::IsUnbounded(sub(inner)),
        SvaExpr::BitNot(inner) => SvaExpr::BitNot(sub(inner)),
        SvaExpr::ReductionAnd(inner) => SvaExpr::ReductionAnd(sub(inner)),
        SvaExpr::ReductionOr(inner) => SvaExpr::ReductionOr(sub(inner)),
        SvaExpr::ReductionXor(inner) => SvaExpr::ReductionXor(sub(inner)),
        SvaExpr::ConstCast(inner) => SvaExpr::ConstCast(sub(inner)),

        // ── Unary with extra data ──
        SvaExpr::Past(inner, n) => SvaExpr::Past(sub(inner), *n),
        SvaExpr::Nexttime(inner, n) => SvaExpr::Nexttime(sub(inner), *n),
        SvaExpr::SNexttime(inner, n) => SvaExpr::SNexttime(sub(inner), *n),
        SvaExpr::CountBits(inner, chars) => SvaExpr::CountBits(sub(inner), chars.clone()),
        SvaExpr::GotoRepetition { body, count } => SvaExpr::GotoRepetition {
            body: sub(body), count: *count,
        },

        // ── Binary ──
        SvaExpr::And(l, r) => SvaExpr::And(sub(l), sub(r)),
        SvaExpr::Or(l, r) => SvaExpr::Or(sub(l), sub(r)),
        SvaExpr::Eq(l, r) => SvaExpr::Eq(sub(l), sub(r)),
        SvaExpr::NotEq(l, r) => SvaExpr::NotEq(sub(l), sub(r)),
        SvaExpr::LessThan(l, r) => SvaExpr::LessThan(sub(l), sub(r)),
        SvaExpr::GreaterThan(l, r) => SvaExpr::GreaterThan(sub(l), sub(r)),
        SvaExpr::LessEqual(l, r) => SvaExpr::LessEqual(sub(l), sub(r)),
        SvaExpr::GreaterEqual(l, r) => SvaExpr::GreaterEqual(sub(l), sub(r)),
        SvaExpr::PropertyImplies(l, r) => SvaExpr::PropertyImplies(sub(l), sub(r)),
        SvaExpr::PropertyIff(l, r) => SvaExpr::PropertyIff(sub(l), sub(r)),
        SvaExpr::SequenceAnd(l, r) => SvaExpr::SequenceAnd(sub(l), sub(r)),
        SvaExpr::SequenceOr(l, r) => SvaExpr::SequenceOr(sub(l), sub(r)),
        SvaExpr::BitAnd(l, r) => SvaExpr::BitAnd(sub(l), sub(r)),
        SvaExpr::BitOr(l, r) => SvaExpr::BitOr(sub(l), sub(r)),
        SvaExpr::BitXor(l, r) => SvaExpr::BitXor(sub(l), sub(r)),

        // ── Struct-like with named fields ──
        SvaExpr::Implication { antecedent, consequent, overlapping } => SvaExpr::Implication {
            antecedent: sub(antecedent), consequent: sub(consequent), overlapping: *overlapping,
        },
        SvaExpr::Delay { body, min, max } => SvaExpr::Delay {
            body: sub(body), min: *min, max: *max,
        },
        SvaExpr::Repetition { body, min, max } => SvaExpr::Repetition {
            body: sub(body), min: *min, max: *max,
        },
        SvaExpr::NonConsecRepetition { body, min, max } => SvaExpr::NonConsecRepetition {
            body: sub(body), min: *min, max: *max,
        },
        SvaExpr::DisableIff { condition, body } => SvaExpr::DisableIff {
            condition: sub(condition), body: sub(body),
        },
        SvaExpr::IfElse { condition, then_expr, else_expr } => SvaExpr::IfElse {
            condition: sub(condition), then_expr: sub(then_expr), else_expr: sub(else_expr),
        },
        SvaExpr::Ternary { condition, then_expr, else_expr } => SvaExpr::Ternary {
            condition: sub(condition), then_expr: sub(then_expr), else_expr: sub(else_expr),
        },
        SvaExpr::Throughout { signal, sequence } => SvaExpr::Throughout {
            signal: sub(signal), sequence: sub(sequence),
        },
        SvaExpr::Within { inner, outer } => SvaExpr::Within {
            inner: sub(inner), outer: sub(outer),
        },
        SvaExpr::Intersect { left, right } => SvaExpr::Intersect {
            left: sub(left), right: sub(right),
        },
        SvaExpr::AcceptOn { condition, body } => SvaExpr::AcceptOn {
            condition: sub(condition), body: sub(body),
        },
        SvaExpr::RejectOn { condition, body } => SvaExpr::RejectOn {
            condition: sub(condition), body: sub(body),
        },
        SvaExpr::SyncAcceptOn { condition, body } => SvaExpr::SyncAcceptOn {
            condition: sub(condition), body: sub(body),
        },
        SvaExpr::SyncRejectOn { condition, body } => SvaExpr::SyncRejectOn {
            condition: sub(condition), body: sub(body),
        },
        SvaExpr::FollowedBy { antecedent, consequent, overlapping } => SvaExpr::FollowedBy {
            antecedent: sub(antecedent), consequent: sub(consequent), overlapping: *overlapping,
        },
        SvaExpr::Until { lhs, rhs, strong, inclusive } => SvaExpr::Until {
            lhs: sub(lhs), rhs: sub(rhs), strong: *strong, inclusive: *inclusive,
        },
        SvaExpr::AlwaysBounded { body, min, max } => SvaExpr::AlwaysBounded {
            body: sub(body), min: *min, max: *max,
        },
        SvaExpr::SAlwaysBounded { body, min, max } => SvaExpr::SAlwaysBounded {
            body: sub(body), min: *min, max: *max,
        },
        SvaExpr::EventuallyBounded { body, min, max } => SvaExpr::EventuallyBounded {
            body: sub(body), min: *min, max: *max,
        },
        SvaExpr::SEventuallyBounded { body, min, max } => SvaExpr::SEventuallyBounded {
            body: sub(body), min: *min, max: *max,
        },
        SvaExpr::FieldAccess { signal, field } => SvaExpr::FieldAccess {
            signal: sub(signal), field: field.clone(),
        },
        SvaExpr::BitSelect { signal, index } => SvaExpr::BitSelect {
            signal: sub(signal), index: sub(index),
        },
        SvaExpr::PartSelect { signal, high, low } => SvaExpr::PartSelect {
            signal: sub(signal), high: *high, low: *low,
        },
        SvaExpr::ImmediateAssert { expression, deferred } => SvaExpr::ImmediateAssert {
            expression: sub(expression), deferred: deferred.clone(),
        },

        // ── Vec children ──
        SvaExpr::Concat(items) => SvaExpr::Concat(sub_vec(items)),

        // ── Complex structures ──
        SvaExpr::PropertyCase { expression, items, default } => SvaExpr::PropertyCase {
            expression: sub(expression),
            items: items.iter().map(|(vals, prop)| {
                (sub_vec(vals), sub(prop))
            }).collect(),
            default: default.as_ref().map(|d| sub(d)),
        },
        SvaExpr::SequenceAction { expression, assignments } => SvaExpr::SequenceAction {
            expression: sub(expression),
            assignments: assignments.iter().map(|(var_name, rhs)| {
                (var_name.clone(), sub(rhs))
            }).collect(),
        },
        SvaExpr::Clocked { clock, edge, body } => SvaExpr::Clocked {
            clock: clock.clone(),
            edge: edge.clone(),
            body: sub(body),
        },
        SvaExpr::ArrayMap { array, iterator, with_expr } => SvaExpr::ArrayMap {
            array: sub(array),
            iterator: iterator.clone(),
            with_expr: sub(with_expr),
        },
        SvaExpr::TypeThis => SvaExpr::TypeThis,
        SvaExpr::RealConst(v) => SvaExpr::RealConst(*v),
    }
}

/// Parse error for SVA subset.
#[derive(Debug)]
pub struct SvaParseError {
    pub message: String,
}

impl std::fmt::Display for SvaParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SVA parse error: {}", self.message)
    }
}

/// Parse a concurrent assertion directive (IEEE 16.14).
///
/// Handles: `assert property(...)`, `assume property(...)`, `cover property(...)`,
/// `cover sequence(...)`, `restrict property(...)`.
/// Optionally prefixed by a label: `label: assert property(...)`.
/// Optionally contains action blocks: `assert property(p) $info("ok"); else $error("fail");`
pub fn parse_sva_directive(input: &str) -> Result<SvaDirective, SvaParseError> {
    let input = input.trim().trim_end_matches(';');
    let input = input.trim();

    // Check for label prefix: `label: directive...`
    let (label, rest) = if let Some(colon_pos) = input.find(':') {
        let potential_label = input[..colon_pos].trim();
        // Only treat as label if it's a simple identifier (no spaces, no keywords)
        if !potential_label.is_empty()
            && potential_label.chars().all(|c| c.is_alphanumeric() || c == '_')
            && !potential_label.starts_with("assert")
            && !potential_label.starts_with("assume")
            && !potential_label.starts_with("cover")
            && !potential_label.starts_with("restrict")
        {
            (Some(potential_label.to_string()), input[colon_pos + 1..].trim())
        } else {
            (None, input)
        }
    } else {
        (None, input)
    };

    // Determine directive kind
    let (kind, after_kind) = if rest.starts_with("assert property") {
        (SvaDirectiveKind::Assert, rest["assert property".len()..].trim())
    } else if rest.starts_with("assume property") {
        (SvaDirectiveKind::Assume, rest["assume property".len()..].trim())
    } else if rest.starts_with("cover sequence") {
        (SvaDirectiveKind::CoverSequence, rest["cover sequence".len()..].trim())
    } else if rest.starts_with("cover property") {
        (SvaDirectiveKind::Cover, rest["cover property".len()..].trim())
    } else if rest.starts_with("restrict property") {
        (SvaDirectiveKind::Restrict, rest["restrict property".len()..].trim())
    } else {
        return Err(SvaParseError {
            message: format!("expected assertion directive, got: '{}'", rest),
        });
    };

    // Parse the property expression in parentheses
    if !after_kind.starts_with('(') {
        return Err(SvaParseError {
            message: format!("expected '(' after directive keyword, got: '{}'", after_kind),
        });
    }
    let close = find_balanced_close(after_kind, 0).ok_or_else(|| SvaParseError {
        message: "unbalanced parentheses in directive".to_string(),
    })?;
    let prop_str = &after_kind[1..close];
    let after_prop = after_kind[close + 1..].trim();

    // Parse optional clock: @(posedge clk)
    let (clock, prop_rest) = if prop_str.trim().starts_with("@(") {
        if let Some(clock_close) = prop_str.find(')') {
            let clock_str = prop_str[2..clock_close].trim().to_string();
            (Some(clock_str), prop_str[clock_close + 1..].trim())
        } else {
            (None, prop_str.trim())
        }
    } else {
        (None, prop_str.trim())
    };

    // Parse optional disable iff
    let (disable_iff, prop_body) = if prop_rest.starts_with("disable iff") {
        let after_disable = prop_rest["disable iff".len()..].trim();
        if after_disable.starts_with('(') {
            if let Some(di_close) = find_balanced_close(after_disable, 0) {
                let di_str = &after_disable[1..di_close];
                let body = after_disable[di_close + 1..].trim();
                (Some(parse_sva(di_str)?), body)
            } else {
                (None, prop_rest)
            }
        } else {
            (None, prop_rest)
        }
    } else {
        (None, prop_rest)
    };

    let property = parse_sva(prop_body)?;

    // Parse optional action blocks after the closing paren
    let mut action_pass = None;
    let mut action_fail = None;
    let remaining = after_prop;
    if !remaining.is_empty() {
        // Look for `else` keyword to separate pass/fail, but skip occurrences
        // inside string literals (both regular "..." and triple-quoted """...""").
        if let Some(else_pos) = find_else_outside_strings(remaining) {
            let pass_part = remaining[..else_pos].trim();
            let fail_part = remaining[else_pos + 4..].trim();
            if !pass_part.is_empty() {
                action_pass = Some(pass_part.trim_end_matches(';').trim().to_string());
            }
            if !fail_part.is_empty() {
                action_fail = Some(fail_part.trim_end_matches(';').trim().to_string());
            }
        } else if remaining.starts_with("$") || remaining.starts_with("else") {
            let part = remaining.trim_end_matches(';').trim();
            if remaining.starts_with("else") {
                action_fail = Some(part["else".len()..].trim().to_string());
            } else {
                action_pass = Some(part.to_string());
            }
        }
    }

    // Restrict property cannot have action blocks (IEEE 16.14.4)
    if kind == SvaDirectiveKind::Restrict && (action_pass.is_some() || action_fail.is_some()) {
        return Err(SvaParseError {
            message: "restrict property cannot have action blocks".to_string(),
        });
    }

    Ok(SvaDirective {
        kind,
        property,
        label,
        clock,
        disable_iff,
        action_pass,
        action_fail,
    })
}

/// Parse a subset of SVA text into an SvaExpr.
///
/// Supports: signals, `$rose()`, `$fell()`, `s_eventually()`,
/// `!()`, `&&`, `||`, `==`, `|->`, `|=>`.
pub fn parse_sva(input: &str) -> Result<SvaExpr, SvaParseError> {
    let input = input.trim();

    // Parse clock sensitivity prefix: @(posedge clk), @(negedge clk), @(edge clk)
    // IEEE 16.13: preserve clock annotation as Clocked variant for multi-clock support
    if input.starts_with("@(") {
        if let Some(pos) = input.find(')') {
            let clock_spec = input[2..pos].trim();
            let rest = input[pos + 1..].trim();

            let (edge, clock_name) = if clock_spec.starts_with("posedge ") {
                (ClockEdge::Posedge, clock_spec[8..].trim().to_string())
            } else if clock_spec.starts_with("negedge ") {
                (ClockEdge::Negedge, clock_spec[8..].trim().to_string())
            } else if clock_spec.starts_with("edge ") {
                (ClockEdge::Edge, clock_spec[5..].trim().to_string())
            } else {
                // Unknown edge type — treat as posedge
                (ClockEdge::Posedge, clock_spec.to_string())
            };

            let body = parse_toplevel(rest)?;
            return Ok(SvaExpr::Clocked {
                clock: clock_name,
                edge,
                body: Box::new(body),
            });
        }
    }

    parse_toplevel(input)
}

fn parse_toplevel(input: &str) -> Result<SvaExpr, SvaParseError> {
    let input = input.trim();

    // Immediate assertions: assert(expr), assert #0(expr), assert final(expr) (Sprint 7)
    if input.starts_with("assert ") || input.starts_with("assert(") || input.starts_with("assert#") {
        let rest = input["assert".len()..].trim();
        if rest.starts_with("#0") {
            let body = rest[2..].trim();
            if body.starts_with('(') {
                if let Some(close) = find_balanced_close(body, 0) {
                    let inner = &body[1..close];
                    return Ok(SvaExpr::ImmediateAssert {
                        expression: Box::new(parse_implication(inner)?),
                        deferred: Some(ImmediateDeferred::Observed),
                    });
                }
            }
        } else if rest.starts_with("final") {
            let body = rest[5..].trim();
            if body.starts_with('(') {
                if let Some(close) = find_balanced_close(body, 0) {
                    let inner = &body[1..close];
                    return Ok(SvaExpr::ImmediateAssert {
                        expression: Box::new(parse_implication(inner)?),
                        deferred: Some(ImmediateDeferred::Final),
                    });
                }
            }
        } else if rest.starts_with('(') {
            if let Some(close) = find_balanced_close(rest, 0) {
                let inner = &rest[1..close];
                return Ok(SvaExpr::ImmediateAssert {
                    expression: Box::new(parse_implication(inner)?),
                    deferred: None,
                });
            }
        }
    }

    // sync_accept_on(cond) body — synchronous property abort (Sprint 3)
    if input.starts_with("sync_accept_on") {
        let rest = input["sync_accept_on".len()..].trim();
        if rest.starts_with('(') {
            if let Some(close) = find_balanced_close(rest, 0) {
                let cond = &rest[1..close];
                let body = rest[close + 1..].trim();
                return Ok(SvaExpr::SyncAcceptOn {
                    condition: Box::new(parse_implication(cond)?),
                    body: Box::new(parse_toplevel(body)?),
                });
            }
        }
    }

    // sync_reject_on(cond) body — synchronous property abort (Sprint 3)
    if input.starts_with("sync_reject_on") {
        let rest = input["sync_reject_on".len()..].trim();
        if rest.starts_with('(') {
            if let Some(close) = find_balanced_close(rest, 0) {
                let cond = &rest[1..close];
                let body = rest[close + 1..].trim();
                return Ok(SvaExpr::SyncRejectOn {
                    condition: Box::new(parse_implication(cond)?),
                    body: Box::new(parse_toplevel(body)?),
                });
            }
        }
    }

    // accept_on(cond) body — property abort operators
    if input.starts_with("accept_on") {
        let rest = input["accept_on".len()..].trim();
        if rest.starts_with('(') {
            if let Some(close) = find_balanced_close(rest, 0) {
                let cond = &rest[1..close];
                let body = rest[close + 1..].trim();
                return Ok(SvaExpr::AcceptOn {
                    condition: Box::new(parse_implication(cond)?),
                    body: Box::new(parse_toplevel(body)?),
                });
            }
        }
    }

    // reject_on(cond) body
    if input.starts_with("reject_on") {
        let rest = input["reject_on".len()..].trim();
        if rest.starts_with('(') {
            if let Some(close) = find_balanced_close(rest, 0) {
                let cond = &rest[1..close];
                let body = rest[close + 1..].trim();
                return Ok(SvaExpr::RejectOn {
                    condition: Box::new(parse_implication(cond)?),
                    body: Box::new(parse_toplevel(body)?),
                });
            }
        }
    }

    // disable iff (cond) body — must be checked before implication
    if input.starts_with("disable iff") {
        let rest = input["disable iff".len()..].trim();
        if rest.starts_with('(') {
            if let Some(close) = find_balanced_close(rest, 0) {
                let cond = &rest[1..close];
                let body = rest[close + 1..].trim();
                return Ok(SvaExpr::DisableIff {
                    condition: Box::new(parse_implication(cond)?),
                    body: Box::new(parse_implication(body)?),
                });
            }
        }
    }

    // if (cond) P else Q — must be checked before implication
    if input.starts_with("if ") || input.starts_with("if(") {
        let rest = input[2..].trim();
        if rest.starts_with('(') {
            if let Some(close) = find_balanced_close(rest, 0) {
                let cond = &rest[1..close];
                let after_cond = rest[close + 1..].trim();
                if let Some(else_pos) = find_else_keyword(after_cond) {
                    let then_part = after_cond[..else_pos].trim();
                    let else_part = after_cond[else_pos + 4..].trim();
                    return Ok(SvaExpr::IfElse {
                        condition: Box::new(parse_implication(cond)?),
                        then_expr: Box::new(parse_implication(then_part)?),
                        else_expr: Box::new(parse_implication(else_part)?),
                    });
                } else {
                    return Ok(SvaExpr::IfElse {
                        condition: Box::new(parse_implication(cond)?),
                        then_expr: Box::new(parse_implication(after_cond)?),
                        else_expr: Box::new(SvaExpr::Signal("1".to_string())),
                    });
                }
            }
        }
    }

    parse_implication(input)
}

fn parse_implication(input: &str) -> Result<SvaExpr, SvaParseError> {
    // Check for |-> or |=> or #-# or #=#
    // Scan not inside parentheses
    let mut depth = 0i32;
    let chars: Vec<char> = input.chars().collect();
    for i in 0..chars.len().saturating_sub(2) {
        match chars[i] {
            '(' => depth += 1,
            ')' => depth -= 1,
            '|' if depth == 0 => {
                if i + 2 < chars.len() && chars[i + 1] == '-' && chars[i + 2] == '>' {
                    let lhs = input[..i].trim();
                    let rhs = input[i + 3..].trim();
                    return Ok(SvaExpr::Implication {
                        antecedent: Box::new(parse_property_implies(lhs)?),
                        consequent: Box::new(parse_property_implies(rhs)?),
                        overlapping: true,
                    });
                }
                if i + 2 < chars.len() && chars[i + 1] == '=' && chars[i + 2] == '>' {
                    let lhs = input[..i].trim();
                    let rhs = input[i + 3..].trim();
                    return Ok(SvaExpr::Implication {
                        antecedent: Box::new(parse_property_implies(lhs)?),
                        consequent: Box::new(parse_property_implies(rhs)?),
                        overlapping: false,
                    });
                }
            }
            // #-# (followed-by overlapping) and #=# (followed-by non-overlapping)
            '#' if depth == 0 && i + 2 < chars.len() => {
                if chars[i + 1] == '-' && chars[i + 2] == '#' {
                    let lhs = input[..i].trim();
                    let rhs = input[i + 3..].trim();
                    if !lhs.is_empty() {
                        return Ok(SvaExpr::FollowedBy {
                            antecedent: Box::new(parse_property_implies(lhs)?),
                            consequent: Box::new(parse_property_implies(rhs)?),
                            overlapping: true,
                        });
                    }
                }
                if chars[i + 1] == '=' && chars[i + 2] == '#' {
                    let lhs = input[..i].trim();
                    let rhs = input[i + 3..].trim();
                    if !lhs.is_empty() {
                        return Ok(SvaExpr::FollowedBy {
                            antecedent: Box::new(parse_property_implies(lhs)?),
                            consequent: Box::new(parse_property_implies(rhs)?),
                            overlapping: false,
                        });
                    }
                }
            }
            _ => {}
        }
    }
    parse_property_implies(input)
}

/// Parse property-level `implies` (IEEE 16.12.8).
/// Lower precedence than `iff`, higher than `|->` / `|=>`.
fn parse_property_implies(input: &str) -> Result<SvaExpr, SvaParseError> {
    let input = input.trim();
    if let Some(pos) = find_keyword_at_depth_0(input, "implies") {
        let lhs = input[..pos].trim();
        let rhs = input[pos + 7..].trim();
        return Ok(SvaExpr::PropertyImplies(
            Box::new(parse_property_iff(lhs)?),
            Box::new(parse_property_implies(rhs)?), // right-associative
        ));
    }
    parse_property_iff(input)
}

/// Parse property-level `iff` (IEEE 16.12.8).
/// Higher precedence than `implies`, lower than `until`.
fn parse_property_iff(input: &str) -> Result<SvaExpr, SvaParseError> {
    let input = input.trim();
    if let Some(pos) = find_keyword_at_depth_0(input, "iff") {
        let lhs = input[..pos].trim();
        let rhs = input[pos + 3..].trim();
        return Ok(SvaExpr::PropertyIff(
            Box::new(parse_until(lhs)?),
            Box::new(parse_property_iff(rhs)?), // right-associative
        ));
    }
    parse_until(input)
}

/// Parse `until` / `s_until` / `until_with` / `s_until_with` (IEEE 16.12.12).
/// Higher precedence than `iff`, lower than `||`.
fn parse_until(input: &str) -> Result<SvaExpr, SvaParseError> {
    let input = input.trim();
    // Check in order: longest keywords first to avoid partial matches
    for (keyword, strong, inclusive) in &[
        ("s_until_with", true, true),
        ("until_with", false, true),
        ("s_until", true, false),
        ("until", false, false),
    ] {
        if let Some(pos) = find_keyword_at_depth_0(input, keyword) {
            let lhs = input[..pos].trim();
            let rhs = input[pos + keyword.len()..].trim();
            return Ok(SvaExpr::Until {
                lhs: Box::new(parse_or(lhs)?),
                rhs: Box::new(parse_until(rhs)?), // right-associative
                strong: *strong,
                inclusive: *inclusive,
            });
        }
    }
    parse_or(input)
}

fn parse_or(input: &str) -> Result<SvaExpr, SvaParseError> {
    let mut depth = 0i32;
    let chars: Vec<char> = input.chars().collect();
    for i in 0..chars.len().saturating_sub(1) {
        match chars[i] {
            '(' => depth += 1,
            ')' => depth -= 1,
            '|' if depth == 0 && i + 1 < chars.len() && chars[i + 1] == '|' => {
                let lhs = input[..i].trim();
                let rhs = input[i + 2..].trim();
                return Ok(SvaExpr::Or(
                    Box::new(parse_seq_ops(lhs)?),
                    Box::new(parse_or(rhs)?),
                ));
            }
            _ => {}
        }
    }
    parse_seq_ops(input)
}

/// Parse sequence-level operators: `and`, `or`, `throughout`, `within`, `intersect`.
/// `and` / `or` are sequence-level (thread/union semantics), distinct from `&&` / `||`.
/// These bind tighter than `||` but looser than `&&`.
fn parse_seq_ops(input: &str) -> Result<SvaExpr, SvaParseError> {
    let input_trimmed = input.trim();

    // Check for keyword operators at depth 0
    // Must scan for these as whole words (not inside identifiers)
    // Check `throughout`, `within`, `intersect` first (higher precedence than `and`/`or`)
    for keyword in &["throughout", "within", "intersect"] {
        if let Some(pos) = find_keyword_at_depth_0(input_trimmed, keyword) {
            let lhs = input_trimmed[..pos].trim();
            let rhs = input_trimmed[pos + keyword.len()..].trim();
            return match *keyword {
                "throughout" => Ok(SvaExpr::Throughout {
                    signal: Box::new(parse_and(lhs)?),
                    sequence: Box::new(parse_and(rhs)?),
                }),
                "within" => Ok(SvaExpr::Within {
                    inner: Box::new(parse_and(lhs)?),
                    outer: Box::new(parse_and(rhs)?),
                }),
                "intersect" => Ok(SvaExpr::Intersect {
                    left: Box::new(parse_and(lhs)?),
                    right: Box::new(parse_and(rhs)?),
                }),
                _ => unreachable!(),
            };
        }
    }

    // Sequence-level `or` (IEEE 16.9.7) — union semantics
    // Check before `and` since `or` has lower precedence
    if let Some(pos) = find_keyword_at_depth_0(input_trimmed, "or") {
        let lhs = input_trimmed[..pos].trim();
        let rhs = input_trimmed[pos + 2..].trim();
        return Ok(SvaExpr::SequenceOr(
            Box::new(parse_seq_and(lhs)?),
            Box::new(parse_seq_ops(rhs)?), // right-associative
        ));
    }

    parse_seq_and(input)
}

/// Parse sequence-level `and` (IEEE 16.9.5) — thread semantics.
/// Higher precedence than sequence `or`, lower than `&&`.
fn parse_seq_and(input: &str) -> Result<SvaExpr, SvaParseError> {
    let input_trimmed = input.trim();
    if let Some(pos) = find_keyword_at_depth_0(input_trimmed, "and") {
        let lhs = input_trimmed[..pos].trim();
        let rhs = input_trimmed[pos + 3..].trim();
        return Ok(SvaExpr::SequenceAnd(
            Box::new(parse_and(lhs)?),
            Box::new(parse_seq_and(rhs)?), // right-associative
        ));
    }
    parse_and(input)
}

/// Find a keyword at parenthesis depth 0, respecting word boundaries.
fn find_keyword_at_depth_0(input: &str, keyword: &str) -> Option<usize> {
    let mut depth = 0i32;
    let bytes = input.as_bytes();
    let klen = keyword.len();
    for i in 0..input.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => depth -= 1,
            _ if depth == 0 && i + klen <= input.len() => {
                if &input[i..i + klen] == keyword {
                    // Check word boundaries
                    let before_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
                    let after_ok = i + klen >= input.len() || !bytes[i + klen].is_ascii_alphanumeric();
                    if before_ok && after_ok {
                        return Some(i);
                    }
                }
            }
            _ => {}
        }
    }
    None
}

fn parse_and(input: &str) -> Result<SvaExpr, SvaParseError> {
    let mut depth = 0i32;
    let chars: Vec<char> = input.chars().collect();
    for i in 0..chars.len().saturating_sub(1) {
        match chars[i] {
            '(' => depth += 1,
            ')' => depth -= 1,
            '&' if depth == 0 && i + 1 < chars.len() && chars[i + 1] == '&' => {
                let lhs = input[..i].trim();
                let rhs = input[i + 2..].trim();
                return Ok(SvaExpr::And(
                    Box::new(parse_sequence(lhs)?),
                    Box::new(parse_and(rhs)?),
                ));
            }
            _ => {}
        }
    }
    parse_sequence(input)
}

/// Parse infix sequence concatenation: `req ##N ack` or `req ##[min:max] ack`.
/// In IEEE 1800, `##` between two expressions is a sequence delay operator.
/// This binds tighter than `&&` but looser than `==`/`!=`.
fn parse_sequence(input: &str) -> Result<SvaExpr, SvaParseError> {
    let input = input.trim();
    let bytes = input.as_bytes();
    let mut depth = 0i32;

    // Scan for infix `##` at depth 0 (not at position 0 — that's prefix delay)
    // Start from i=0 to track parens, but only match ## when i > 0
    for i in 0..input.len().saturating_sub(1) {
        match bytes[i] {
            b'(' => { depth += 1; continue; }
            b')' => { depth -= 1; continue; }
            b'#' if depth == 0 && i > 0 && i + 1 < input.len() && bytes[i + 1] == b'#' => {
                // Found `##` not at start — this is infix sequence concatenation
                let lhs = input[..i].trim();
                if lhs.is_empty() { continue; }
                let delay_and_rhs = &input[i..]; // starts with "##..."
                // Parse the delay part: ##N or ##[min:max]
                let rest = &delay_and_rhs[2..];
                if rest.starts_with('[') {
                    // ##[min:max] rhs
                    if let Some(bracket_end) = rest.find(']') {
                        let range_str = &rest[1..bracket_end];
                        let rhs = rest[bracket_end + 1..].trim();
                        let parts: Vec<&str> = range_str.split(':').collect();
                        if parts.len() == 2 {
                            let min = parts[0].trim().parse::<u32>().unwrap_or(0);
                            let max_str = parts[1].trim();
                            let max = if max_str == "$" {
                                None // $ = unbounded
                            } else {
                                Some(max_str.parse::<u32>().unwrap_or(0))
                            };
                            return Ok(SvaExpr::Implication {
                                antecedent: Box::new(parse_eq(lhs)?),
                                consequent: Box::new(SvaExpr::Delay {
                                    body: Box::new(parse_sequence(rhs)?),
                                    min,
                                    max,
                                }),
                                overlapping: true,
                            });
                        }
                    }
                } else {
                    // ##N rhs — exact delay
                    let mut num_end = 0;
                    for c in rest.chars() {
                        if c.is_ascii_digit() { num_end += 1; } else { break; }
                    }
                    if num_end > 0 {
                        let n = rest[..num_end].parse::<u32>().unwrap_or(0);
                        let rhs = rest[num_end..].trim();
                        return Ok(SvaExpr::Implication {
                            antecedent: Box::new(parse_eq(lhs)?),
                            consequent: Box::new(SvaExpr::Delay {
                                body: Box::new(parse_sequence(rhs)?),
                                min: n,
                                max: Some(n), // exact: min == max
                            }),
                            overlapping: true,
                        });
                    }
                }
            }
            _ => {}
        }
    }
    parse_eq(input)
}

fn parse_eq(input: &str) -> Result<SvaExpr, SvaParseError> {
    let mut depth = 0i32;
    let chars: Vec<char> = input.chars().collect();
    let len = chars.len();

    // Scan for ternary: `cond ? then : else` (lowest precedence in this group)
    for i in 0..len {
        match chars[i] {
            '(' => depth += 1,
            ')' => depth -= 1,
            '?' if depth == 0 => {
                let cond = input[..i].trim();
                let rest = &input[i + 1..];
                // Find the matching ':'
                let mut d2 = 0i32;
                for j in 0..rest.len() {
                    match rest.as_bytes()[j] {
                        b'(' => d2 += 1,
                        b')' => d2 -= 1,
                        b':' if d2 == 0 => {
                            let then_part = rest[..j].trim();
                            let else_part = rest[j + 1..].trim();
                            return Ok(SvaExpr::Ternary {
                                condition: Box::new(parse_eq(cond)?),
                                then_expr: Box::new(parse_eq(then_part)?),
                                else_expr: Box::new(parse_eq(else_part)?),
                            });
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    depth = 0;
    // Scan for comparison operators: ==, !=, <=, >=, <, >
    // Must check two-char operators before single-char ones
    // Track both () and [] depth so [-> and [= don't conflict with < and >
    for i in 0..len {
        match chars[i] {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            _ if depth != 0 => {}
            '!' if i + 1 < len && chars[i + 1] == '=' => {
                let lhs = input[..i].trim();
                let rhs = input[i + 2..].trim();
                return Ok(SvaExpr::NotEq(
                    Box::new(parse_unary(lhs)?),
                    Box::new(parse_unary(rhs)?),
                ));
            }
            '=' if i + 1 < len && chars[i + 1] == '=' => {
                let lhs = input[..i].trim();
                let rhs = input[i + 2..].trim();
                return Ok(SvaExpr::Eq(
                    Box::new(parse_unary(lhs)?),
                    Box::new(parse_unary(rhs)?),
                ));
            }
            '<' if i + 1 < len && chars[i + 1] == '=' => {
                let lhs = input[..i].trim();
                let rhs = input[i + 2..].trim();
                return Ok(SvaExpr::LessEqual(
                    Box::new(parse_unary(lhs)?),
                    Box::new(parse_unary(rhs)?),
                ));
            }
            '>' if i + 1 < len && chars[i + 1] == '=' => {
                let lhs = input[..i].trim();
                let rhs = input[i + 2..].trim();
                return Ok(SvaExpr::GreaterEqual(
                    Box::new(parse_unary(lhs)?),
                    Box::new(parse_unary(rhs)?),
                ));
            }
            '<' if depth == 0 => {
                let lhs = input[..i].trim();
                let rhs = input[i + 1..].trim();
                return Ok(SvaExpr::LessThan(
                    Box::new(parse_unary(lhs)?),
                    Box::new(parse_unary(rhs)?),
                ));
            }
            '>' if depth == 0 => {
                let lhs = input[..i].trim();
                let rhs = input[i + 1..].trim();
                return Ok(SvaExpr::GreaterThan(
                    Box::new(parse_unary(lhs)?),
                    Box::new(parse_unary(rhs)?),
                ));
            }
            _ => {}
        }
    }
    parse_unary(input)
}

fn parse_unary(input: &str) -> Result<SvaExpr, SvaParseError> {
    let input = input.trim();

    // strong(seq) — sequence must complete within bound (Sprint 3)
    if let Some(result) = try_parse_function_call(input, "strong", |inner| {
        Ok(SvaExpr::Strong(Box::new(parse_implication(inner)?)))
    })? { return Ok(result); }

    // weak(seq) — sequence may not complete (Sprint 3)
    if let Some(result) = try_parse_function_call(input, "weak", |inner| {
        Ok(SvaExpr::Weak(Box::new(parse_implication(inner)?)))
    })? { return Ok(result); }

    // s_nexttime[N](body) — with explicit count (Sprint 3)
    if input.starts_with("s_nexttime[") {
        if let Some(bracket_end) = input.find(']') {
            let n_str = &input[11..bracket_end];
            if let Ok(n) = n_str.parse::<u32>() {
                let rest = input[bracket_end + 1..].trim();
                if rest.starts_with('(') {
                    if let Some(close) = find_balanced_close(rest, 0) {
                        let inner = &rest[1..close];
                        return Ok(SvaExpr::SNexttime(
                            Box::new(parse_implication(inner.trim())?),
                            n,
                        ));
                    }
                }
            }
        }
    }

    // s_nexttime(body) — default count = 1 (Sprint 3)
    if let Some(result) = try_parse_function_call(input, "s_nexttime", |inner| {
        Ok(SvaExpr::SNexttime(Box::new(parse_implication(inner)?), 1))
    })? { return Ok(result); }

    // Property negation: `not expr` (IEEE 16.12.3)
    // Must be before `!` check. `not` is a keyword (word boundary checked).
    if input.starts_with("not ") || input.starts_with("not(") {
        let rest = input[3..].trim();
        return Ok(SvaExpr::PropertyNot(Box::new(parse_unary(rest)?)));
    }

    // always [m:n] body — bounded weak always (IEEE 16.12.11)
    // Must check before `always ` (unbounded) to avoid consuming the bracket
    if input.starts_with("always [") || input.starts_with("always[") {
        let rest = input["always".len()..].trim();
        if rest.starts_with('[') {
            if let Some(bracket_end) = rest.find(']') {
                let range_str = &rest[1..bracket_end];
                let body_str = rest[bracket_end + 1..].trim();
                let parts: Vec<&str> = range_str.split(':').collect();
                if parts.len() == 2 {
                    let min = parts[0].trim().parse::<u32>().map_err(|_| SvaParseError {
                        message: format!("invalid always min: '{}'", parts[0]),
                    })?;
                    let max_str = parts[1].trim();
                    let max = if max_str == "$" {
                        None // weak allows $
                    } else {
                        Some(max_str.parse::<u32>().map_err(|_| SvaParseError {
                            message: format!("invalid always max: '{}'", max_str),
                        })?)
                    };
                    return Ok(SvaExpr::AlwaysBounded {
                        body: Box::new(parse_unary(body_str)?),
                        min,
                        max,
                    });
                }
            }
        }
    }

    // always body — unbounded weak always (IEEE 16.12.11)
    if input.starts_with("always ") || input.starts_with("always(") {
        let rest = input["always".len()..].trim();
        return Ok(SvaExpr::Always(Box::new(parse_unary(rest)?)));
    }

    // s_always [m:n] body — bounded strong always (IEEE 16.12.11)
    // Must check BEFORE the existing s_always() function-call parse
    if input.starts_with("s_always [") || input.starts_with("s_always[") {
        let rest = input["s_always".len()..].trim();
        if rest.starts_with('[') {
            if let Some(bracket_end) = rest.find(']') {
                let range_str = &rest[1..bracket_end];
                let body_str = rest[bracket_end + 1..].trim();
                let parts: Vec<&str> = range_str.split(':').collect();
                if parts.len() == 2 {
                    let min = parts[0].trim().parse::<u32>().map_err(|_| SvaParseError {
                        message: format!("invalid s_always min: '{}'", parts[0]),
                    })?;
                    let max_str = parts[1].trim();
                    if max_str == "$" {
                        return Err(SvaParseError {
                            message: "s_always range must be bounded ($ not allowed)".to_string(),
                        });
                    }
                    let max = max_str.parse::<u32>().map_err(|_| SvaParseError {
                        message: format!("invalid s_always max: '{}'", max_str),
                    })?;
                    return Ok(SvaExpr::SAlwaysBounded {
                        body: Box::new(parse_unary(body_str)?),
                        min,
                        max,
                    });
                }
            }
        }
    }

    // eventually [m:n] body — bounded weak eventually (IEEE 16.12.13)
    if input.starts_with("eventually [") || input.starts_with("eventually[") {
        let rest = input["eventually".len()..].trim();
        if rest.starts_with('[') {
            if let Some(bracket_end) = rest.find(']') {
                let range_str = &rest[1..bracket_end];
                let body_str = rest[bracket_end + 1..].trim();
                let parts: Vec<&str> = range_str.split(':').collect();
                if parts.len() == 2 {
                    let min = parts[0].trim().parse::<u32>().map_err(|_| SvaParseError {
                        message: format!("invalid eventually min: '{}'", parts[0]),
                    })?;
                    let max_str = parts[1].trim();
                    if max_str == "$" {
                        return Err(SvaParseError {
                            message: "weak eventually range must be bounded ($ not allowed)".to_string(),
                        });
                    }
                    let max = max_str.parse::<u32>().map_err(|_| SvaParseError {
                        message: format!("invalid eventually max: '{}'", max_str),
                    })?;
                    return Ok(SvaExpr::EventuallyBounded {
                        body: Box::new(parse_unary(body_str)?),
                        min,
                        max,
                    });
                }
            }
        }
    }

    // s_eventually [m:n] body — bounded strong eventually (IEEE 16.12.13)
    // Must check BEFORE the existing s_eventually() function-call parse
    if input.starts_with("s_eventually [") || input.starts_with("s_eventually[") {
        let rest = input["s_eventually".len()..].trim();
        if rest.starts_with('[') {
            if let Some(bracket_end) = rest.find(']') {
                let range_str = &rest[1..bracket_end];
                let body_str = rest[bracket_end + 1..].trim();
                let parts: Vec<&str> = range_str.split(':').collect();
                if parts.len() == 2 {
                    let min = parts[0].trim().parse::<u32>().map_err(|_| SvaParseError {
                        message: format!("invalid s_eventually min: '{}'", parts[0]),
                    })?;
                    let max_str = parts[1].trim();
                    let max = if max_str == "$" {
                        None // strong eventually CAN use $
                    } else {
                        Some(max_str.parse::<u32>().map_err(|_| SvaParseError {
                            message: format!("invalid s_eventually max: '{}'", max_str),
                        })?)
                    };
                    return Ok(SvaExpr::SEventuallyBounded {
                        body: Box::new(parse_unary(body_str)?),
                        min,
                        max,
                    });
                }
            }
        }
    }

    // Delay: ##N body or ##[min:max] body or ##[*] or ##[+]
    if input.starts_with("##") {
        let rest = &input[2..];
        if rest.starts_with('[') {
            // Check for ##[*] and ##[+] shorthands first
            // Convention: max: None = unbounded ($), max: Some(n) = bounded
            if rest.starts_with("[*]") {
                let body_str = rest[3..].trim();
                return Ok(SvaExpr::Delay {
                    body: Box::new(parse_unary(body_str)?),
                    min: 0,
                    max: None, // [*] = [0:$]
                });
            }
            if rest.starts_with("[+]") {
                let body_str = rest[3..].trim();
                return Ok(SvaExpr::Delay {
                    body: Box::new(parse_unary(body_str)?),
                    min: 1,
                    max: None, // [+] = [1:$]
                });
            }
            // ##[min:max] body
            if let Some(bracket_end) = rest.find(']') {
                let range_str = &rest[1..bracket_end];
                let body_str = rest[bracket_end + 1..].trim();
                let parts: Vec<&str> = range_str.split(':').collect();
                if parts.len() == 2 {
                    let min = parts[0].trim().parse::<u32>().map_err(|_| SvaParseError {
                        message: format!("invalid delay min: '{}'", parts[0]),
                    })?;
                    let max_str = parts[1].trim();
                    let max = if max_str == "$" {
                        None // $ = unbounded
                    } else {
                        Some(max_str.parse::<u32>().map_err(|_| SvaParseError {
                            message: format!("invalid delay max: '{}'", max_str),
                        })?)
                    };
                    return Ok(SvaExpr::Delay {
                        body: Box::new(parse_unary(body_str)?),
                        min,
                        max,
                    });
                }
            }
        } else {
            // ##N body — exact delay
            let mut num_end = 0;
            for c in rest.chars() {
                if c.is_ascii_digit() {
                    num_end += 1;
                } else {
                    break;
                }
            }
            if num_end > 0 {
                let n = rest[..num_end].parse::<u32>().map_err(|_| SvaParseError {
                    message: format!("invalid delay number: '{}'", &rest[..num_end]),
                })?;
                let body_str = rest[num_end..].trim();
                return Ok(SvaExpr::Delay {
                    body: Box::new(parse_unary(body_str)?),
                    min: n,
                    max: Some(n), // exact delay: min == max
                });
            }
        }
    }

    // Negation: !(...)
    if input.starts_with('!') {
        let inner = input[1..].trim();
        let inner = strip_parens(inner);
        return Ok(SvaExpr::Not(Box::new(parse_implication(inner)?)));
    }

    // IEEE 1800 system functions — $onehot0 BEFORE $onehot (prefix clarity)
    if let Some(result) = try_parse_function_call(input, "$onehot0", |inner| {
        Ok(SvaExpr::OneHot0(Box::new(parse_implication(inner)?)))
    })? { return Ok(result); }

    if let Some(result) = try_parse_function_call(input, "$onehot", |inner| {
        Ok(SvaExpr::OneHot(Box::new(parse_implication(inner)?)))
    })? { return Ok(result); }

    if let Some(result) = try_parse_function_call(input, "$countones", |inner| {
        Ok(SvaExpr::CountOnes(Box::new(parse_implication(inner)?)))
    })? { return Ok(result); }

    if let Some(result) = try_parse_function_call(input, "$isunknown", |inner| {
        Ok(SvaExpr::IsUnknown(Box::new(parse_implication(inner)?)))
    })? { return Ok(result); }

    if let Some(result) = try_parse_function_call(input, "$sampled", |inner| {
        Ok(SvaExpr::Sampled(Box::new(parse_implication(inner)?)))
    })? { return Ok(result); }

    if let Some(result) = try_parse_function_call(input, "$bits", |inner| {
        Ok(SvaExpr::Bits(Box::new(parse_implication(inner)?)))
    })? { return Ok(result); }

    if let Some(result) = try_parse_function_call(input, "$clog2", |inner| {
        Ok(SvaExpr::Clog2(Box::new(parse_implication(inner)?)))
    })? { return Ok(result); }

    // $countbits(sig, '0', '1', ...) — generalized bit counting (IEEE 20.9)
    if input.starts_with("$countbits(") {
        if let Some(close) = find_balanced_close(input, "$countbits".len()) {
            let inner = &input["$countbits".len() + 1..close];
            // Parse: first arg is signal, rest are control chars
            let parts: Vec<&str> = inner.split(',').collect();
            if !parts.is_empty() {
                let sig = parse_implication(parts[0].trim())?;
                let mut control_chars = Vec::new();
                for part in &parts[1..] {
                    let trimmed = part.trim().trim_matches('\'');
                    if let Some(c) = trimmed.chars().next() {
                        control_chars.push(c);
                    }
                }
                return Ok(SvaExpr::CountBits(Box::new(sig), control_chars));
            }
        }
    }

    // $isunbounded(param) — parameter bound check (IEEE 20.9)
    if let Some(result) = try_parse_function_call(input, "$isunbounded", |inner| {
        Ok(SvaExpr::IsUnbounded(Box::new(parse_implication(inner)?)))
    })? { return Ok(result); }

    // $rose(...), $fell(...), $stable(...), $changed(...), s_eventually(...), $nexttime(...)
    // Use balanced paren matching so "$fell(sda) && scl" correctly parses $fell(sda) only
    if let Some(result) = try_parse_function_call(input, "$rose", |inner| {
        Ok(SvaExpr::Rose(Box::new(parse_implication(inner)?)))
    })? { return Ok(result); }

    if let Some(result) = try_parse_function_call(input, "$fell", |inner| {
        Ok(SvaExpr::Fell(Box::new(parse_implication(inner)?)))
    })? { return Ok(result); }

    if let Some(result) = try_parse_function_call(input, "$stable", |inner| {
        Ok(SvaExpr::Stable(Box::new(parse_implication(inner)?)))
    })? { return Ok(result); }

    if let Some(result) = try_parse_function_call(input, "$changed", |inner| {
        Ok(SvaExpr::Changed(Box::new(parse_implication(inner)?)))
    })? { return Ok(result); }

    if let Some(result) = try_parse_function_call(input, "s_eventually", |inner| {
        Ok(SvaExpr::SEventually(Box::new(parse_implication(inner)?)))
    })? { return Ok(result); }

    if let Some(result) = try_parse_function_call(input, "s_always", |inner| {
        Ok(SvaExpr::SAlways(Box::new(parse_implication(inner)?)))
    })? { return Ok(result); }

    // nexttime[N](body) — with explicit count
    if input.starts_with("nexttime[") {
        if let Some(bracket_end) = input.find(']') {
            let n_str = &input[9..bracket_end];
            if let Ok(n) = n_str.parse::<u32>() {
                let rest = input[bracket_end + 1..].trim();
                if rest.starts_with('(') {
                    if let Some(close) = find_balanced_close(rest, 0) {
                        let inner = &rest[1..close];
                        return Ok(SvaExpr::Nexttime(
                            Box::new(parse_implication(inner.trim())?),
                            n,
                        ));
                    }
                }
            }
        }
    }

    // nexttime(body) — default count = 1
    if let Some(result) = try_parse_function_call(input, "nexttime", |inner| {
        Ok(SvaExpr::Nexttime(Box::new(parse_implication(inner)?), 1))
    })? { return Ok(result); }

    if let Some(result) = try_parse_function_call(input, "$nexttime", |inner| {
        Ok(SvaExpr::Nexttime(Box::new(parse_implication(inner)?), 1))
    })? { return Ok(result); }

    if let Some(result) = try_parse_function_call(input, "first_match", |inner| {
        Ok(SvaExpr::FirstMatch(Box::new(parse_implication(inner)?)))
    })? { return Ok(result); }

    if let Some(result) = try_parse_function_call(input, "$past", |inner| {
        // $past(sig, n) — parse the signal and count
        if let Some(comma) = inner.find(',') {
            let sig = inner[..comma].trim();
            let n_str = inner[comma + 1..].trim();
            let n = n_str.parse::<u32>().unwrap_or(1);
            Ok(SvaExpr::Past(Box::new(parse_atom(sig)?), n))
        } else {
            Ok(SvaExpr::Past(Box::new(parse_atom(inner)?), 1))
        }
    })? { return Ok(result); }

    // Parenthesized expression
    if input.starts_with('(') && input.ends_with(')') {
        return parse_implication(&input[1..input.len() - 1]);
    }

    parse_atom(input)
}

/// Find the `else` keyword in action block text, skipping occurrences inside
/// string literals (both regular `"..."` and triple-quoted `"""..."""`).
/// Returns the byte offset of the `else` keyword, or None if not found outside strings.
fn find_else_outside_strings(input: &str) -> Option<usize> {
    let bytes = input.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    while i < len {
        // Check for triple-quoted string: """..."""
        if i + 2 < len && bytes[i] == b'"' && bytes[i + 1] == b'"' && bytes[i + 2] == b'"' {
            i += 3; // skip opening """
            while i < len {
                if bytes[i] == b'\\' && i + 1 < len {
                    i += 2; // skip escape sequence
                } else if i + 2 < len && bytes[i] == b'"' && bytes[i + 1] == b'"' && bytes[i + 2] == b'"' {
                    i += 3; // skip closing """
                    break;
                } else {
                    i += 1;
                }
            }
            continue;
        }
        // Check for regular string: "..."
        if bytes[i] == b'"' {
            i += 1; // skip opening "
            while i < len {
                if bytes[i] == b'\\' && i + 1 < len {
                    i += 2; // skip escape sequence
                } else if bytes[i] == b'"' {
                    i += 1; // skip closing "
                    break;
                } else {
                    i += 1;
                }
            }
            continue;
        }
        // Check for `else` keyword at word boundary
        if i + 4 <= len && &input[i..i + 4] == "else" {
            // Verify it's at a word boundary (not part of a larger identifier)
            let before_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
            let after_ok = i + 4 >= len || !bytes[i + 4].is_ascii_alphanumeric();
            if before_ok && after_ok {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

/// Find the closing paren that balances the opening paren at `start`.
/// Returns the index of the closing ')' relative to the input string.
fn find_balanced_close(input: &str, start: usize) -> Option<usize> {
    let chars: Vec<char> = input.chars().collect();
    let mut depth = 0i32;
    for i in start..chars.len() {
        match chars[i] {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Try to parse a function call like `$rose(expr)` with balanced parens.
/// If the input starts with `prefix(`, extracts the balanced inner expression,
/// parses it with the provided closure, and returns the result.
/// If there's content after the closing paren, this returns None so the caller
/// can try parsing at a higher level (e.g., `$rose(sig) && other` should be
/// parsed as And($rose(sig), other) at the And level, not here).
fn try_parse_function_call<F>(
    input: &str,
    prefix: &str,
    parse_inner: F,
) -> Result<Option<SvaExpr>, SvaParseError>
where
    F: FnOnce(&str) -> Result<SvaExpr, SvaParseError>,
{
    let full_prefix = format!("{}(", prefix);
    if !input.starts_with(&full_prefix) {
        return Ok(None);
    }
    let paren_start = full_prefix.len() - 1; // index of '('
    if let Some(close) = find_balanced_close(input, paren_start) {
        let inner = &input[full_prefix.len()..close];
        let remaining = input[close + 1..].trim();
        if remaining.is_empty() {
            // Simple case: $rose(sig) with nothing after
            return Ok(Some(parse_inner(inner.trim())?));
        }
        // There's stuff after the closing paren (e.g., "$rose(sig) && other")
        // Parse just the function call, then let the caller handle the rest
        // We can't handle this at the unary level — return None so the
        // expression gets reparsed at the binary operator level.
        // But we need to handle it: wrap as atom.
        // Actually, re-parse the entire input through the binary operators:
        // The issue is that "$fell(sda) && scl" is at the AND level, not unary.
        // So we parse just "$fell(sda)" as the left side of AND.
        return Ok(None);
    }
    Err(SvaParseError {
        message: format!("unbalanced parens in {}", prefix),
    })
}

/// Find the position of the top-level "else" keyword (not inside parens).
fn find_else_keyword(input: &str) -> Option<usize> {
    let mut depth = 0i32;
    let bytes = input.as_bytes();
    for i in 0..input.len().saturating_sub(3) {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => depth -= 1,
            b'e' if depth == 0 => {
                if input[i..].starts_with("else") {
                    // Check word boundary
                    let before_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
                    let after_ok = i + 4 >= input.len() || !bytes[i + 4].is_ascii_alphanumeric();
                    if before_ok && after_ok {
                        return Some(i);
                    }
                }
            }
            _ => {}
        }
    }
    None
}

fn parse_atom(input: &str) -> Result<SvaExpr, SvaParseError> {
    let input = input.trim();
    if input.is_empty() {
        return Err(SvaParseError {
            message: "empty expression".to_string(),
        });
    }

    // Check for [+] repetition shorthand: signal[+] ≡ signal[*1:$]
    if let Some(bracket_pos) = input.find("[+]") {
        let signal_part = input[..bracket_pos].trim();
        let body = parse_atom(signal_part)?;
        return Ok(SvaExpr::Repetition {
            body: Box::new(body),
            min: 1,
            max: None,
        });
    }

    // Check for repetition: signal[*N] or signal[*min:max] or signal[*]
    if let Some(bracket_pos) = input.find("[*") {
        let signal_part = input[..bracket_pos].trim();
        let rep_part = &input[bracket_pos + 2..];
        if let Some(close_bracket) = rep_part.find(']') {
            let range_str = &rep_part[..close_bracket].trim();
            // [*] shorthand ≡ [*0:$]
            if range_str.is_empty() {
                let body = parse_atom(signal_part)?;
                return Ok(SvaExpr::Repetition {
                    body: Box::new(body),
                    min: 0,
                    max: None,
                });
            }
            let body = parse_atom(signal_part)?;
            if let Some(colon) = range_str.find(':') {
                let min_str = range_str[..colon].trim();
                let max_str = range_str[colon + 1..].trim();
                let min = min_str.parse::<u32>().map_err(|_| SvaParseError {
                    message: format!("invalid repetition min: '{}'", min_str),
                })?;
                let max = if max_str == "$" {
                    None
                } else {
                    Some(max_str.parse::<u32>().map_err(|_| SvaParseError {
                        message: format!("invalid repetition max: '{}'", max_str),
                    })?)
                };
                return Ok(SvaExpr::Repetition {
                    body: Box::new(body),
                    min,
                    max,
                });
            } else {
                // Exact repetition: [*N]
                let n = range_str.trim().parse::<u32>().map_err(|_| SvaParseError {
                    message: format!("invalid repetition count: '{}'", range_str),
                })?;
                return Ok(SvaExpr::Repetition {
                    body: Box::new(body),
                    min: n,
                    max: Some(n),
                });
            }
        }
    }

    // Goto repetition: signal[->N]
    if let Some(bracket_pos) = input.find("[->") {
        let signal_part = input[..bracket_pos].trim();
        let rep_part = &input[bracket_pos + 3..];
        if let Some(close_bracket) = rep_part.find(']') {
            let count_str = rep_part[..close_bracket].trim();
            let count = count_str.parse::<u32>().map_err(|_| SvaParseError {
                message: format!("invalid goto repetition count: '{}'", count_str),
            })?;
            return Ok(SvaExpr::GotoRepetition {
                body: Box::new(parse_atom(signal_part)?),
                count,
            });
        }
    }

    // Non-consecutive repetition: signal[=N] or signal[=min:max] or signal[=min:$]
    if let Some(bracket_pos) = input.find("[=") {
        let signal_part = input[..bracket_pos].trim();
        let rep_part = &input[bracket_pos + 2..];
        if let Some(close_bracket) = rep_part.find(']') {
            let range_str = &rep_part[..close_bracket];
            let body = parse_atom(signal_part)?;
            if let Some(colon) = range_str.find(':') {
                let min = range_str[..colon].trim().parse::<u32>().map_err(|_| SvaParseError {
                    message: format!("invalid non-consec repetition min: '{}'", &range_str[..colon]),
                })?;
                let max_str = range_str[colon + 1..].trim();
                let max = if max_str == "$" {
                    None
                } else {
                    Some(max_str.parse::<u32>().map_err(|_| SvaParseError {
                        message: format!("invalid non-consec repetition max: '{}'", max_str),
                    })?)
                };
                return Ok(SvaExpr::NonConsecRepetition {
                    body: Box::new(body),
                    min,
                    max,
                });
            } else {
                let n = range_str.trim().parse::<u32>().map_err(|_| SvaParseError {
                    message: format!("invalid non-consec repetition count: '{}'", range_str),
                })?;
                return Ok(SvaExpr::NonConsecRepetition {
                    body: Box::new(body),
                    min: n,
                    max: Some(n),
                });
            }
        }
    }

    // IEEE 1800-2023 5.7.2: Real literal constants (e.g., 1.5, 1.2E3, 1.30e-2)
    // Must have digit on each side of decimal point (.12 and 9. are invalid)
    if (input.contains('.') || input.contains('e') || input.contains('E'))
        && input.chars().next().map_or(false, |c| c.is_ascii_digit())
    {
        if let Ok(v) = input.parse::<f64>() {
            return Ok(SvaExpr::RealConst(v));
        }
    }

    // Check if it's a number (plain or Verilog-style width'd value)
    if let Ok(n) = input.parse::<u64>() {
        return Ok(SvaExpr::Const(n, 32));
    }
    // Verilog numeric literal: N'd M or N'hXX etc.
    if let Some(tick_pos) = input.find('\'') {
        let width_str = &input[..tick_pos];
        let rest = &input[tick_pos + 1..];
        if let Ok(width) = width_str.parse::<u32>() {
            let (radix, value_str) = if rest.starts_with('d') || rest.starts_with('D') {
                (10, &rest[1..])
            } else if rest.starts_with('h') || rest.starts_with('H') {
                (16, &rest[1..])
            } else if rest.starts_with('b') || rest.starts_with('B') {
                (2, &rest[1..])
            } else if rest.starts_with('o') || rest.starts_with('O') {
                (8, &rest[1..])
            } else {
                (10, rest)
            };
            if let Ok(value) = u64::from_str_radix(value_str, radix) {
                return Ok(SvaExpr::Const(value, width));
            }
        }
    }

    // IEEE 1800-2023: type(this) construct (IEEE 6.23)
    if input == "type(this)" {
        return Ok(SvaExpr::TypeThis);
    }

    // IEEE 1800-2023: A.map(x) with (expr) — array map method (IEEE 7.12)
    if let Some(dot_map_pos) = input.find(".map(") {
        let array_part = &input[..dot_map_pos].trim();
        let after_map = &input[dot_map_pos + 5..]; // skip ".map("
        // Find closing paren for iterator args
        if let Some(iter_close) = after_map.find(')') {
            let iter_part = after_map[..iter_close].trim();
            let iterator = if iter_part.is_empty() {
                "item".to_string() // default iterator name per IEEE 7.12
            } else {
                // May have iterator and optional index arg: "x" or "x, i"
                iter_part.split(',').next().unwrap_or("item").trim().to_string()
            };
            let after_iter_close = after_map[iter_close + 1..].trim();
            // Expect `with (expr)`
            if after_iter_close.starts_with("with") {
                let with_body = after_iter_close["with".len()..].trim();
                let with_expr_str = strip_parens(with_body);
                let array_expr = parse_atom(array_part)?;
                let with_expr = parse_sva(with_expr_str)?;
                return Ok(SvaExpr::ArrayMap {
                    array: Box::new(array_expr),
                    iterator,
                    with_expr: Box::new(with_expr),
                });
            }
        }
    }

    // Must be a signal name
    if input
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_')
    {
        return Ok(SvaExpr::Signal(input.to_string()));
    }

    Err(SvaParseError {
        message: format!("unexpected token: '{}'", input),
    })
}

/// Render an SvaExpr back to valid SVA text.
/// Closes the round-trip: parse_sva(text) → SvaExpr → sva_expr_to_string → text.
pub fn sva_expr_to_string(expr: &SvaExpr) -> String {
    match expr {
        SvaExpr::Signal(name) => name.clone(),
        SvaExpr::Const(value, width) => format!("{}'d{}", width, value),
        SvaExpr::Rose(inner) => format!("$rose({})", sva_expr_to_string(inner)),
        SvaExpr::Fell(inner) => format!("$fell({})", sva_expr_to_string(inner)),
        SvaExpr::Past(inner, n) => format!("$past({}, {})", sva_expr_to_string(inner), n),
        SvaExpr::And(left, right) => {
            format!("({} && {})", sva_expr_to_string(left), sva_expr_to_string(right))
        }
        SvaExpr::Or(left, right) => {
            format!("({} || {})", sva_expr_to_string(left), sva_expr_to_string(right))
        }
        SvaExpr::Not(inner) => format!("!({})", sva_expr_to_string(inner)),
        SvaExpr::Eq(left, right) => {
            format!("({} == {})", sva_expr_to_string(left), sva_expr_to_string(right))
        }
        SvaExpr::Implication {
            antecedent,
            consequent,
            overlapping,
        } => {
            let op = if *overlapping { "|->" } else { "|=>" };
            format!(
                "{} {} {}",
                sva_expr_to_string(antecedent),
                op,
                sva_expr_to_string(consequent)
            )
        }
        SvaExpr::Delay { body, min, max } => match (min, max) {
            // Unified convention: None = unbounded ($), Some(n) = bounded
            (0, None) => format!("##[*] {}", sva_expr_to_string(body)),
            (1, None) => format!("##[+] {}", sva_expr_to_string(body)),
            (_, None) => format!("##[{}:$] {}", min, sva_expr_to_string(body)),
            (_, Some(max_val)) if min == max_val => format!("##{} {}", min, sva_expr_to_string(body)),
            (_, Some(max_val)) => format!("##[{}:{}] {}", min, max_val, sva_expr_to_string(body)),
        },
        SvaExpr::Repetition { body, min, max } => {
            let body_str = sva_expr_to_string(body);
            match (min, max) {
                (0, None) => format!("{}[*]", body_str),
                (1, None) => format!("{}[+]", body_str),
                (_, Some(m)) if *m == *min => format!("{}[*{}]", body_str, min),
                (_, Some(m)) => format!("{}[*{}:{}]", body_str, min, m),
                (_, None) => format!("{}[*{}:$]", body_str, min),
            }
        }
        SvaExpr::SEventually(inner) => format!("s_eventually({})", sva_expr_to_string(inner)),
        SvaExpr::SAlways(inner) => format!("s_always({})", sva_expr_to_string(inner)),
        SvaExpr::Stable(inner) => format!("$stable({})", sva_expr_to_string(inner)),
        SvaExpr::Changed(inner) => format!("$changed({})", sva_expr_to_string(inner)),
        SvaExpr::Nexttime(inner, n) => {
            if *n == 1 {
                format!("nexttime({})", sva_expr_to_string(inner))
            } else {
                format!("nexttime[{}]({})", n, sva_expr_to_string(inner))
            }
        }
        SvaExpr::DisableIff { condition, body } => {
            format!("disable iff ({}) {}", sva_expr_to_string(condition), sva_expr_to_string(body))
        }
        SvaExpr::IfElse { condition, then_expr, else_expr } => {
            format!(
                "if ({}) {} else {}",
                sva_expr_to_string(condition),
                sva_expr_to_string(then_expr),
                sva_expr_to_string(else_expr),
            )
        }
        // IEEE 1800 extended (Sprint 1B)
        SvaExpr::NotEq(l, r) => format!("({} != {})", sva_expr_to_string(l), sva_expr_to_string(r)),
        SvaExpr::LessThan(l, r) => format!("({} < {})", sva_expr_to_string(l), sva_expr_to_string(r)),
        SvaExpr::GreaterThan(l, r) => format!("({} > {})", sva_expr_to_string(l), sva_expr_to_string(r)),
        SvaExpr::LessEqual(l, r) => format!("({} <= {})", sva_expr_to_string(l), sva_expr_to_string(r)),
        SvaExpr::GreaterEqual(l, r) => format!("({} >= {})", sva_expr_to_string(l), sva_expr_to_string(r)),
        SvaExpr::Ternary { condition, then_expr, else_expr } => {
            format!("{} ? {} : {}",
                sva_expr_to_string(condition),
                sva_expr_to_string(then_expr),
                sva_expr_to_string(else_expr),
            )
        }
        SvaExpr::Throughout { signal, sequence } => {
            format!("{} throughout ({})",
                sva_expr_to_string(signal),
                sva_expr_to_string(sequence),
            )
        }
        SvaExpr::Within { inner, outer } => {
            format!("({}) within ({})",
                sva_expr_to_string(inner),
                sva_expr_to_string(outer),
            )
        }
        SvaExpr::FirstMatch(inner) => format!("first_match({})", sva_expr_to_string(inner)),
        SvaExpr::Intersect { left, right } => {
            format!("({}) intersect ({})",
                sva_expr_to_string(left),
                sva_expr_to_string(right),
            )
        }
        // IEEE 1800 system functions (Audit)
        SvaExpr::OneHot0(inner) => format!("$onehot0({})", sva_expr_to_string(inner)),
        SvaExpr::OneHot(inner) => format!("$onehot({})", sva_expr_to_string(inner)),
        SvaExpr::CountOnes(inner) => format!("$countones({})", sva_expr_to_string(inner)),
        SvaExpr::IsUnknown(inner) => format!("$isunknown({})", sva_expr_to_string(inner)),
        SvaExpr::Sampled(inner) => format!("$sampled({})", sva_expr_to_string(inner)),
        SvaExpr::Bits(inner) => format!("$bits({})", sva_expr_to_string(inner)),
        SvaExpr::Clog2(inner) => format!("$clog2({})", sva_expr_to_string(inner)),
        SvaExpr::CountBits(inner, chars) => {
            let char_args: Vec<String> = chars.iter().map(|c| format!("'{}'", c)).collect();
            format!("$countbits({}, {})", sva_expr_to_string(inner), char_args.join(", "))
        }
        SvaExpr::IsUnbounded(inner) => format!("$isunbounded({})", sva_expr_to_string(inner)),
        // Advanced sequences (Audit)
        SvaExpr::GotoRepetition { body, count } => {
            format!("{}[->{}]", sva_expr_to_string(body), count)
        }
        SvaExpr::NonConsecRepetition { body, min, max } => {
            let body_str = sva_expr_to_string(body);
            match max {
                Some(m) if *m == *min => format!("{}[={}]", body_str, min),
                Some(m) => format!("{}[={}:{}]", body_str, min, m),
                None => format!("{}[={}:$]", body_str, min),
            }
        }
        // Property abort operators (Audit)
        SvaExpr::AcceptOn { condition, body } => {
            format!("accept_on({}) {}", sva_expr_to_string(condition), sva_expr_to_string(body))
        }
        SvaExpr::RejectOn { condition, body } => {
            format!("reject_on({}) {}", sva_expr_to_string(condition), sva_expr_to_string(body))
        }
        // Property connectives (Sprint 1, IEEE 16.12.3-8)
        SvaExpr::PropertyNot(inner) => format!("not {}", sva_expr_to_string(inner)),
        SvaExpr::PropertyImplies(l, r) => {
            format!("{} implies {}", sva_expr_to_string(l), sva_expr_to_string(r))
        }
        SvaExpr::PropertyIff(l, r) => {
            format!("{} iff {}", sva_expr_to_string(l), sva_expr_to_string(r))
        }
        // LTL temporal operators (Sprint 2)
        SvaExpr::Always(inner) => format!("always({})", sva_expr_to_string(inner)),
        SvaExpr::AlwaysBounded { body, min, max } => match max {
            Some(m) => format!("always [{}:{}] {}", min, m, sva_expr_to_string(body)),
            None => format!("always [{}:$] {}", min, sva_expr_to_string(body)),
        },
        SvaExpr::SAlwaysBounded { body, min, max } => {
            format!("s_always [{}:{}] {}", min, max, sva_expr_to_string(body))
        }
        SvaExpr::EventuallyBounded { body, min, max } => {
            format!("eventually [{}:{}] {}", min, max, sva_expr_to_string(body))
        }
        SvaExpr::SEventuallyBounded { body, min, max } => match max {
            Some(m) => format!("s_eventually [{}:{}] {}", min, m, sva_expr_to_string(body)),
            None => format!("s_eventually [{}:$] {}", min, sva_expr_to_string(body)),
        },
        SvaExpr::Until { lhs, rhs, strong, inclusive } => {
            let op = match (strong, inclusive) {
                (false, false) => "until",
                (true, false) => "s_until",
                (false, true) => "until_with",
                (true, true) => "s_until_with",
            };
            format!("{} {} {}", sva_expr_to_string(lhs), op, sva_expr_to_string(rhs))
        }
        // Sprint 3
        SvaExpr::Strong(inner) => format!("strong({})", sva_expr_to_string(inner)),
        SvaExpr::Weak(inner) => format!("weak({})", sva_expr_to_string(inner)),
        SvaExpr::SNexttime(inner, n) => {
            if *n == 1 {
                format!("s_nexttime({})", sva_expr_to_string(inner))
            } else {
                format!("s_nexttime[{}]({})", n, sva_expr_to_string(inner))
            }
        }
        SvaExpr::FollowedBy { antecedent, consequent, overlapping } => {
            let op = if *overlapping { "#-#" } else { "#=#" };
            format!("{} {} {}", sva_expr_to_string(antecedent), op, sva_expr_to_string(consequent))
        }
        SvaExpr::PropertyCase { expression, items, default } => {
            let mut s = format!("case({})", sva_expr_to_string(expression));
            for (vals, prop) in items {
                let vs: Vec<String> = vals.iter().map(sva_expr_to_string).collect();
                s.push_str(&format!(" {}: {};", vs.join(", "), sva_expr_to_string(prop)));
            }
            if let Some(d) = default {
                s.push_str(&format!(" default: {};", sva_expr_to_string(d)));
            }
            s.push_str(" endcase");
            s
        }
        SvaExpr::SyncAcceptOn { condition, body } => {
            format!("sync_accept_on({}) {}", sva_expr_to_string(condition), sva_expr_to_string(body))
        }
        SvaExpr::SyncRejectOn { condition, body } => {
            format!("sync_reject_on({}) {}", sva_expr_to_string(condition), sva_expr_to_string(body))
        }
        // Sprint 5
        SvaExpr::SequenceAnd(l, r) => {
            format!("({}) and ({})", sva_expr_to_string(l), sva_expr_to_string(r))
        }
        SvaExpr::SequenceOr(l, r) => {
            format!("({}) or ({})", sva_expr_to_string(l), sva_expr_to_string(r))
        }
        // Sprint 7
        SvaExpr::ImmediateAssert { expression, deferred } => {
            match deferred {
                None => format!("assert({})", sva_expr_to_string(expression)),
                Some(ImmediateDeferred::Observed) => format!("assert #0({})", sva_expr_to_string(expression)),
                Some(ImmediateDeferred::Final) => format!("assert final({})", sva_expr_to_string(expression)),
            }
        }
        // Sprint 13
        SvaExpr::FieldAccess { signal, field } => format!("{}.{}", sva_expr_to_string(signal), field),
        SvaExpr::EnumLiteral { type_name: Some(t), value } => format!("{}::{}", t, value),
        SvaExpr::EnumLiteral { type_name: None, value } => value.clone(),
        // Sprint 14
        SvaExpr::Triggered(name) => format!("{}.triggered", name),
        SvaExpr::Matched(name) => format!("{}.matched", name),
        // Sprint 15
        SvaExpr::BitAnd(l, r) => format!("({} & {})", sva_expr_to_string(l), sva_expr_to_string(r)),
        SvaExpr::BitOr(l, r) => format!("({} | {})", sva_expr_to_string(l), sva_expr_to_string(r)),
        SvaExpr::BitXor(l, r) => format!("({} ^ {})", sva_expr_to_string(l), sva_expr_to_string(r)),
        SvaExpr::BitNot(inner) => format!("~{}", sva_expr_to_string(inner)),
        SvaExpr::ReductionAnd(inner) => format!("&{}", sva_expr_to_string(inner)),
        SvaExpr::ReductionOr(inner) => format!("|{}", sva_expr_to_string(inner)),
        SvaExpr::ReductionXor(inner) => format!("^{}", sva_expr_to_string(inner)),
        SvaExpr::BitSelect { signal, index } => format!("{}[{}]", sva_expr_to_string(signal), sva_expr_to_string(index)),
        SvaExpr::PartSelect { signal, high, low } => format!("{}[{}:{}]", sva_expr_to_string(signal), high, low),
        SvaExpr::Concat(items) => {
            let parts: Vec<String> = items.iter().map(sva_expr_to_string).collect();
            format!("{{{}}}", parts.join(", "))
        }
        // Sprint 18
        SvaExpr::SequenceAction { expression, assignments } => {
            let assigns: Vec<String> = assignments.iter()
                .map(|(name, rhs)| format!("{} = {}", name, sva_expr_to_string(rhs)))
                .collect();
            format!("({}, {})", sva_expr_to_string(expression), assigns.join(", "))
        }
        SvaExpr::LocalVar(name) => name.clone(),
        SvaExpr::ConstCast(inner) => format!("const'({})", sva_expr_to_string(inner)),
        SvaExpr::Clocked { clock, edge, body } => {
            let edge_str = match edge {
                ClockEdge::Posedge => "posedge",
                ClockEdge::Negedge => "negedge",
                ClockEdge::Edge => "edge",
            };
            format!("@({} {}) {}", edge_str, clock, sva_expr_to_string(body))
        }
        // Sprint 23 (IEEE 1800-2023)
        SvaExpr::ArrayMap { array, iterator, with_expr } => {
            format!("{}.map({}) with ({})", sva_expr_to_string(array), iterator, sva_expr_to_string(with_expr))
        }
        SvaExpr::TypeThis => "type(this)".to_string(),
        SvaExpr::RealConst(v) => format!("{}", v),
    }
}

fn strip_parens(input: &str) -> &str {
    let input = input.trim();
    if input.starts_with('(') && input.ends_with(')') {
        &input[1..input.len() - 1]
    } else {
        input
    }
}

/// Check if two SvaExpr trees are structurally equivalent.
pub fn sva_exprs_structurally_equivalent(a: &SvaExpr, b: &SvaExpr) -> bool {
    match (a, b) {
        (SvaExpr::Signal(sa), SvaExpr::Signal(sb)) => sa == sb,
        (SvaExpr::Const(va, wa), SvaExpr::Const(vb, wb)) => va == vb && wa == wb,
        (SvaExpr::Rose(ia), SvaExpr::Rose(ib)) => sva_exprs_structurally_equivalent(ia, ib),
        (SvaExpr::Fell(ia), SvaExpr::Fell(ib)) => sva_exprs_structurally_equivalent(ia, ib),
        (SvaExpr::Past(ia, na), SvaExpr::Past(ib, nb)) => {
            na == nb && sva_exprs_structurally_equivalent(ia, ib)
        }
        (SvaExpr::And(la, ra), SvaExpr::And(lb, rb)) => {
            sva_exprs_structurally_equivalent(la, lb)
                && sva_exprs_structurally_equivalent(ra, rb)
        }
        (SvaExpr::Or(la, ra), SvaExpr::Or(lb, rb)) => {
            sva_exprs_structurally_equivalent(la, lb)
                && sva_exprs_structurally_equivalent(ra, rb)
        }
        (SvaExpr::Not(ia), SvaExpr::Not(ib)) => sva_exprs_structurally_equivalent(ia, ib),
        (SvaExpr::Eq(la, ra), SvaExpr::Eq(lb, rb)) => {
            sva_exprs_structurally_equivalent(la, lb)
                && sva_exprs_structurally_equivalent(ra, rb)
        }
        (
            SvaExpr::Implication {
                antecedent: aa,
                consequent: ca,
                overlapping: oa,
            },
            SvaExpr::Implication {
                antecedent: ab,
                consequent: cb,
                overlapping: ob,
            },
        ) => {
            oa == ob
                && sva_exprs_structurally_equivalent(aa, ab)
                && sva_exprs_structurally_equivalent(ca, cb)
        }
        (
            SvaExpr::Delay {
                body: ba,
                min: mna,
                max: mxa,
            },
            SvaExpr::Delay {
                body: bb,
                min: mnb,
                max: mxb,
            },
        ) => mna == mnb && mxa == mxb && sva_exprs_structurally_equivalent(ba, bb),
        (
            SvaExpr::Repetition { body: ba, min: mna, max: mxa },
            SvaExpr::Repetition { body: bb, min: mnb, max: mxb },
        ) => mna == mnb && mxa == mxb && sva_exprs_structurally_equivalent(ba, bb),
        (SvaExpr::SEventually(ia), SvaExpr::SEventually(ib)) => {
            sva_exprs_structurally_equivalent(ia, ib)
        }
        (SvaExpr::SAlways(ia), SvaExpr::SAlways(ib)) => {
            sva_exprs_structurally_equivalent(ia, ib)
        }
        (SvaExpr::Stable(ia), SvaExpr::Stable(ib)) => sva_exprs_structurally_equivalent(ia, ib),
        (SvaExpr::Changed(ia), SvaExpr::Changed(ib)) => sva_exprs_structurally_equivalent(ia, ib),
        (SvaExpr::Nexttime(ia, na), SvaExpr::Nexttime(ib, nb)) => {
            na == nb && sva_exprs_structurally_equivalent(ia, ib)
        }
        (
            SvaExpr::DisableIff { condition: ca, body: ba },
            SvaExpr::DisableIff { condition: cb, body: bb },
        ) => {
            sva_exprs_structurally_equivalent(ca, cb)
                && sva_exprs_structurally_equivalent(ba, bb)
        }
        (
            SvaExpr::IfElse { condition: ca, then_expr: ta, else_expr: ea },
            SvaExpr::IfElse { condition: cb, then_expr: tb, else_expr: eb },
        ) => {
            sva_exprs_structurally_equivalent(ca, cb)
                && sva_exprs_structurally_equivalent(ta, tb)
                && sva_exprs_structurally_equivalent(ea, eb)
        }
        // IEEE 1800 extended (Sprint 1B)
        (SvaExpr::NotEq(la, ra), SvaExpr::NotEq(lb, rb)) => {
            sva_exprs_structurally_equivalent(la, lb) && sva_exprs_structurally_equivalent(ra, rb)
        }
        (SvaExpr::LessThan(la, ra), SvaExpr::LessThan(lb, rb)) => {
            sva_exprs_structurally_equivalent(la, lb) && sva_exprs_structurally_equivalent(ra, rb)
        }
        (SvaExpr::GreaterThan(la, ra), SvaExpr::GreaterThan(lb, rb)) => {
            sva_exprs_structurally_equivalent(la, lb) && sva_exprs_structurally_equivalent(ra, rb)
        }
        (SvaExpr::LessEqual(la, ra), SvaExpr::LessEqual(lb, rb)) => {
            sva_exprs_structurally_equivalent(la, lb) && sva_exprs_structurally_equivalent(ra, rb)
        }
        (SvaExpr::GreaterEqual(la, ra), SvaExpr::GreaterEqual(lb, rb)) => {
            sva_exprs_structurally_equivalent(la, lb) && sva_exprs_structurally_equivalent(ra, rb)
        }
        (
            SvaExpr::Ternary { condition: ca, then_expr: ta, else_expr: ea },
            SvaExpr::Ternary { condition: cb, then_expr: tb, else_expr: eb },
        ) => {
            sva_exprs_structurally_equivalent(ca, cb)
                && sva_exprs_structurally_equivalent(ta, tb)
                && sva_exprs_structurally_equivalent(ea, eb)
        }
        (
            SvaExpr::Throughout { signal: sa, sequence: qa },
            SvaExpr::Throughout { signal: sb, sequence: qb },
        ) => {
            sva_exprs_structurally_equivalent(sa, sb) && sva_exprs_structurally_equivalent(qa, qb)
        }
        (
            SvaExpr::Within { inner: ia, outer: oa },
            SvaExpr::Within { inner: ib, outer: ob },
        ) => {
            sva_exprs_structurally_equivalent(ia, ib) && sva_exprs_structurally_equivalent(oa, ob)
        }
        (SvaExpr::FirstMatch(ia), SvaExpr::FirstMatch(ib)) => {
            sva_exprs_structurally_equivalent(ia, ib)
        }
        (
            SvaExpr::Intersect { left: la, right: ra },
            SvaExpr::Intersect { left: lb, right: rb },
        ) => {
            sva_exprs_structurally_equivalent(la, lb) && sva_exprs_structurally_equivalent(ra, rb)
        }
        // IEEE 1800 system functions (Audit)
        (SvaExpr::OneHot0(ia), SvaExpr::OneHot0(ib)) => sva_exprs_structurally_equivalent(ia, ib),
        (SvaExpr::OneHot(ia), SvaExpr::OneHot(ib)) => sva_exprs_structurally_equivalent(ia, ib),
        (SvaExpr::CountOnes(ia), SvaExpr::CountOnes(ib)) => sva_exprs_structurally_equivalent(ia, ib),
        (SvaExpr::IsUnknown(ia), SvaExpr::IsUnknown(ib)) => sva_exprs_structurally_equivalent(ia, ib),
        (SvaExpr::Sampled(ia), SvaExpr::Sampled(ib)) => sva_exprs_structurally_equivalent(ia, ib),
        (SvaExpr::Bits(ia), SvaExpr::Bits(ib)) => sva_exprs_structurally_equivalent(ia, ib),
        (SvaExpr::Clog2(ia), SvaExpr::Clog2(ib)) => sva_exprs_structurally_equivalent(ia, ib),
        (SvaExpr::CountBits(ia, ca), SvaExpr::CountBits(ib, cb)) =>
            ca == cb && sva_exprs_structurally_equivalent(ia, ib),
        (SvaExpr::IsUnbounded(ia), SvaExpr::IsUnbounded(ib)) => sva_exprs_structurally_equivalent(ia, ib),
        // Advanced sequences (Audit)
        (
            SvaExpr::GotoRepetition { body: ba, count: ca },
            SvaExpr::GotoRepetition { body: bb, count: cb },
        ) => ca == cb && sva_exprs_structurally_equivalent(ba, bb),
        (
            SvaExpr::NonConsecRepetition { body: ba, min: mna, max: mxa },
            SvaExpr::NonConsecRepetition { body: bb, min: mnb, max: mxb },
        ) => mna == mnb && mxa == mxb && sva_exprs_structurally_equivalent(ba, bb),
        // Property abort operators (Audit)
        (
            SvaExpr::AcceptOn { condition: ca, body: ba },
            SvaExpr::AcceptOn { condition: cb, body: bb },
        ) => {
            sva_exprs_structurally_equivalent(ca, cb)
                && sva_exprs_structurally_equivalent(ba, bb)
        }
        (
            SvaExpr::RejectOn { condition: ca, body: ba },
            SvaExpr::RejectOn { condition: cb, body: bb },
        ) => {
            sva_exprs_structurally_equivalent(ca, cb)
                && sva_exprs_structurally_equivalent(ba, bb)
        }
        // Property connectives (Sprint 1)
        (SvaExpr::PropertyNot(ia), SvaExpr::PropertyNot(ib)) => {
            sva_exprs_structurally_equivalent(ia, ib)
        }
        (SvaExpr::PropertyImplies(la, ra), SvaExpr::PropertyImplies(lb, rb)) => {
            sva_exprs_structurally_equivalent(la, lb)
                && sva_exprs_structurally_equivalent(ra, rb)
        }
        (SvaExpr::PropertyIff(la, ra), SvaExpr::PropertyIff(lb, rb)) => {
            sva_exprs_structurally_equivalent(la, lb)
                && sva_exprs_structurally_equivalent(ra, rb)
        }
        // LTL temporal operators (Sprint 2)
        (SvaExpr::Always(ia), SvaExpr::Always(ib)) => sva_exprs_structurally_equivalent(ia, ib),
        (
            SvaExpr::AlwaysBounded { body: ba, min: mna, max: mxa },
            SvaExpr::AlwaysBounded { body: bb, min: mnb, max: mxb },
        ) => mna == mnb && mxa == mxb && sva_exprs_structurally_equivalent(ba, bb),
        (
            SvaExpr::SAlwaysBounded { body: ba, min: mna, max: mxa },
            SvaExpr::SAlwaysBounded { body: bb, min: mnb, max: mxb },
        ) => mna == mnb && mxa == mxb && sva_exprs_structurally_equivalent(ba, bb),
        (
            SvaExpr::EventuallyBounded { body: ba, min: mna, max: mxa },
            SvaExpr::EventuallyBounded { body: bb, min: mnb, max: mxb },
        ) => mna == mnb && mxa == mxb && sva_exprs_structurally_equivalent(ba, bb),
        (
            SvaExpr::SEventuallyBounded { body: ba, min: mna, max: mxa },
            SvaExpr::SEventuallyBounded { body: bb, min: mnb, max: mxb },
        ) => mna == mnb && mxa == mxb && sva_exprs_structurally_equivalent(ba, bb),
        (
            SvaExpr::Until { lhs: la, rhs: ra, strong: sa, inclusive: ia },
            SvaExpr::Until { lhs: lb, rhs: rb, strong: sb, inclusive: ib },
        ) => {
            sa == sb && ia == ib
                && sva_exprs_structurally_equivalent(la, lb)
                && sva_exprs_structurally_equivalent(ra, rb)
        }
        // Sprint 3
        (SvaExpr::Strong(ia), SvaExpr::Strong(ib)) => sva_exprs_structurally_equivalent(ia, ib),
        (SvaExpr::Weak(ia), SvaExpr::Weak(ib)) => sva_exprs_structurally_equivalent(ia, ib),
        (SvaExpr::SNexttime(ia, na), SvaExpr::SNexttime(ib, nb)) => {
            na == nb && sva_exprs_structurally_equivalent(ia, ib)
        }
        (
            SvaExpr::FollowedBy { antecedent: aa, consequent: ca, overlapping: oa },
            SvaExpr::FollowedBy { antecedent: ab, consequent: cb, overlapping: ob },
        ) => {
            oa == ob
                && sva_exprs_structurally_equivalent(aa, ab)
                && sva_exprs_structurally_equivalent(ca, cb)
        }
        (
            SvaExpr::PropertyCase { expression: ea, items: ia, default: da },
            SvaExpr::PropertyCase { expression: eb, items: ib, default: db },
        ) => {
            sva_exprs_structurally_equivalent(ea, eb)
                && ia.len() == ib.len()
                && ia.iter().zip(ib.iter()).all(|((va, pa), (vb, pb))| {
                    va.len() == vb.len()
                        && va.iter().zip(vb.iter()).all(|(a, b)| sva_exprs_structurally_equivalent(a, b))
                        && sva_exprs_structurally_equivalent(pa, pb)
                })
                && match (da, db) {
                    (Some(a), Some(b)) => sva_exprs_structurally_equivalent(a, b),
                    (None, None) => true,
                    _ => false,
                }
        }
        (
            SvaExpr::SyncAcceptOn { condition: ca, body: ba },
            SvaExpr::SyncAcceptOn { condition: cb, body: bb },
        ) => {
            sva_exprs_structurally_equivalent(ca, cb)
                && sva_exprs_structurally_equivalent(ba, bb)
        }
        (
            SvaExpr::SyncRejectOn { condition: ca, body: ba },
            SvaExpr::SyncRejectOn { condition: cb, body: bb },
        ) => {
            sva_exprs_structurally_equivalent(ca, cb)
                && sva_exprs_structurally_equivalent(ba, bb)
        }
        // Sprint 5
        (SvaExpr::SequenceAnd(la, ra), SvaExpr::SequenceAnd(lb, rb)) => {
            sva_exprs_structurally_equivalent(la, lb)
                && sva_exprs_structurally_equivalent(ra, rb)
        }
        (SvaExpr::SequenceOr(la, ra), SvaExpr::SequenceOr(lb, rb)) => {
            sva_exprs_structurally_equivalent(la, lb)
                && sva_exprs_structurally_equivalent(ra, rb)
        }
        // Sprint 7
        (
            SvaExpr::ImmediateAssert { expression: ea, deferred: da },
            SvaExpr::ImmediateAssert { expression: eb, deferred: db },
        ) => da == db && sva_exprs_structurally_equivalent(ea, eb),
        // Sprint 13
        (SvaExpr::FieldAccess { signal: sa, field: fa }, SvaExpr::FieldAccess { signal: sb, field: fb }) => {
            fa == fb && sva_exprs_structurally_equivalent(sa, sb)
        }
        (SvaExpr::EnumLiteral { type_name: ta, value: va }, SvaExpr::EnumLiteral { type_name: tb, value: vb }) => {
            ta == tb && va == vb
        }
        // Sprint 14
        (SvaExpr::Triggered(a), SvaExpr::Triggered(b)) => a == b,
        (SvaExpr::Matched(a), SvaExpr::Matched(b)) => a == b,
        // Sprint 15
        (SvaExpr::BitAnd(la, ra), SvaExpr::BitAnd(lb, rb)) => {
            sva_exprs_structurally_equivalent(la, lb) && sva_exprs_structurally_equivalent(ra, rb)
        }
        (SvaExpr::BitOr(la, ra), SvaExpr::BitOr(lb, rb)) => {
            sva_exprs_structurally_equivalent(la, lb) && sva_exprs_structurally_equivalent(ra, rb)
        }
        (SvaExpr::BitXor(la, ra), SvaExpr::BitXor(lb, rb)) => {
            sva_exprs_structurally_equivalent(la, lb) && sva_exprs_structurally_equivalent(ra, rb)
        }
        (SvaExpr::BitNot(ia), SvaExpr::BitNot(ib)) => sva_exprs_structurally_equivalent(ia, ib),
        (SvaExpr::ReductionAnd(ia), SvaExpr::ReductionAnd(ib)) => sva_exprs_structurally_equivalent(ia, ib),
        (SvaExpr::ReductionOr(ia), SvaExpr::ReductionOr(ib)) => sva_exprs_structurally_equivalent(ia, ib),
        (SvaExpr::ReductionXor(ia), SvaExpr::ReductionXor(ib)) => sva_exprs_structurally_equivalent(ia, ib),
        (SvaExpr::BitSelect { signal: sa, index: ia }, SvaExpr::BitSelect { signal: sb, index: ib }) => {
            sva_exprs_structurally_equivalent(sa, sb) && sva_exprs_structurally_equivalent(ia, ib)
        }
        (SvaExpr::PartSelect { signal: sa, high: ha, low: la }, SvaExpr::PartSelect { signal: sb, high: hb, low: lb }) => {
            ha == hb && la == lb && sva_exprs_structurally_equivalent(sa, sb)
        }
        (SvaExpr::Concat(ia), SvaExpr::Concat(ib)) => {
            ia.len() == ib.len() && ia.iter().zip(ib.iter()).all(|(a, b)| sva_exprs_structurally_equivalent(a, b))
        }
        // Sprint 18
        (SvaExpr::SequenceAction { expression: ea, assignments: aa },
         SvaExpr::SequenceAction { expression: eb, assignments: ab }) => {
            sva_exprs_structurally_equivalent(ea, eb)
                && aa.len() == ab.len()
                && aa.iter().zip(ab.iter()).all(|((na, ra), (nb, rb))|
                    na == nb && sva_exprs_structurally_equivalent(ra, rb))
        }
        (SvaExpr::LocalVar(a), SvaExpr::LocalVar(b)) => a == b,
        (SvaExpr::ConstCast(ia), SvaExpr::ConstCast(ib)) => sva_exprs_structurally_equivalent(ia, ib),
        (SvaExpr::Clocked { clock: ca, edge: ea, body: ba },
         SvaExpr::Clocked { clock: cb, edge: eb, body: bb }) => {
            ca == cb && ea == eb && sva_exprs_structurally_equivalent(ba, bb)
        }
        // Sprint 23 (IEEE 1800-2023)
        (SvaExpr::ArrayMap { array: aa, iterator: ia, with_expr: wa },
         SvaExpr::ArrayMap { array: ab, iterator: ib, with_expr: wb }) => {
            ia == ib && sva_exprs_structurally_equivalent(aa, ab) && sva_exprs_structurally_equivalent(wa, wb)
        }
        (SvaExpr::TypeThis, SvaExpr::TypeThis) => true,
        (SvaExpr::RealConst(a), SvaExpr::RealConst(b)) => a.to_bits() == b.to_bits(),
        _ => false,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Elaboration Pass — Default Clocking & Disable Iff (IEEE 16.15-16)
// ═══════════════════════════════════════════════════════════════════════════

/// Context for elaborating directives with default clocking and disable iff.
/// IEEE 16.15: `default clocking` applies to all assertions in scope that
/// lack an explicit clock. IEEE 16.16: `default disable iff` applies the
/// reset condition to all assertions lacking explicit `disable iff`.
#[derive(Debug, Clone, Default)]
pub struct ElaborationContext {
    pub default_clocking: Option<String>,
    pub default_disable_iff: Option<SvaExpr>,
}

/// Elaborate a set of directives by applying default clocking and disable iff.
///
/// IEEE 16.15-16: For each directive:
/// - If it lacks an explicit clock and a default clocking exists, apply the default.
/// - If it lacks an explicit disable_iff and a default disable_iff exists, wrap
///   the property body in `DisableIff { condition, body }`.
/// - Explicit annotations override defaults (no clobbering).
pub fn elaborate_directives(
    directives: &[SvaDirective],
    ctx: &ElaborationContext,
) -> Vec<SvaDirective> {
    directives.iter().map(|d| {
        let mut elaborated = d.clone();

        // Apply default clock if missing
        if elaborated.clock.is_none() {
            if let Some(ref default_clk) = ctx.default_clocking {
                elaborated.clock = Some(default_clk.clone());
            }
        }

        // Apply default disable iff if missing
        if elaborated.disable_iff.is_none() {
            if let Some(ref default_dis) = ctx.default_disable_iff {
                elaborated.disable_iff = Some(default_dis.clone());
            }
        }

        elaborated
    }).collect()
}

/// Resolve `let` declarations by inlining expression substitution.
/// IEEE 11.12: `let` is pure expression substitution with no temporal semantics.
pub fn resolve_let_instance(
    decls: &[LetDecl],
    name: &str,
    args: &[SvaExpr],
) -> Result<SvaExpr, SvaParseError> {
    let decl = decls.iter().find(|d| d.name == name).ok_or_else(|| SvaParseError {
        message: format!("undeclared let: '{}'", name),
    })?;

    let required_count = decl.ports.iter().filter(|p| p.default.is_none()).count();
    if args.len() < required_count || args.len() > decl.ports.len() {
        return Err(SvaParseError {
            message: format!(
                "let '{}' expects {} to {} arguments, got {}",
                name, required_count, decl.ports.len(), args.len()
            ),
        });
    }

    let mut result = decl.body.clone();
    for (i, port) in decl.ports.iter().enumerate() {
        let actual = if i < args.len() {
            args[i].clone()
        } else {
            port.default.clone().ok_or_else(|| SvaParseError {
                message: format!("missing required argument '{}' for let '{}'", port.name, name),
            })?
        };
        result = substitute_signal(&result, &port.name, &actual);
    }
    Ok(result)
}

/// Translate dist items into range constraints for formal verification.
/// IEEE 16.14.2: In formal mode, `dist` ≡ `inside` (range restriction only).
pub fn translate_dist_to_ranges(items: &[DistItem]) -> Vec<(u64, u64)> {
    items.iter().map(|item| {
        let max = item.max.unwrap_or(item.min);
        (item.min, max)
    }).collect()
}

/// Validate dist list is non-empty.
pub fn validate_dist(items: &[DistItem]) -> Result<(), SvaParseError> {
    if items.is_empty() {
        return Err(SvaParseError { message: "empty dist list".to_string() });
    }
    Ok(())
}

/// Resolve a checker instance by binding ports into its assertions.
pub fn resolve_checker(
    checker: &CheckerDecl,
    port_bindings: &[(String, SvaExpr)],
) -> Result<Vec<SvaDirective>, SvaParseError> {
    checker.assertions.iter().map(|directive| {
        let mut resolved_prop = directive.property.clone();
        for (port_name, actual) in port_bindings {
            resolved_prop = substitute_signal(&resolved_prop, port_name, actual);
        }
        Ok(SvaDirective {
            kind: directive.kind.clone(),
            property: resolved_prop,
            label: directive.label.clone(),
            clock: directive.clock.clone(),
            disable_iff: directive.disable_iff.clone(),
            action_pass: directive.action_pass.clone(),
            action_fail: directive.action_fail.clone(),
        })
    }).collect()
}

/// Get checker's random variable quantifier structure.
/// Returns (const_rand_vars, per_timestep_rand_vars).
pub fn checker_quantifier_structure(checker: &CheckerDecl) -> (Vec<&RandVar>, Vec<&RandVar>) {
    let const_vars: Vec<&RandVar> = checker.rand_vars.iter().filter(|v| v.is_const).collect();
    let nonconst_vars: Vec<&RandVar> = checker.rand_vars.iter().filter(|v| !v.is_const).collect();
    (const_vars, nonconst_vars)
}
