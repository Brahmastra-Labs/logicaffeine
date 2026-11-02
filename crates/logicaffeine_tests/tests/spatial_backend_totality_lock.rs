//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! BACKEND-TOTALITY LOCK — the spatial IR is a portable ALGEBRA, not a Tensix dialect.
//!
//! For the whole cross product (workload × backend), a backend is a TOTAL function: it either
//! LOWERS the program, or it REFUSES IT BY NAME. There is no third outcome.
//!
//!   1. NO PANIC. Every pair returns `Ok` or a typed `Err`. Each call runs inside `catch_unwind`, so
//!      a panic is reported as a failure naming the pair (and printed in the matrix) rather than
//!      aborting the process and taking the rest of the grid's evidence with it.
//!   2. EVERY REFUSAL IS NAMED — and not only the ones today's six workloads happen to reach.
//!      `every_refusal_arm_in_the_backend_sources_names_what_it_refused` reads the 60-odd refusal
//!      literals out of `spatial/lower/**.rs` and holds EACH of them to the bar, so an arm nobody
//!      triggers is under the same discipline as one the grid walks into. An anonymous
//!      `Unsupported("")`, a `"TODO"`, or a bare restatement of the enum variant is a failure: a gap
//!      you cannot read is a gap nobody closes.
//!   3. THE MATRIX IS VISIBLE, AND IT IS NOT ALLOWED TO BE EMPTY. The coverage grid is printed on
//!      every run and embedded in every failure message. Every assertion in the grid loop is inside
//!      that loop, so an empty loop would satisfy all of them: the floors under `the grid must have
//!      teeth` are what stop a front-end regression from turning this lock green by starving it.
//!   4. A LOWERING IS JUSTIFIED. Where a backend does lower, the artifact must be the image of the
//!      IR (its census equals the program's), must actually carry emitted text, must be
//!      deterministic (lowering twice prints the same artifact), and must be a FUNCTION OF THE
//!      PROGRAM — two different workloads may never print byte-identical artifacts, which is what
//!      rules out a backend that "supports" everything by printing a fixture.
//!
//! ★ THIS LOCK COUNTS NO REFUSALS. It never asserts a number of refusals, and never names which
//! backend refuses which workload. That is deliberate: a lock keyed to today's gap list has to be
//! edited every time a gap closes, and a test you edit to stay green stops being a lock. This one
//! asserts the QUALITY of each refusal, so closing a gap flips a cell from `refuse` to `lower` and
//! the test keeps passing untouched — while ADDING a backend, a workload, or a compute op that
//! panics or refuses anonymously goes RED immediately.
//!
//! ★ IF A NEW, WELL-WORDED REFUSAL TRIPS THE VOCABULARY CHECK, ADD THE NOUN TO `IR_VOCABULARY` —
//! never delete the assertion, and never weaken the refusal to a generic string to get past it.
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(spatial_backend_totality_lock)'`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::spatial::eval::{eval_choreography, ChoreoFault, Inputs, Outputs, Value};
use logicaffeine_compile::spatial::ir::{ComputeOp, Phase, SpatialProgram, StageKind, TensorRole};
use logicaffeine_compile::spatial::lower::{
    cuda::CudaPersistentBackend, fabric::FabricBackend, rdu::RduSpatialBackend,
    tensix::TensixBackend, Census, SpatialArtifact, SpatialBackend, SpatialLowerError,
};
use logicaffeine_compile::spatial::passes::SpatialReject;
use logicaffeine_compile::spatial::pipeline::{compile_to_spatial, template_from_source};
use logicaffeine_compile::spatial::target::SpatialTarget;

// ── The workloads ─────────────────────────────────────────────────────────────────────────────────
//
// Every tensor sugar the front end recognizes, one source each. The four the front-end lock pins
// (matmul, elementwise, rmsnorm, swiglu) plus softmax and layernorm, which `spatial_algebra_semantics_lock`
// compiles and checks against the mathematics. ★ EACH SOURCE IS THE SHAPE AN EXISTING GREEN LOCK ALREADY
// COMPILES — `spatial_front_end_lock` for the first four, `spatial_algebra_semantics_lock` for the last
// two — so a `Parse` / `Unrecognized` reject here can only mean the recognizer regressed, which is why
// the grid treats those two as failures rather than as a cell state.
//
// A workload a TARGET cannot host is a different matter: that is a typed capability reject, held to the
// same naming bar as a backend refusal and shown in the matrix as its own cell state.

/// A 2×2 int GEMM — the control that reaches every backend.
const SUGAR_MATMUL: &str = r#"## Main
Let A be a 2 by 2 tensor of Int.
Set item 1 of A to 1.
Set item 2 of A to 2.
Set item 3 of A to 3.
Set item 4 of A to 4.
Let B be a 2 by 2 tensor of Int.
Set item 1 of B to 5.
Set item 2 of B to 6.
Set item 3 of B to 7.
Set item 4 of B to 8.
Let C be A matmul B.
Show item 1 of C.
Show item 4 of C.
"#;

/// Elementwise add — `ComputeOp::Elementwise`.
const SUGAR_PLUS: &str = r#"## Main
Let A be a 2 by 2 tensor of Int.
Set item 1 of A to 1.
Set item 2 of A to 2.
Set item 3 of A to 3.
Set item 4 of A to 4.
Let B be a 2 by 2 tensor of Int.
Set item 1 of B to 5.
Set item 2 of B to 6.
Set item 3 of B to 7.
Set item 4 of B to 8.
Let C be A plus B.
Show item 1 of C.
Show item 4 of C.
"#;

/// Weighted RMSNorm — the split stage chain (`ReduceRows` → `RowXform` → `RowRescale` → weight).
const SUGAR_RMSNORM: &str = r#"## Main
Let X be a 2 by 3 tensor of Real.
Set item 1 of X to 1.0.
Set item 2 of X to 2.0.
Set item 3 of X to 3.0.
Set item 4 of X to 4.0.
Set item 5 of X to 5.0.
Set item 6 of X to 6.0.
Let W be a 1 by 3 tensor of Real.
Set item 1 of W to 0.5.
Set item 2 of W to 1.0.
Set item 3 of W to 2.0.
Let Y be rmsnorm of X with weight W.
Show round(item 1 of Y * 1000000).
Show round(item 4 of Y * 1000000).
"#;

/// SwiGLU — `ComputeOp::SwiGlu`, an SFPU transcendental rather than a matrix-engine op.
const SUGAR_SWIGLU: &str = r#"## Main
Let G be a 1 by 4 tensor of Real.
Set item 1 of G to -1.0.
Set item 2 of G to 0.0.
Set item 3 of G to 1.0.
Set item 4 of G to 2.0.
Let U be a 1 by 4 tensor of Real.
Set item 1 of U to 1.0.
Set item 2 of U to 2.0.
Set item 3 of U to 3.0.
Set item 4 of U to 4.0.
Let Y be swiglu of G and U.
Show round(item 1 of Y * 1000000).
Show round(item 4 of Y * 1000000).
"#;

/// Softmax — the FUSED `ComputeOp::RowReduce`, whose pipeline is not the RMS one (a max pass before
/// the exp), so it is the workload that proves a refusal can be specific about WHICH row op it means.
const SUGAR_SOFTMAX: &str = r#"## Main
Let X be a 1 by 4 tensor of Real.
Set item 1 of X to 1.0.
Set item 2 of X to 2.0.
Set item 3 of X to 3.0.
Set item 4 of X to 4.0.
Let Y be softmax of X.
Show item 1 of Y.
Show item 4 of Y.
"#;

/// LayerNorm — the fused row op with BOTH a per-column weight and a per-column bias (four input
/// tensors), which is also the widest operand arity any workload here presents to a backend.
const SUGAR_LAYERNORM: &str = r#"## Main
Let X be a 2 by 3 tensor of Real.
Set item 1 of X to 1.0.
Set item 2 of X to 2.0.
Set item 3 of X to 3.0.
Set item 4 of X to 4.0.
Set item 5 of X to 5.0.
Set item 6 of X to 6.0.
Let W be a 1 by 3 tensor of Real.
Set item 1 of W to 0.5.
Set item 2 of W to 1.0.
Set item 3 of W to 2.0.
Let B be a 1 by 3 tensor of Real.
Set item 1 of B to 0.25.
Set item 2 of B to 0.5.
Set item 3 of B to 0.75.
Let Y be layernorm of X with weight W and bias B.
Show item 1 of Y.
Show item 4 of Y.
"#;

const WORKLOADS: &[(&str, &str)] = &[
    ("matmul", SUGAR_MATMUL),
    ("elementwise", SUGAR_PLUS),
    ("rmsnorm", SUGAR_RMSNORM),
    ("swiglu", SUGAR_SWIGLU),
    ("softmax", SUGAR_SOFTMAX),
    ("layernorm", SUGAR_LAYERNORM),
];

/// Every backend, each paired with the target it is built for — a backend is only ever handed a
/// program the front end compiled FOR IT, so a refusal is about the workload, never about a target
/// mismatch nobody claimed.
fn backends() -> Vec<(&'static str, Box<dyn SpatialBackend>)> {
    vec![
        (
            "Tensix",
            Box::new(TensixBackend::new(SpatialTarget::wormhole())) as Box<dyn SpatialBackend>,
        ),
        (
            "fabric",
            Box::new(FabricBackend::new(SpatialTarget::sky_fabric(64, 64)))
                as Box<dyn SpatialBackend>,
        ),
        (
            "RDU",
            Box::new(RduSpatialBackend::new()) as Box<dyn SpatialBackend>,
        ),
        (
            "CUDA",
            Box::new(CudaPersistentBackend::new(SpatialTarget::rtx3060()))
                as Box<dyn SpatialBackend>,
        ),
    ]
}

// ── The refusal-quality bar ───────────────────────────────────────────────────────────────────────

/// Words that carry no information about WHAT was refused. A reason made only of these is a shrug.
const STOPWORDS: &[&str] = &[
    "the", "that", "this", "with", "without", "than", "then", "from", "into", "onto", "over",
    "under", "when", "where", "which", "whose", "while", "because", "only", "also", "each",
    "every", "some", "must", "cannot", "never", "always", "does", "done", "have", "has", "had",
    "been", "being", "its", "their", "them", "they", "there", "here", "what", "would", "could",
    "should", "upon", "about", "above", "below", "after", "before", "between", "beyond", "least",
    "most", "more", "less", "one", "two", "and", "but", "nor", "neither", "either", "both", "not",
    "are", "was", "were", "for", "per", "via", "still", "yet",
];

/// Reasons that are placeholders rather than refusals. A backend that says one of these has told you
/// nothing you did not already know from the fact that it failed.
const PLACEHOLDERS: &[&str] = &[
    "",
    "todo",
    "fixme",
    "tbd",
    "n/a",
    "na",
    "none",
    "unknown",
    "error",
    "unsupported",
    "unimplemented",
    "not supported",
    "not implemented",
    "not yet implemented",
    "no",
    "?",
    "-",
    "--",
    "xxx",
    "wip",
    "unhandled",
    "other",
    "op",
    "invalid",
    "failed",
    "failure",
    "bad",
    "oops",
    "sorry",
    "this workload",
    "this op",
];

/// Substrings that mark unfinished text wherever they appear.
const PLACEHOLDER_MARKERS: &[&str] = &[
    "todo",
    "fixme",
    "unimplemented",
    "not implemented",
    "not yet implemented",
    "xxx",
    "???",
];

/// The IR's own nouns. A refusal must speak at least one of them — that is what "names the op or the
/// shape" means operationally: the reason points at an object the program actually contains, not at
/// the backend's mood. A vocabulary word matches a token exactly, or is a prefix of it when it is at
/// least four characters (so `tile` covers `tiled`/`tiles` and `dequant` covers `dequantize`).
///
/// ★ THIS LIST GROWS, IT NEVER SHRINKS. A new refusal whose noun is missing here is a signal to add
/// the noun, not to soften the refusal or delete the check.
const IR_VOCABULARY: &[&str] = &[
    // Program objects.
    "tensor",
    "stage",
    "buffer",
    "edge",
    "semaphore",
    "handshake",
    "schedule",
    "placement",
    "site",
    "engine",
    "kernel",
    "program",
    "tile",
    "page",
    "block",
    "plan",
    "grid",
    "item",
    "work",
    "loop",
    "phase",
    "group",
    "barrier",
    "multicast",
    "mcast",
    "fifo",
    "circular",
    "operand",
    "fill",
    "external",
    "range",
    "stride",
    "lattice",
    "memory",
    "register",
    "descriptor",
    "target",
    "fabric",
    "gpu",
    "rdu",
    "tensix",
    "wmma",
    "noc",
    "shape",
    "layout",
    "space",
    "data",
    "value",
    "counter",
    "certificate",
    "choreography",
    "census",
    // Stage and compute-op names.
    "load",
    "store",
    "recv",
    "compute",
    "matmul",
    "gemm",
    "scalar",
    "elementwise",
    "swiglu",
    "rmsnorm",
    "softmax",
    "layernorm",
    "reduce",
    "row",
    "rescale",
    "broadcast",
    "requant",
    "dequant",
    "quant",
    "bias",
    "relu",
    "pack",
    "accum",
    "spill",
    "epilogue",
    "prologue",
    "oracle",
    "golden",
    "host",
    "serialize",
    "deadlock",
    "align",
    // Numerics.
    "precision",
    "numerics",
    "integer",
    "float",
    "int8",
    "int16",
    "int32",
    "int64",
    "bf16",
    "fp8",
    "fp16",
    "fp32",
    "mantissa",
    "overflow",
    // Roles.
    "input",
    "output",
    "param",
];

/// Hold a refusal to the bar. `Ok(())`, or the RULE it broke — the message names the rule so a fix is
/// obvious from the failure alone.
fn refusal_quality(reason: &str) -> Result<(), String> {
    let trimmed = reason.trim();
    if trimmed.is_empty() {
        return Err("R1: the reason is empty — a refusal must say what it refused".to_string());
    }
    let lower = trimmed.to_ascii_lowercase();
    if PLACEHOLDERS.contains(&lower.as_str()) {
        return Err(format!("R4: `{trimmed}` is a placeholder, not a reason — name the op, the shape or the object the backend could not take"));
    }
    for marker in PLACEHOLDER_MARKERS {
        if lower.contains(*marker) {
            return Err(format!("R4: `{trimmed}` carries the unfinished-text marker `{marker}` — finish the sentence or delete it"));
        }
    }
    let toks = tokens(trimmed);
    if toks.len() < 2 {
        return Err(format!(
            "R2: `{trimmed}` is a single token — a refusal is a phrase, not a label"
        ));
    }
    if !toks
        .iter()
        .any(|t| t.chars().count() >= 4 && !STOPWORDS.contains(&t.as_str()))
    {
        return Err(format!(
            "R3: `{trimmed}` is made only of function words — it names nothing"
        ));
    }
    if !toks.iter().any(|t| names_an_ir_object(t)) {
        return Err(format!(
            "R5: `{trimmed}` names no object of the spatial IR. A refusal must point at what it \
             refused (the op, the shape, the buffer, the precision). If the wording IS specific and \
             the noun is simply missing from `IR_VOCABULARY`, ADD THE NOUN — do not weaken this check"
        ));
    }
    Ok(())
}

fn tokens(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_ascii_lowercase())
        .collect()
}

fn names_an_ir_object(token: &str) -> bool {
    IR_VOCABULARY
        .iter()
        .any(|v| *v == token || (v.len() >= 4 && token.starts_with(*v)))
}

// ── Rendering a refusal ───────────────────────────────────────────────────────────────────────────

/// A backend's refusal as a sentence. The typed variants carry their payload as structured data (a
/// group, a buffer, a precision), so the payload IS the naming; `Unsupported` and `TooBig` carry
/// hand-written prose, which is exactly where the quality bar has to bite.
fn lower_reason(e: &SpatialLowerError) -> String {
    match e {
        SpatialLowerError::NotVerified { phase } => {
            format!("the program is not verified — its phase is {phase:?}")
        }
        SpatialLowerError::Unsupported(what) => (*what).to_string(),
        SpatialLowerError::GroupSpansSites { group } => format!(
            "multicast group {} spans sites this backend cannot address",
            group.idx()
        ),
        SpatialLowerError::NotNearestNeighbour { buffer } => format!(
            "buffer {} needs a hop that is not nearest-neighbour",
            buffer.idx()
        ),
        SpatialLowerError::TooBig { what, need, have } => {
            format!("{what} — it needs {need}, the target has {have}")
        }
        SpatialLowerError::Precision { needed } => {
            format!("the target has no datapath for the {needed:?} precision")
        }
        SpatialLowerError::OverCapacity(r) => {
            let over = r
                .over
                .iter()
                .map(|o| format!("{} {} but the {} part has {}", o.need, o.resource.label(), r.part, o.have))
                .collect::<Vec<_>>()
                .join("; ");
            format!("the program's tensors need {over}")
        }
        SpatialLowerError::Uncertified(fault) => {
            format!("the choreography does not certify — {fault:?}")
        }
    }
}

/// The front end's refusal as a sentence, held to the same bar.
fn reject_reason(e: &SpatialReject) -> String {
    match e {
        SpatialReject::Parse(msg) => format!("the source does not parse into a tensor program — {msg}"),
        SpatialReject::Unrecognized { matmul, elementwise, reduction } => {
            format!("no tensor op recognized in the source (matmul: {matmul:?}; elementwise: {elementwise:?}; reduction: {reduction:?})")
        }
        SpatialReject::NotLowered(what) => (*what).to_string(),
        SpatialReject::ScalarFrontEnd(msg) => format!("the scalar front end cannot build a dataflow body from this program — {msg}"),
        SpatialReject::NotQ8_0 { weight } => format!("the weight {weight} is stored Q8_0 but its values are not an int8 multiple of a scale per 32 rows"),
        SpatialReject::MissingCompute { needed, target } => format!("the target {target} is missing the compute capability: {needed}"),
        SpatialReject::MissingPrecision { needed, target } => format!("the target {target} is missing the {needed:?} precision"),
        SpatialReject::MissingHandshake { needed, target } => format!("the target {target} is missing the {needed:?} handshake"),
        SpatialReject::MissingInterconnect { needed, target } => format!("the target {target} is missing the {needed:?} interconnect"),
        SpatialReject::NoLegalTiling { rows_t, cols_t, k_tiles, acc_tiles, sites } => {
            format!("no legal tiling of a {rows_t}×{cols_t} tile grid over {k_tiles} k-tiles into {acc_tiles} accumulator tiles across {sites} sites")
        }
        SpatialReject::LocalMemExceeded { site, needed_bytes, budget_bytes } => {
            format!("site ({},{}) needs {needed_bytes} bytes of local memory, its budget is {budget_bytes}", site.x, site.y)
        }
        SpatialReject::AccTilesExceeded { needed, available, acc } => format!("the accumulator needs {needed} tiles under {acc:?}, the site holds {available}"),
        SpatialReject::TooManyEngines { site, needed, available } => format!("site ({},{}) needs {needed} engines, the target has {available}", site.x, site.y),
        SpatialReject::GroupCountMismatch { group, counts } => format!("multicast group {} delivers unequal page counts {counts:?}", group.idx()),
        SpatialReject::WrongPhase { pass, expected, found } => format!("the pass {pass} wants a {expected} program, the phase is {found:?}"),
        SpatialReject::Unverified(fault) => format!("the choreography does not evaluate — {fault:?}"),
        SpatialReject::Invalid(v) => format!("the program breaks a structural invariant of the IR — {v:?}"),
    }
}

/// ★ THE TWO REJECTS THAT ARE NEVER A CAPABILITY GAP. `Parse` and `Unrecognized` say the recognizer
/// could not see a tensor op in a source THIS FILE wrote — and every one of these sources is the shape
/// an existing green lock already compiles. Both are target-independent (`recognize_any` runs before
/// the target is consulted), so neither can ever be an honest statement about a backend. Letting them
/// fall through as a `no-prog` cell is how this whole grid would go quietly green after a front-end
/// regression, which is precisely the failure `spatial_algebra_semantics_lock` panics on too.
fn is_recognizer_regression(e: &SpatialReject) -> bool {
    matches!(
        e,
        SpatialReject::Parse(_) | SpatialReject::Unrecognized { .. }
    )
}

/// The compute ops a program carries, named — so a refusal cell says WHAT could not be lowered.
fn compute_ops(p: &SpatialProgram) -> Vec<String> {
    p.stages
        .iter()
        .filter_map(|s| match &s.kind {
            StageKind::Compute(ComputeOp::MatmulTiles { .. }) => Some("MatmulTiles".to_string()),
            StageKind::Compute(other) => Some(
                format!("{other:?}")
                    .split_whitespace()
                    .next()
                    .unwrap_or("?")
                    .to_string(),
            ),
            _ => None,
        })
        .collect()
}

// ── The grid ──────────────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
enum Verdict {
    /// The backend printed an artifact.
    Lowered,
    /// The backend printed a certified placement and named the vendor SDK it is gated on — a success
    /// it CAN justify, held to the refusal bar for its gate reason.
    Gated,
    /// The backend refused, by name.
    Refused,
    /// The front end never built a program for this target — a typed capability reject, also held to
    /// the bar. (A `Parse` / `Unrecognized` reject is NOT this: see `is_recognizer_regression`.)
    FrontEnd,
    /// The only outcome this lock forbids.
    Panicked,
}

impl Verdict {
    fn tag(self) -> &'static str {
        match self {
            Verdict::Lowered => "lower",
            Verdict::Gated => "gated",
            Verdict::Refused => "refuse",
            Verdict::FrontEnd => "no-prog",
            Verdict::Panicked => "PANIC!",
        }
    }
    /// The backend was actually handed a verified program and answered for itself.
    fn reached_the_backend(self) -> bool {
        matches!(self, Verdict::Lowered | Verdict::Gated | Verdict::Refused)
    }
    fn printed_an_artifact(self) -> bool {
        matches!(self, Verdict::Lowered | Verdict::Gated)
    }
}

struct Cell {
    verdict: Verdict,
    detail: String,
    /// Everything the backend emitted for this pair, concatenated — `None` unless it lowered. Kept so
    /// the grid can prove an artifact is a FUNCTION of the program rather than a fixture.
    emitted: Option<String>,
}

/// Run `f`, converting a panic into a message instead of an aborted process. ★ WITHOUT THIS the very
/// first panicking pair would take the whole matrix down with it, and the gap list — the thing this
/// lock exists to show — would never be printed.
fn caught<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).map_err(|payload| {
        if let Some(s) = payload.downcast_ref::<&'static str>() {
            (*s).to_string()
        } else if let Some(s) = payload.downcast_ref::<String>() {
            s.clone()
        } else {
            "a panic with a non-string payload".to_string()
        }
    })
}

/// Inputs from the program's own recorded fills, TOTAL: a tensor whose data is external to the
/// program contributes zeros rather than a panic. ★ A totality lock that panics while building its
/// own inputs would be the joke that writes itself.
fn deterministic_inputs(p: &SpatialProgram) -> Inputs {
    let mut inputs = Inputs::default();
    for t in &p.tensors {
        if t.role == TensorRole::Output {
            continue;
        }
        let n = t.elems.rows * t.elems.cols;
        if t.init.is_float() {
            inputs.tensors.insert(
                t.id,
                Value::Float(
                    (0..n)
                        .map(|i| t.init.element_f64(i).unwrap_or(0.0))
                        .collect(),
                ),
            );
        } else {
            inputs.tensors.insert(
                t.id,
                Value::Int((0..n).map(|i| t.init.element(i).unwrap_or(0)).collect()),
            );
        }
    }
    inputs
}

/// An evaluation outcome, compactly — a whole `Outputs` in a failure message buries the sentence that
/// matters under the tensor it produced.
fn describe_eval(r: &Result<Outputs, ChoreoFault>) -> String {
    match r {
        Ok(o) => format!(
            "{} output tensor(s) after {} steps",
            o.tensors.len(),
            o.steps
        ),
        Err(f) => format!("the fault {f:?}"),
    }
}

/// What an artifact exposes: its census, every byte of source it emitted, and the gate reason where
/// the lane is honestly SDK-gated.
fn artifact_evidence(art: &SpatialArtifact) -> (Census, String, Option<String>) {
    match art {
        SpatialArtifact::Tensix(a) => {
            let mut text = String::new();
            text.push_str(&a.host_cpp);
            text.push_str(&a.cmake);
            text.push_str(&a.oracle_c);
            for k in &a.kernels {
                text.push_str(&k.name);
                text.push_str(&k.source);
            }
            (a.census.clone(), text, None)
        }
        SpatialArtifact::Verilog(a) => {
            (a.census.clone(), format!("{}{}", a.rtl, a.testbench), None)
        }
        SpatialArtifact::Cuda(a) => (
            a.census.clone(),
            format!("{}{}", a.program_cu, a.bench_cu),
            None,
        ),
        SpatialArtifact::SdkGated(a) => (
            a.census.clone(),
            format!("{}{}", a.json, a.pretty),
            Some(a.reason.to_string()),
        ),
    }
}

fn render_matrix(labels: &[String], rows: &[(String, Vec<Cell>)]) -> String {
    let head = "workload";
    let name_w = rows
        .iter()
        .map(|(n, _)| n.len())
        .chain(std::iter::once(head.len()))
        .max()
        .unwrap_or(head.len());
    let mut widths: Vec<usize> = labels.iter().map(|l| l.len()).collect();
    for (_, cells) in rows {
        for (i, c) in cells.iter().enumerate() {
            widths[i] = widths[i].max(c.verdict.tag().len());
        }
    }
    let mut s = String::new();
    s.push_str(&format!("  {head:<name_w$} "));
    for (i, l) in labels.iter().enumerate() {
        let w = widths[i];
        s.push_str(&format!("| {l:<w$} "));
    }
    s.push('\n');
    s.push_str(&format!("  {:-<name_w$}-", ""));
    for w in &widths {
        let w = *w;
        s.push_str(&format!("+{:-<w$}--", ""));
    }
    s.push('\n');
    for (name, cells) in rows {
        s.push_str(&format!("  {name:<name_w$} "));
        for (i, c) in cells.iter().enumerate() {
            let w = widths[i];
            let tag = c.verdict.tag();
            s.push_str(&format!("| {tag:<w$} "));
        }
        s.push('\n');
    }
    s.push_str(
        "\n  lower = an artifact was printed · gated = certified placement, vendor SDK named ·\n",
    );
    s.push_str(
        "  refuse = a typed, named refusal · no-prog = the target cannot host this workload ·\n",
    );
    s.push_str("  PANIC! = the one outcome this lock forbids.\n\n");
    for (name, cells) in rows {
        for (i, c) in cells.iter().enumerate() {
            if !c.detail.is_empty() {
                s.push_str(&format!("  · {name} × {}: {}\n", labels[i], c.detail));
            }
        }
    }
    s
}

// ── The lock ──────────────────────────────────────────────────────────────────────────────────────

#[test]
fn every_workload_and_backend_pair_lowers_or_refuses_by_name() {
    let backend_list = backends();
    let labels: Vec<String> = backend_list
        .iter()
        .map(|(kind, b)| format!("{kind}({})", b.name()))
        .collect();
    let mut rows: Vec<(String, Vec<Cell>)> = Vec::new();
    let mut failures: Vec<String> = Vec::new();

    for (workload, src) in WORKLOADS {
        let mut cells: Vec<Cell> = Vec::new();
        // `Parse` / `Unrecognized` is target-independent, so it is one defect, not four. Reported once
        // per workload — the same sentence in every column would bury the rest of the grid.
        let mut flagged_recognizer = false;
        for ((kind, backend), label) in backend_list.iter().zip(&labels) {
            let pair = format!("{workload} × {label}");
            // The front end, under the same no-panic rule: a recognizer that panics on a sugar it
            // does not handle is the same defect as a backend that panics on an op it cannot print.
            let compiled = match caught(|| compile_to_spatial(*src, backend.target())) {
                Err(msg) => {
                    failures.push(format!("{pair}: the FRONT END panicked — `{msg}`. A recognizer refuses typed, it never unwinds."));
                    cells.push(Cell {
                        verdict: Verdict::Panicked,
                        detail: format!("the front end panicked — `{msg}`"),
                        emitted: None,
                    });
                    continue;
                }
                Ok(r) => r,
            };
            let p = match compiled {
                Err(reject) => {
                    let reason = reject_reason(&reject);
                    if let Err(rule) = refusal_quality(&reason) {
                        failures.push(format!("{pair}: the front-end reject is unreadable. {rule}\n      the typed reject was {reject:?}"));
                    }
                    if is_recognizer_regression(&reject) && !flagged_recognizer {
                        flagged_recognizer = true;
                        failures.push(format!(
                            "{pair}: the recognizer no longer sees a tensor op in the `{workload}` source — {reject:?}\n      \
                             `Parse` / `Unrecognized` is target-independent, so it is never a statement about this backend: \
                             it means the sugar or the recognizer moved. Fix `tensor_recognize` / the parser, or fix the \
                             source constant above — NEVER let this fall through as a `no-prog` cell, which is how the whole \
                             grid would go green while testing nothing.\n{src}"
                        ));
                    }
                    cells.push(Cell {
                        verdict: Verdict::FrontEnd,
                        detail: format!("the target cannot host it — {reason}"),
                        emitted: None,
                    });
                    continue;
                }
                Ok(p) => p,
            };
            if p.phase() != Phase::Verified {
                failures.push(format!("{pair}: `compile_to_spatial` returned Ok at phase {:?} — a compiled program is a VERIFIED program.", p.phase()));
            }
            let ops = compute_ops(&p);

            // The meaning of the program, taken BEFORE the lowering runs.
            let before = eval_choreography(&p, &deterministic_inputs(&p));
            let snapshot = p.clone();

            let lowered = match caught(|| backend.lower(&p)) {
                Err(msg) => {
                    failures.push(format!(
                        "{pair}: the {kind} lowering PANICKED — `{msg}`.\n      \
                         compute ops in the program: {ops:?}\n      \
                         A backend is total: it lowers the program or returns a NAMED `SpatialLowerError`. \
                         The fix is a typed refusal arm in the backend, never a weakening of this lock."
                    ));
                    cells.push(Cell {
                        verdict: Verdict::Panicked,
                        detail: format!("panicked — `{msg}` (ops {ops:?})"),
                        emitted: None,
                    });
                    continue;
                }
                Ok(r) => r,
            };

            match lowered {
                Err(e) => {
                    let reason = lower_reason(&e);
                    if let Err(rule) = refusal_quality(&reason) {
                        failures.push(format!(
                            "{pair}: the refusal is anonymous. {rule}\n      the typed error was {e:?}\n      \
                             compute ops in the program: {ops:?}\n      \
                             Refusing is FINE — refusing without saying what you refused is not."
                        ));
                    }
                    // A refusal must also be deterministic: the same program twice is the same verdict.
                    match caught(|| backend.lower(&p)) {
                        Ok(Err(again)) if again == e => {}
                        Ok(Err(again)) => failures.push(format!("{pair}: the refusal is not deterministic — first {e:?}, then {again:?}")),
                        Ok(Ok(_)) => failures.push(format!("{pair}: the backend refused, then lowered the SAME program — a lowering must be a function")),
                        Err(msg) => failures.push(format!("{pair}: the backend refused, then panicked on the SAME program — `{msg}`")),
                    }
                    cells.push(Cell {
                        verdict: Verdict::Refused,
                        detail: format!("refused — {reason} (ops {ops:?})"),
                        emitted: None,
                    });
                    continue;
                }
                Ok(art) => {
                    let (census, emitted, gate) = artifact_evidence(&art);
                    let ir_census = Census::of(&p);
                    if census != ir_census {
                        failures.push(format!(
                            "{pair}: the artifact's census {census:?} is not the IR's {ir_census:?} — every construct a \
                             backend emits must be the image of an IR object."
                        ));
                    }
                    if emitted.is_empty() {
                        failures.push(format!("{pair}: the backend reported success and emitted nothing — a lowering that prints no source has not lowered anything."));
                    }
                    if let SpatialArtifact::SdkGated(neutral) = &art {
                        if !neutral.certificate.recheck(&p) {
                            failures.push(format!("{pair}: the SDK-gated artifact ships a certificate that does not re-verify against its own program."));
                        }
                    }
                    // ★ WHAT THIS PAIR OF EVALUATIONS DOES AND DOES NOT PROVE. `lower` takes `&SpatialProgram`,
                    // so the compiler already forbids it from editing the program — the honest content of this
                    // check is narrower and still worth having: the lowering disturbed no state the EVALUATOR
                    // reads (a global cache, an interner, a thread-local), and the evaluator itself is
                    // deterministic across it. The `p != snapshot` line below is the matching tripwire for the
                    // day someone gives the trait a `&mut` or hides a `Cell` in the IR.
                    //
                    // ★ AND `Err == Err` PASSES ON PURPOSE. `verify` certifies the DRY (token-only) run, so a
                    // verified program whose evaluator arm has no arithmetic for this op yet faults identically
                    // before and after. That gap belongs to `spatial_algebra_semantics_lock`, not to a backend's
                    // name, so it is carried into the matrix cell and stays visible rather than asserted away.
                    let after = eval_choreography(&p, &deterministic_inputs(&p));
                    let mut note = String::new();
                    if before != after {
                        failures.push(format!(
                            "{pair}: `eval_choreography` gave {} before the lowering and {} after it, on the SAME \
                             program and the SAME inputs. Either the backend scribbled on state the evaluator reads, \
                             or the evaluator is not deterministic. A backend reads the IR; it never moves anything.",
                            describe_eval(&before),
                            describe_eval(&after)
                        ));
                    } else if let Err(f) = &before {
                        note = format!(
                            "lowered, but the choreography has no arithmetic meaning yet — {f:?}"
                        );
                    }
                    if p != snapshot {
                        failures.push(format!("{pair}: the lowering MUTATED the program it was handed — a backend reads the IR, it never edits it."));
                    }
                    match caught(|| backend.lower(&p)) {
                        Ok(Ok(again)) => {
                            let (census2, emitted2, _) = artifact_evidence(&again);
                            if census2 != census || emitted2 != emitted {
                                failures.push(format!(
                                    "{pair}: lowering is not deterministic — two runs of the SAME backend on the SAME program \
                                     printed different artifacts ({} vs {} bytes).",
                                    emitted.len(),
                                    emitted2.len()
                                ));
                            }
                        }
                        Ok(Err(e)) => failures.push(format!("{pair}: the backend lowered, then refused the SAME program — {e:?}")),
                        Err(msg) => failures.push(format!("{pair}: the backend lowered, then panicked on the SAME program — `{msg}`")),
                    }
                    let verdict = if let Some(reason) = gate {
                        if let Err(rule) = refusal_quality(&reason) {
                            failures.push(format!(
                                "{pair}: the SDK gate is unreadable. {rule}\n      \
                                 An honest gate is a refusal with an artifact attached — it owes the same named reason."
                            ));
                        }
                        Verdict::Gated
                    } else {
                        Verdict::Lowered
                    };
                    cells.push(Cell {
                        verdict,
                        detail: note,
                        emitted: Some(emitted),
                    });
                }
            }
        }
        rows.push((workload.to_string(), cells));
    }

    // ── The grid must have teeth ──────────────────────────────────────────────────────────────────
    //
    // ★ EVERY ASSERTION ABOVE LIVES INSIDE THE GRID LOOP, SO AN EMPTY GRID SATISFIES ALL OF THEM.
    // These four floors are the only thing standing between this lock and a green run that proves
    // nothing: they fail if the workload table or the backend list is emptied, if the front end stops
    // handing any backend a program, if a whole column is never exercised, or if not one backend in
    // the matrix ever printed anything. NONE of them counts refusals or names a gap — closing a gap
    // still flips a cell and leaves every floor satisfied.
    if rows.len() != WORKLOADS.len() {
        failures.push(format!("the grid printed {} rows for {} workloads — the matrix is not the cross product it claims to be", rows.len(), WORKLOADS.len()));
    }
    if WORKLOADS.len() < 6 {
        failures.push(format!(
            "`WORKLOADS` holds {} source(s). It held six — every tensor sugar the front end recognizes — and this \
             floor is a RATCHET: a seventh sugar keeps it green, deleting one to dodge a refusal does not.",
            WORKLOADS.len()
        ));
    }
    if labels.len() < 4 {
        failures.push(format!(
            "the grid runs {} backend(s). All four lowering lanes (Tensix, our fabric, the RDU, the persistent GPU \
             kernel) belong in `backends()` — a lane dropped from here is a lane nobody holds to totality.",
            labels.len()
        ));
    }
    for (name, cells) in &rows {
        if cells.len() != labels.len() {
            failures.push(format!(
                "the `{name}` row has {} cells for {} backends — the grid is ragged",
                cells.len(),
                labels.len()
            ));
        }
    }
    for (i, label) in labels.iter().enumerate() {
        let reached = rows
            .iter()
            .filter(|(_, cells)| {
                cells
                    .get(i)
                    .map(|c| c.verdict.reached_the_backend())
                    .unwrap_or(false)
            })
            .count();
        if reached == 0 {
            failures.push(format!(
                "{label} was never handed a single verified program: every cell in its column is `no-prog` or `PANIC!`, so \
                 nothing in this lock ever asked that backend a question. The front end must compile at least one of these \
                 six workloads for its target — fix the pipeline, never this floor."
            ));
        }
    }
    let printed = rows
        .iter()
        .flat_map(|(_, c)| c.iter())
        .filter(|c| c.verdict.printed_an_artifact())
        .count();
    if printed == 0 {
        failures.push(
            "not one pair in the whole grid printed an artifact, so every `lower`-side assertion (census, emitted bytes, \
             determinism, artifact-is-a-function-of-the-program) ran zero times. A matrix of nothing but refusals is a \
             regression, not a pass."
                .to_string(),
        );
    }
    // ★ AN ARTIFACT IS A FUNCTION OF THE PROGRAM, NOT A FIXTURE. Two different workloads handed to the
    // same backend describe different stages, shapes and goldens, so they can never print identical
    // text. This is the assertion that rules out the cheapest possible fake totality: a backend that
    // "supports" everything by returning a canned bundle.
    for (i, label) in labels.iter().enumerate() {
        for a in 0..rows.len() {
            for b in (a + 1)..rows.len() {
                let (name_a, cells_a) = &rows[a];
                let (name_b, cells_b) = &rows[b];
                let (Some(x), Some(y)) = (
                    cells_a.get(i).and_then(|c| c.emitted.as_ref()),
                    cells_b.get(i).and_then(|c| c.emitted.as_ref()),
                ) else {
                    continue;
                };
                if x == y {
                    failures.push(format!(
                        "{label}: `{name_a}` and `{name_b}` are different programs, yet the backend printed the IDENTICAL \
                         artifact ({} bytes) for both. An artifact is the image of the program it was handed — a lowering \
                         that prints the same text whatever it is given has not lowered anything.",
                        x.len()
                    ));
                }
            }
        }
    }

    let matrix = render_matrix(&labels, &rows);
    println!("\nSPATIAL BACKEND TOTALITY — workload × backend\n\n{matrix}");
    assert!(
        failures.is_empty(),
        "\n{} totality violation(s) across the workload × backend grid:\n\n{}\n\nthe grid:\n\n{}\n\
         ★ Every fix belongs in the BACKEND (a typed, named refusal arm, or a real lowering) or in the \
         FRONT END — never in this lock. It counts no refusals and names no gap, so closing one flips a \
         cell and this test keeps passing untouched.\n",
        failures.len(),
        failures.join("\n\n"),
        matrix
    );
}

#[test]
fn every_backend_refuses_an_unverified_program_with_its_phase() {
    // ★ THE PRECONDITION IS PART OF TOTALITY. Handed a TEMPLATE — a program that has not been placed,
    // sized, scheduled or certified — a backend must refuse by phase, not index into a `None`
    // placement and panic. This is the seam every one of the four shares (`require_verified` is the
    // FIRST line of all four `lower` bodies), and it is the cheapest place for a new backend to get it
    // wrong. Because all four already open with it, this test demands the `NotVerified` arm by name:
    // "some other typed refusal" would mean a backend got further into a template than it should.
    let mut failures: Vec<String> = Vec::new();
    let mut exercised = 0usize;
    let all = backends();
    let total = all.len();
    for (kind, backend) in all {
        let pair = format!("{kind}({})", backend.name());
        let template = match caught(|| template_from_source(SUGAR_MATMUL, backend.target())) {
            Err(msg) => {
                failures.push(format!("{pair}: building the template panicked — `{msg}`"));
                continue;
            }
            Ok(Err(reject)) => {
                let reason = reject_reason(&reject);
                if let Err(rule) = refusal_quality(&reason) {
                    failures.push(format!(
                        "{pair}: the front-end reject is unreadable. {rule}"
                    ));
                }
                failures.push(format!(
                    "{pair}: the front end refused to TEMPLATE a plain 2×2 integer GEMM — {reject:?}\n      \
                     `template_from_source` runs no tiling and no placement, so a matmul every one of these targets \
                     declares a matrix engine for must lift to a template on all of them (`spatial_one_ir_lock` \
                     templates the same op on all four). Fix the front end — do not skip the backend here, or this \
                     test stops covering that lane's `require_verified`."
                ));
                continue;
            }
            Ok(Ok(t)) => t,
        };
        let phase = template.phase();
        if phase == Phase::Verified {
            failures.push(format!("{pair}: `template_from_source` returned a VERIFIED program — the template phase is the whole point of this test"));
            continue;
        }
        exercised += 1;
        match caught(|| backend.lower(&template)) {
            Err(msg) => failures.push(format!(
                "{pair}: lowering an UNVERIFIED program panicked — `{msg}`. Every backend opens with \
                 `require_verified`; this one reached past it."
            )),
            Ok(Ok(_)) => failures.push(format!(
                "{pair}: lowered an UNVERIFIED program (phase {phase:?}). An artifact printed from a program with no \
                 placement, no schedule and no certificate is a claim the backend cannot support."
            )),
            Ok(Err(SpatialLowerError::NotVerified { phase: reported })) => {
                if reported != phase {
                    failures.push(format!(
                        "{pair}: refused as unverified but reported phase {reported:?} for a {phase:?} program — the \
                         refusal must carry the program's OWN phase, or it cannot tell you how far the pipeline got."
                    ));
                }
                let reason = lower_reason(&SpatialLowerError::NotVerified { phase: reported });
                if let Err(rule) = refusal_quality(&reason) {
                    failures.push(format!("{pair}: {rule}"));
                }
            }
            Ok(Err(other)) => {
                let reason = lower_reason(&other);
                let rule = refusal_quality(&reason).err().unwrap_or_else(|| "(the reason itself is well-formed)".to_string());
                failures.push(format!(
                    "{pair}: refused a TEMPLATE with {other:?} instead of `NotVerified {{ phase: {phase:?} }}`. {rule}\n      \
                     All four backends open with `require_verified(p)?`, so reaching any other refusal arm means this one \
                     inspected a program it had already established it may not read. Put `require_verified(p)?` back on the \
                     FIRST line — never relax this arm to accept a later refusal."
                ));
            }
        }
    }
    if exercised != total {
        failures.push(format!(
            "only {exercised} of {total} backends were actually handed a template, so the rest never had their \
             `require_verified` seam tested at all. This floor exists because every assertion above is inside the \
             loop: a lane that never gets a program passes this test vacuously."
        ));
    }
    assert!(
        failures.is_empty(),
        "\n{} backend(s) mishandle an unverified program:\n\n{}\n\n\
         The fix is `require_verified(p)?` as the FIRST line of the lowering — never a relaxation here.\n",
        failures.len(),
        failures.join("\n\n")
    );
}

// ── The bar itself, checked in both directions ────────────────────────────────────────────────────

#[test]
fn the_refusal_bar_rejects_placeholders_and_accepts_real_refusals() {
    // ★ THE ANTI-VACUITY LOCK. A quality bar that accepts everything is worse than no bar — it reads
    // as coverage while asserting nothing. So the checker is itself checked, in both directions,
    // against the exact shapes a rushed refusal takes.
    let must_fail: &[(&str, &str)] = &[
        ("", "empty"),
        ("   ", "whitespace only"),
        ("TODO", "a marker"),
        ("todo: handle this", "a marker in a sentence"),
        ("unsupported", "the enum variant restated"),
        ("Unsupported", "the enum variant restated, capitalized"),
        ("not implemented", "a shrug"),
        ("unimplemented!()", "the macro leaked into the message"),
        ("error", "a category, not a reason"),
        ("op", "a single label"),
        ("?", "punctuation"),
        ("the that this with", "function words only"),
        (
            "we cannot do that here",
            "a sentence that names no IR object",
        ),
        ("FIXME name this later", "an unfinished marker"),
    ];
    for (bad, why) in must_fail {
        assert!(
            refusal_quality(bad).is_err(),
            "the refusal bar accepted `{bad}` ({why}). A backend could then refuse anonymously and this \
             lock would call it named — fix `refusal_quality`, and NEVER loosen it to make a backend pass."
        );
    }

    // Real refusals the backends emit today, verbatim. These must pass, or the bar is mis-tuned and
    // would push a well-worded refusal toward vaguer wording — the opposite of the point.
    let must_pass: &[&str] = &[
        "a non-matmul compute op on Tensix (P4)",
        "a fused row-reduce (softmax / layernorm) on Tensix — only rmsnorm is built as a stage chain (P4)",
        "a float row op on the integer fabric",
        "a tile shape other than WMMA's 16×16",
        "a program without a tile matmul",
        "a semaphore handshake on sm_86 (no cluster-wide multicast)",
        "integer accumulate on a float-primary matrix engine",
        "an int8 requant epilogue on Tensix (P4b)",
        "a golden for quantize / dequantize on Tensix (P4b)",
        "an accumulator spill on the GPU (the fragments never leave the registers)",
        "an integer oracle for external data",
        "a site's work items are neither a linear range nor a stride lattice",
        "engine programs the block cannot serialize without deadlock",
        "Tensix tiles are 32×32",
        "a tiled plan",
        "a matrix engine",
        "an output tensor",
        "multi-input scalar bodies",
        "RDA/SambaFlow config emission is gated on the vendor SDK; the certified placement and schedule are ready to bridge",
    ];
    for good in must_pass {
        assert!(
            refusal_quality(good).is_ok(),
            "the refusal bar rejected a REAL, well-worded refusal: `{good}` — {}. Widen `IR_VOCABULARY` \
             (nouns only go in, never out); do not reword the backend's message to satisfy a mis-tuned check.",
            refusal_quality(good).unwrap_err()
        );
    }
}

// ── Every refusal arm, not just the reachable ones ────────────────────────────────────────────────

/// The `spatial/lower` tree, as the test binary sees it.
fn lowering_sources() -> Vec<(String, String)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the workspace root resolves from the test crate");
    let dir = root.join("crates/logicaffeine_compile/src/spatial/lower");
    fn walk(d: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for e in
            std::fs::read_dir(d).unwrap_or_else(|err| panic!("cannot read {}: {err}", d.display()))
        {
            let p = e.expect("a directory entry").path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().map(|x| x == "rs").unwrap_or(false) {
                out.push(p);
            }
        }
    }
    let mut files = Vec::new();
    walk(&dir, &mut files);
    files
        .into_iter()
        .map(|f| {
            let rel = f
                .strip_prefix(&root)
                .unwrap_or(f.as_path())
                .to_string_lossy()
                .replace('\\', "/");
            let text =
                std::fs::read_to_string(&f).unwrap_or_else(|e| panic!("cannot read {rel}: {e}"));
            (rel, text)
        })
        .collect()
}

/// Every string literal that follows `marker` in `text`. Deliberately a lexer of one rule — the
/// refusal literals are plain single-line strings, and a literal that ever grows an escape still
/// reaches the bar with its words intact.
fn literals_after(text: &str, marker: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut from = 0usize;
    while let Some(rel) = text[from..].find(marker) {
        let start = from + rel + marker.len();
        let rest = &text[start..];
        let mut end = rest.len();
        let mut skip = false;
        for (i, c) in rest.char_indices() {
            if skip {
                skip = false;
                continue;
            }
            match c {
                '\\' => skip = true,
                '"' => {
                    end = i;
                    break;
                }
                _ => {}
            }
        }
        out.push(rest[..end].to_string());
        // ★ `min` IS THE GUARD, NOT A TIDY-UP. An unterminated literal leaves `end == rest.len()`, and
        // `start + end + 1` would then be one past the source — the next `text[from..]` would panic
        // inside the very lock whose subject is "never panic".
        from = (start + end + 1).min(text.len());
    }
    out
}

/// ★ THE GRID ONLY REACHES THE ARMS SIX WORKLOADS HAPPEN TO TRIGGER. Most of a backend's refusal arms
/// are for programs no sugar in this file produces — an int8 requant epilogue, a wide literal fill, a
/// group barrier — and those are exactly the arms a hurried hand writes as `Unsupported("TODO")`. So
/// the bar is applied STATICALLY, to every refusal literal in the lowering sources, whether or not any
/// test can reach it.
///
/// ★ IF THIS GOES RED, THE FIX IS THE BACKEND'S SENTENCE, OR A NOUN ADDED TO `IR_VOCABULARY`. Never
/// the scan, and never the floor below it.
#[test]
fn every_refusal_arm_in_the_backend_sources_names_what_it_refused() {
    let mut found = 0usize;
    let mut failures: Vec<String> = Vec::new();
    for (rel, text) in lowering_sources() {
        for marker in ["Unsupported(\"", "TooBig { what: \""] {
            for lit in literals_after(&text, marker) {
                found += 1;
                if let Err(rule) = refusal_quality(&lit) {
                    failures.push(format!("{rel}: {rule}"));
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "\n{} refusal arm(s) in `spatial/lower` say nothing about what they refused:\n\n{}\n\n\
         A refusal is a sentence naming the op, the shape, the buffer or the precision the backend could not \
         take. Rewrite the arm — never delete this check, and never soften the bar.\n",
        failures.len(),
        failures.join("\n")
    );
    assert!(
        found >= 40,
        "the static scan found only {found} refusal literal(s) under `crates/logicaffeine_compile/src/spatial/lower`. \
         There were 67 when this lock was written, so a number this low means the scan lost the sources (a moved \
         directory, a renamed variant) and is now checking nothing. Fix the path or the markers in `literals_after` \
         — do not lower this floor to make the test green."
    );
}
