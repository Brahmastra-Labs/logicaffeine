//! LOGOS → **mixed-signal XPU** — the unification of the digital whole-program engine
//! ([`crate::codegen_sva`]'s wprog engine, exact ≡VM) and the analog plane
//! ([`crate::codegen_ana`], ε-certified floats / ½-grid-certified-exact integers).
//!
//! ## The dispatch covenant (settled 2026-08-30)
//! The analog plane is an ACCELERATOR, never a correctness dependency. A region function
//! dispatches to the plane iff its certificate ADMITTED it (the float budget or the integer
//! exactness gate, calibrated); the DIGITAL lowering of every region remains available in the
//! same artifact, so stripping every binding leaves a program the engine runs ≡VM unchanged.
//! SoC correctness is therefore the digital story; analog changes only energy and latency —
//! inside the compiler's own ε, or exactly, per binding.
//!
//! This module owns the PARTITION (which functions bind to which lane, with ledgered refusals
//! for the rest) and, in later phases, the composed artifact (engine config + region configs +
//! the `MixedCertificate`) and the port models. Region compilation itself is entirely
//! [`crate::codegen_ana::try_region`] — panic-safe, typed refusals, never a crash on arbitrary
//! program shapes.

use crate::codegen_ana::{try_region, AnalogArtifact, AnalogTarget, RegionLane};
use logicaffeine_language::ast::stmt::{Expr, Stmt};

/// One region function bound to the analog plane, certificate-admitted.
#[derive(Debug, Clone)]
pub struct RegionBinding {
    pub fn_name: String,
    pub lane: RegionLane,
    pub artifact: AnalogArtifact,
}

/// One region function that stays digital, with the NAMED reason — the ledger the partition
/// lock pins (shrink-toward-binding is the win condition).
#[derive(Debug, Clone, PartialEq)]
pub struct RegionRefusal {
    pub fn_name: String,
    pub reason: String,
}

/// The program's analog/digital split. `bindings` dispatch; `refusals` stay on the engine;
/// nothing is silently skipped — every function appears in exactly one list.
#[derive(Debug, Clone)]
pub struct MixedPartition {
    pub bindings: Vec<RegionBinding>,
    pub refusals: Vec<RegionRefusal>,
}

impl MixedPartition {
    /// Every function name, in declaration order — the totality witness.
    pub fn all_fns(&self) -> Vec<&str> {
        let mut v: Vec<(&str, bool)> = self
            .bindings
            .iter()
            .map(|b| (b.fn_name.as_str(), true))
            .chain(self.refusals.iter().map(|r| (r.fn_name.as_str(), false)))
            .collect();
        v.sort_by_key(|(n, _)| n.to_string());
        v.into_iter().map(|(n, _)| n).collect()
    }
}

/// Partition every function of `source` against `target`: certificate-admitted regions bind
/// (int-exact or float-ε per [`try_region`]'s lane decision); everything else is ledgered with
/// its typed reason. Deterministic: same source + target ⇒ the same partition, always.
pub fn partition(source: &str, target: &AnalogTarget) -> MixedPartition {
    let fn_names: Vec<String> =
        crate::ui_bridge::with_parsed_program(source, |parsed, interner| match parsed {
            Ok((stmts, _types, _pol)) => stmts
                .iter()
                .filter_map(|s| match s {
                    Stmt::FunctionDef { name, .. } => Some(interner.resolve(*name).to_string()),
                    _ => None,
                })
                .collect(),
            Err(_) => Vec::new(),
        });
    let mut bindings = Vec::new();
    let mut refusals = Vec::new();
    for fn_name in fn_names {
        match try_region(source, &fn_name, target) {
            Ok((lane, artifact)) => bindings.push(RegionBinding {
                fn_name,
                lane,
                artifact,
            }),
            Err(e) => refusals.push(RegionRefusal {
                fn_name,
                reason: e.to_string(),
            }),
        }
    }
    MixedPartition { bindings, refusals }
}

/// One binding PLACED on the stack, with its certified thermal map — the S6 composition.
#[derive(Debug, Clone)]
pub struct StackBinding {
    pub binding: RegionBinding,
    pub placement: crate::codegen_ana::stack_place::StackPlacement,
    /// The certified steady-state map for THIS binding's placement (judged by
    /// `logicaffeine_proof::thermal_replay::check_thermal_claim` — the lock re-judges it).
    pub thermal: crate::codegen_ana::thermal::ThermalMap,
}

/// The stacked partition: every certificate-admitted region placed on the stack's analog
/// tiers with a certified temperature map that MEETS the declared cooling budget; a region
/// that placed but cannot be cooled is a LEDGERED refusal, not a hot surprise. The
/// accelerator covenant is unchanged — stripping every binding leaves the digital program.
#[derive(Debug, Clone)]
pub struct StackPartition {
    pub bindings: Vec<StackBinding>,
    pub refusals: Vec<RegionRefusal>,
}

/// Partition `source` onto a STACK: region compilation against the stack's analog tier,
/// 3D placement, the exact-ℚ thermal solve, and the cooling gate — in that order, each
/// failure a named ledger entry. Deterministic end to end.
pub fn partition_on_stack(
    source: &str,
    stack: &crate::codegen_ana::stack::StackTarget,
    hop_cost_pitches: f64,
) -> StackPartition {
    use crate::codegen_ana::stack_place::place_stack;
    use crate::codegen_ana::thermal::{check_cooling, thermal_default_opts, thermal_map};

    let Some((_, target)) = stack.analog_tier() else {
        return StackPartition {
            bindings: Vec::new(),
            refusals: vec![RegionRefusal {
                fn_name: "*".to_string(),
                reason: "stack has no analog tier".to_string(),
            }],
        };
    };
    let flat = partition(source, &target.clone());
    let mut bindings = Vec::new();
    let mut refusals = flat.refusals;
    for b in flat.bindings {
        let placed = match place_stack(&b.artifact.schedule.quantized, stack, hop_cost_pitches) {
            Ok(p) => p,
            Err(e) => {
                refusals.push(RegionRefusal {
                    fn_name: b.fn_name,
                    reason: e.to_string(),
                });
                continue;
            }
        };
        let map = match thermal_map(
            stack,
            &b.artifact.schedule.quantized,
            &placed,
            &thermal_default_opts(),
        ) {
            Ok(m) => m,
            Err(e) => {
                refusals.push(RegionRefusal {
                    fn_name: b.fn_name,
                    reason: e.to_string(),
                });
                continue;
            }
        };
        if let Err(e) = check_cooling(&map, stack) {
            refusals.push(RegionRefusal {
                fn_name: b.fn_name,
                reason: e.to_string(),
            });
            continue;
        }
        bindings.push(StackBinding {
            binding: b,
            placement: placed,
            thermal: map,
        });
    }
    StackPartition { bindings, refusals }
}

/// The four faces of the machine (R4): which laws of physics serve a region.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fabric {
    /// `y = f(x)` — the ε-certified analog dataflow lanes.
    Dataflow,
    /// `ẋ = f(x)` — equation solving by settling (`Solve … for x.`).
    Dynamics,
    /// `minimize H` — soft-spin / gradient-flow relaxation (`Minimize … over x.`).
    Relaxation,
    /// Exact bits on the engine — everything else, refusing nothing.
    Digital,
}

/// Classify every function's fabric AFFINITY: a certificate-admitted region is `Dataflow`;
/// a body reaching the `Solve` desugar (`solveLinear`) is `Dynamics`; one reaching the
/// `Minimize` desugar (`minimizeQuadratic`) is `Relaxation`; everything else is `Digital` —
/// the face that refuses nothing. Deterministic, total.
pub fn fabrics(source: &str, target: &AnalogTarget) -> Vec<(String, Fabric)> {
    fn expr_calls(e: &Expr<'_>, needle: &str, interner: &crate::intern::Interner) -> bool {
        match e {
            Expr::Call { function, args } => {
                interner.resolve(*function) == needle
                    || args.iter().any(|a| expr_calls(a, needle, interner))
            }
            Expr::BinaryOp { left, right, .. } => {
                expr_calls(left, needle, interner) || expr_calls(right, needle, interner)
            }
            _ => false,
        }
    }
    fn body_calls(stmts: &[Stmt<'_>], needle: &str, interner: &crate::intern::Interner) -> bool {
        stmts.iter().any(|s| match s {
            Stmt::Let { value, .. } | Stmt::Set { value, .. } => {
                expr_calls(value, needle, interner)
            }
            Stmt::Return { value: Some(v) } => expr_calls(v, needle, interner),
            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
                body_calls(then_block, needle, interner)
                    || else_block
                        .map(|e| body_calls(e, needle, interner))
                        .unwrap_or(false)
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } => {
                body_calls(body, needle, interner)
            }
            _ => false,
        })
    }
    let names_and_shapes: Vec<(String, bool, bool)> =
        crate::ui_bridge::with_parsed_program(source, |parsed, interner| match parsed {
            Ok((stmts, _t, _p)) => stmts
                .iter()
                .filter_map(|s| match s {
                    Stmt::FunctionDef { name, body, .. } => Some((
                        interner.resolve(*name).to_string(),
                        body_calls(body, "solveLinear", interner),
                        body_calls(body, "minimizeQuadratic", interner),
                    )),
                    _ => None,
                })
                .collect(),
            Err(_) => Vec::new(),
        });
    names_and_shapes
        .into_iter()
        .map(|(name, solves, minimizes)| {
            let fabric = if minimizes {
                Fabric::Relaxation
            } else if solves {
                Fabric::Dynamics
            } else if try_region(source, &name, target).is_ok() {
                Fabric::Dataflow
            } else {
                Fabric::Digital
            };
            (name, fabric)
        })
        .collect()
}
