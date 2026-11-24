//! LEAN BRIDGE — "show me the LOGOS that proves this Lean."
//!
//! `lean_parity_matrix.rs` tracks the 248 corpus *cases*. This tracks the 2,968 *declarations* the
//! corpus authors wrote in them — the obligations. (Lean 4.14.0 elaborates 5,827 declarations in total
//! from the same files; the difference is auto-generated recursors, constructors, equation lemmas and
//! instances, discharged through the authored declaration that caused them.) The authored declaration
//! is the unit that cannot be gamed.
//!
//! A LOGOS-only test proves we do *something*. It cannot prove we do what **Lean** does, because
//! the expected value was written by whoever wrote the implementation. The corpus specification
//! makes exactly this point about its own translator: *the target kernel cannot be your oracle.* It is
//! equally true of us.
//!
//! So the expected values come from Lean. `scripts/lean/conformance-lean-oracle.sh` runs the real compiler
//! over the real corpus and records, per declaration, its universe parameters, its elaborated type,
//! and its transitive axiom set. `crates/logicaffeine_tests/tests/fixtures/lean_conformance/bridge/` then answers, for each one, which LOGOS
//! artifact discharges it and which test proves the match. A declaration is `open` until it does.
//!
//! The open count is a **downward ratchet**. Zero open is parity with Lean over this corpus.
//!
//!  ⚠️  YOU DO NOT GET TO WEAKEN THIS LOCK TO MAKE IT PASS.  ⚠️
//!  Raising OPEN_CEILING to go green converts "we cannot do this" into "we do not track that we
//!  cannot do this", which is strictly worse than the gap it hides.
//!
//! Run `cargo nextest run -p logicaffeine-tests --test lean_bridge_lock -- --nocapture` for the
//! worklist: what is blocking, ranked by how many declarations each blocker unlocks.

#![cfg(not(target_arch = "wasm32"))]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::Deserialize;

/// Every corpus case must have ground truth extracted. Pinned so a half-run oracle cannot make the
/// open count look small.
const CORPUS_CASE_COUNT: usize = 248;

/// Lean must compile all 248 — verified 2026-09-14 against `leanprover/lean4:v4.14.0`
/// (commit 410fab728470). A case Lean can no longer compile means the corpus or the toolchain
/// moved, and every number below is suspect until that is explained.
const LEAN_VERSION: &str = "4.14.0";

// ---- THE RATCHET. Only a human moves this, only DOWN. ---------------------------------------
/// Authored Lean declarations with no LOGOS counterpart yet.
///
/// 2026-09-14: seeded at 2968 — the full authored surface, nothing bridged.
/// 2026-09-22: banked at 2919 — the 49 bridges of `EARNED_BRIDGES`, each re-verified green.
/// 2026-09-22: 2909 — ten T00-003 constant, inductive, axiom and level-instance bridges (lane LF).
/// 2026-09-23: 2906 — T04-015 accCount, accIsAType and ltWf (lane LF).
/// 2026-09-23: 307 — 2599 harvested original-root bridges (`original_harvest_bridge`, lane LA).
/// 2026-09-23: 306 — T04-015 wfSum, computing through Lean's unfolded theorems (lane LF).
/// 2026-09-23: 296 — 10 harvested original-root bridges (`original_harvest_bridge`, lane LA).
/// 2026-09-23: 304 — CORRECTION, not a regression of earned work: 8 harvested rows were banked from a
///   run in which their tests passed and time out in the next suite run, so they never held under the suite's
///   budget; they are withheld (HARVEST.json `withheld`) and re-opened with a triage record.
/// 2026-09-23: 302 — T02-014 forestSize and T03-008 Labelled.binductionOn_1, re-checked after the
///   `recheck::open_mutual` shift fix (lane LF).
/// 2026-09-23: 236 — 66 harvested original-root bridges (`original_harvest_bridge`, lane LA).
/// 2026-09-23: 210 — 26 harvested original-root bridges (`original_harvest_bridge`, lane LA): the rows the
///   importer audit had excluded, re-checked under Lean's `#print axioms` rule (the package's rule, lane LD).
/// 2026-09-24: 193 — 17 trust-escape traps refused completely (`original_refusal_bridge`, lane LH).
/// 2026-09-25: 183 — 10 harvested original-root bridges (`original_harvest_bridge`, lane LH).
/// 2026-09-25: 178 — 5 trust-escape traps refused completely (`original_refusal_bridge`, lane LH).
/// 2026-09-25: 164 — 14 harvested original-root bridges (`original_harvest_bridge`, lane LH).
/// 2026-09-25: 137 — 27 harvested original-root bridges (`original_harvest_bridge`, lane LH).
/// 2026-09-25: 95 — 42 harvested original-root bridges (`original_harvest_bridge`, lane LH).
/// 2026-09-27: 93 — two original _stdin instance bridges (`original_stdin_identity_bridge`, lane LH).
/// Every wave lowers it. Lowering is the work; raising it is forbidden.
const OPEN_CEILING: usize = 93;

/// The authored declaration count and a digest of their identities. A declaration that disappears,
/// is renamed or is duplicated changes one of these, so shrinking the denominator can never pass for
/// closing an obligation.
const DECLARATION_COUNT: usize = 2968;
const DECLARATION_IDENTITY_FNV1A64: u64 = 0xdfe0_ab6f_5cd6_2f0f;

/// Every declaration Lean 4.14.0 elaborates from the 248 files, authored or generated (the header's
/// 5,827). Pinned so the header comment is a measurement, not a memory.
const ELABORATED_DECLARATION_COUNT: usize = 5827;

/// SHA-256 over `id\tpath\tsha256(source)\n` for every corpus case, sorted by id. The original Lean
/// source is the obligation's identity: an edited, removed or added corpus file moves this.
const CORPUS_SOURCE_SHA256: &str =
    "831c80dde64833d74ab435691d94c4e1c33241e1124d9933bfe4e999c65d5005";

/// SHA-256 over one compact JSON line `[case, lean, kind, levels, type, axioms]` per ledger row,
/// sorted by (case, lean). The Lean oracle's facts — the elaborated type, universe parameters, kind
/// and transitive axioms — are the expected values; a changed oracle type moves this even when the
/// ledger and the ground truth move together.
const ORACLE_FACTS_SHA256: &str =
    "3de93463a4e2a520614f91de05405446cdc04b15136457fb600bb0ad00a7015f";

/// Every row closed as `refused`, by identity — pinned exactly like `EARNED_BRIDGES`, and never folded
/// into it. A refusal is earned by a named test showing the import refuses the declaration correctly:
/// Lean's own `#print axioms` names a trust escape (`sorryAx`, `lcProof`, `Lean.trustCompiler`), the
/// submission is translated and checked by both kernels, the assumption policy refuses it with Lean's
/// exact axioms and the taint path to each escape, and nothing is published (Tristen's ruling of
/// 2026-09-24). Append as refusals are earned; never remove.
const EARNED_REFUSALS: &[(&str, &str)] = &[
    ("T06-017", "T06_017.dataSorry"),
    ("T06-017", "T06_017.tacticSorry"),
    ("T06-017", "T06_017.taintedLemma"),
    ("T06-017", "T06_017.taintedUser"),
    ("T08-028", "T08_028.bareAdmit"),
    ("T08-028", "T08_028.bareSorry"),
    ("T08-028", "T08_028.dependsOnSorry"),
    ("T08-028", "T08_028.partialSorry"),
    ("T08-028", "T08_028.sorryUnderBinder"),
    ("T08-028", "T08_028.stubbedFunction"),
    ("T08-028", "T08_028.termSorry"),
    ("T11-014", "T11_014.alsoUnproved"),
    ("T11-014", "T11_014.dependsOnIt"),
    ("T11-014", "T11_014.dependsTransitively"),
    ("T11-014", "T11_014.instTrivial'Nat"),
    ("T11-014", "T11_014.obviouslyUnproved"),
    ("T11-014", "T11_014.sorryNat"),
    ("T02-008", "T02_008.coerceAnything"),
    ("T02-008", "T02_008.useCoerce"),
    ("T06-014", "T06_014.bridgeShape"),
    ("T06-014", "T06_014.theBridgeAxiom"),
    ("T06-014", "T06_014.theEvaluatorAxiom"),
];

/// Every bridge that has been earned, by identity. A count alone lets one lost bridge hide behind one
/// new bridge; this list does not. Append as bridges are earned; never remove.
const EARNED_BRIDGES: &[(&str, &str)] = &[
    ("T00-001", "T00_001.binderNamedLikeAGlobal"),
    ("T00-001", "T00_001.identityNat"),
    ("T00-001", "T00_001.innerTypeMentionsOuter"),
    ("T00-001", "T00_001.pickOuter"),
    ("T00-001", "T00_001.shadowedInnermost"),
    ("T00-001", "T00_001.shadowedOutermost"),
    ("T00-001", "T00_001.usesSectionVar"),
    ("T00-002", "T00_002.appliedToProp"),
    ("T00-002", "T00_002.appliedToType"),
    ("T00-002", "T00_002.idTypeAtOne"),
    ("T00-002", "T00_002.idTypeAtZero"),
    ("T00-002", "T00_002.levelLadder"),
    ("T00-002", "T00_002.propAlias"),
    ("T00-002", "T00_002.quantifiesOverProps"),
    ("T00-002", "T00_002.quantifiesOverTypes"),
    ("T00-002", "T00_002.sortOneAlias"),
    ("T00-002", "T00_002.sortTwoAlias"),
    ("T00-002", "T00_002.sortZeroAlias"),
    ("T00-002", "T00_002.typeAlias"),
    ("T00-002", "T00_002.typeOneAlias"),
    ("T00-002", "T00_002.typeZeroAlias"),
    ("T00-004", "T00_004.dependentArrow"),
    ("T00-004", "T00_004.plainArrow"),
    ("T00-004", "T00_004.shadowedBinder"),
    ("T00-004", "T00_004.usedOnlyInLaterBinderType"),
    ("T00-005", "T00_005.anonymousImplicit"),
    ("T00-005", "T00_005.applyToPolyFn"),
    ("T00-005", "T00_005.domainFromExpectedType"),
    ("T00-005", "T00_005.idNat"),
    ("T00-005", "T00_005.implicitId"),
    ("T00-005", "T00_005.instanceDefault"),
    ("T00-005", "T00_005.mixedKinds"),
    ("T00-005", "T00_005.mixedTelescope"),
    ("T00-005", "T00_005.pickMiddle"),
    ("T00-005", "T00_005.strictImplicitId"),
    ("T00-014", "T00_014.etaInType"),
    ("T00-014", "T00_014.needsFunext"),
    ("T00-019", "T00_019.letValueVisible"),
    ("T00-019", "T00_019.nestedHaves"),
    ("T00-019", "T00_019.sharedProof"),
    ("T00-019", "T00_019.viaHave"),
    ("T00-019", "T00_019.viaLet"),
    ("T04-015", "T04_015.accDepth"),
    ("T06-002", "T06_002.bigMotive"),
    ("T06-002", "T06_002.jDependent"),
    ("T06-002", "T06_002.symmViaRec"),
    ("T06-002", "T06_002.transport"),
    ("T06-002", "T06_002.transportRaw"),
    ("T06-002", "T06_002.viaMpr"),
    ("T00-003", "T00_003.Flag"),
    ("T00-003", "T00_003.Flag.off"),
    ("T00-003", "T00_003.Flag.on"),
    ("T00-003", "T00_003.monoInstanceOfPolyConst"),
    ("T00-003", "T00_003.opaqueNat"),
    ("T00-003", "T00_003.refersToAxiom"),
    ("T00-003", "T00_003.refersToConstructor"),
    ("T00-003", "T00_003.refersToFlagCtor"),
    ("T00-003", "T00_003.refersToFlagType"),
    ("T00-003", "T00_003.refersToInductive"),
    ("T04-015", "T04_015.accCount"),
    ("T04-015", "T04_015.accIsAType"),
    ("T04-015", "T04_015.ltWf"),
    ("T04-015", "T04_015.wfSum"),
    ("T00-003", "T00_003.FlagCount"),
    ("T00-003", "T00_003.base"),
    ("T00-003", "T00_003.refersToBase"),
    ("T00-006", "T00_006.appliedInOrder"),
    ("T00-006", "T00_006.applyTwice"),
    ("T00-006", "T00_006.constFn"),
    ("T00-006", "T00_006.dependentApply"),
    ("T00-006", "T00_006.explicitSpine"),
    ("T00-006", "T00_006.f5"),
    ("T00-006", "T00_006.hoSpine"),
    ("T00-006", "T00_006.implicitSpine"),
    ("T00-006", "T00_006.partial1"),
    ("T00-006", "T00_006.partial3"),
    ("T00-006", "T00_006.polyHeadSpine"),
    ("T00-006", "T00_006.saturated"),
    ("T00-006", "T00_006.spineHead"),
    ("T00-006", "T00_006.sub2"),
    ("T00-007", "T00_007.letOfFunction"),
    ("T00-007", "T00_007.letOfType"),
    ("T00-007", "T00_007.nestedLets"),
    ("T00-007", "T00_007.repeatedUse"),
    ("T00-007", "T00_007.shadowingLet"),
    ("T00-007", "T00_007.simpleLet"),
    ("T00-007", "T00_007.zetaProof"),
    ("T00-008", "T00_008.hugeLit"),
    ("T00-008", "T00_008.intLit"),
    ("T00-008", "T00_008.mediumLit"),
    ("T00-008", "T00_008.negIntLit"),
    ("T00-008", "T00_008.pred"),
    ("T00-008", "T00_008.sameNumeralAsInt"),
    ("T00-008", "T00_008.sameNumeralAsNat"),
    ("T00-008", "T00_008.smallLit"),
    ("T00-008", "T00_008.zeroLit"),
    ("T00-009", "T00_009.annotatedHave"),
    ("T00-009", "T00_009.annotatedHead"),
    ("T00-009", "T00_009.annotatedUnderBinder"),
    ("T00-009", "T00_009.anonPair"),
    ("T00-009", "T00_009.ascribed"),
    ("T00-009", "T00_009.documented"),
    ("T00-009", "T00_009.explicitPair"),
    ("T00-010", "T00_010.constFn"),
    ("T00-010", "T00_010.existsViaHole"),
    ("T00-010", "T00_010.hiddenPosition"),
    ("T00-010", "T00_010.implicitTypeArg"),
    ("T00-010", "T00_010.underscoreFromExpectedType"),
    ("T00-010", "T00_010.underscoreFromLaterArg"),
    ("T00-010", "T00_010.viaTactic"),
    ("T00-011", "T00_011.deepProj"),
    ("T00-011", "T00_011.nestedPair"),
    ("T00-011", "T00_011.pfst"),
    ("T00-011", "T00_011.positives"),
    ("T00-011", "T00_011.psnd"),
    ("T00-011", "T00_011.sigmaFst"),
    ("T00-011", "T00_011.sigmaSnd"),
    ("T00-011", "T00_011.snonDep"),
    ("T00-011", "T00_011.subProp"),
    ("T00-011", "T00_011.subVal"),
    ("T00-012", "T00_012.bump"),
    ("T00-012", "T00_012.dup"),
    ("T00-012", "T00_012.tower"),
    ("T00-013", "T00_013.BoolAlias"),
    ("T00-013", "T00_013.NatAlias"),
    ("T00-013", "T00_013.OpaqueType"),
    ("T00-013", "T00_013.genuinelyOpaque"),
    ("T00-013", "T00_013.intoOpaque"),
    ("T00-013", "T00_013.ordinary"),
    ("T00-013", "T00_013.secret"),
    ("T00-013", "T00_013.usesAbbrev"),
    ("T00-013", "T00_013.usesAlias"),
    ("T00-013", "T00_013.usesSecret"),
    ("T00-015", "T00_015.alphaA"),
    ("T00-015", "T00_015.alphaB"),
    ("T00-015", "T00_015.fiveDeep"),
    ("T00-015", "T00_015.generatedNameCollision"),
    ("T00-015", "T00_015.letShadowsLambda"),
    ("T00-015", "T00_015.loop"),
    ("T00-015", "T00_015.outerX"),
    ("T00-015", "T00_015.shadowInPiType"),
    ("T00-015", "T00_015.threeX"),
    ("T00-016", "T00_016.ascribedWithImplicit"),
    ("T00-016", "T00_016.constFn"),
    ("T00-016", "T00_016.dependentMotive"),
    ("T00-016", "T00_016.eqWithImplicitType"),
    ("T00-016", "T00_016.implicitsPresent"),
    ("T00-016", "T00_016.orElim"),
    ("T00-016", "T00_016.polyAtLevelZero"),
    ("T00-016", "T00_016.resolvedInstance"),
    ("T00-016", "T00_016.usesInstance"),
    ("T00-017", "T00_017.letOfTypeDependent"),
    ("T00-017", "T00_017.nonDependentControl"),
    ("T00-018", "T00_018.Col"),
    ("T00-018", "T00_018.Col.blue"),
    ("T00-018", "T00_018.Col.green"),
    ("T00-018", "T00_018.Col.red"),
    ("T00-018", "T00_018.Opt"),
    ("T00-018", "T00_018.Opt.none"),
    ("T00-018", "T00_018.Opt.some"),
    ("T00-018", "T00_018.Tagged"),
    ("T00-018", "T00_018.Tagged.tag"),
    ("T00-018", "T00_018.depMatch"),
    ("T00-018", "T00_018.pc"),
    ("T00-018", "T00_018.predOrOne"),
    ("T00-018", "T00_018.readTag"),
    ("T00-018", "T00_018.unwrapOr"),
    ("T00-018", "T00_018.viaMatchExpr"),
    ("T00-020", "T00_020.ascribedFn"),
    ("T00-020", "T00_020.ascribedNat"),
    ("T00-020", "T00_020.emptyListAscribed"),
    ("T00-020", "T00_020.numeralAsInt"),
    ("T00-020", "T00_020.numeralAsNat"),
    ("T00-020", "T00_020.pinnedLevel"),
    ("T00-020", "T00_020.plainNat"),
    ("T00-020", "T00_020.tripleAscribed"),
    ("T00-020", "T00_020.viaShowBy"),
    ("T00-020", "T00_020.viaShowFrom"),
    ("T00-021", "T00_021.bump"),
    ("T00-021", "T00_021.depTelescope"),
    ("T00-021", "T00_021.f20"),
    ("T00-021", "T00_021.nestedHeadSpine"),
    ("T00-021", "T00_021.saturated20"),
    ("T00-021", "T00_021.sharedSubterm"),
    ("T00-021", "T00_021.tower"),
    ("T00-022", "T00_022.bigType"),
    ("T00-022", "T00_022.dup"),
    ("T00-022", "T00_022.dup10"),
    ("T00-022", "T00_022.dup4"),
    ("T00-022", "T00_022.f20"),
    ("T00-022", "T00_022.fourBigTypes"),
    ("T00-022", "T00_022.letDoesNotShare"),
    ("T00-022", "T00_022.nestedLetBlowup"),
    ("T00-022", "T00_022.shared20"),
    ("T00-022", "T00_022.twice"),
    ("T00-022", "T00_022.twiceOfTwice"),
    ("T00-022", "T00_022.usesGlobalSharing"),
    ("T01-001", "T01_001.Both"),
    ("T01-001", "T01_001.Both.mk"),
    ("T01-001", "T01_001.Choice"),
    ("T01-001", "T01_001.Choice.left"),
    ("T01-001", "T01_001.Choice.right"),
    ("T01-001", "T01_001.choiceComm"),
    ("T01-001", "T01_001.impredicativeForall"),
    ("T01-001", "T01_001.impredicativeForallAtTypeOne"),
    ("T01-001", "T01_001.predicativeForall"),
    ("T01-001", "T01_001.propFamily"),
    ("T01-001", "T01_001.propSort"),
    ("T01-001", "T01_001.quantifiesOverProps"),
    ("T01-001", "T01_001.simpleProp"),
    ("T01-001", "T01_001.sortZero"),
    ("T01-002", "T01_002.acrossLevels"),
    ("T01-002", "T01_002.bareType"),
    ("T01-002", "T01_002.concreteType"),
    ("T01-002", "T01_002.container"),
    ("T01-002", "T01_002.containerAtNat"),
    ("T01-002", "T01_002.higherAtContainer"),
    ("T01-002", "T01_002.higherContainer"),
    ("T01-002", "T01_002.ladder0"),
    ("T01-002", "T01_002.ladder1"),
    ("T01-002", "T01_002.ladder2"),
    ("T01-002", "T01_002.sortOne"),
    ("T01-002", "T01_002.sortTwo"),
    ("T01-002", "T01_002.typeOne"),
    ("T01-002", "T01_002.typeTwo"),
    ("T01-002", "T01_002.typeZero"),
    ("T01-002", "T01_002.typeZeroInhabitsTypeOne"),
    ("T01-003", "T01_003.atProp"),
    ("T01-003", "T01_003.atTypeOne"),
    ("T01-003", "T01_003.atTypeZero"),
    ("T01-003", "T01_003.polyAxiom"),
    ("T01-003", "T01_003.polyId"),
    ("T01-003", "T01_003.polyInBodyOnly"),
    ("T01-003", "T01_003.polyPair"),
    ("T01-003", "T01_003.polySecond"),
    ("T01-003", "T01_003.polyTypeId"),
    ("T01-003", "T01_003.usesPolyAxiom"),
    ("T01-003", "T01_003.usesPolyAxiomAtProp"),
    ("T01-003", "T01_003.usesW1"),
    ("T01-003", "T01_003.usesW2"),
    ("T01-004", "T01_004.arrow"),
    ("T01-004", "T01_004.arrow3"),
    ("T01-004", "T01_004.arrow3Assoc"),
    ("T01-004", "T01_004.arrowAt00"),
    ("T01-004", "T01_004.arrowAt01"),
    ("T01-004", "T01_004.arrowAt10"),
    ("T01-004", "T01_004.arrowSameLevel"),
    ("T01-004", "T01_004.arrowSameLevelPlain"),
    ("T01-004", "T01_004.maxWithProp"),
    ("T01-004", "T01_004.maxWithSucc"),
    ("T01-004", "T01_004.piComputedMax"),
    ("T01-004", "T01_004.secondOnly"),
    ("T01-005", "T01_005.dependentProp"),
    ("T01-005", "T01_005.forallOverTypeOneIsProp"),
    ("T01-005", "T01_005.forallOverTypesIsProp"),
    ("T01-005", "T01_005.forallOverTypesIsType"),
    ("T01-005", "T01_005.piIntoProp"),
    ("T01-005", "T01_005.piIntoType"),
    ("T01-005", "T01_005.piNested"),
    ("T01-005", "T01_005.piOverMax"),
    ("T01-005", "T01_005.piSameLevel"),
    ("T01-005", "T01_005.piSort"),
    ("T01-005", "T01_005.piSymbolic"),
    ("T01-005", "T01_005.symbolicAtProp"),
    ("T01-005", "T01_005.symbolicAtType"),
    ("T01-006", "T01_006.mixedReference"),
    ("T01-006", "T01_006.monoId"),
    ("T01-006", "T01_006.polyArrow"),
    ("T01-006", "T01_006.polyAtSucc"),
    ("T01-006", "T01_006.polyCompose"),
    ("T01-006", "T01_006.polyEqRefl"),
    ("T01-006", "T01_006.polyId"),
    ("T01-006", "T01_006.polyIdAtProp"),
    ("T01-006", "T01_006.polyIdAtType0"),
    ("T01-006", "T01_006.polyIdAtType1"),
    ("T01-006", "T01_006.polySecond"),
    ("T01-006", "T01_006.secondAt01"),
    ("T01-006", "T01_006.secondAt10"),
    ("T01-007", "T01_007.Box"),
    ("T01-007", "T01_007.Box.mk"),
    ("T01-007", "T01_007.Either"),
    ("T01-007", "T01_007.Either.left"),
    ("T01-007", "T01_007.Either.right"),
    ("T01-007", "T01_007.Squash"),
    ("T01-007", "T01_007.Squash.mk"),
    ("T01-007", "T01_007.atLevel0"),
    ("T01-007", "T01_007.atLevel1"),
    ("T01-008", "T01_008.Box"),
    ("T01-008", "T01_008.Box.mk"),
    ("T01-008", "T01_008.boxBool"),
    ("T01-008", "T01_008.boxBoxNat"),
    ("T01-008", "T01_008.boxNat"),
    ("T01-008", "T01_008.boxNatRefl"),
    ("T01-008", "T01_008.boxOfBoxType"),
    ("T01-008", "T01_008.boxType"),
    ("T01-008", "T01_008.boxTypeAgain"),
    ("T01-008", "T01_008.boxTypeRefl"),
    ("T01-008", "T01_008.mapAtOne"),
    ("T01-008", "T01_008.mapAtZero"),
    ("T01-008", "T01_008.mapBox"),
    ("T01-008", "T01_008.polyBoxId"),
    ("T01-008", "T01_008.unbox"),
    ("T01-008", "T01_008.unboxType"),
    ("T01-009", "T01_009.Cell"),
    ("T01-009", "T01_009.Cell.mk"),
    ("T01-009", "T01_009.Cell.val"),
    ("T01-009", "T01_009.appliedViaLift"),
    ("T01-009", "T01_009.big"),
    ("T01-009", "T01_009.cellOfLifted"),
    ("T01-009", "T01_009.consumesTypeOne"),
    ("T01-009", "T01_009.covariantApplication"),
    ("T01-009", "T01_009.downNat"),
    ("T01-009", "T01_009.idAtOne"),
    ("T01-009", "T01_009.idAtZero"),
    ("T01-009", "T01_009.liftedFamily"),
    ("T01-009", "T01_009.liftedNat"),
    ("T01-009", "T01_009.liftedProp"),
    ("T01-009", "T01_009.propUp"),
    ("T01-009", "T01_009.propViaLift"),
    ("T01-009", "T01_009.someTypeOneFn"),
    ("T01-009", "T01_009.typeZeroInhabitsTypeOne"),
    ("T01-009", "T01_009.upNat"),
    ("T01-009", "T01_009.usesOne"),
    ("T01-009", "T01_009.usesZero"),
    ("T01-009", "T01_009.variantApplication"),
    ("T01-009", "T01_009.wantsAType"),
    ("T01-009", "T01_009.wantsTypeOneFamily"),
    ("T01-010", "T01_010.Box"),
    ("T01-010", "T01_010.Box.mk"),
    ("T01-010", "T01_010.Holder"),
    ("T01-010", "T01_010.Holder.mk"),
    ("T01-010", "T01_010.Holder.p"),
    ("T01-010", "T01_010.Mixed"),
    ("T01-010", "T01_010.Mixed.mk"),
    ("T01-010", "T01_010.Mixed.n"),
    ("T01-010", "T01_010.Mixed.q"),
    ("T01-010", "T01_010.aHolder"),
    ("T01-010", "T01_010.boxOfProp"),
    ("T01-010", "T01_010.familyOverProps"),
    ("T01-010", "T01_010.forallOverProps"),
    ("T01-010", "T01_010.holderIsAType"),
    ("T01-010", "T01_010.listOfProps"),
    ("T01-010", "T01_010.mixedIsAType"),
    ("T01-010", "T01_010.polyId"),
    ("T01-010", "T01_010.polyIdAtProp"),
    ("T01-010", "T01_010.polyIdOnProp"),
    ("T01-010", "T01_010.predicateOnProps"),
    ("T01-010", "T01_010.propIsAType"),
    ("T01-010", "T01_010.propToNat"),
    ("T01-010", "T01_010.propToNatType"),
    ("T01-010", "T01_010.propToPropType"),
    ("T01-010", "T01_010.propToTypeType"),
    ("T01-010", "T01_010.usesForallOverProps"),
    ("T01-011", "T01_011.AutoBox"),
    ("T01-011", "T01_011.AutoBox.mk"),
    ("T01-011", "T01_011.AutoCell"),
    ("T01-011", "T01_011.AutoCell.mk"),
    ("T01-011", "T01_011.AutoCell.val"),
    ("T01-011", "T01_011.autoAtProp"),
    ("T01-011", "T01_011.autoAtType"),
    ("T01-011", "T01_011.autoAxiom"),
    ("T01-011", "T01_011.autoBoxNat"),
    ("T01-011", "T01_011.autoBoxType"),
    ("T01-011", "T01_011.autoCellNat"),
    ("T01-011", "T01_011.autoId"),
    ("T01-011", "T01_011.autoPair"),
    ("T01-011", "T01_011.autoSecond"),
    ("T01-011", "T01_011.autoSecondAt01"),
    ("T01-011", "T01_011.autoSecondAt10"),
    ("T01-011", "T01_011.autoSortId"),
    ("T01-011", "T01_011.occurrenceOrder"),
    ("T01-011", "T01_011.solvedLevel"),
    ("T01-011", "T01_011.underscoreLevel"),
    ("T01-011", "T01_011.usesAutoAxiom"),
    ("T01-011", "T01_011.usesSolvedLevel"),
    ("T01-012", "T01_012.ambiguousAtProp"),
    ("T01-012", "T01_012.ambiguousAtType"),
    ("T01-012", "T01_012.ambiguousInIsolation"),
    ("T01-012", "T01_012.autoBoundNotSolved"),
    ("T01-012", "T01_012.constFn"),
    ("T01-012", "T01_012.interdependent"),
    ("T01-012", "T01_012.levelByUnificationAtOne"),
    ("T01-012", "T01_012.levelByUnificationAtProp"),
    ("T01-012", "T01_012.levelByUnificationAtZero"),
    ("T01-012", "T01_012.levelPinnedByAscription"),
    ("T01-012", "T01_012.nestedLevelSolving"),
    ("T01-012", "T01_012.polyId"),
    ("T01-012", "T01_012.solvedFromLaterArg"),
    ("T01-012", "T01_012.solvedFromResult"),
    ("T01-012", "T01_012.usesAtProp"),
    ("T01-012", "T01_012.usesAtType"),
    ("T01-012", "T01_012.usesSolvedFromResult"),
    ("T01-013", "T01_013.MySquash"),
    ("T01-013", "T01_013.MySquash.mk"),
    ("T01-013", "T01_013.MySubsingleton"),
    ("T01-013", "T01_013.MySubsingleton.allEq"),
    ("T01-013", "T01_013.MySubsingleton.mk"),
    ("T01-013", "T01_013.Triv"),
    ("T01-013", "T01_013.Triv.mk"),
    ("T01-013", "T01_013.TwoWays"),
    ("T01-013", "T01_013.TwoWays.first"),
    ("T01-013", "T01_013.TwoWays.second"),
    ("T01-013", "T01_013.Wrapped"),
    ("T01-013", "T01_013.Wrapped.mk"),
    ("T01-013", "T01_013.eqProofsIrrelevant"),
    ("T01-013", "T01_013.instMySubsingletonTrue"),
    ("T01-013", "T01_013.squashNat"),
    ("T01-013", "T01_013.squashNatAgain"),
    ("T01-013", "T01_013.squashesEqual"),
    ("T01-013", "T01_013.trivIrrelevant"),
    ("T01-013", "T01_013.twoWaysSmallElim"),
    ("T01-013", "T01_013.wrappedIrrelevant"),
    ("T01-014", "T01_014.Carries"),
    ("T01-014", "T01_014.Carries.mk"),
    ("T01-014", "T01_014.NotSneaky"),
    ("T01-014", "T01_014.NotSneaky.mk"),
    ("T01-014", "T01_014.Sneaky"),
    ("T01-014", "T01_014.Sneaky.mk"),
    ("T01-014", "T01_014.accToNat"),
    ("T01-014", "T01_014.andToNat"),
    ("T01-014", "T01_014.carriesIrrelevant"),
    ("T01-014", "T01_014.eqRecToType"),
    ("T01-014", "T01_014.exFalsoAtTypeOne"),
    ("T01-014", "T01_014.exFalsoAtTypeZero"),
    ("T01-014", "T01_014.orSmallElim"),
    ("T01-014", "T01_014.sneak0"),
    ("T01-014", "T01_014.sneak1"),
    ("T01-014", "T01_014.sneaksEqual"),
    ("T02-001", "T02_001.Count"),
    ("T02-001", "T02_001.Endo"),
    ("T02-001", "T02_001.double"),
    ("T02-001", "T02_001.idNat"),
    ("T02-001", "T02_001.quadruple"),
    ("T02-001", "T02_001.replicateType"),
    ("T02-001", "T02_001.zeroCount"),
    ("T02-002", "T02_002.add_zero_eq"),
    ("T02-002", "T02_002.add_zero_zero"),
    ("T02-002", "T02_002.all_refl"),
    ("T02-002", "T02_002.isZero"),
    ("T02-002", "T02_002.isZero_zero"),
    ("T02-002", "T02_002.trivialProof"),
    ("T02-002", "T02_002.zero_add_eq"),
    ("T02-003", "T02_003.choiceLike"),
    ("T02-003", "T02_003.mysteryNat"),
    ("T02-003", "T02_003.separateName"),
    ("T02-003", "T02_003.transportLike"),
    ("T02-003", "T02_003.usesAxiom"),
    ("T02-004", "T02_004.A0"),
    ("T02-004", "T02_004.A1"),
    ("T02-004", "T02_004.A2"),
    ("T02-004", "T02_004.A3"),
    ("T02-004", "T02_004.Flag"),
    ("T02-004", "T02_004.NatPair"),
    ("T02-004", "T02_004.NatPairOpaque"),
    ("T02-004", "T02_004.sealedZero"),
    ("T02-004", "T02_004.useChain"),
    ("T02-005", "T02_005.Threshold"),
    ("T02-005", "T02_005.example_1"),
    ("T02-006", "T02_006.defaultOf"),
    ("T02-006", "T02_006.hash"),
    ("T02-006", "T02_006.sealed"),
    ("T02-006", "T02_006.secretNat"),
    ("T02-007", "T02_007.collatzSteps"),
    ("T02-007", "T02_007.isEvenP"),
    ("T02-007", "T02_007.isEvenStruct"),
    ("T02-007", "T02_007.isOddP"),
    ("T02-007", "T02_007.spin"),
    ("T02-008", "T02_008.safeId"),
    ("T02-008", "T02_008.safeId_eq"),
    ("T02-009", "T02_009.decideProp"),
    ("T02-009", "T02_009.plainlyFine"),
    ("T02-009", "T02_009.plainlyFineComputable"),
    ("T02-009", "T02_009.someNat"),
    ("T02-010", "T02_010.Pointed"),
    ("T02-010", "T02_010.Pointed.mk"),
    ("T02-010", "T02_010.Pointed.point"),
    ("T02-010", "T02_010.Tag"),
    ("T02-010", "T02_010.Tag.mk"),
    ("T02-010", "T02_010.Tag.name"),
    ("T02-010", "T02_010.getPoint"),
    ("T02-010", "T02_010.instPointedBool"),
    ("T02-010", "T02_010.instPointedNat"),
    ("T02-010", "T02_010.instPointedProd"),
    ("T02-010", "T02_010.instPointedUnit"),
    ("T02-010", "T02_010.tagAny"),
    ("T02-010", "T02_010.tagNat"),
    ("T02-011", "T02_011.Outer.Inner.usesInner"),
    ("T02-011", "T02_011.Outer.Inner.usesOuter"),
    ("T02-011", "T02_011.Outer.Inner.value"),
    ("T02-011", "T02_011.Outer.value"),
    ("T02-011", "T02_011.Outer_Inner_value"),
    ("T02-011", "T02_011.Outer_value"),
    ("T02-011", "T02_011.Shadow.Nat"),
    ("T02-012", "T02_012.Scoped.scopedWeightBool"),
    ("T02-012", "T02_012.Weight"),
    ("T02-012", "T02_012.Weight.mk"),
    ("T02-012", "T02_012.Weight.w"),
    ("T02-012", "T02_012.defaultWeight"),
    ("T02-012", "T02_012.overrideWeight"),
    ("T02-012", "T02_012.protectedThing"),
    ("T02-012", "T02_012.publicUser"),
    ("T02-013", "T02_013.Box"),
    ("T02-013", "T02_013.Box.ext'"),
    ("T02-013", "T02_013.Box.ext'_iff"),
    ("T02-013", "T02_013.Box.mk"),
    ("T02-013", "T02_013.Box.val"),
    ("T02-013", "T02_013.Marked"),
    ("T02-013", "T02_013.Marked.mk"),
    ("T02-013", "T02_013.Marked.tag"),
    ("T02-013", "T02_013.fastAdd"),
    ("T02-013", "T02_013.fastAdd_eq"),
    ("T02-013", "T02_013.markedNatImpl"),
    ("T02-013", "T02_013.size"),
    ("T02-013", "T02_013.size_eq"),
    ("T02-014", "T02_014.Forest"),
    ("T02-014", "T02_014.Forest.cons"),
    ("T02-014", "T02_014.Forest.nil"),
    ("T02-014", "T02_014.Tree"),
    ("T02-014", "T02_014.Tree.node"),
    ("T02-014", "T02_014.isEven"),
    ("T02-014", "T02_014.isOdd"),
    ("T02-014", "T02_014.modThree0"),
    ("T02-014", "T02_014.modThree1"),
    ("T02-014", "T02_014.modThree2"),
    ("T02-015", "T02_015.countUp"),
    ("T02-015", "T02_015.countUp.go"),
    ("T02-015", "T02_015.describe"),
    ("T02-015", "T02_015.describe.inner"),
    ("T02-015", "T02_015.describe.outer"),
    ("T02-015", "T02_015.parity"),
    ("T02-015", "T02_015.parity.ev"),
    ("T02-015", "T02_015.parity.od"),
    ("T02-015", "T02_015.sumOfSquares"),
    ("T02-015", "T02_015.sumOfSquares.sq"),
    ("T02-015", "T02_015.sumOfSquares_sq"),
    ("T02-016", "T02_016.nestedRec"),
    ("T02-016", "T02_016.nestedRec.outer"),
    ("T02-016", "T02_016.nestedRec.outer.inner"),
    ("T02-016", "T02_016.plainLet"),
    ("T02-016", "T02_016.scaledSum"),
    ("T02-016", "T02_016.scaledSum.go"),
    ("T02-016", "T02_016.shadowedRec"),
    ("T02-016", "T02_016.shadowedRec.loop"),
    ("T02-016", "T02_016.sumTo"),
    ("T02-016", "T02_016.sumTo.go"),
    ("T02-017", "T02_017.Chain"),
    ("T02-017", "T02_017.Chain.link"),
    ("T02-017", "T02_017.Chain.stop"),
    ("T02-017", "T02_017.Colour"),
    ("T02-017", "T02_017.Colour.blue"),
    ("T02-017", "T02_017.Colour.green"),
    ("T02-017", "T02_017.Colour.ofNat_toCtorIdx"),
    ("T02-017", "T02_017.Colour.red"),
    ("T02-017", "T02_017.Pixel"),
    ("T02-017", "T02_017.Pixel.hue"),
    ("T02-017", "T02_017.Pixel.mk"),
    ("T02-017", "T02_017.Pixel.x"),
    ("T02-017", "T02_017.Pixel.y"),
    ("T02-017", "T02_017.Shape"),
    ("T02-017", "T02_017.Shape.dot"),
    ("T02-017", "T02_017.Shape.line"),
    ("T02-017", "T02_017.instBEqChain"),
    ("T02-017", "T02_017.instBEqColour"),
    ("T02-017", "T02_017.instBEqPixel"),
    ("T02-017", "T02_017.instBEqShape"),
    ("T02-017", "T02_017.instDecidableEqColour"),
    ("T02-017", "T02_017.instDecidableEqPixel"),
    ("T02-017", "T02_017.instInhabitedColour"),
    ("T02-018", "T02_018.applyTwice"),
    ("T02-018", "T02_018.bothAB"),
    ("T02-018", "T02_018.headOrDefault"),
    ("T02-018", "T02_018.noN"),
    ("T02-018", "T02_018.onlyB"),
    ("T02-018", "T02_018.topLevelK"),
    ("T02-018", "T02_018.useN"),
    ("T02-018", "T02_018.useNTwice"),
    ("T02-019", "T02_019.Alpha.shared"),
    ("T02-019", "T02_019.Alpha.token"),
    ("T02-019", "T02_019.Beta.betaOnly"),
    ("T02-019", "T02_019.Beta.shared"),
    ("T02-019", "T02_019.Beta.token"),
    ("T02-019", "T02_019.disambiguated"),
    ("T02-019", "T02_019.fromAlpha"),
    ("T02-019", "T02_019.fromBeta"),
    ("T02-019", "T02_019.usesSucc"),
    ("T02-019", "T02_019.viaHiding"),
    ("T02-019", "T02_019.viaRenaming"),
    ("T02-020", "T02_020.Cross.kind"),
    ("T02-020", "T02_020.Cross_kind"),
    ("T02-020", "T02_020.Cross_kind.only"),
    ("T02-020", "T02_020.Foo.bar"),
    ("T02-020", "T02_020.Foo_bar"),
    ("T02-020", "T02_020.Point"),
    ("T02-020", "T02_020.Point.mk"),
    ("T02-020", "T02_020.Point.x"),
    ("T02-020", "T02_020.Point.y"),
    ("T02-020", "T02_020.Point_mk"),
    ("T02-020", "T02_020.Point_x"),
    ("T02-020", "T02_020.RTree_to_TList"),
    ("T02-020", "T02_020.Shadow.Nat"),
    ("T02-020", "T02_020.Shadow.Succ"),
    ("T02-020", "T02_020.withAux"),
    ("T02-020", "T02_020.withAux.go"),
    ("T02-020", "T02_020.withAux_go"),
    ("T03-001", "T03_001.Colour"),
    ("T03-001", "T03_001.Colour.blue"),
    ("T03-001", "T03_001.Colour.green"),
    ("T03-001", "T03_001.Colour.red"),
    ("T03-001", "T03_001.OnlyOne"),
    ("T03-001", "T03_001.OnlyOne.sole"),
    ("T03-001", "T03_001.Weekday"),
    ("T03-001", "T03_001.Weekday.friday"),
    ("T03-001", "T03_001.Weekday.monday"),
    ("T03-001", "T03_001.Weekday.wednesday"),
    ("T03-001", "T03_001.code"),
    ("T03-001", "T03_001.dayIndex"),
    ("T03-001", "T03_001.isWarm"),
    ("T03-002", "T03_002.Branching"),
    ("T03-002", "T03_002.Branching.fork"),
    ("T03-002", "T03_002.Branching.stop"),
    ("T03-002", "T03_002.MyNat"),
    ("T03-002", "T03_002.MyNat.succ"),
    ("T03-002", "T03_002.MyNat.zero"),
    ("T03-002", "T03_002.Tree"),
    ("T03-002", "T03_002.Tree.leaf"),
    ("T03-002", "T03_002.Tree.node"),
    ("T03-002", "T03_002.addMyNat"),
    ("T03-002", "T03_002.depth"),
    ("T03-002", "T03_002.fromNat"),
    ("T03-002", "T03_002.toNat"),
    ("T03-002", "T03_002.total"),
    ("T03-003", "T03_003.Boxed"),
    ("T03-003", "T03_003.Boxed.wrap"),
    ("T03-003", "T03_003.MyList"),
    ("T03-003", "T03_003.MyList.cons"),
    ("T03-003", "T03_003.MyList.nil"),
    ("T03-003", "T03_003.MyPair"),
    ("T03-003", "T03_003.MyPair.mk"),
    ("T03-003", "T03_003.boolList"),
    ("T03-003", "T03_003.firstOf"),
    ("T03-003", "T03_003.lengthGen"),
    ("T03-003", "T03_003.lengthOf"),
    ("T03-003", "T03_003.natList"),
    ("T03-003", "T03_003.nestedList"),
    ("T03-003", "T03_003.unwrap"),
    ("T03-004", "T03_004.Parity"),
    ("T03-004", "T03_004.Parity.even"),
    ("T03-004", "T03_004.Parity.odd"),
    ("T03-004", "T03_004.Tagged"),
    ("T03-004", "T03_004.Tagged.marked"),
    ("T03-004", "T03_004.Tagged.plain"),
    ("T03-004", "T03_004.Vect"),
    ("T03-004", "T03_004.Vect.cons"),
    ("T03-004", "T03_004.Vect.nil"),
    ("T03-004", "T03_004.parityWitness"),
    ("T03-004", "T03_004.untag"),
    ("T03-004", "T03_004.v0"),
    ("T03-004", "T03_004.v2"),
    ("T03-004", "T03_004.vlength"),
    ("T03-004", "T03_004.vreplicate"),
    ("T03-004", "T03_004.vsnoc"),
    ("T03-004", "T03_004.vtoList"),
    ("T03-005", "T03_005.Even"),
    ("T03-005", "T03_005.Even.below.plusTwo"),
    ("T03-005", "T03_005.Even.below.zero"),
    ("T03-005", "T03_005.Even.plusTwo"),
    ("T03-005", "T03_005.Even.zero"),
    ("T03-005", "T03_005.Path"),
    ("T03-005", "T03_005.Path.refl"),
    ("T03-005", "T03_005.Path.step"),
    ("T03-005", "T03_005.Shape"),
    ("T03-005", "T03_005.Shape.ofBool"),
    ("T03-005", "T03_005.Vect"),
    ("T03-005", "T03_005.Vect.cons"),
    ("T03-005", "T03_005.Vect.nil"),
    ("T03-005", "T03_005.emptyVect"),
    ("T03-005", "T03_005.isEmpty"),
    ("T03-005", "T03_005.oneVect"),
    ("T03-005", "T03_005.pathEnds"),
    ("T03-005", "T03_005.shapeIsNat"),
    ("T03-006", "T03_006.A3"),
    ("T03-006", "T03_006.A3.a"),
    ("T03-006", "T03_006.B3"),
    ("T03-006", "T03_006.B3.b"),
    ("T03-006", "T03_006.B3.bstop"),
    ("T03-006", "T03_006.C3"),
    ("T03-006", "T03_006.C3.c"),
    ("T03-006", "T03_006.C3.cstop"),
    ("T03-006", "T03_006.EvenP"),
    ("T03-006", "T03_006.EvenP.below.succ"),
    ("T03-006", "T03_006.EvenP.below.zero"),
    ("T03-006", "T03_006.EvenP.succ"),
    ("T03-006", "T03_006.EvenP.zero"),
    ("T03-006", "T03_006.Forest"),
    ("T03-006", "T03_006.Forest.cons"),
    ("T03-006", "T03_006.Forest.nil"),
    ("T03-006", "T03_006.OddP"),
    ("T03-006", "T03_006.OddP.below.succ"),
    ("T03-006", "T03_006.OddP.succ"),
    ("T03-006", "T03_006.PForest"),
    ("T03-006", "T03_006.PForest.pcons"),
    ("T03-006", "T03_006.PForest.pnil"),
    ("T03-006", "T03_006.PTree"),
    ("T03-006", "T03_006.PTree.pnode"),
    ("T03-006", "T03_006.Tree"),
    ("T03-006", "T03_006.Tree.node"),
    ("T03-006", "T03_006.sample"),
    ("T03-007", "T03_007.Chain"),
    ("T03-007", "T03_007.Chain.more"),
    ("T03-007", "T03_007.Chain.stop"),
    ("T03-007", "T03_007.EvenL"),
    ("T03-007", "T03_007.EvenL.below.succ"),
    ("T03-007", "T03_007.EvenL.below.zero"),
    ("T03-007", "T03_007.EvenL.succ"),
    ("T03-007", "T03_007.EvenL.zero"),
    ("T03-007", "T03_007.OddL"),
    ("T03-007", "T03_007.OddL.below.succ"),
    ("T03-007", "T03_007.OddL.succ"),
    ("T03-007", "T03_007.PA"),
    ("T03-007", "T03_007.PA.branch"),
    ("T03-007", "T03_007.PA.leaf"),
    ("T03-007", "T03_007.PB"),
    ("T03-007", "T03_007.PB.empty"),
    ("T03-007", "T03_007.PB.hold"),
    ("T03-007", "T03_007.Tag"),
    ("T03-007", "T03_007.Tag.t1"),
    ("T03-007", "T03_007.Tag.t2"),
    ("T03-008", "T03_008.Labelled"),
    ("T03-008", "T03_008.Labelled.below_1"),
    ("T03-008", "T03_008.Labelled.ibelow_1"),
    ("T03-008", "T03_008.Labelled.lnode"),
    ("T03-008", "T03_008.Labelled.rec_1"),
    ("T03-008", "T03_008.PRTree"),
    ("T03-008", "T03_008.PRTree.below_1"),
    ("T03-008", "T03_008.PRTree.ibelow_1"),
    ("T03-008", "T03_008.PRTree.pnode"),
    ("T03-008", "T03_008.PRTree.rec_1"),
    ("T03-008", "T03_008.RTree"),
    ("T03-008", "T03_008.RTree.below_1"),
    ("T03-008", "T03_008.RTree.ibelow_1"),
    ("T03-008", "T03_008.RTree.rec_1"),
    ("T03-008", "T03_008.RTree.rnode"),
    ("T03-008", "T03_008.RTree2"),
    ("T03-008", "T03_008.RTree2.below_1"),
    ("T03-008", "T03_008.RTree2.below_2"),
    ("T03-008", "T03_008.RTree2.ibelow_1"),
    ("T03-008", "T03_008.RTree2.ibelow_2"),
    ("T03-008", "T03_008.RTree2.rec_1"),
    ("T03-008", "T03_008.RTree2.rec_2"),
    ("T03-008", "T03_008.RTree2.rnode2"),
    ("T03-008", "T03_008.TList"),
    ("T03-008", "T03_008.TList.tcons"),
    ("T03-008", "T03_008.TList.tnil"),
    ("T03-008", "T03_008.WithNats"),
    ("T03-008", "T03_008.WithNats.wn"),
    ("T03-008", "T03_008.leafR"),
    ("T03-008", "T03_008.leafR2"),
    ("T03-008", "T03_008.pleaf"),
    ("T03-009", "T03_009.BigTree"),
    ("T03-009", "T03_009.BigTree.bnode"),
    ("T03-009", "T03_009.BigTree.bty"),
    ("T03-009", "T03_009.FnTree"),
    ("T03-009", "T03_009.FnTree.fnode"),
    ("T03-009", "T03_009.FnTree.fstop"),
    ("T03-009", "T03_009.LTree"),
    ("T03-009", "T03_009.LTree.below_1"),
    ("T03-009", "T03_009.LTree.ibelow_1"),
    ("T03-009", "T03_009.LTree.lnode"),
    ("T03-009", "T03_009.LTree.rec_1"),
    ("T03-009", "T03_009.PTree"),
    ("T03-009", "T03_009.PTree.below_1"),
    ("T03-009", "T03_009.PTree.ibelow_1"),
    ("T03-009", "T03_009.PTree.pnode"),
    ("T03-009", "T03_009.PTree.pstop"),
    ("T03-009", "T03_009.PTree.rec_1"),
    ("T03-009", "T03_009.QTree"),
    ("T03-009", "T03_009.QTree.below_1"),
    ("T03-009", "T03_009.QTree.ibelow_1"),
    ("T03-009", "T03_009.QTree.qnode"),
    ("T03-009", "T03_009.QTree.qstop"),
    ("T03-009", "T03_009.QTree.rec_1"),
    ("T03-009", "T03_009.TList"),
    ("T03-009", "T03_009.TList.tcons"),
    ("T03-009", "T03_009.TList.tnil"),
    ("T03-009", "T03_009.TPair"),
    ("T03-009", "T03_009.TPair.tpair"),
    ("T03-009", "T03_009.Tagged"),
    ("T03-009", "T03_009.Tagged.tcons"),
    ("T03-009", "T03_009.Tagged.tnil"),
    ("T03-009", "T03_009.fdepth"),
    ("T03-009", "T03_009.lleaf"),
    ("T03-009", "T03_009.pOne"),
    ("T03-009", "T03_009.pTwo"),
    ("T03-009", "T03_009.qOne"),
    ("T03-010", "T03_010.Pair"),
    ("T03-010", "T03_010.Pair.fst"),
    ("T03-010", "T03_010.Pair.mk"),
    ("T03-010", "T03_010.Pair.snd"),
    ("T03-010", "T03_010.Point"),
    ("T03-010", "T03_010.Point.mk"),
    ("T03-010", "T03_010.Point.x"),
    ("T03-010", "T03_010.Point.y"),
    ("T03-010", "T03_010.Segment"),
    ("T03-010", "T03_010.Segment.mk"),
    ("T03-010", "T03_010.Segment.start"),
    ("T03-010", "T03_010.Segment.stop"),
    ("T03-010", "T03_010.Sized"),
    ("T03-010", "T03_010.Sized.bound"),
    ("T03-010", "T03_010.Sized.mk"),
    ("T03-010", "T03_010.Sized.wit"),
    ("T03-010", "T03_010.Triple"),
    ("T03-010", "T03_010.Triple.a"),
    ("T03-010", "T03_010.Triple.b"),
    ("T03-010", "T03_010.Triple.c"),
    ("T03-010", "T03_010.Triple.mk"),
    ("T03-010", "T03_010.UnitLike"),
    ("T03-010", "T03_010.UnitLike.mk"),
    ("T03-010", "T03_010.Wrapper"),
    ("T03-010", "T03_010.Wrapper.mk"),
    ("T03-010", "T03_010.unwrapped"),
    ("T03-011", "T03_011.Pair"),
    ("T03-011", "T03_011.Pair.fst"),
    ("T03-011", "T03_011.Pair.mk"),
    ("T03-011", "T03_011.Pair.snd"),
    ("T03-011", "T03_011.Point"),
    ("T03-011", "T03_011.Point.mk"),
    ("T03-011", "T03_011.Point.x"),
    ("T03-011", "T03_011.Point.y"),
    ("T03-011", "T03_011.getFst"),
    ("T03-011", "T03_011.getX"),
    ("T03-012", "T03_012.Bounded"),
    ("T03-012", "T03_012.Bounded.mk"),
    ("T03-012", "T03_012.Bounded.proof"),
    ("T03-012", "T03_012.Bounded.value"),
    ("T03-012", "T03_012.Bundle"),
    ("T03-012", "T03_012.Bundle.base"),
    ("T03-012", "T03_012.Bundle.fibre"),
    ("T03-012", "T03_012.Bundle.mk"),
    ("T03-012", "T03_012.Carrier"),
    ("T03-012", "T03_012.Carrier.mk"),
    ("T03-012", "T03_012.Carrier.point"),
    ("T03-012", "T03_012.Carrier.ty"),
    ("T03-012", "T03_012.Conj"),
    ("T03-012", "T03_012.Conj.left"),
    ("T03-012", "T03_012.Conj.mk"),
    ("T03-012", "T03_012.Conj.right"),
    ("T03-012", "T03_012.Disj"),
    ("T03-012", "T03_012.Disj.inl"),
    ("T03-012", "T03_012.Disj.inr"),
    ("T03-012", "T03_012.Point"),
    ("T03-012", "T03_012.Point.mk"),
    ("T03-012", "T03_012.Point.x"),
    ("T03-012", "T03_012.Point.y"),
    ("T03-012", "T03_012.boolBundle"),
    ("T03-012", "T03_012.conjSwap"),
    ("T03-012", "T03_012.disjSwap"),
    ("T03-012", "T03_012.natCarrier"),
    ("T03-013", "T03_013.Base"),
    ("T03-013", "T03_013.Base.mk"),
    ("T03-013", "T03_013.Base.x"),
    ("T03-013", "T03_013.Base.y"),
    ("T03-013", "T03_013.Checked"),
    ("T03-013", "T03_013.Checked.mk"),
    ("T03-013", "T03_013.Checked.ok"),
    ("T03-013", "T03_013.Checked.toBase"),
    ("T03-013", "T03_013.Diamond"),
    ("T03-013", "T03_013.Diamond.mk"),
    ("T03-013", "T03_013.Diamond.r"),
    ("T03-013", "T03_013.Diamond.toLeftArm"),
    ("T03-013", "T03_013.Diamond.toRightArm"),
    ("T03-013", "T03_013.Extended"),
    ("T03-013", "T03_013.Extended.mk"),
    ("T03-013", "T03_013.Extended.toBase"),
    ("T03-013", "T03_013.Extended.z"),
    ("T03-013", "T03_013.LeftArm"),
    ("T03-013", "T03_013.LeftArm.l"),
    ("T03-013", "T03_013.LeftArm.mk"),
    ("T03-013", "T03_013.LeftArm.toBase"),
    ("T03-013", "T03_013.PBase"),
    ("T03-013", "T03_013.PBase.mk"),
    ("T03-013", "T03_013.PBase.val"),
    ("T03-013", "T03_013.PExt"),
    ("T03-013", "T03_013.PExt.count"),
    ("T03-013", "T03_013.PExt.mk"),
    ("T03-013", "T03_013.PExt.toPBase"),
    ("T03-013", "T03_013.RightArm"),
    ("T03-013", "T03_013.RightArm.mk"),
    ("T03-013", "T03_013.RightArm.r"),
    ("T03-013", "T03_013.RightArm.toBase"),
    ("T03-013", "T03_013.baseSum"),
    ("T03-013", "T03_013.c1"),
    ("T03-013", "T03_013.d"),
    ("T03-013", "T03_013.e"),
    ("T03-013", "T03_013.pe"),
    ("T03-014", "T03_014.Convert"),
    ("T03-014", "T03_014.Convert.convert"),
    ("T03-014", "T03_014.Convert.mk"),
    ("T03-014", "T03_014.Measured"),
    ("T03-014", "T03_014.Measured.height"),
    ("T03-014", "T03_014.Measured.mk"),
    ("T03-014", "T03_014.Measured.weight"),
    ("T03-014", "T03_014.Pointed"),
    ("T03-014", "T03_014.Pointed.mk"),
    ("T03-014", "T03_014.Pointed.point"),
    ("T03-014", "T03_014.Sizeable"),
    ("T03-014", "T03_014.Sizeable.mk"),
    ("T03-014", "T03_014.Sizeable.size"),
    ("T03-014", "T03_014.instConvertBoolNat"),
    ("T03-014", "T03_014.instConvertNat"),
    ("T03-014", "T03_014.instMeasuredBool"),
    ("T03-014", "T03_014.instPointedBool"),
    ("T03-014", "T03_014.instPointedNat"),
    ("T03-014", "T03_014.instSizeableList"),
    ("T03-014", "T03_014.instSizeableNat"),
    ("T03-014", "T03_014.pointOfSame"),
    ("T03-014", "T03_014.repeatPoint"),
    ("T03-015", "T03_015.GoodFn"),
    ("T03-015", "T03_015.GoodFn.mk"),
    ("T03-015", "T03_015.GoodFn.stop"),
    ("T03-015", "T03_015.GoodNested"),
    ("T03-015", "T03_015.GoodNested.mk"),
    ("T03-015", "T03_015.GoodNested.stop"),
    ("T03-015", "T03_015.GoodRec"),
    ("T03-015", "T03_015.GoodRec.leaf"),
    ("T03-015", "T03_015.GoodRec.node"),
    ("T03-015", "T03_015.PingOk"),
    ("T03-015", "T03_015.PingOk.p"),
    ("T03-015", "T03_015.PingOk.pstop"),
    ("T03-015", "T03_015.PongOk"),
    ("T03-015", "T03_015.PongOk.q"),
    ("T03-015", "T03_015.PongOk.qstop"),
    ("T03-015", "T03_015.UsesOther"),
    ("T03-015", "T03_015.UsesOther.mk"),
    ("T03-015", "T03_015.applyOther"),
    ("T03-015", "T03_015.depthG"),
    ("T03-015", "T03_015.probeG"),
    ("T03-015", "T03_015.sumG"),
    ("T03-016", "T03_016.AllProps"),
    ("T03-016", "T03_016.AllProps.mk"),
    ("T03-016", "T03_016.Choice"),
    ("T03-016", "T03_016.Choice.takeLeft"),
    ("T03-016", "T03_016.Choice.takeRight"),
    ("T03-016", "T03_016.Positive"),
    ("T03-016", "T03_016.Positive.below.more"),
    ("T03-016", "T03_016.Positive.below.one"),
    ("T03-016", "T03_016.Positive.more"),
    ("T03-016", "T03_016.Positive.one"),
    ("T03-016", "T03_016.Wrapped"),
    ("T03-016", "T03_016.Wrapped.wrap"),
    ("T03-016", "T03_016.choiceSwap"),
    ("T03-016", "T03_016.exFalso"),
    ("T03-016", "T03_016.orIdem"),
    ("T03-016", "T03_016.orSwap"),
    ("T03-016", "T03_016.unwrapToType"),
    ("T03-017", "T03_017.Colour"),
    ("T03-017", "T03_017.Colour.green"),
    ("T03-017", "T03_017.Colour.red"),
    ("T03-017", "T03_017.Sized"),
    ("T03-017", "T03_017.Sized.mk"),
    ("T03-017", "T03_017.Tri"),
    ("T03-017", "T03_017.Tri.a"),
    ("T03-017", "T03_017.Tri.b"),
    ("T03-017", "T03_017.Tri.c"),
    ("T03-017", "T03_017.Weird"),
    ("T03-017", "T03_017.Weird.mk"),
    ("T03-017", "T03_017.Weird2"),
    ("T03-017", "T03_017.Weird2.mk"),
    ("T03-017", "T03_017.accDepth"),
    ("T03-017", "T03_017.andToNat"),
    ("T03-017", "T03_017.colourProp"),
    ("T03-017", "T03_017.exFalsoNat"),
    ("T03-017", "T03_017.exFalsoType"),
    ("T03-017", "T03_017.orIdem"),
    ("T03-017", "T03_017.sizedToProp"),
    ("T03-017", "T03_017.transportType"),
    ("T03-017", "T03_017.triToTrue"),
    ("T03-017", "T03_017.w2a"),
    ("T03-017", "T03_017.w2b"),
    ("T03-017", "T03_017.weirdToNat"),
    ("T03-018", "T03_018.Chain"),
    ("T03-018", "T03_018.Chain.link"),
    ("T03-018", "T03_018.Chain.stop"),
    ("T03-018", "T03_018.Colour"),
    ("T03-018", "T03_018.Colour.blue"),
    ("T03-018", "T03_018.Colour.green"),
    ("T03-018", "T03_018.Colour.ofNat_toCtorIdx"),
    ("T03-018", "T03_018.Colour.red"),
    ("T03-018", "T03_018.Tri"),
    ("T03-018", "T03_018.Tri.a"),
    ("T03-018", "T03_018.Tri.b"),
    ("T03-018", "T03_018.Tri.c"),
    ("T03-018", "T03_018.instDecidableEqColour"),
    ("T03-018", "T03_018.isRed"),
    ("T03-018", "T03_018.linkHead"),
    ("T03-019", "T03_019.Even"),
    ("T03-019", "T03_019.Even.below.plusTwo"),
    ("T03-019", "T03_019.Even.below.zero"),
    ("T03-019", "T03_019.Even.plusTwo"),
    ("T03-019", "T03_019.Even.zero"),
    ("T03-019", "T03_019.Tagged"),
    ("T03-019", "T03_019.Tagged.marked"),
    ("T03-019", "T03_019.Tagged.plain"),
    ("T03-019", "T03_019.Three"),
    ("T03-019", "T03_019.Three.atOne"),
    ("T03-019", "T03_019.Three.atTwo"),
    ("T03-019", "T03_019.Three.atZero"),
    ("T03-019", "T03_019.Vect"),
    ("T03-019", "T03_019.Vect.cons"),
    ("T03-019", "T03_019.Vect.nil"),
    ("T03-019", "T03_019.isEmpty"),
    ("T03-019", "T03_019.markOf"),
    ("T03-019", "T03_019.v3"),
    ("T03-019", "T03_019.vcount"),
    ("T03-020", "T03_020.NeverAt"),
    ("T03-020", "T03_020.NotNothing"),
    ("T03-020", "T03_020.NotNothing.oops"),
    ("T03-020", "T03_020.Nothing"),
    ("T03-020", "T03_020.Void"),
    ("T03-020", "T03_020.VoidOf"),
    ("T03-020", "T03_020.elimToProp"),
    ("T03-020", "T03_020.elimToSort"),
    ("T03-020", "T03_020.elimToType"),
    ("T03-020", "T03_020.neverElim"),
    ("T03-020", "T03_020.notBoth"),
    ("T03-020", "T03_020.nothingElim"),
    ("T03-020", "T03_020.nothingToNat"),
    ("T03-020", "T03_020.viaNomatch"),
    ("T03-020", "T03_020.voidElim"),
    ("T03-020", "T03_020.voidOfElim"),
    ("T03-021", "T03_021.Colour"),
    ("T03-021", "T03_021.Colour.blue"),
    ("T03-021", "T03_021.Colour.green"),
    ("T03-021", "T03_021.Colour.red"),
    ("T03-021", "T03_021.Tree"),
    ("T03-021", "T03_021.Tree.leaf"),
    ("T03-021", "T03_021.Tree.node"),
    ("T03-021", "T03_021.bigMotive"),
    ("T03-021", "T03_021.code"),
    ("T03-021", "T03_021.code'"),
    ("T03-021", "T03_021.double"),
    ("T03-021", "T03_021.doubleMatch"),
    ("T03-021", "T03_021.isLeaf"),
    ("T03-021", "T03_021.totalRec"),
    ("T03-022", "T03_022.Box"),
    ("T03-022", "T03_022.Box.mk"),
    ("T03-022", "T03_022.Carrier"),
    ("T03-022", "T03_022.Carrier.mk"),
    ("T03-022", "T03_022.Carrier.point"),
    ("T03-022", "T03_022.Carrier.ty"),
    ("T03-022", "T03_022.Good"),
    ("T03-022", "T03_022.Good.mk"),
    ("T03-022", "T03_022.MA"),
    ("T03-022", "T03_022.MA.ma"),
    ("T03-022", "T03_022.MB"),
    ("T03-022", "T03_022.MB.mb"),
    ("T03-022", "T03_022.MB.mstop"),
    ("T03-022", "T03_022.Package"),
    ("T03-022", "T03_022.Package.mk"),
    ("T03-022", "T03_022.PropPackage"),
    ("T03-022", "T03_022.PropPackage.mk"),
    ("T03-022", "T03_022.boxed"),
    ("T03-022", "T03_022.goodNat"),
    ("T03-022", "T03_022.maNat"),
    ("T03-022", "T03_022.mbNat"),
    ("T03-022", "T03_022.natCarrier"),
    ("T03-022", "T03_022.packCarrier"),
    ("T03-022", "T03_022.packNat"),
    ("T03-022", "T03_022.ppack"),
    ("T03-022", "T03_022.unbox"),
    ("T03-023", "T03_023.Cell"),
    ("T03-023", "T03_023.Cell.flag"),
    ("T03-023", "T03_023.Cell.mk"),
    ("T03-023", "T03_023.Cell.n"),
    ("T03-023", "T03_023.Chain"),
    ("T03-023", "T03_023.Chain.link"),
    ("T03-023", "T03_023.Chain.stop"),
    ("T03-023", "T03_023.Colour"),
    ("T03-023", "T03_023.Colour.blue"),
    ("T03-023", "T03_023.Colour.green"),
    ("T03-023", "T03_023.Colour.ofNat_toCtorIdx"),
    ("T03-023", "T03_023.Colour.red"),
    ("T03-023", "T03_023.Small"),
    ("T03-023", "T03_023.Small.no"),
    ("T03-023", "T03_023.Small.yes"),
    ("T03-023", "T03_023.Tagged"),
    ("T03-023", "T03_023.Tagged.left"),
    ("T03-023", "T03_023.Tagged.right"),
    ("T03-023", "T03_023.WithInt"),
    ("T03-023", "T03_023.WithInt.mk"),
    ("T03-023", "T03_023.WithInt.v"),
    ("T03-023", "T03_023.instDecidableEqCell"),
    ("T03-023", "T03_023.instDecidableEqChain"),
    ("T03-023", "T03_023.instDecidableEqColour"),
    ("T03-023", "T03_023.instDecidableEqSmallTrue"),
    ("T03-023", "T03_023.instDecidableEqTagged"),
    ("T03-023", "T03_023.instDecidableEqWithInt"),
    ("T03-024", "T03_024.BigQuot"),
    ("T03-024", "T03_024.Parity"),
    ("T03-024", "T03_024.ParityQ"),
    ("T03-024", "T03_024.PropQuot"),
    ("T03-024", "T03_024.bigRel"),
    ("T03-024", "T03_024.mkBig"),
    ("T03-024", "T03_024.mkParity"),
    ("T03-024", "T03_024.mkParityQ"),
    ("T03-024", "T03_024.mkPropQuot"),
    ("T03-024", "T03_024.parityBit"),
    ("T03-024", "T03_024.parityBitQ"),
    ("T03-024", "T03_024.parityIsEven"),
    ("T03-024", "T03_024.paritySetoid"),
    ("T03-024", "T03_024.sameParity"),
    ("T04-001", "T04_001.addAcc"),
    ("T04-001", "T04_001.double"),
    ("T04-001", "T04_001.half2"),
    ("T04-001", "T04_001.sumTo"),
    ("T04-002", "T04_002.TList"),
    ("T04-002", "T04_002.TList.cons"),
    ("T04-002", "T04_002.TList.nil"),
    ("T04-002", "T04_002.app"),
    ("T04-002", "T04_002.leanLen"),
    ("T04-002", "T04_002.len"),
    ("T04-002", "T04_002.sumList"),
    ("T04-003", "T04_003.Forest"),
    ("T04-003", "T04_003.Forest.cons"),
    ("T04-003", "T04_003.Forest.nil"),
    ("T04-003", "T04_003.Tree"),
    ("T04-003", "T04_003.Tree.node"),
    ("T04-003", "T04_003.evenFrom"),
    ("T04-003", "T04_003.isEven"),
    ("T04-003", "T04_003.isOdd"),
    ("T04-003", "T04_003.oddFrom"),
    ("T04-006", "T04_006.addRec"),
    ("T04-006", "T04_006.addRecZero"),
    ("T04-006", "T04_006.isZero"),
    ("T04-006", "T04_006.repeatNat"),
    ("T04-006", "T04_006.tower"),
    ("T04-006", "T04_006.towerRec"),
    ("T04-007", "T04_007.eqBool"),
    ("T04-007", "T04_007.firstTrue"),
    ("T04-007", "T04_007.sub2"),
    ("T04-007", "T04_007.zipLen"),
    ("T04-008", "T04_008.Shape"),
    ("T04-008", "T04_008.Shape.circle"),
    ("T04-008", "T04_008.Shape.rect"),
    ("T04-008", "T04_008.classify"),
    ("T04-008", "T04_008.describe"),
    ("T04-008", "T04_008.firstComponents"),
    ("T04-008", "T04_008.shadowed"),
    ("T04-008", "T04_008.sumPairs"),
    ("T04-009", "T04_009.both"),
    ("T04-009", "T04_009.bucket"),
    ("T04-009", "T04_009.collapse"),
    ("T04-009", "T04_009.pick"),
    ("T04-010", "T04_010.Color"),
    ("T04-010", "T04_010.Color.blue"),
    ("T04-010", "T04_010.Color.cyan"),
    ("T04-010", "T04_010.Color.green"),
    ("T04-010", "T04_010.Color.magenta"),
    ("T04-010", "T04_010.Color.red"),
    ("T04-010", "T04_010.countReds"),
    ("T04-010", "T04_010.isNil"),
    ("T04-010", "T04_010.isRed"),
    ("T04-010", "T04_010.notRed"),
    ("T04-010", "T04_010.pickCol"),
    ("T04-010", "T04_010.rank"),
    ("T04-011", "T04_011.Carrier"),
    ("T04-011", "T04_011.Vect"),
    ("T04-011", "T04_011.Vect.cons"),
    ("T04-011", "T04_011.Vect.nil"),
    ("T04-011", "T04_011.mkCarrier"),
    ("T04-011", "T04_011.mkCarrier'"),
    ("T04-011", "T04_011.notVar"),
    ("T04-011", "T04_011.transportNat"),
    ("T04-011", "T04_011.vreverseStep"),
    ("T04-011", "T04_011.zeroOrPos"),
    ("T04-012", "T04_012.findSub"),
    ("T04-012", "T04_012.firstEven"),
    ("T04-012", "T04_012.firstPos"),
    ("T04-012", "T04_012.parity"),
    ("T04-013", "T04_013.Vect"),
    ("T04-013", "T04_013.Vect.cons"),
    ("T04-013", "T04_013.Vect.nil"),
    ("T04-013", "T04_013.absurdElim"),
    ("T04-013", "T04_013.nomatchEmpty"),
    ("T04-013", "T04_013.vnilOnly"),
    ("T04-014", "T04_014.Even"),
    ("T04-014", "T04_014.Even.below.step"),
    ("T04-014", "T04_014.Even.below.zero"),
    ("T04-014", "T04_014.Even.step"),
    ("T04-014", "T04_014.Even.zero"),
    ("T04-014", "T04_014.Vect"),
    ("T04-014", "T04_014.Vect.cons"),
    ("T04-014", "T04_014.Vect.nil"),
    ("T04-014", "T04_014.vnilFill"),
    ("T04-016", "T04_016.belowType"),
    ("T04-016", "T04_016.lenBrec"),
    ("T04-016", "T04_016.plainStructural"),
    ("T04-016", "T04_016.strongLe"),
    ("T04-016", "T04_016.sumBrec"),
    ("T04-017", "T04_017.dbl"),
    ("T04-017", "T04_017.firstNonZero"),
    ("T04-018", "T04_018.fib"),
    ("T04-018", "T04_018.halve"),
    ("T04-018", "T04_018.letAlias"),
    ("T04-018", "T04_018.letAliasNat"),
    ("T04-018", "T04_018.pairSum"),
    ("T04-019", "T04_019.clamp"),
    ("T04-019", "T04_019.countOdds"),
    ("T04-019", "T04_019.isZero"),
    ("T04-019", "T04_019.pickB"),
    ("T04-019", "T04_019.pickCond"),
    ("T04-019", "T04_019.pickExplicit"),
    ("T04-019", "T04_019.pickNotB"),
    ("T04-019", "T04_019.sign3"),
    ("T04-020", "T04_020.Carrier"),
    ("T04-020", "T04_020.byCasesEx"),
    ("T04-020", "T04_020.describe"),
    ("T04-020", "T04_020.diteExplicit"),
    ("T04-020", "T04_020.dropZeros"),
    ("T04-020", "T04_020.firstBig"),
    ("T04-020", "T04_020.predOf"),
    ("T04-020", "T04_020.safeDiv"),
    ("T05-001", "T05_001.applyTo"),
    ("T05-001", "T05_001.constMid"),
    ("T05-001", "T05_001.emptyOf"),
    ("T05-001", "T05_001.idf"),
    ("T05-001", "T05_001.idfEq"),
    ("T05-001", "T05_001.pairUp"),
    ("T05-002", "T05_002.memOf"),
    ("T05-002", "T05_002.memOf'"),
    ("T05-002", "T05_002.memPair"),
    ("T05-002", "T05_002.mixed"),
    ("T05-002", "T05_002.nilOf"),
    ("T05-003", "T05_003.Defaultable"),
    ("T05-003", "T05_003.Defaultable.dflt"),
    ("T05-003", "T05_003.Defaultable.mk"),
    ("T05-003", "T05_003.Sizeable"),
    ("T05-003", "T05_003.Sizeable.mk"),
    ("T05-003", "T05_003.Sizeable.size"),
    ("T05-003", "T05_003.defaultOf"),
    ("T05-003", "T05_003.instDefaultableBool"),
    ("T05-003", "T05_003.instDefaultableNat"),
    ("T05-003", "T05_003.instSizeableBool"),
    ("T05-003", "T05_003.instSizeableList"),
    ("T05-003", "T05_003.instSizeableNat"),
    ("T05-003", "T05_003.sizeBoth"),
    ("T05-003", "T05_003.sizeList"),
    ("T05-003", "T05_003.sizeOf'"),
    ("T05-004", "T05_004.Pair"),
    ("T05-004", "T05_004.Pair.fst"),
    ("T05-004", "T05_004.Pair.mk"),
    ("T05-004", "T05_004.Pair.snd"),
    ("T05-004", "T05_004.Sizeable"),
    ("T05-004", "T05_004.Sizeable.mk"),
    ("T05-004", "T05_004.Sizeable.size"),
    ("T05-004", "T05_004.compose"),
    ("T05-004", "T05_004.idf"),
    ("T05-004", "T05_004.instSizeableNat"),
    ("T05-004", "T05_004.natId"),
    ("T05-004", "T05_004.polyId"),
    ("T05-004", "T05_004.sizeOf'"),
    ("T05-005", "T05_005.Point"),
    ("T05-005", "T05_005.Point.mk"),
    ("T05-005", "T05_005.Point.x"),
    ("T05-005", "T05_005.Point.y"),
    ("T05-005", "T05_005.depNamed"),
    ("T05-005", "T05_005.idf"),
    ("T05-005", "T05_005.skipping"),
    ("T05-005", "T05_005.sub3"),
    ("T05-005", "T05_005.withDefaults"),
    ("T05-006", "T05_006.Wrap"),
    ("T05-006", "T05_006.Wrap.mk"),
    ("T05-006", "T05_006.Wrap.tag"),
    ("T05-006", "T05_006.Wrap.val"),
    ("T05-006", "T05_006.bigDefault"),
    ("T05-006", "T05_006.midDefault"),
    ("T05-006", "T05_006.rawOpt"),
    ("T05-006", "T05_006.sized"),
    ("T05-006", "T05_006.twoDefaults"),
    ("T05-006", "T05_006.w1"),
    ("T05-006", "T05_006.withDefault"),
    ("T05-008", "T05_008.BoxOf"),
    ("T05-008", "T05_008.BoxOf.mk"),
    ("T05-008", "T05_008.applyF"),
    ("T05-008", "T05_008.collide"),
    ("T05-008", "T05_008.constf"),
    ("T05-008", "T05_008.idf"),
    ("T05-008", "T05_008.idfEq"),
    ("T05-008", "T05_008.replicateN"),
    ("T05-008", "T05_008.sortId"),
    ("T05-009", "T05_009.NestedPair"),
    ("T05-009", "T05_009.NestedPair.head"),
    ("T05-009", "T05_009.NestedPair.mk"),
    ("T05-009", "T05_009.NestedPair.rest"),
    ("T05-009", "T05_009.Pair"),
    ("T05-009", "T05_009.Pair.fst"),
    ("T05-009", "T05_009.Pair.mk"),
    ("T05-009", "T05_009.Pair.snd"),
    ("T05-009", "T05_009.Point"),
    ("T05-009", "T05_009.Point.mk"),
    ("T05-009", "T05_009.Point.x"),
    ("T05-009", "T05_009.Point.y"),
    ("T05-009", "T05_009.Triple"),
    ("T05-009", "T05_009.Triple.a"),
    ("T05-009", "T05_009.Triple.b"),
    ("T05-009", "T05_009.Triple.c"),
    ("T05-009", "T05_009.Triple.mk"),
    ("T05-009", "T05_009.emptyPair"),
    ("T05-010", "T05_010.Color"),
    ("T05-010", "T05_010.Color.blue"),
    ("T05-010", "T05_010.Color.green"),
    ("T05-010", "T05_010.Color.red"),
    ("T05-010", "T05_010.MyPoint"),
    ("T05-010", "T05_010.Pair"),
    ("T05-010", "T05_010.Pair.fst"),
    ("T05-010", "T05_010.Pair.mk"),
    ("T05-010", "T05_010.Pair.snd"),
    ("T05-010", "T05_010.Point"),
    ("T05-010", "T05_010.Point.mk"),
    ("T05-010", "T05_010.Point.norm1"),
    ("T05-010", "T05_010.Point.shift"),
    ("T05-010", "T05_010.Point.x"),
    ("T05-010", "T05_010.Point.y"),
    ("T05-010", "T05_010.c"),
    ("T05-010", "T05_010.firstOrRed"),
    ("T05-010", "T05_010.getFst"),
    ("T05-010", "T05_010.mp"),
    ("T05-011", "T05_011.Bit"),
    ("T05-011", "T05_011.Bit.one"),
    ("T05-011", "T05_011.Bit.zero"),
    ("T05-011", "T05_011.Wrapped"),
    ("T05-011", "T05_011.Wrapped.mk"),
    ("T05-011", "T05_011.Wrapped.unwrap"),
    ("T05-011", "T05_011.bitSum"),
    ("T05-011", "T05_011.bitToBool"),
    ("T05-011", "T05_011.bits"),
    ("T05-011", "T05_011.boolToNat"),
    ("T05-011", "T05_011.boolToNat'"),
    ("T05-011", "T05_011.instCoeBitBool"),
    ("T05-011", "T05_011.instCoeWrapped"),
    ("T05-011", "T05_011.sumWith"),
    ("T05-011", "T05_011.takesNat"),
    ("T05-011", "T05_011.unwrapNat"),
    ("T05-012", "T05_012.Arity"),
    ("T05-012", "T05_012.Arity.mk"),
    ("T05-012", "T05_012.Arity.n"),
    ("T05-012", "T05_012.ArityFun"),
    ("T05-012", "T05_012.Fn"),
    ("T05-012", "T05_012.Fn.f"),
    ("T05-012", "T05_012.Fn.mk"),
    ("T05-012", "T05_012.Named"),
    ("T05-012", "T05_012.Named.mk"),
    ("T05-012", "T05_012.Named.name"),
    ("T05-012", "T05_012.Named.run"),
    ("T05-012", "T05_012.PropLike"),
    ("T05-012", "T05_012.PropLike.holds"),
    ("T05-012", "T05_012.PropLike.mk"),
    ("T05-012", "T05_012.Ty"),
    ("T05-012", "T05_012.Ty.carrier"),
    ("T05-012", "T05_012.Ty.mk"),
    ("T05-012", "T05_012.assertIt"),
    ("T05-012", "T05_012.idOfTy"),
    ("T05-012", "T05_012.instCoeFunArityArityFun"),
    ("T05-012", "T05_012.instCoeFunFnForallNat"),
    ("T05-012", "T05_012.instCoeFunNamedForallNat"),
    ("T05-012", "T05_012.instCoeSortPropLikeProp"),
    ("T05-012", "T05_012.instCoeSortTyType"),
    ("T05-012", "T05_012.useFn"),
    ("T05-012", "T05_012.useNamed"),
    ("T05-013", "T05_013.Pair"),
    ("T05-013", "T05_013.Pair.fst"),
    ("T05-013", "T05_013.Pair.mk"),
    ("T05-013", "T05_013.Pair.snd"),
    ("T05-013", "T05_013.applyTo"),
    ("T05-013", "T05_013.boolCase"),
    ("T05-013", "T05_013.compose"),
    ("T05-013", "T05_013.holeProof"),
    ("T05-013", "T05_013.idf"),
    ("T05-013", "T05_013.viaLater"),
    ("T05-014", "T05_014.Alias"),
    ("T05-014", "T05_014.Carrier"),
    ("T05-014", "T05_014.mk"),
    ("T05-015", "T05_015.Defaultable"),
    ("T05-015", "T05_015.Defaultable.dflt"),
    ("T05-015", "T05_015.Defaultable.mk"),
    ("T05-015", "T05_015.MyNum"),
    ("T05-015", "T05_015.MyNum.mk"),
    ("T05-015", "T05_015.MyNum.v"),
    ("T05-015", "T05_015.Sizeable"),
    ("T05-015", "T05_015.Sizeable.mk"),
    ("T05-015", "T05_015.Sizeable.size"),
    ("T05-015", "T05_015.addOne"),
    ("T05-015", "T05_015.addOne'"),
    ("T05-015", "T05_015.addOneInt"),
    ("T05-015", "T05_015.applyIt"),
    ("T05-015", "T05_015.defaultOf"),
    ("T05-015", "T05_015.instDefaultableBool"),
    ("T05-015", "T05_015.instDefaultableNat"),
    ("T05-015", "T05_015.instOfNatMyNumOfNatNat"),
    ("T05-015", "T05_015.instSizeableBool"),
    ("T05-015", "T05_015.instSizeableNat"),
    ("T05-015", "T05_015.laterPin"),
    ("T05-015", "T05_015.sizeOf'"),
    ("T05-015", "T05_015.three"),
    ("T05-016", "T05_016.Outer"),
    ("T05-016", "T05_016.Outer.hd"),
    ("T05-016", "T05_016.Outer.inner"),
    ("T05-016", "T05_016.Outer.mk"),
    ("T05-016", "T05_016.Three"),
    ("T05-016", "T05_016.Three.mk"),
    ("T05-016", "T05_016.Three.x"),
    ("T05-016", "T05_016.Three.y"),
    ("T05-016", "T05_016.Three.z"),
    ("T05-016", "T05_016.anyThree"),
    ("T05-016", "T05_016.bumpX"),
    ("T05-016", "T05_016.emptyNats"),
    ("T05-016", "T05_016.getX"),
    ("T05-016", "T05_016.innerX"),
    ("T05-016", "T05_016.sumOuters"),
    ("T05-016", "T05_016.t0"),
    ("T05-017", "T05_017.MyNum"),
    ("T05-017", "T05_017.MyNum.mk"),
    ("T05-017", "T05_017.MyNum.v"),
    ("T05-017", "T05_017.WrapNat"),
    ("T05-017", "T05_017.WrapNat.mk"),
    ("T05-017", "T05_017.WrapNat.unwrap"),
    ("T05-017", "T05_017.atInt"),
    ("T05-017", "T05_017.atMyNum"),
    ("T05-017", "T05_017.atMyNum7"),
    ("T05-017", "T05_017.atNat"),
    ("T05-017", "T05_017.bare"),
    ("T05-017", "T05_017.huge"),
    ("T05-017", "T05_017.instAllNums"),
    ("T05-017", "T05_017.instOfNatMyNumOfNatNat"),
    ("T05-017", "T05_017.instOfNatWrapNat"),
    ("T05-017", "T05_017.negFive"),
    ("T05-017", "T05_017.takesInt"),
    ("T05-017", "T05_017.wrapped"),
    ("T05-018", "T05_018.Carrier"),
    ("T05-018", "T05_018.aProp"),
    ("T05-018", "T05_018.asInt"),
    ("T05-018", "T05_018.asNat"),
    ("T05-018", "T05_018.atType0"),
    ("T05-018", "T05_018.boolToNat"),
    ("T05-018", "T05_018.coerced"),
    ("T05-018", "T05_018.idf"),
    ("T05-018", "T05_018.mk"),
    ("T05-018", "T05_018.pinned"),
    ("T05-018", "T05_018.pinnedFun"),
    ("T05-018", "T05_018.showBy"),
    ("T05-018", "T05_018.showFrom"),
    ("T05-019", "T05_019.firstBig"),
    ("T05-019", "T05_019.guarded"),
    ("T05-019", "T05_019.optAdd"),
    ("T05-019", "T05_019.optInc"),
    ("T05-019", "T05_019.pureDo"),
    ("T05-019", "T05_019.sumTo"),
    ("T05-020", "T05_020.MyOpt"),
    ("T05-020", "T05_020.MyOpt.bind"),
    ("T05-020", "T05_020.MyOpt.none"),
    ("T05-020", "T05_020.MyOpt.some"),
    ("T05-020", "T05_020.bindEx"),
    ("T05-020", "T05_020.decideEx"),
    ("T05-020", "T05_020.instMonadMyOpt"),
    ("T05-020", "T05_020.mapEx"),
    ("T05-020", "T05_020.myTwice"),
    ("T05-020", "T05_020.pureEx"),
    ("T05-020", "T05_020.readNothing"),
    ("T05-020", "T05_020.seqEx"),
    ("T05-020", "T05_020.twice"),
    ("T06-001", "T06_001.explicitRefl"),
    ("T06-001", "T06_001.funRefl"),
    ("T06-001", "T06_001.largeEq"),
    ("T06-001", "T06_001.natCompute"),
    ("T06-001", "T06_001.natRefl"),
    ("T06-001", "T06_001.polyRefl"),
    ("T06-001", "T06_001.propEq"),
    ("T06-003", "T06_003.chained"),
    ("T06-003", "T06_003.congrArgCross"),
    ("T06-003", "T06_003.congrArgNat"),
    ("T06-003", "T06_003.congrBoth"),
    ("T06-003", "T06_003.congrFunNat"),
    ("T06-003", "T06_003.sym"),
    ("T06-003", "T06_003.trans"),
    ("T06-004", "T06_004.Bounded"),
    ("T06-004", "T06_004.Bounded.isSmall"),
    ("T06-004", "T06_004.Bounded.mk"),
    ("T06-004", "T06_004.Bounded.val"),
    ("T06-004", "T06_004.Small"),
    ("T06-004", "T06_004.boundedEta"),
    ("T06-004", "T06_004.irrelevanceUnderBinder"),
    ("T06-004", "T06_004.orProofs"),
    ("T06-004", "T06_004.reducedProps"),
    ("T06-004", "T06_004.twoProofs"),
    ("T06-005", "T06_005.andBoth"),
    ("T06-005", "T06_005.andLeft"),
    ("T06-005", "T06_005.andRight"),
    ("T06-005", "T06_005.arrowIsNot"),
    ("T06-005", "T06_005.exFalso"),
    ("T06-005", "T06_005.exFalsoLarge"),
    ("T06-005", "T06_005.iffAsAnd"),
    ("T06-005", "T06_005.iffIntro"),
    ("T06-005", "T06_005.iffMp"),
    ("T06-005", "T06_005.iffMpr"),
    ("T06-005", "T06_005.mixed"),
    ("T06-005", "T06_005.notIsArrow"),
    ("T06-005", "T06_005.orElim"),
    ("T06-005", "T06_005.orIntroL"),
    ("T06-005", "T06_005.orIntroR"),
    ("T06-005", "T06_005.trueIntro"),
    ("T06-006", "T06_006.bigDomain"),
    ("T06-006", "T06_006.existsAnon"),
    ("T06-006", "T06_006.existsElim"),
    ("T06-006", "T06_006.existsNat"),
    ("T06-006", "T06_006.existsNested"),
    ("T06-006", "T06_006.existsWithEq"),
    ("T06-006", "T06_006.forallExists"),
    ("T06-006", "T06_006.propDomain"),
    ("T06-007", "T06_007.alternating"),
    ("T06-007", "T06_007.bigDomainProp"),
    ("T06-007", "T06_007.forallDependent"),
    ("T06-007", "T06_007.forallRefl"),
    ("T06-007", "T06_007.higherOrder"),
    ("T06-007", "T06_007.implication"),
    ("T06-007", "T06_007.quantifyOverProp"),
    ("T06-007", "T06_007.shadowed"),
    ("T06-008", "T06_008.byContra"),
    ("T06-008", "T06_008.cases"),
    ("T06-008", "T06_008.deMorgan"),
    ("T06-008", "T06_008.dne"),
    ("T06-008", "T06_008.dneUnfolded"),
    ("T06-008", "T06_008.dni"),
    ("T06-008", "T06_008.em"),
    ("T06-008", "T06_008.notNotIff"),
    ("T06-009", "T06_009.chooseBranch"),
    ("T06-009", "T06_009.decideAnything"),
    ("T06-009", "T06_009.describe"),
    ("T06-009", "T06_009.innocuous"),
    ("T06-009", "T06_009.pick"),
    ("T06-009", "T06_009.witnessOf"),
    ("T06-009", "T06_009.witnessSpec"),
    ("T06-010", "T06_010.collapse"),
    ("T06-010", "T06_010.ext"),
    ("T06-010", "T06_010.innocuous"),
    ("T06-010", "T06_010.predExt"),
    ("T06-010", "T06_010.rewriteByIff"),
    ("T06-010", "T06_010.toEq"),
    ("T06-010", "T06_010.toEqByRw"),
    ("T06-011", "T06_011.dependentFunext"),
    ("T06-011", "T06_011.ext"),
    ("T06-011", "T06_011.extSimple"),
    ("T06-011", "T06_011.higherOrderExt"),
    ("T06-011", "T06_011.innocuous"),
    ("T06-011", "T06_011.pointwiseNotDefEq"),
    ("T06-011", "T06_011.predExt"),
    ("T06-012", "T06_012.soundness"),
    ("T06-013", "T06_013.Colour"),
    ("T06-013", "T06_013.Colour.blue"),
    ("T06-013", "T06_013.Colour.green"),
    ("T06-013", "T06_013.Colour.ofNat_toCtorIdx"),
    ("T06-013", "T06_013.Colour.red"),
    ("T06-013", "T06_013.bridge"),
    ("T06-013", "T06_013.choose"),
    ("T06-013", "T06_013.cmp"),
    ("T06-013", "T06_013.decideCustom"),
    ("T06-013", "T06_013.instDecidableEqColour"),
    ("T06-013", "T06_013.natDecide"),
    ("T06-013", "T06_013.no"),
    ("T06-013", "T06_013.yes"),
    ("T06-014", "T06_014.bigCheck"),
    ("T06-015", "T06_015.MySubsingleton"),
    ("T06-015", "T06_015.MySubsingleton.elim"),
    ("T06-015", "T06_015.MySubsingleton.mk"),
    ("T06-015", "T06_015.bigSquash"),
    ("T06-015", "T06_015.instMySubsingletonUnit"),
    ("T06-015", "T06_015.propSubsingleton"),
    ("T06-015", "T06_015.squashEq"),
    ("T06-015", "T06_015.squashLift"),
    ("T06-015", "T06_015.squashed"),
    ("T06-015", "T06_015.useElim"),
    ("T06-016", "T06_016.Vect"),
    ("T06-016", "T06_016.Vect.cons"),
    ("T06-016", "T06_016.Vect.nil"),
    ("T06-016", "T06_016.acrossTypes"),
    ("T06-016", "T06_016.fromHEq"),
    ("T06-016", "T06_016.heqRefl"),
    ("T06-016", "T06_016.heqTrans"),
    ("T06-016", "T06_016.toHEq"),
    ("T06-016", "T06_016.vectHEq"),
    ("T06-017", "T06_017.clean"),
    ("T06-018", "T06_018.blanketInstance"),
    ("T06-018", "T06_018.byCases"),
    ("T06-018", "T06_018.choiceFunction"),
    ("T06-018", "T06_018.decEqAnything"),
    ("T06-018", "T06_018.notForall"),
    ("T06-018", "T06_018.skolem"),
    ("T07-001", "T07_001.BigClass"),
    ("T07-001", "T07_001.BigClass.mk"),
    ("T07-001", "T07_001.BigClass.pick"),
    ("T07-001", "T07_001.Lonely"),
    ("T07-001", "T07_001.Lonely.mk"),
    ("T07-001", "T07_001.Lonely.witness"),
    ("T07-001", "T07_001.Magma"),
    ("T07-001", "T07_001.Magma.mk"),
    ("T07-001", "T07_001.Magma.op"),
    ("T07-001", "T07_001.Magma.unit"),
    ("T07-001", "T07_001.Pointed"),
    ("T07-001", "T07_001.Pointed.mk"),
    ("T07-001", "T07_001.Pointed.point"),
    ("T07-001", "T07_001.PropClass"),
    ("T07-001", "T07_001.PropClass.holds"),
    ("T07-001", "T07_001.PropClass.mk"),
    ("T07-001", "T07_001.combine"),
    ("T07-001", "T07_001.getPoint"),
    ("T07-001", "T07_001.instMagmaNat"),
    ("T07-001", "T07_001.instPointedNat"),
    ("T07-001", "T07_001.instPropClassNat"),
    ("T07-001", "T07_001.useLonely"),
    ("T07-002", "T07_002.Pointed"),
    ("T07-002", "T07_002.Pointed.mk"),
    ("T07-002", "T07_002.Pointed.point"),
    ("T07-002", "T07_002.Sized"),
    ("T07-002", "T07_002.Sized.mk"),
    ("T07-002", "T07_002.Sized.size"),
    ("T07-002", "T07_002.Tagged"),
    ("T07-002", "T07_002.Tagged.mk"),
    ("T07-002", "T07_002.Tagged.tag"),
    ("T07-002", "T07_002.dup₁"),
    ("T07-002", "T07_002.dup₂"),
    ("T07-002", "T07_002.instPointedNat"),
    ("T07-002", "T07_002.measure"),
    ("T07-002", "T07_002.pointedBool"),
    ("T07-002", "T07_002.sizedNat"),
    ("T07-002", "T07_002.usesBool"),
    ("T07-002", "T07_002.usesNat"),
    ("T07-002", "T07_002.whichTag"),
    ("T07-003", "T07_003.Pointed"),
    ("T07-003", "T07_003.Pointed.mk"),
    ("T07-003", "T07_003.Pointed.point"),
    ("T07-003", "T07_003.Sized"),
    ("T07-003", "T07_003.Sized.mk"),
    ("T07-003", "T07_003.Sized.size"),
    ("T07-003", "T07_003.earlyResolution"),
    ("T07-003", "T07_003.fromExpectedType"),
    ("T07-003", "T07_003.generic"),
    ("T07-003", "T07_003.instPointedBool"),
    ("T07-003", "T07_003.instPointedNat"),
    ("T07-003", "T07_003.nested"),
    ("T07-003", "T07_003.sizedBool"),
    ("T07-003", "T07_003.sizedNat"),
    ("T07-003", "T07_003.twoPremises"),
    ("T07-004", "T07_004.Box"),
    ("T07-004", "T07_004.Box.mk"),
    ("T07-004", "T07_004.Pair2"),
    ("T07-004", "T07_004.Pair2.mk"),
    ("T07-004", "T07_004.Pair2Show"),
    ("T07-004", "T07_004.Pair2Show.mk"),
    ("T07-004", "T07_004.Pair2Show.render"),
    ("T07-004", "T07_004.Sized"),
    ("T07-004", "T07_004.Sized.mk"),
    ("T07-004", "T07_004.Sized.size"),
    ("T07-004", "T07_004.boxedBoxedNat"),
    ("T07-004", "T07_004.deepChain"),
    ("T07-004", "T07_004.instPair2ShowBoxOfSized"),
    ("T07-004", "T07_004.instPair2ShowNat"),
    ("T07-004", "T07_004.sizeOf'"),
    ("T07-004", "T07_004.sizedBox"),
    ("T07-004", "T07_004.sizedNat"),
    ("T07-004", "T07_004.sizedPair2"),
    ("T07-005", "T07_005.Bar"),
    ("T07-005", "T07_005.Bar.left"),
    ("T07-005", "T07_005.Bar.mk"),
    ("T07-005", "T07_005.Bar.toFoo"),
    ("T07-005", "T07_005.Baz"),
    ("T07-005", "T07_005.Baz.mk"),
    ("T07-005", "T07_005.Baz.right"),
    ("T07-005", "T07_005.Baz.toFoo"),
    ("T07-005", "T07_005.Foo"),
    ("T07-005", "T07_005.Foo.base"),
    ("T07-005", "T07_005.Foo.mk"),
    ("T07-005", "T07_005.Qux"),
    ("T07-005", "T07_005.Qux.join"),
    ("T07-005", "T07_005.Qux.mk"),
    ("T07-005", "T07_005.Qux.right"),
    ("T07-005", "T07_005.Qux.toBar"),
    ("T07-005", "T07_005.Qux.toBaz"),
    ("T07-005", "T07_005.allFour"),
    ("T07-005", "T07_005.baseViaQux"),
    ("T07-005", "T07_005.instBarNat"),
    ("T07-005", "T07_005.instBazNat"),
    ("T07-005", "T07_005.instFooNat"),
    ("T07-005", "T07_005.instQuxNat"),
    ("T07-005", "T07_005.parentOfBar"),
    ("T07-005", "T07_005.useFoo"),
    ("T07-005", "T07_005.useQux"),
    ("T07-005", "T07_005.viaBar"),
    ("T07-005", "T07_005.viaBaz"),
    ("T07-006", "T07_006.Decode"),
    ("T07-006", "T07_006.Decode.decode"),
    ("T07-006", "T07_006.Decode.mk"),
    ("T07-006", "T07_006.HPlus"),
    ("T07-006", "T07_006.HPlus.hPlus"),
    ("T07-006", "T07_006.HPlus.mk"),
    ("T07-006", "T07_006.Mem"),
    ("T07-006", "T07_006.Mem.mem"),
    ("T07-006", "T07_006.Mem.mk"),
    ("T07-006", "T07_006.Rel"),
    ("T07-006", "T07_006.Rel.mk"),
    ("T07-006", "T07_006.Rel.related"),
    ("T07-006", "T07_006.decodeToNat"),
    ("T07-006", "T07_006.decoded"),
    ("T07-006", "T07_006.instHPlusNat"),
    ("T07-006", "T07_006.instHPlusNatBool"),
    ("T07-006", "T07_006.instMemNatList"),
    ("T07-006", "T07_006.instRelNat"),
    ("T07-006", "T07_006.isIn"),
    ("T07-006", "T07_006.sum₁"),
    ("T07-006", "T07_006.sum₂"),
    ("T07-007", "T07_007.Money"),
    ("T07-007", "T07_007.Money.cents"),
    ("T07-007", "T07_007.Money.mk"),
    ("T07-007", "T07_007.Zeroish"),
    ("T07-007", "T07_007.Zeroish.mk"),
    ("T07-007", "T07_007.Zeroish.zero"),
    ("T07-007", "T07_007.defaulted"),
    ("T07-007", "T07_007.instOfNatMoney"),
    ("T07-007", "T07_007.intLit"),
    ("T07-007", "T07_007.lit"),
    ("T07-007", "T07_007.notDefaulted"),
    ("T07-007", "T07_007.plus"),
    ("T07-007", "T07_007.price"),
    ("T07-007", "T07_007.zeroishBool"),
    ("T07-007", "T07_007.zeroishNat"),
    ("T07-008", "T07_008.Box"),
    ("T07-008", "T07_008.Box.mk"),
    ("T07-008", "T07_008.Inverted"),
    ("T07-008", "T07_008.Inverted.mk"),
    ("T07-008", "T07_008.Inverted.value"),
    ("T07-008", "T07_008.Ranked"),
    ("T07-008", "T07_008.Ranked.mk"),
    ("T07-008", "T07_008.Ranked.rank"),
    ("T07-008", "T07_008.Render"),
    ("T07-008", "T07_008.Render.mk"),
    ("T07-008", "T07_008.Render.render"),
    ("T07-008", "T07_008.Scoped"),
    ("T07-008", "T07_008.Scoped.mk"),
    ("T07-008", "T07_008.Scoped.scopedValue"),
    ("T07-008", "T07_008.Tagged"),
    ("T07-008", "T07_008.Tagged.mk"),
    ("T07-008", "T07_008.Tagged.tag"),
    ("T07-008", "T07_008.earlyHigh"),
    ("T07-008", "T07_008.firstDeclared"),
    ("T07-008", "T07_008.generalBeatsSpecific"),
    ("T07-008", "T07_008.generalBox"),
    ("T07-008", "T07_008.highPriority"),
    ("T07-008", "T07_008.instRenderNat"),
    ("T07-008", "T07_008.invertedWins"),
    ("T07-008", "T07_008.lateLow"),
    ("T07-008", "T07_008.laterWins"),
    ("T07-008", "T07_008.localOnly"),
    ("T07-008", "T07_008.lowPriority"),
    ("T07-008", "T07_008.priorityWins"),
    ("T07-008", "T07_008.secondDeclared"),
    ("T07-008", "T07_008.specificBox"),
    ("T07-008", "T07_008.usesLocal"),
    ("T07-009", "T07_009.Colour"),
    ("T07-009", "T07_009.Colour.blue"),
    ("T07-009", "T07_009.Colour.green"),
    ("T07-009", "T07_009.Colour.ofNat_toCtorIdx"),
    ("T07-009", "T07_009.Colour.red"),
    ("T07-009", "T07_009.Flag"),
    ("T07-009", "T07_009.Flag.off"),
    ("T07-009", "T07_009.Flag.on"),
    ("T07-009", "T07_009.Pt"),
    ("T07-009", "T07_009.Pt.mk"),
    ("T07-009", "T07_009.Pt.x"),
    ("T07-009", "T07_009.Pt.y"),
    ("T07-009", "T07_009.Tree"),
    ("T07-009", "T07_009.Tree.leaf"),
    ("T07-009", "T07_009.Tree.node"),
    ("T07-009", "T07_009.boolEq"),
    ("T07-009", "T07_009.colourEq"),
    ("T07-009", "T07_009.flagEq"),
    ("T07-009", "T07_009.instDecidableEqColour"),
    ("T07-009", "T07_009.instDecidableEqFlag"),
    ("T07-009", "T07_009.instDecidableEqPt"),
    ("T07-009", "T07_009.instDecidableEqTree"),
    ("T07-009", "T07_009.intEq"),
    ("T07-009", "T07_009.listEq"),
    ("T07-009", "T07_009.natEq"),
    ("T07-009", "T07_009.pairEq"),
    ("T07-009", "T07_009.treeEq"),
    ("T07-010", "T07_010.Box"),
    ("T07-010", "T07_010.Box.mk"),
    ("T07-010", "T07_010.MyInhabited"),
    ("T07-010", "T07_010.MyInhabited.deflt"),
    ("T07-010", "T07_010.MyInhabited.mk"),
    ("T07-010", "T07_010.MyNonempty"),
    ("T07-010", "T07_010.MyNonempty.intro"),
    ("T07-010", "T07_010.boxedDefault"),
    ("T07-010", "T07_010.getDefault"),
    ("T07-010", "T07_010.inhabitedNonempty"),
    ("T07-010", "T07_010.instMyInhabitedBool"),
    ("T07-010", "T07_010.instMyInhabitedBox"),
    ("T07-010", "T07_010.instMyInhabitedNat"),
    ("T07-010", "T07_010.natNonempty"),
    ("T07-010", "T07_010.nonemptyElim"),
    ("T07-010", "T07_010.opaqueThing"),
    ("T07-011", "T07_011.Action"),
    ("T07-011", "T07_011.Action.mk"),
    ("T07-011", "T07_011.Action.smul"),
    ("T07-011", "T07_011.Group"),
    ("T07-011", "T07_011.Group.inv"),
    ("T07-011", "T07_011.Group.inv_mul"),
    ("T07-011", "T07_011.Group.mk"),
    ("T07-011", "T07_011.Group.toMonoid"),
    ("T07-011", "T07_011.Monoid"),
    ("T07-011", "T07_011.Monoid.mk"),
    ("T07-011", "T07_011.Monoid.mul_one"),
    ("T07-011", "T07_011.Monoid.one_mul"),
    ("T07-011", "T07_011.Monoid.toOne"),
    ("T07-011", "T07_011.Monoid.toSemigroup"),
    ("T07-011", "T07_011.Mul"),
    ("T07-011", "T07_011.Mul.mk"),
    ("T07-011", "T07_011.Mul.mul"),
    ("T07-011", "T07_011.One"),
    ("T07-011", "T07_011.One.mk"),
    ("T07-011", "T07_011.One.one"),
    ("T07-011", "T07_011.Semigroup"),
    ("T07-011", "T07_011.Semigroup.mk"),
    ("T07-011", "T07_011.Semigroup.mul_assoc"),
    ("T07-011", "T07_011.Semigroup.toMul"),
    ("T07-011", "T07_011.assoc4"),
    ("T07-011", "T07_011.endoMul"),
    ("T07-011", "T07_011.instActionNat"),
    ("T07-011", "T07_011.instActionNatForall"),
    ("T07-011", "T07_011.instMonoidNat"),
    ("T07-011", "T07_011.instMulForallNat"),
    ("T07-011", "T07_011.instMulNat"),
    ("T07-011", "T07_011.instOneNat"),
    ("T07-011", "T07_011.instSemigroupNat"),
    ("T07-011", "T07_011.mulType"),
    ("T07-012", "T07_012.BHom"),
    ("T07-012", "T07_012.BHom.map_one"),
    ("T07-012", "T07_012.BHom.mk"),
    ("T07-012", "T07_012.BHom.toFun"),
    ("T07-012", "T07_012.BMonoid"),
    ("T07-012", "T07_012.BMonoid.carrier"),
    ("T07-012", "T07_012.BMonoid.mk"),
    ("T07-012", "T07_012.BMonoid.mul"),
    ("T07-012", "T07_012.BMonoid.one"),
    ("T07-012", "T07_012.BMonoid.one_mul"),
    ("T07-012", "T07_012.UMonoid"),
    ("T07-012", "T07_012.UMonoid.mk"),
    ("T07-012", "T07_012.UMonoid.mul"),
    ("T07-012", "T07_012.UMonoid.one"),
    ("T07-012", "T07_012.UMonoid.one_mul"),
    ("T07-012", "T07_012.bcombine"),
    ("T07-012", "T07_012.idHom"),
    ("T07-012", "T07_012.instUMonoidNat"),
    ("T07-012", "T07_012.natBMonoid"),
    ("T07-012", "T07_012.toBundled"),
    ("T07-012", "T07_012.toUnbundled"),
    ("T07-012", "T07_012.ucombine"),
    ("T07-013", "T07_013.Base"),
    ("T07-013", "T07_013.Base.mk"),
    ("T07-013", "T07_013.Base.stamp"),
    ("T07-013", "T07_013.Both"),
    ("T07-013", "T07_013.Both.joinMark"),
    ("T07-013", "T07_013.Both.mk"),
    ("T07-013", "T07_013.Both.rightMark"),
    ("T07-013", "T07_013.Both.toLeft"),
    ("T07-013", "T07_013.Both.toRight"),
    ("T07-013", "T07_013.Left"),
    ("T07-013", "T07_013.Left.leftMark"),
    ("T07-013", "T07_013.Left.mk"),
    ("T07-013", "T07_013.Left.toBase"),
    ("T07-013", "T07_013.Right"),
    ("T07-013", "T07_013.Right.mk"),
    ("T07-013", "T07_013.Right.rightMark"),
    ("T07-013", "T07_013.Right.toBase"),
    ("T07-013", "T07_013.bothNat"),
    ("T07-013", "T07_013.bothPaths"),
    ("T07-013", "T07_013.implicitPath"),
    ("T07-013", "T07_013.leftNat"),
    ("T07-013", "T07_013.readStamp"),
    ("T07-013", "T07_013.rightNat"),
    ("T07-013", "T07_013.viaBoth"),
    ("T07-013", "T07_013.viaLeft"),
    ("T07-013", "T07_013.viaRight"),
    ("T07-014", "T07_014.Binop"),
    ("T07-014", "T07_014.Binop.mk"),
    ("T07-014", "T07_014.Binop.op"),
    ("T07-014", "T07_014.Convert"),
    ("T07-014", "T07_014.Convert.convert"),
    ("T07-014", "T07_014.Convert.mk"),
    ("T07-014", "T07_014.Pointed"),
    ("T07-014", "T07_014.Pointed.mk"),
    ("T07-014", "T07_014.Pointed.point"),
    ("T07-014", "T07_014.Pointed_point"),
    ("T07-014", "T07_014.Show0"),
    ("T07-014", "T07_014.Show0.mk"),
    ("T07-014", "T07_014.Show0.render"),
    ("T07-014", "T07_014.Wrap"),
    ("T07-014", "T07_014.Wrap.mk"),
    ("T07-014", "T07_014.Wrap.wrap"),
    ("T07-014", "T07_014.computesOnLiteral"),
    ("T07-014", "T07_014.computesThroughInstance"),
    ("T07-014", "T07_014.dotOnDictionary"),
    ("T07-014", "T07_014.etaRoundTrip"),
    ("T07-014", "T07_014.instBinopNat"),
    ("T07-014", "T07_014.instConvertNatBool"),
    ("T07-014", "T07_014.instPointedNat"),
    ("T07-014", "T07_014.instShow0Bool"),
    ("T07-014", "T07_014.instShow0Nat"),
    ("T07-014", "T07_014.instWrapNat"),
    ("T07-014", "T07_014.nestedPremise"),
    ("T07-014", "T07_014.twoParams"),
    ("T07-014", "T07_014.useBinop"),
    ("T07-014", "T07_014.usePoint"),
    ("T08-001", "T08_001.anonymousIntro"),
    ("T08-001", "T08_001.captureHazard"),
    ("T08-001", "T08_001.forallIntro"),
    ("T08-001", "T08_001.impIntro"),
    ("T08-001", "T08_001.introOnNot"),
    ("T08-001", "T08_001.mixedIntro"),
    ("T08-001", "T08_001.multiIntro"),
    ("T08-002", "T08_002.exactAnonCtor"),
    ("T08-002", "T08_002.exactApplied"),
    ("T08-002", "T08_002.exactBareHyp"),
    ("T08-002", "T08_002.exactDefeq"),
    ("T08-002", "T08_002.exactGlobal"),
    ("T08-002", "T08_002.exactIota"),
    ("T08-002", "T08_002.exactLambda"),
    ("T08-002", "T08_002.viaAssumption"),
    ("T08-003", "T08_003.applyChain"),
    ("T08-003", "T08_003.applyConclusionIsGoal"),
    ("T08-003", "T08_003.applyGlobal"),
    ("T08-003", "T08_003.applyImplicit"),
    ("T08-003", "T08_003.applyOnePremise"),
    ("T08-003", "T08_003.applyTwoPremises"),
    ("T08-004", "T08_004.commBoundary"),
    ("T08-004", "T08_004.conjunctViaCases"),
    ("T08-004", "T08_004.defeqBoundary"),
    ("T08-004", "T08_004.firstMatch"),
    ("T08-004", "T08_004.plain"),
    ("T08-004", "T08_004.trivialOnTrue"),
    ("T08-005", "T08_005.Wrap"),
    ("T08-005", "T08_005.Wrap.mk"),
    ("T08-005", "T08_005.Wrap.val"),
    ("T08-005", "T08_005.asTerm"),
    ("T08-005", "T08_005.beta"),
    ("T08-005", "T08_005.delta"),
    ("T08-005", "T08_005.double"),
    ("T08-005", "T08_005.iota"),
    ("T08-005", "T08_005.leRefl"),
    ("T08-005", "T08_005.structEta"),
    ("T08-005", "T08_005.syntactic"),
    ("T08-006", "T08_006.Both"),
    ("T08-006", "T08_006.Both.l"),
    ("T08-006", "T08_006.Both.mk"),
    ("T08-006", "T08_006.Both.r"),
    ("T08-006", "T08_006.andIntro"),
    ("T08-006", "T08_006.anonCtor"),
    ("T08-006", "T08_006.existsIntro"),
    ("T08-006", "T08_006.iffIntro"),
    ("T08-006", "T08_006.orIntro"),
    ("T08-006", "T08_006.structIntro"),
    ("T08-006", "T08_006.symmetricAnd"),
    ("T08-007", "T08_007.Choice"),
    ("T08-007", "T08_007.Choice.first"),
    ("T08-007", "T08_007.Choice.second"),
    ("T08-007", "T08_007.asTerm"),
    ("T08-007", "T08_007.choiceLeft"),
    ("T08-007", "T08_007.leftIntro"),
    ("T08-007", "T08_007.nestedOr"),
    ("T08-007", "T08_007.rightIntro"),
    ("T08-007", "T08_007.sumLeft"),
    ("T08-007", "T08_007.symmetricOr"),
    ("T08-008", "T08_008.andElim"),
    ("T08-008", "T08_008.andElimWith"),
    ("T08-008", "T08_008.clearingDiffers"),
    ("T08-008", "T08_008.describe"),
    ("T08-008", "T08_008.existsAfterInduction"),
    ("T08-008", "T08_008.existsElim"),
    ("T08-008", "T08_008.nestedAnd"),
    ("T08-008", "T08_008.orElim"),
    ("T08-009", "T08_009.contextVariable"),
    ("T08-009", "T08_009.dependentHypothesis"),
    ("T08-009", "T08_009.ihOrdering"),
    ("T08-009", "T08_009.namedCases"),
    ("T08-009", "T08_009.outermost"),
    ("T08-010", "T08_010.LList"),
    ("T08-010", "T08_010.LList.cons"),
    ("T08-010", "T08_010.LList.nil"),
    ("T08-010", "T08_010.Tree"),
    ("T08-010", "T08_010.Tree.leaf"),
    ("T08-010", "T08_010.Tree.node"),
    ("T08-010", "T08_010.Tree.tip"),
    ("T08-010", "T08_010.app"),
    ("T08-010", "T08_010.insensitive"),
    ("T08-010", "T08_010.len"),
    ("T08-010", "T08_010.size"),
    ("T08-010", "T08_010.sizeNonneg"),
    ("T08-010", "T08_010.strongOnNat"),
    ("T08-011", "T08_011.atHypothesis"),
    ("T08-011", "T08_011.binderCollision"),
    ("T08-011", "T08_011.chain"),
    ("T08-011", "T08_011.groundBoth"),
    ("T08-011", "T08_011.groundResidual"),
    ("T08-011", "T08_011.quantified"),
    ("T08-011", "T08_011.reverse"),
    ("T08-011", "T08_011.substDiffers"),
    ("T08-012", "T08_012.bareSimp"),
    ("T08-012", "T08_012.localSimpLemma"),
    ("T08-012", "T08_012.residueMatters"),
    ("T08-012", "T08_012.simpAll"),
    ("T08-012", "T08_012.simpAt"),
    ("T08-012", "T08_012.simpOnly"),
    ("T08-012", "T08_012.simpWithHyp"),
    ("T08-012", "T08_012.stagedNormalisation"),
    ("T08-012", "T08_012.usesLocalSimpLemma"),
    ("T08-014", "T08_014.Colour"),
    ("T08-014", "T08_014.Colour.blue"),
    ("T08-014", "T08_014.Colour.green"),
    ("T08-014", "T08_014.Colour.ofNat_toCtorIdx"),
    ("T08-014", "T08_014.Colour.red"),
    ("T08-014", "T08_014.bigNumeral"),
    ("T08-014", "T08_014.boundedForall"),
    ("T08-014", "T08_014.enumEq"),
    ("T08-014", "T08_014.groundArith"),
    ("T08-014", "T08_014.groundBool"),
    ("T08-014", "T08_014.groundComparison"),
    ("T08-014", "T08_014.instDecidableEqColour"),
    ("T08-014", "T08_014.listDecidable"),
    ("T08-014", "T08_014.propStructure"),
    ("T08-015", "T08_015.CommRingLike"),
    ("T08-015", "T08_015.CommRingLike.add"),
    ("T08-015", "T08_015.CommRingLike.add_comm"),
    ("T08-015", "T08_015.CommRingLike.mk"),
    ("T08-015", "T08_015.CommRingLike.mul"),
    ("T08-015", "T08_015.abstractCarrier"),
    ("T08-015", "T08_015.distrib"),
    ("T08-015", "T08_015.needsNormalOnly"),
    ("T08-016", "T08_016.OrderedFieldLike"),
    ("T08-016", "T08_016.OrderedFieldLike.add"),
    ("T08-016", "T08_016.OrderedFieldLike.half"),
    ("T08-016", "T08_016.OrderedFieldLike.half_between"),
    ("T08-016", "T08_016.OrderedFieldLike.le"),
    ("T08-016", "T08_016.OrderedFieldLike.le_trans"),
    ("T08-016", "T08_016.OrderedFieldLike.mk"),
    ("T08-016", "T08_016.OrderedFieldLike.one"),
    ("T08-016", "T08_016.fieldTrans"),
    ("T08-016", "T08_016.nonlinear"),
    ("T08-016", "T08_016.ratFieldSatisfiable"),
    ("T08-017", "T08_017.appliedWitness"),
    ("T08-017", "T08_017.asTerm"),
    ("T08-017", "T08_017.closedWitness"),
    ("T08-017", "T08_017.functionWitness"),
    ("T08-017", "T08_017.localVariableWitness"),
    ("T08-017", "T08_017.nestedExists"),
    ("T08-017", "T08_017.useCloses"),
    ("T08-017", "T08_017.witnessRoundTrip"),
    ("T08-018", "T08_018.afterCases"),
    ("T08-018", "T08_018.bothByAssumption"),
    ("T08-018", "T08_018.chained"),
    ("T08-018", "T08_018.cleanChain"),
    ("T08-018", "T08_018.leakAfterSplit"),
    ("T08-018", "T08_018.partialFailureShape"),
    ("T08-018", "T08_018.precedence"),
    ("T08-019", "T08_019.firstAlternative"),
    ("T08-019", "T08_019.firstBacktracks"),
    ("T08-019", "T08_019.nested"),
    ("T08-019", "T08_019.repeatIntros"),
    ("T08-019", "T08_019.repeatNoOp"),
    ("T08-019", "T08_019.tryApplies"),
    ("T08-019", "T08_019.tryMultiStep"),
    ("T08-019", "T08_019.tryNoOp"),
    ("T08-020", "T08_020.allGoalsAfterConstructor"),
    ("T08-020", "T08_020.allGoalsNonClosing"),
    ("T08-020", "T08_020.allGoalsTwoStepsBack"),
    ("T08-020", "T08_020.anyGoals"),
    ("T08-020", "T08_020.focused"),
    ("T08-020", "T08_020.iterated"),
    ("T08-021", "T08_021.chainAsTerm"),
    ("T08-021", "T08_021.chainEq"),
    ("T08-021", "T08_021.chainMixed"),
    ("T08-021", "T08_021.chainRepeatedWaypoint"),
    ("T08-021", "T08_021.chainWithQuantifiedStep"),
    ("T08-021", "T08_021.oneStep"),
    ("T08-022", "T08_022.contraDirect"),
    ("T08-022", "T08_022.contraUnderIntro"),
    ("T08-022", "T08_022.dneCore"),
    ("T08-022", "T08_022.em"),
    ("T08-022", "T08_022.negativeGoal"),
    ("T08-022", "T08_022.pushNegAnd"),
    ("T08-022", "T08_022.pushNegForall"),
    ("T08-023", "T08_023.convAtHyp"),
    ("T08-023", "T08_023.enterArgument"),
    ("T08-023", "T08_023.lhsOnly"),
    ("T08-023", "T08_023.nthOccurrence"),
    ("T08-023", "T08_023.singleOccurrence"),
    ("T08-023", "T08_023.underBinder"),
    ("T08-024", "T08_024.aesopMixed"),
    ("T08-024", "T08_024.aesopViaSimp"),
    ("T08-024", "T08_024.classicalTauto"),
    ("T08-024", "T08_024.hardForAuto"),
    ("T08-024", "T08_024.irrelevantHypothesis"),
    ("T08-024", "T08_024.orTauto"),
    ("T08-024", "T08_024.propTauto"),
    ("T08-025", "T08_025.bulletEnforcesClosure"),
    ("T08-025", "T08_025.bulletsHappyPath"),
    ("T08-025", "T08_025.bulletsMultiStep"),
    ("T08-025", "T08_025.caseFromCases"),
    ("T08-025", "T08_025.caseFromInduction"),
    ("T08-025", "T08_025.caseTagsReordered"),
    ("T08-025", "T08_025.nextBlocks"),
    ("T08-026", "T08_026.closedHave"),
    ("T08-026", "T08_026.haveByTerm"),
    ("T08-026", "T08_026.haveMentionsLocal"),
    ("T08-026", "T08_026.showDefeq"),
    ("T08-026", "T08_026.showIdentical"),
    ("T08-026", "T08_026.showUnfolds"),
    ("T08-026", "T08_026.sufficesStep"),
    ("T08-026", "T08_026.tacticLet"),
    ("T08-026", "T08_026.twice"),
    ("T08-027", "T08_027.refineAnd"),
    ("T08-027", "T08_027.refineArgument"),
    ("T08-027", "T08_027.refineNamed"),
    ("T08-027", "T08_027.refineNested"),
    ("T08-027", "T08_027.refineOutOfOrder"),
    ("T08-027", "T08_027.refineUnderBinder"),
    ("T08-027", "T08_027.refineWhole"),
    ("T09-001", "T09_001.bracketing"),
    ("T09-001", "T09_001.compoundArgument"),
    ("T09-001", "T09_001.divides"),
    ("T09-001", "T09_001.inTacticProof"),
    ("T09-001", "T09_001.myOp"),
    ("T09-001", "T09_001.myOther"),
    ("T09-001", "T09_001.notationIsTransparent"),
    ("T09-001", "T09_001.precedenceMatters"),
    ("T09-001", "T09_001.propNotation"),
    ("T09-001", "T09_001.wrap"),
    ("T09-001", "T09_001.writtenOut"),
    ("T09-002", "T09_002.mixedPrecedence"),
    ("T09-002", "T09_002.neg"),
    ("T09-002", "T09_002.postfixExpands"),
    ("T09-002", "T09_002.prefixExpands"),
    ("T09-003", "T09_003.MyList"),
    ("T09-003", "T09_003.MyList.cons"),
    ("T09-003", "T09_003.MyList.nil"),
    ("T09-003", "T09_003.compound"),
    ("T09-003", "T09_003.expandsEmpty"),
    ("T09-003", "T09_003.expandsFully"),
    ("T09-003", "T09_003.hygienic"),
    ("T09-003", "T09_003.macroTactic"),
    ("T09-003", "T09_003.overlappingRules"),
    ("T09-004", "T09_004.mixed"),
    ("T09-004", "T09_004.mixedTranslates"),
    ("T09-004", "T09_004.usesBranchingElab"),
    ("T09-004", "T09_004.usesBranchingElabOdd"),
    ("T09-004", "T09_004.usesCustomElab"),
    ("T09-004", "T09_004.usesCustomTactic"),
    ("T09-004", "T09_004.usesEnvReadingElab"),
    ("T09-005", "T09_005.countArgs"),
    ("T09-005", "T09_005.expansionTranslates"),
    ("T09-005", "T09_005.tacticExpansionTranslates"),
    ("T09-006", "T09_006.Chain"),
    ("T09-006", "T09_006.Chain.link"),
    ("T09-006", "T09_006.Chain.stop"),
    ("T09-006", "T09_006.Colour"),
    ("T09-006", "T09_006.Colour.blue"),
    ("T09-006", "T09_006.Colour.green"),
    ("T09-006", "T09_006.Colour.ofNat_toCtorIdx"),
    ("T09-006", "T09_006.Colour.red"),
    ("T09-006", "T09_006.Flag"),
    ("T09-006", "T09_006.Flag.down"),
    ("T09-006", "T09_006.Flag.up"),
    ("T09-006", "T09_006.Flag_tag"),
    ("T09-006", "T09_006.MyTag"),
    ("T09-006", "T09_006.MyTag.mk"),
    ("T09-006", "T09_006.MyTag.tag"),
    ("T09-006", "T09_006.Pair"),
    ("T09-006", "T09_006.Pair.a"),
    ("T09-006", "T09_006.Pair.b"),
    ("T09-006", "T09_006.Pair.mk"),
    ("T09-006", "T09_006.Pair_tag"),
    ("T09-006", "T09_006.derivedBEq"),
    ("T09-006", "T09_006.derivedDecEq"),
    ("T09-006", "T09_006.derivedInhabited"),
    ("T09-006", "T09_006.derivedRecursiveBEq"),
    ("T09-006", "T09_006.generatedDefTranslates"),
    ("T09-006", "T09_006.generatedInstanceTranslates"),
    ("T09-006", "T09_006.instBEqChain"),
    ("T09-006", "T09_006.instBEqColour"),
    ("T09-006", "T09_006.instDecidableEqChain"),
    ("T09-006", "T09_006.instDecidableEqColour"),
    ("T09-006", "T09_006.instInhabitedColour"),
    ("T09-006", "T09_006.instMyTagFlag"),
    ("T09-006", "T09_006.instMyTagPair"),
    ("T09-006", "T09_006.pairGenerated"),
    ("T09-007", "T09_007.counterRef"),
    ("T09-007", "T09_007.dependsOnAttribute"),
    ("T09-007", "T09_007.dependsOnLaterAttribute"),
    ("T09-007", "T09_007.laterTagged"),
    ("T09-007", "T09_007.plainDef"),
    ("T09-007", "T09_007.plainDefTranslates"),
    ("T09-007", "T09_007.tagged"),
    ("T09-008", "T09_008.cosmetic"),
    ("T09-008", "T09_008.idAuto"),
    ("T09-008", "T09_008.idAutoWorks"),
    ("T09-008", "T09_008.idExplicit"),
    ("T09-008", "T09_008.idNoAuto"),
    ("T09-008", "T09_008.idNoAutoWorks"),
    ("T09-008", "T09_008.inTactic"),
    ("T09-008", "T09_008.linterSilenced"),
    ("T09-008", "T09_008.resourceBound"),
    ("T09-008", "T09_008.sameArity"),
    ("T09-009", "T09_009.realDef"),
    ("T09-009", "T09_009.realTheorem"),
    ("T09-010", "T09_010.Documentable"),
    ("T09-010", "T09_010.Documentable.mk"),
    ("T09-010", "T09_010.Documentable.tag"),
    ("T09-010", "T09_010.Documented"),
    ("T09-010", "T09_010.Documented.fst"),
    ("T09-010", "T09_010.Documented.mk"),
    ("T09-010", "T09_010.Documented.snd"),
    ("T09-010", "T09_010.DocumentedEnum"),
    ("T09-010", "T09_010.DocumentedEnum.no"),
    ("T09-010", "T09_010.DocumentedEnum.yes"),
    ("T09-010", "T09_010.afterModuleDoc"),
    ("T09-010", "T09_010.documented"),
    ("T09-010", "T09_010.documentedTheorem"),
    ("T09-010", "T09_010.inheritsDoc"),
    ("T09-010", "T09_010.markupHeavy"),
    ("T09-010", "T09_010.original"),
    ("T09-011", "T09_011.Inner.value"),
    ("T09-011", "T09_011.collisionHazard"),
    ("T09-011", "T09_011.dotted"),
    ("T09-011", "T09_011.idAlpha"),
    ("T09-011", "T09_011.idGamma"),
    ("T09-011", "T09_011.pairAlphaGamma"),
    ("T09-011", "T09_011.primed"),
    ("T09-011", "T09_011.subscripted"),
    ("T09-011", "T09_011.usesGuillemet"),
    ("T09-011", "T09_011.weird?!name"),
    ("T09-011", "T09_011.x₁"),
    ("T09-011", "T09_011.x₂"),
    ("T09-011", "T09_011.ℕplus"),
    ("T09-012", "T09_012.generatedByRunCmd"),
    ("T09-012", "T09_012.generatedTranslates"),
    ("T09-012", "T09_012.optionDo"),
    ("T09-012", "T09_012.optionDoWorks"),
    ("T09-012", "T09_012.pureDecl"),
    ("T09-012", "T09_012.pureDeclTranslates"),
    ("T09-013", "T09_013.Inner.insideNamespace"),
    ("T09-013", "T09_013.alt"),
    ("T09-013", "T09_013.firstMeaning"),
    ("T09-013", "T09_013.insideSection"),
    ("T09-013", "T09_013.interleaved"),
    ("T09-013", "T09_013.laterWins"),
    ("T09-013", "T09_013.op"),
    ("T09-013", "T09_013.redefinedMeaning"),
    ("T09-013", "T09_013.scopedUse"),
    ("T09-013", "T09_013.usesVariable"),
    ("T09-013", "T09_013.variableInserted"),
    ("T09-014", "T09_014.MyMem"),
    ("T09-014", "T09_014.boundedExistsCustom"),
    ("T09-014", "T09_014.boundedExistsDesugared"),
    ("T09-014", "T09_014.boundedForallCustom"),
    ("T09-014", "T09_014.boundedForallDesugared"),
    ("T09-014", "T09_014.boundedMembership"),
    ("T09-014", "T09_014.inRange"),
    ("T09-014", "T09_014.pos"),
    ("T09-014", "T09_014.rangeBoundedDesugared"),
    ("T10-001", "T10_001.fiveLit"),
    ("T10-001", "T10_001.isZero"),
    ("T10-001", "T10_001.largeButMachine"),
    ("T10-001", "T10_001.zeroLit"),
    ("T10-002", "T10_002.justBelowI64"),
    ("T10-002", "T10_002.justPastI64"),
    ("T10-002", "T10_002.justPastU64"),
    ("T10-002", "T10_002.wideDecimal"),
    ("T10-003", "T10_003.belowI64Min"),
    ("T10-003", "T10_003.i64Min"),
    ("T10-003", "T10_003.minusFive"),
    ("T10-003", "T10_003.signOf"),
    ("T10-004", "T10_004.backToNat"),
    ("T10-004", "T10_004.double"),
    ("T10-004", "T10_004.toInt"),
    ("T10-005", "T10_005.echo"),
    ("T10-006", "T10_006.countChar"),
    ("T10-007", "T10_007.identity"),
    ("T10-008", "T10_008.myAnd"),
    ("T10-008", "T10_008.swapIsWellTyped"),
    ("T10-008", "T10_008.toNat"),
    ("T10-008", "T10_008.toNat'"),
    ("T10-011", "T10_011.ofTypes"),
    ("T10-011", "T10_011.small"),
    ("T10-011", "T10_011.sumAll"),
    ("T10-012", "T10_012.append'"),
    ("T10-012", "T10_012.length'"),
    ("T10-012", "T10_012.listOfProps"),
    ("T10-012", "T10_012.listOfTypes"),
    ("T10-012", "T10_012.nested"),
    ("T10-012", "T10_012.nestedAtTypeOne"),
    ("T10-012", "T10_012.small"),
    ("T10-012", "T10_012.sumViaRec"),
    ("T10-014", "T10_014.Mod7"),
    ("T10-014", "T10_014.Mod7.mk"),
    ("T10-014", "T10_014.Mod7.rep"),
    ("T10-014", "T10_014.asInt"),
    ("T10-014", "T10_014.asNat"),
    ("T10-014", "T10_014.bare"),
    ("T10-014", "T10_014.instDecidableEqMod7"),
    ("T10-014", "T10_014.instOfNatMod7"),
    ("T10-014", "T10_014.negInt"),
    ("T10-014", "T10_014.twiceLit"),
    ("T10-015", "T10_015.addThreeViaRec"),
    ("T10-015", "T10_015.bigLit"),
    ("T10-015", "T10_015.minusThree"),
    ("T10-015", "T10_015.pred"),
    ("T10-017", "T10_017.natSub"),
    ("T10-017", "T10_017.natSub_eq"),
    ("T10-018", "T10_018.ediv'"),
    ("T10-018", "T10_018.emod'"),
    ("T11-001", "T11_001.Holder"),
    ("T11-001", "T11_001.Holder.fld"),
    ("T11-001", "T11_001.Holder.mk"),
    ("T11-001", "T11_001.applyToType"),
    ("T11-001", "T11_001.holdType"),
    ("T11-001", "T11_001.idT"),
    ("T11-001", "T11_001.listOfProps"),
    ("T11-001", "T11_001.natAtItsOwnLevel"),
    ("T11-001", "T11_001.propAtProp"),
    ("T11-001", "T11_001.sameLevel"),
    ("T11-002", "T11_002.constSort"),
    ("T11-002", "T11_002.idSort"),
    ("T11-002", "T11_002.propAsLowerBound"),
    ("T11-002", "T11_002.propFamily"),
    ("T11-002", "T11_002.propInTypeZero"),
    ("T11-003", "T11_003.Alpha.size"),
    ("T11-003", "T11_003.Baz.Colour"),
    ("T11-003", "T11_003.Baz.Colour.green"),
    ("T11-003", "T11_003.Baz.Colour.ofNat_toCtorIdx"),
    ("T11-003", "T11_003.Baz.Colour.red"),
    ("T11-003", "T11_003.Baz.instDecidableEqColour"),
    ("T11-003", "T11_003.Baz_Colour_red"),
    ("T11-003", "T11_003.Beta.size"),
    ("T11-003", "T11_003.Foo.bar"),
    ("T11-003", "T11_003.Foo_bar"),
    ("T11-003", "T11_003.Foo_bar.mk"),
    ("T11-003", "T11_003.Gamma.dup"),
    ("T11-003", "T11_003.Gamma_dup"),
    ("T11-003", "T11_003.add"),
    ("T11-003", "T11_003.le"),
    ("T11-004", "T11_004.boolSide"),
    ("T11-004", "T11_004.compose"),
    ("T11-004", "T11_004.explicitUnderscore"),
    ("T11-004", "T11_004.idImp"),
    ("T11-004", "T11_004.idPoly"),
    ("T11-004", "T11_004.natSide"),
    ("T11-004", "T11_004.replicate"),
    ("T11-004", "T11_004.useDecEq"),
    ("T11-005", "T11_005.countDown"),
    ("T11-005", "T11_005.f"),
    ("T11-005", "T11_005.g"),
    ("T11-005", "T11_005.s0"),
    ("T11-005", "T11_005.s1"),
    ("T11-005", "T11_005.s2"),
    ("T11-005", "T11_005.s3"),
    ("T11-005", "T11_005.s4"),
    ("T11-005", "T11_005.tower"),
    ("T11-006", "T11_006.captureViaSubst"),
    ("T11-006", "T11_006.dependentShadow"),
    ("T11-006", "T11_006.hygienic"),
    ("T11-006", "T11_006.innerWins"),
    ("T11-006", "T11_006.letShadow"),
    ("T11-006", "T11_006.outerWins"),
    ("T11-006", "T11_006.shadowsAGlobal"),
    ("T11-006", "T11_006.tripleShadow"),
    ("T11-007", "T11_007.Bundled"),
    ("T11-007", "T11_007.Bundled.assoc"),
    ("T11-007", "T11_007.Bundled.carrier"),
    ("T11-007", "T11_007.Bundled.leftId"),
    ("T11-007", "T11_007.Bundled.mk"),
    ("T11-007", "T11_007.Bundled.op"),
    ("T11-007", "T11_007.Bundled.rightId"),
    ("T11-007", "T11_007.Bundled.zero"),
    ("T11-007", "T11_007.deepDependent"),
    ("T11-007", "T11_007.forcesFreshen"),
    ("T11-007", "T11_007.natBundled"),
    ("T11-007", "T11_007.shared"),
    ("T11-007", "T11_007.sharedUnderBinder"),
    ("T11-007", "T11_007.spine10"),
    ("T11-007", "T11_007.usesAll"),
    ("T11-007", "T11_007.usesSharedManyTimes"),
    ("T11-008", "T11_008.down"),
    ("T11-008", "T11_008.isZero"),
    ("T11-009", "T11_009.Bounded"),
    ("T11-009", "T11_009.Bounded.mk"),
    ("T11-009", "T11_009.Bounded.small"),
    ("T11-009", "T11_009.Bounded.val"),
    ("T11-009", "T11_009.b1"),
    ("T11-009", "T11_009.b2"),
    ("T11-009", "T11_009.bigProp"),
    ("T11-009", "T11_009.bp1"),
    ("T11-009", "T11_009.bp2"),
    ("T11-009", "T11_009.eq1"),
    ("T11-009", "T11_009.eq2"),
    ("T11-009", "T11_009.f1"),
    ("T11-009", "T11_009.f2"),
    ("T11-009", "T11_009.pos1"),
    ("T11-009", "T11_009.pos2"),
    ("T11-009", "T11_009.proofA"),
    ("T11-009", "T11_009.proofB"),
    ("T11-010", "T11_010.Tagged"),
    ("T11-010", "T11_010.Tagged.no"),
    ("T11-010", "T11_010.Tagged.yes"),
    ("T11-010", "T11_010.Vect"),
    ("T11-010", "T11_010.Vect.cons"),
    ("T11-010", "T11_010.Vect.nil"),
    ("T11-010", "T11_010.absurdVect"),
    ("T11-010", "T11_010.fromEmptyFin"),
    ("T11-010", "T11_010.isEmpty"),
    ("T11-010", "T11_010.onlyNo"),
    ("T11-010", "T11_010.onlyYes"),
    ("T11-010", "T11_010.transport"),
    ("T11-011", "T11_011.EvenT"),
    ("T11-011", "T11_011.EvenT.below.succ"),
    ("T11-011", "T11_011.EvenT.below.zero"),
    ("T11-011", "T11_011.EvenT.succ"),
    ("T11-011", "T11_011.EvenT.zero"),
    ("T11-011", "T11_011.Expr'"),
    ("T11-011", "T11_011.Expr'.app"),
    ("T11-011", "T11_011.Expr'.lit"),
    ("T11-011", "T11_011.Forest"),
    ("T11-011", "T11_011.Forest.cons"),
    ("T11-011", "T11_011.Forest.nil"),
    ("T11-011", "T11_011.OddT"),
    ("T11-011", "T11_011.OddT.below.succ"),
    ("T11-011", "T11_011.OddT.succ"),
    ("T11-011", "T11_011.Stmt'"),
    ("T11-011", "T11_011.Stmt'.seq"),
    ("T11-011", "T11_011.Stmt'.skip"),
    ("T11-011", "T11_011.Tree"),
    ("T11-011", "T11_011.Tree.node"),
    ("T11-012", "T11_012.ATree"),
    ("T11-012", "T11_012.ATree.below_1"),
    ("T11-012", "T11_012.ATree.below_2"),
    ("T11-012", "T11_012.ATree.ibelow_1"),
    ("T11-012", "T11_012.ATree.ibelow_2"),
    ("T11-012", "T11_012.ATree.node"),
    ("T11-012", "T11_012.ATree.rec_1"),
    ("T11-012", "T11_012.ATree.rec_2"),
    ("T11-012", "T11_012.LTree"),
    ("T11-012", "T11_012.LTree.below_1"),
    ("T11-012", "T11_012.LTree.ibelow_1"),
    ("T11-012", "T11_012.LTree.node"),
    ("T11-012", "T11_012.LTree.rec_1"),
    ("T11-012", "T11_012.Labelled"),
    ("T11-012", "T11_012.Labelled.cons"),
    ("T11-012", "T11_012.Labelled.nil"),
    ("T11-012", "T11_012.PairTree"),
    ("T11-012", "T11_012.PairTree.below_1"),
    ("T11-012", "T11_012.PairTree.fork"),
    ("T11-012", "T11_012.PairTree.ibelow_1"),
    ("T11-012", "T11_012.PairTree.leaf"),
    ("T11-012", "T11_012.PairTree.rec_1"),
    ("T11-012", "T11_012.RoseList"),
    ("T11-012", "T11_012.RoseList.below_1"),
    ("T11-012", "T11_012.RoseList.ibelow_1"),
    ("T11-012", "T11_012.RoseList.node"),
    ("T11-012", "T11_012.RoseList.rec_1"),
    ("T11-012", "T11_012.Vect"),
    ("T11-012", "T11_012.Vect.cons"),
    ("T11-012", "T11_012.Vect.nil"),
    ("T11-012", "T11_012.boolRose"),
    ("T11-012", "T11_012.natRose"),
    ("T11-013", "T11_013.A'"),
    ("T11-013", "T11_013.A'.mk"),
    ("T11-013", "T11_013.AnyProp"),
    ("T11-013", "T11_013.AnyProp.mk"),
    ("T11-013", "T11_013.B'"),
    ("T11-013", "T11_013.B'.mk"),
    ("T11-013", "T11_013.Good"),
    ("T11-013", "T11_013.Good.mk"),
    ("T11-013", "T11_013.MyList"),
    ("T11-013", "T11_013.MyList.cons"),
    ("T11-013", "T11_013.MyList.nil"),
    ("T11-013", "T11_013.MyOption"),
    ("T11-013", "T11_013.MyOption.none"),
    ("T11-013", "T11_013.MyOption.some"),
    ("T11-013", "T11_013.MyProd"),
    ("T11-013", "T11_013.MyProd.fst"),
    ("T11-013", "T11_013.MyProd.mk"),
    ("T11-013", "T11_013.MyProd.snd"),
    ("T11-013", "T11_013.Univ"),
    ("T11-013", "T11_013.Univ.mk"),
    ("T11-013", "T11_013.myLength"),
    ("T11-013", "T11_013.unfold"),
    ("T11-014", "T11_014.Trivial'"),
    ("T11-014", "T11_014.Trivial'.mk"),
    ("T11-014", "T11_014.Trivial'.witness"),
    ("T11-014", "T11_014.actuallyProved"),
    ("T11-014", "T11_014.deliberateAssumption"),
    ("T11-014", "T11_014.usesInstance"),
    ("T11-015", "T11_015.heightBound"),
    ("T11-015", "T11_015.indirect"),
    ("T11-015", "T11_015.pureComputation"),
    ("T11-015", "T11_015.usesAssumption"),
    ("T11-015", "T11_015.usesClassical"),
    ("T11-015", "T11_015.usesPropext"),
    ("T11-016", "T11_016.nonNegOfNat"),
    ("T11-016", "T11_016.sign"),
    ("T11-017", "T11_017.allBelow"),
    ("T11-017", "T11_017.allBelowSmall"),
    ("T11-017", "T11_017.pure"),
    ("T11-017", "T11_017.smallArithmetic"),
    ("T11-018", "T11_018.NatList"),
    ("T11-018", "T11_018.NatList.hd"),
    ("T11-018", "T11_018.NatList.mk"),
    ("T11-018", "T11_018.NatList.tl"),
    ("T11-018", "T11_018.countDown"),
    ("T11-018", "T11_018.everyOther"),
    ("T11-018", "T11_018.shrink"),
    ("T11-018", "T11_018.sumAcc"),
    ("T11-018", "T11_018.sumFuel"),
    ("T11-018", "T11_018.sumTail"),
    ("T11-018", "T11_018.sumViaProjection"),
    ("T11-018", "T11_018.underLet"),
    ("T11-019", "T11_019.AtLevelOne"),
    ("T11-019", "T11_019.AtLevelOne.mk"),
    ("T11-019", "T11_019.BigEnough"),
    ("T11-019", "T11_019.BigEnough.mk"),
    ("T11-019", "T11_019.HoldsAProp"),
    ("T11-019", "T11_019.HoldsAProp.mk"),
    ("T11-019", "T11_019.PropHoldingAType"),
    ("T11-019", "T11_019.PropHoldingAType.mk"),
    ("T11-019", "T11_019.Sum'"),
    ("T11-019", "T11_019.Sum'.inl"),
    ("T11-019", "T11_019.Sum'.inr"),
    ("T11-019", "T11_019.Wrapper"),
    ("T11-019", "T11_019.Wrapper.mk"),
    ("T11-019", "T11_019.eliminateIntoTypeOne"),
    ("T11-019", "T11_019.tryEliminate"),
    ("T11-019", "T11_019.wrapNat"),
    ("T11-019", "T11_019.wrapType"),
    ("T11-020", "T11_020.Sized"),
    ("T11-020", "T11_020.Sized.mk"),
    ("T11-020", "T11_020.Tag"),
    ("T11-020", "T11_020.Tag.mk"),
    ("T11-020", "T11_020.Wrap"),
    ("T11-020", "T11_020.Wrap.mk"),
    ("T11-020", "T11_020.accToNat"),
    ("T11-020", "T11_020.sizedTrivial"),
    ("T11-020", "T11_020.tagIrrelevant"),
    ("T11-020", "T11_020.tagTrivial"),
    ("T11-020", "T11_020.wrapTrivial"),
    ("T11-022", "T11_022.doubleIsAdd"),
    ("T11-022", "T11_022.doubleZero"),
    ("T11-022", "T11_022.doubleZeroAgain"),
    ("T11-022", "T11_022.firstOne"),
    ("T11-022", "T11_022.secondOne"),
    ("T11-022", "T11_022.thirdOne"),
    ("T11-022", "T11_022.triple"),
    ("T11-022", "T11_022.tripleOne"),
    ("T11-022", "T11_022.tripleZero"),
    ("T11-022", "T11_022.withPremise"),
    ("T12-001", "T12_001.Entity"),
    ("T12-001", "T12_001.socratesDoom"),
    ("T12-001", "T12_001.socratesMortality"),
    ("T12-002", "T12_002.addZeroLeft"),
    ("T12-002", "T12_002.addZeroRight"),
    ("T12-002", "T12_002.leSucc"),
    ("T12-002", "T12_002.mulOneRight"),
    ("T12-003", "T12_003.andCommutative"),
    ("T12-003", "T12_003.needsTermExact"),
    ("T12-003", "T12_003.orCommutative"),
    ("T12-003", "T12_003.sadImpliesHappy"),
    ("T12-004", "T12_004.Entity"),
    ("T12-004", "T12_004.mixedNaming"),
    ("T12-004", "T12_004.orCommNamed"),
    ("T12-004", "T12_004.someMortalIsAMan"),
    ("T12-005", "T12_005.allP"),
    ("T12-005", "T12_005.lengthAppendNil"),
    ("T12-006", "T12_006.InnerTransitivity"),
    ("T12-006", "T12_006.Point"),
    ("T12-006", "T12_006.PseudoReflexivity"),
    ("T12-006", "T12_006.nullSegment"),
    ("T12-006", "T12_006.reflexivity"),
    ("T12-006", "T12_006.symmetry"),
    ("T12-006", "T12_006.transitivity"),
    ("T12-007", "T12_007.Add"),
    ("T12-007", "T12_007.AddRel"),
    ("T12-007", "T12_007.Person"),
    ("T12-007", "T12_007.addRelZeroRight"),
    ("T12-007", "T12_007.addZeroRight"),
    ("T12-007", "T12_007.addZeroTwice"),
    ("T12-007", "T12_007.lovesGrandfather"),
    ("T12-007", "T12_007.lovesOwnFather"),
    ("T12-008", "T12_008.addComm'"),
    ("T12-008", "T12_008.fiveGeThree"),
    ("T12-008", "T12_008.gtToGe"),
    ("T12-008", "T12_008.leRefl"),
    ("T12-008", "T12_008.linearCoeff"),
    ("T12-008", "T12_008.ltSucc"),
    ("T12-008", "T12_008.mixedArith"),
    ("T12-008", "T12_008.twoLtFive"),
    ("T12-009", "T12_009.allImpliesOne"),
    ("T12-009", "T12_009.alternating"),
    ("T12-009", "T12_009.dependentArrow"),
    ("T12-009", "T12_009.existsSucc"),
    ("T12-009", "T12_009.leReflQuantified"),
    ("T12-010", "T12_010.Colour"),
    ("T12-010", "T12_010.Colour.blue"),
    ("T12-010", "T12_010.Colour.green"),
    ("T12-010", "T12_010.Colour.red"),
    ("T12-010", "T12_010.appendNilRight"),
    ("T12-010", "T12_010.colourInduction"),
    ("T12-010", "T12_010.listInduction"),
    ("T12-010", "T12_010.natSumFormula"),
    ("T12-011", "T12_011.Entity"),
    ("T12-011", "T12_011.Point"),
    ("T12-011", "T12_011.Pointed"),
    ("T12-011", "T12_011.Pointed.mk"),
    ("T12-011", "T12_011.Pointed.point"),
    ("T12-011", "T12_011.Vect"),
    ("T12-011", "T12_011.Vect.cons"),
    ("T12-011", "T12_011.Vect.length"),
    ("T12-011", "T12_011.Vect.nil"),
    ("T12-011", "T12_011.double"),
    ("T12-011", "T12_011.routeA"),
    ("T12-011", "T12_011.routeB"),
    ("T12-011", "T12_011.routeC"),
    ("T12-011", "T12_011.routeK_alternating"),
    ("T12-011", "T12_011.routeK_class"),
    ("T12-011", "T12_011.routeK_default"),
    ("T12-011", "T12_011.routeK_dependent"),
    ("T12-011", "T12_011.routeK_indexed"),
    ("T12-011", "T12_011.routeK_universe"),
    ("T12-011", "T12_011.usesRouteA"),
    ("T12-012", "T12_012.byInduction"),
    ("T12-012", "T12_012.byModusPonens"),
    ("T12-012", "T12_012.byUniversalInst"),
    ("T12-012", "T12_012.liveAuto"),
    ("T12-012", "T12_012.liveScript"),
    ("T12-013", "T12_013.Box"),
    ("T12-013", "T12_013.certifiable"),
    ("T12-013", "T12_013.modalDistribution"),
    ("T12-014", "T12_014.Thing"),
    ("T12-014", "T12_014.betaIn2004"),
    ("T12-014", "T12_014.betaInMaine"),
    ("T12-014", "T12_014.socratesMortality"),
    ("T12-014", "T12_014.someoneInMaine"),
    ("T02-011", "T02_011.«odd name.with dots»"),
    ("T06-012", "T06_012.Parity"),
    ("T06-012", "T06_012.bigQuot"),
    ("T06-012", "T06_012.indProp"),
    ("T06-012", "T06_012.liftComputes"),
    ("T06-012", "T06_012.mk"),
    ("T06-012", "T06_012.parityBit"),
    ("T06-012", "T06_012.recIntoType"),
    ("T06-012", "T06_012.rel"),
    ("T09-011", "T09_011.«a name with spaces»"),
    ("T02-014", "T02_014.forestSize"),
    ("T03-008", "T03_008.Labelled.binductionOn_1"),
    ("T02-014", "T02_014.treeSize"),
    ("T03-006", "T03_006.forestSum"),
    ("T03-006", "T03_006.pcount"),
    ("T03-006", "T03_006.pfcount"),
    ("T03-006", "T03_006.sizeA"),
    ("T03-006", "T03_006.sizeB"),
    ("T03-006", "T03_006.sizeC"),
    ("T03-006", "T03_006.treeSum"),
    ("T03-007", "T03_007.chainLen"),
    ("T03-007", "T03_007.paDepth"),
    ("T03-008", "T03_008.Labelled.brecOn_1"),
    ("T03-008", "T03_008.PRTree.binductionOn_1"),
    ("T03-008", "T03_008.PRTree.brecOn_1"),
    ("T03-008", "T03_008.RTree.binductionOn_1"),
    ("T03-008", "T03_008.RTree.brecOn_1"),
    ("T03-008", "T03_008.RTree2.binductionOn_1"),
    ("T03-008", "T03_008.RTree2.binductionOn_2"),
    ("T03-008", "T03_008.RTree2.brecOn_1"),
    ("T03-008", "T03_008.RTree2.brecOn_2"),
    ("T03-008", "T03_008.llistSum"),
    ("T03-008", "T03_008.lsum"),
    ("T03-008", "T03_008.rlistSize"),
    ("T03-008", "T03_008.rsize"),
    ("T03-009", "T03_009.LTree.binductionOn_1"),
    ("T03-009", "T03_009.LTree.brecOn_1"),
    ("T03-009", "T03_009.PTree.binductionOn_1"),
    ("T03-009", "T03_009.PTree.brecOn_1"),
    ("T03-009", "T03_009.QTree.binductionOn_1"),
    ("T03-009", "T03_009.QTree.brecOn_1"),
    ("T03-009", "T03_009.lsize"),
    ("T03-009", "T03_009.ltagSize"),
    ("T03-009", "T03_009.pdepth"),
    ("T03-009", "T03_009.qdepth"),
    ("T03-015", "T03_015.pingSize"),
    ("T03-015", "T03_015.pongSize"),
    ("T04-003", "T04_003.forestSize"),
    ("T04-003", "T04_003.treeSize"),
    ("T11-011", "T11_011.forestSize"),
    ("T11-011", "T11_011.treeSize"),
    ("T11-012", "T11_012.ATree.binductionOn_1"),
    ("T11-012", "T11_012.ATree.binductionOn_2"),
    ("T11-012", "T11_012.ATree.brecOn_1"),
    ("T11-012", "T11_012.ATree.brecOn_2"),
    ("T11-012", "T11_012.BigRose"),
    ("T11-012", "T11_012.BigRose.below_1"),
    ("T11-012", "T11_012.BigRose.binductionOn_1"),
    ("T11-012", "T11_012.BigRose.brecOn_1"),
    ("T11-012", "T11_012.BigRose.ibelow_1"),
    ("T11-012", "T11_012.BigRose.node"),
    ("T11-012", "T11_012.BigRose.rec_1"),
    ("T11-012", "T11_012.LTree.binductionOn_1"),
    ("T11-012", "T11_012.LTree.brecOn_1"),
    ("T11-012", "T11_012.PairTree.binductionOn_1"),
    ("T11-012", "T11_012.PairTree.brecOn_1"),
    ("T11-012", "T11_012.RoseList.binductionOn_1"),
    ("T11-012", "T11_012.RoseList.brecOn_1"),
    ("T11-012", "T11_012.VTree"),
    ("T11-012", "T11_012.VTree.below_1"),
    ("T11-012", "T11_012.VTree.binductionOn_1"),
    ("T11-012", "T11_012.VTree.brecOn_1"),
    ("T11-012", "T11_012.VTree.ibelow_1"),
    ("T11-012", "T11_012.VTree.node"),
    ("T11-012", "T11_012.VTree.rec_1"),
    ("T11-012", "T11_012.pairSize"),
    ("T11-012", "T11_012.roseListSize"),
    ("T11-012", "T11_012.roseSize"),
    ("T12-001", "T12_001.Doomed"),
    ("T12-001", "T12_001.Man"),
    ("T12-001", "T12_001.Mortal"),
    ("T12-001", "T12_001.socrates"),
    ("T12-001", "T12_001.trivialRestatement"),
    ("T12-004", "T12_004.Man"),
    ("T12-004", "T12_004.Mortal"),
    ("T12-006", "T12_006.Cong"),
    ("T12-006", "T12_006.segmentConstruction"),
    ("T12-007", "T12_007.Loves"),
    ("T12-007", "T12_007.father"),
    ("T12-011", "T12_011.Cong"),
    ("T12-011", "T12_011.Doomed"),
    ("T12-011", "T12_011.Man"),
    ("T12-011", "T12_011.Mortal"),
    ("T12-011", "T12_011.socrates"),
    ("T12-013", "T12_013.boxMP"),
    ("T12-014", "T12_014.In2003"),
    ("T12-014", "T12_014.In2004"),
    ("T12-014", "T12_014.InFlorida"),
    ("T12-014", "T12_014.InMaine"),
    ("T12-014", "T12_014.Man"),
    ("T12-014", "T12_014.Mortal"),
    ("T12-014", "T12_014.Trip"),
    ("T12-014", "T12_014.alpha"),
    ("T12-014", "T12_014.beta"),
    ("T00-008", "T00_008.emptyStr"),
    ("T00-008", "T00_008.helloStr"),
    ("T00-008", "T00_008.unicodeStr"),
    ("T10-005", "T10_005.empty"),
    ("T10-005", "T10_005.escaped"),
    ("T10-005", "T10_005.greeting"),
    ("T10-005", "T10_005.unicode"),
    ("T10-006", "T10_006.codePoint"),
    ("T10-006", "T10_006.firstChar"),
    ("T10-006", "T10_006.letterA"),
    ("T02-017", "T02_017.instReprColour"),
    ("T05-007", "T05_007.Bounded"),
    ("T05-007", "T05_007.Bounded.h"),
    ("T05-007", "T05_007.Bounded.i"),
    ("T05-007", "T05_007.Bounded.mk"),
    ("T05-007", "T05_007.Bounded.n"),
    ("T05-007", "T05_007.atLeast"),
    ("T05-007", "T05_007.boundedPick"),
    ("T05-007", "T05_007.predOf"),
    ("T05-007", "T05_007.safeDiv"),
    ("T05-020", "T05_020.prog"),
    ("T05-020", "T05_020.progBind"),
    ("T09-006", "T09_006.instReprColour"),
    ("T10-016", "T10_016.describe"),
    ("T09-001", "T09_001.«term_∣∣_»"),
    ("T09-001", "T09_001.«term_⊕_»"),
    ("T09-001", "T09_001.«term_⊗_»"),
    ("T09-001", "T09_001.«term⟦_⟧»"),
    ("T09-002", "T09_002.«term_‼»"),
    ("T09-002", "T09_002.«term_⊙_»"),
    ("T09-002", "T09_002.«term_⋄_»"),
    ("T09-002", "T09_002.«term_▷_»"),
    ("T09-002", "T09_002.«term∽_»"),
    ("T09-003", "T09_003.tacticReflAll"),
    ("T09-003", "T09_003.termTwiceIt_"),
    ("T09-003", "T09_003.«termNested![_]»"),
    ("T09-003", "T09_003.«termPick![_]»"),
    ("T09-004", "T09_004.tacticCloseWithRfl"),
    ("T09-004", "T09_004.termDoubleIfEven_"),
    ("T09-004", "T09_004.termEnvIsNonEmpty"),
    ("T09-004", "T09_004.termTheAnswer"),
    ("T09-005", "T09_005.builtSyntax"),
    ("T09-005", "T09_005.tacticJustRfl"),
    ("T09-005", "T09_005.termTwoPlusTwo"),
    ("T09-012", "T09_012.ioValue"),
    ("T09-013", "T09_013.Inner.«term_⊠_»"),
    ("T09-013", "T09_013.termBump_"),
    ("T09-013", "T09_013.termTwice_"),
    ("T09-013", "T09_013.«term_⊡_»"),
    ("T09-014", "T09_014.«binderTerm≫_»"),
    ("T11-006", "T11_006.termTwice_"),
    ("T04-004", "T04_004.gcd'"),
    ("T04-005", "T04_005.gcdSteps"),
    ("T09-002", "T09_002.assocLeft"),
    ("T09-002", "T09_002.assocRight"),
    ("T09-002", "T09_002.bang"),
    ("T09-002", "T09_002.f"),
    ("T09-002", "T09_002.g"),
    ("T09-002", "T09_002.h"),
    ("T09-002", "T09_002.infixNonAssoc"),
    ("T09-002", "T09_002.localFixity"),
    ("T10-009", "T10_009.byteLit"),
    ("T10-009", "T10_009.checksum"),
    ("T10-009", "T10_009.sizeLit"),
    ("T10-009", "T10_009.wordLit"),
    ("T10-010", "T10_010.appendBits"),
    ("T10-010", "T10_010.w8"),
    ("T10-010", "T10_010.zero8"),
    ("T10-013", "T10_013.dependentPair"),
    ("T10-013", "T10_013.index"),
    ("T10-013", "T10_013.leftVal"),
    ("T10-013", "T10_013.maybeFive"),
    ("T10-013", "T10_013.nothing"),
    ("T10-013", "T10_013.orZero"),
    ("T10-013", "T10_013.pair"),
    ("T10-013", "T10_013.positive"),
    ("T10-013", "T10_013.propSubtype"),
    ("T10-013", "T10_013.sumToNat"),
    ("T10-013", "T10_013.typePair"),
    ("T11-021", "T11_021.always"),
    ("T11-021", "T11_021.box"),
    ("T11-021", "T11_021.eventually"),
    ("T11-021", "T11_021.modalElim"),
    ("T11-021", "T11_021.modalIntro"),
    ("T11-021", "T11_021.plainAndIntro"),
    ("T11-021", "T11_021.plainModusPonens"),
    ("T11-021", "T11_021.progress"),
    ("T11-021", "T11_021.safe"),
    ("T11-021", "T11_021.safeAlways"),
    ("T11-021", "T11_021.state"),
    ("T11-021", "T11_021.timeTrans"),
    ("T11-021", "T11_021.unfoldAlways"),
    ("T11-021", "T11_021.until'"),
    ("T05-011", "T05_011.instCoeBoolNat__stdin"),
    ("T05-018", "T05_018.instCoeBoolNat__stdin"),
];

#[derive(Debug, Deserialize)]
struct GroundTruth {
    case: String,
    lean: String,
    compiles: bool,
    declaration_count: usize,
    declarations: Vec<TruthDeclaration>,
}

/// One declaration exactly as Lean elaborated it.
#[derive(Debug, Deserialize)]
struct TruthDeclaration {
    name: String,
    kind: String,
    levels: Vec<String>,
    #[serde(rename = "type")]
    ty: String,
    axioms: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct CorpusManifest {
    cases: Vec<CorpusCase>,
}

#[derive(Debug, Deserialize)]
struct CorpusCase {
    id: String,
    path: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BridgeFile {
    case: String,
    #[allow(dead_code)]
    lean: String,
    #[allow(dead_code)]
    lean_compiles: bool,
    declarations: Vec<BridgeEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BridgeEntry {
    /// The Lean declaration's full name — the obligation.
    lean: String,
    lean_kind: String,
    /// Lean's universe parameters. Non-empty means this declaration is universe-polymorphic.
    lean_levels: Vec<String>,
    /// The type Lean elaborated. This is the expected value our LOGOS side must match.
    lean_type: String,
    /// The transitive axiom set Lean needed — `#print axioms`, programmatically.
    lean_axioms: Vec<String>,
    status: Status,
    /// The LOGOS test that proves the match. Required when `bridged`.
    #[serde(default)]
    test: String,
    /// The LOGOS artifact itself. Required when `bridged`.
    #[serde(default)]
    logos: String,
    /// The argument. Required when `impossible` or `refused`.
    #[serde(default)]
    why: String,
    /// What the refusal reports. Present exactly when `refused`.
    #[serde(default)]
    refusal: Option<RefusalRecord>,
}

/// A refused row's report, as its named test asserts it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct RefusalRecord {
    /// The `logos-lean-result/1` outcome: `disallowed_assumptions`.
    outcome: String,
    /// The diagnostic code of every refusal diagnostic: `disallowed_assumption`.
    code: String,
    /// The axioms the refusal names — exactly Lean's `#print axioms`.
    axioms: Vec<String>,
    /// One path per escape: the target first, the escape last.
    taint: Vec<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Status {
    /// No LOGOS counterpart yet. This is the number we drive to zero.
    Open,
    /// A LOGOS artifact exists and a named test proves its type is the translation of the type
    /// Lean recorded.
    Bridged,
    /// The import refuses the declaration, correctly and completely, and a named test shows it: Lean's
    /// own `#print axioms` names a trust escape. Closed, and counted apart from `Bridged`.
    Refused,
    /// We deliberately will not do this, and `why` carries the argument.
    Impossible,
}

/// What stands between an open declaration and its LOGOS counterpart. Derived from Lean's own
/// facts, never from our opinion — which is what makes the worklist trustworthy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Blocker {
    /// A universe-polymorphic INDUCTIVE or CONSTRUCTOR.
    UniversePolymorphicInductive,
    /// A universe-polymorphic definition. We have these (`Term::Const { levels }`).
    UniversePolymorphicDefinition,
    /// Needs `propext` (an axiom of the `logos-coc-4` prelude).
    NeedsPropext,
    /// Needs `Classical.choice` (an axiom of the `logos-coc-4` prelude).
    NeedsChoice,
    /// Needs `Quot.sound` (an axiom of the `logos-coc-4` prelude).
    UsesQuotSound,
    /// The case deliberately contains a `sorry`. Our obligation is to refuse, not to prove.
    ContainsSorry,
    /// Needs the compiler/native-decide trust door.
    NeedsNativeTrust,
    /// Nothing exotic — plain monomorphic mathematics. These should be the cheapest wins.
    Plain,
}

impl Blocker {
    fn of(e: &BridgeEntry) -> Self {
        let ax = |n: &str| e.lean_axioms.iter().any(|a| a == n);
        if ax("sorryAx") {
            return Blocker::ContainsSorry;
        }
        if ax("Classical.choice") {
            return Blocker::NeedsChoice;
        }
        if ax("Lean.ofReduceBool") || ax("Lean.trustCompiler") {
            return Blocker::NeedsNativeTrust;
        }
        if ax("propext") {
            return Blocker::NeedsPropext;
        }
        if !e.lean_levels.is_empty() {
            return match e.lean_kind.as_str() {
                "inductive" | "ctor" => Blocker::UniversePolymorphicInductive,
                _ => Blocker::UniversePolymorphicDefinition,
            };
        }
        if ax("Quot.sound") {
            return Blocker::UsesQuotSound;
        }
        Blocker::Plain
    }

    fn label(self) -> &'static str {
        match self {
            Blocker::UniversePolymorphicInductive => "universe-polymorphic inductive",
            Blocker::UniversePolymorphicDefinition => "universe-polymorphic definition",
            Blocker::NeedsPropext => "uses propext",
            Blocker::NeedsChoice => "uses Classical.choice",
            Blocker::UsesQuotSound => "uses Quot.sound",
            Blocker::ContainsSorry => "contains sorry — we must REFUSE",
            Blocker::NeedsNativeTrust => "native-decide / compiler trust door",
            Blocker::Plain => "plain monomorphic maths",
        }
    }
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn load<T: serde::de::DeserializeOwned>(dir: &str) -> Vec<T> {
    let d = root().join(dir);
    let Ok(entries) = fs::read_dir(&d) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension().and_then(|e| e.to_str()) == Some("json")
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with('T'))
        })
        .collect();
    paths.sort();
    paths
        .into_iter()
        .map(|p| {
            let raw = fs::read_to_string(&p)
                .unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()));
            serde_json::from_str(&raw)
                .unwrap_or_else(|e| panic!("{} is malformed: {e}", p.display()))
        })
        .collect()
}

fn truth() -> &'static Vec<GroundTruth> {
    static T: OnceLock<Vec<GroundTruth>> = OnceLock::new();
    T.get_or_init(|| {
        load("crates/logicaffeine_tests/tests/fixtures/lean_conformance/lean_ground_truth")
    })
}

fn bridge() -> &'static Vec<BridgeFile> {
    static B: OnceLock<Vec<BridgeFile>> = OnceLock::new();
    B.get_or_init(|| load("crates/logicaffeine_tests/tests/fixtures/lean_conformance/bridge"))
}

fn all_entries() -> impl Iterator<Item = &'static BridgeEntry> {
    bridge().iter().flat_map(|b| b.declarations.iter())
}

// ============================================================================================
// THE ORACLE MUST HAVE RUN, AND LEAN MUST STILL AGREE
// ============================================================================================

#[test]
fn lean_ground_truth_exists_for_every_case() {
    let t = truth();
    assert_eq!(
        t.len(),
        CORPUS_CASE_COUNT,
        "ground truth covers {} of {CORPUS_CASE_COUNT} cases — run scripts/lean/conformance-lean-oracle.sh",
        t.len()
    );
}

#[test]
fn lean_compiles_every_case_in_the_corpus() {
    let broken: Vec<&str> = truth()
        .iter()
        .filter(|g| !g.compiles)
        .map(|g| g.case.as_str())
        .collect();
    assert!(
        broken.is_empty(),
        "Lean {LEAN_VERSION} must compile all {CORPUS_CASE_COUNT} corpus cases; these failed, so \
         every number in this lock is suspect until it is explained: {broken:?}"
    );
}

#[test]
fn the_ground_truth_was_taken_from_the_pinned_toolchain() {
    let wrong: Vec<&str> = truth()
        .iter()
        .filter(|g| g.lean != LEAN_VERSION)
        .map(|g| g.case.as_str())
        .collect();
    assert!(
        wrong.is_empty(),
        "the corpus is pinned to Lean {LEAN_VERSION}; these were extracted with another: {wrong:?}"
    );
}

#[test]
fn every_case_with_ground_truth_has_a_bridge_file() {
    let have: BTreeSet<&str> = bridge().iter().map(|b| b.case.as_str()).collect();
    let missing: Vec<&str> = truth()
        .iter()
        .map(|g| g.case.as_str())
        .filter(|c| !have.contains(c))
        .collect();
    assert!(
        missing.is_empty(),
        "these cases have Lean ground truth but no bridge ledger — run \
         scripts/lean/conformance-bridge-seed.py: {missing:?}"
    );
}

// ============================================================================================
// HONESTY — a claim must carry its evidence
// ============================================================================================

#[test]
fn every_bridged_declaration_names_its_test_and_its_logos() {
    let bad: Vec<String> = all_entries()
        .filter(|e| {
            e.status == Status::Bridged && (e.test.trim().is_empty() || e.logos.trim().is_empty())
        })
        .map(|e| format!("{} (test={:?} logos={:?})", e.lean, e.test, e.logos))
        .collect();
    assert!(
        bad.is_empty(),
        "\"bridged\" means a named LOGOS test exhibits a term matching the type LEAN recorded. \
         Without both fields it is an assertion, not a bridge:\n  {}",
        bad.join("\n  ")
    );
}

#[test]
fn every_impossible_declaration_states_its_argument() {
    const MIN: usize = 40;
    let bad: Vec<&str> = all_entries()
        .filter(|e| e.status == Status::Impossible && e.why.trim().len() < MIN)
        .map(|e| e.lean.as_str())
        .collect();
    assert!(
        bad.is_empty(),
        "\"impossible\" is a decision, and a decision needs at least {MIN} characters of reasoning \
         — otherwise it is just a way to make the number go down:\n  {bad:?}"
    );
}

#[test]
fn no_open_declaration_pretends_to_have_evidence() {
    let bad: Vec<&str> = all_entries()
        .filter(|e| {
            e.status == Status::Open && !(e.test.trim().is_empty() && e.logos.trim().is_empty())
        })
        .map(|e| e.lean.as_str())
        .collect();
    assert!(
        bad.is_empty(),
        "an open declaration carries no test and no logos — if the work is done, set status to \
         \"bridged\":\n  {bad:?}"
    );
}

#[test]
fn every_bridged_declaration_is_decided_by_its_own_test() {
    let mut by_test: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for e in all_entries().filter(|e| e.status == Status::Bridged) {
        by_test
            .entry(e.test.trim())
            .or_default()
            .push(e.lean.as_str());
    }
    // A single LOGOS test legitimately discharges a small family (an inductive and its
    // constructors, say). It does not legitimately discharge dozens.
    const MAX_PER_TEST: usize = 8;
    let overloaded: Vec<String> = by_test
        .iter()
        .filter(|(_, ds)| ds.len() > MAX_PER_TEST)
        .map(|(t, ds)| format!("{t} claims {} declarations", ds.len()))
        .collect();
    assert!(
        overloaded.is_empty(),
        "one test cannot honestly discharge more than {MAX_PER_TEST} Lean declarations — past that \
         the others were never really checked:\n  {}",
        overloaded.join("\n  ")
    );
}

// ============================================================================================
// IDENTITY — progress is banked by name, and the denominator cannot quietly shrink
// ============================================================================================

fn fnv1a64(bytes: impl Iterator<Item = u8>) -> u64 {
    bytes.fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ b as u64).wrapping_mul(0x0100_0000_01b3)
    })
}

#[test]
fn no_declaration_disappears_or_duplicates() {
    let mut seen: BTreeMap<(&str, &str), usize> = BTreeMap::new();
    for b in bridge() {
        for e in &b.declarations {
            *seen.entry((b.case.as_str(), e.lean.as_str())).or_default() += 1;
        }
    }
    let duplicated: Vec<String> = seen
        .iter()
        .filter(|(_, n)| **n > 1)
        .map(|((c, l), n)| format!("{c} {l} ×{n}"))
        .collect();
    assert!(
        duplicated.is_empty(),
        "a declaration identity appears twice:\n  {}",
        duplicated.join("\n  ")
    );
    let total: usize = seen.values().sum();
    assert_eq!(
        total, DECLARATION_COUNT,
        "the ledger has {total} authored declarations, pinned at {DECLARATION_COUNT}. A changed \
         denominator must be explained and re-pinned by a human, never absorbed into the open count."
    );
    let digest = fnv1a64(
        seen.keys()
            .flat_map(|(c, l)| format!("{c}\t{l}\n").into_bytes()),
    );
    assert_eq!(
        digest, DECLARATION_IDENTITY_FNV1A64,
        "the set of declaration identities changed (a rename, removal or substitution) although the \
         count did not: digest {digest:#018x}"
    );
}

fn sha256_hex(bytes: &[u8]) -> String {
    logicaffeine_compile::crypto_accel::sha256(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn lean_sources_under(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            lean_sources_under(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("lean") {
            out.push(path);
        }
    }
}

#[test]
fn every_corpus_source_is_present_and_unchanged() {
    let corpus = root().join("crates/logicaffeine_tests/tests/fixtures/lean_conformance");
    let raw = fs::read_to_string(corpus.join("manifest.json"))
        .expect("crates/logicaffeine_tests/tests/fixtures/lean_conformance/manifest.json");
    let manifest: CorpusManifest = serde_json::from_str(&raw).expect("corpus manifest parses");
    let mut cases: Vec<&CorpusCase> = manifest.cases.iter().collect();
    cases.sort_by(|a, b| a.id.cmp(&b.id));
    assert_eq!(
        cases.len(),
        CORPUS_CASE_COUNT,
        "the corpus manifest lists {} cases",
        cases.len()
    );

    let mut missing = Vec::new();
    let mut identity = String::new();
    for case in &cases {
        match fs::read(corpus.join(&case.path)) {
            Ok(bytes) => identity.push_str(&format!(
                "{}\t{}\t{}\n",
                case.id,
                case.path,
                sha256_hex(&bytes)
            )),
            Err(e) => missing.push(format!("{} {}: {e}", case.id, case.path)),
        }
    }
    assert!(
        missing.is_empty(),
        "corpus source files are missing:\n  {}",
        missing.join("\n  ")
    );

    let listed: BTreeSet<PathBuf> = cases.iter().map(|c| corpus.join(&c.path)).collect();
    let mut on_disk = Vec::new();
    lean_sources_under(&corpus.join("cases"), &mut on_disk);
    let stray: Vec<String> = on_disk
        .iter()
        .filter(|p| !listed.contains(*p))
        .map(|p| p.display().to_string())
        .collect();
    assert!(
        stray.is_empty(),
        "corpus files the manifest does not list:\n  {}",
        stray.join("\n  ")
    );

    assert_eq!(
        sha256_hex(identity.as_bytes()),
        CORPUS_SOURCE_SHA256,
        "an original corpus source changed, moved, appeared or vanished. Every bridge and triage record \
         was made against the pinned originals; a changed original is re-diagnosed and re-pinned by a \
         human with the corpus author's review, never absorbed silently"
    );
}

#[test]
fn the_ledger_is_exactly_the_lean_oracle() {
    let truth_by_case: BTreeMap<&str, &GroundTruth> =
        truth().iter().map(|g| (g.case.as_str(), g)).collect();
    let mut drift = Vec::new();
    let elaborated: usize = truth().iter().map(|g| g.declarations.len()).sum();
    for g in truth() {
        if g.declarations.len() != g.declaration_count {
            drift.push(format!(
                "{}: declaration_count {} but {} declarations listed",
                g.case,
                g.declaration_count,
                g.declarations.len()
            ));
        }
    }
    let mut rows = Vec::new();
    for b in bridge() {
        let Some(g) = truth_by_case.get(b.case.as_str()) else {
            drift.push(format!(
                "{}: a bridge ledger with no Lean ground truth",
                b.case
            ));
            continue;
        };
        let facts: BTreeMap<&str, &TruthDeclaration> = g
            .declarations
            .iter()
            .map(|d| (d.name.as_str(), d))
            .collect();
        for e in &b.declarations {
            match facts.get(e.lean.as_str()) {
                None => drift.push(format!(
                    "{} {}: Lean did not elaborate this declaration",
                    b.case, e.lean
                )),
                Some(t) => {
                    if (t.kind.as_str(), &t.levels, t.ty.as_str(), &t.axioms)
                        != (
                            e.lean_kind.as_str(),
                            &e.lean_levels,
                            e.lean_type.as_str(),
                            &e.lean_axioms,
                        )
                    {
                        drift.push(format!(
                            "{} {}: ledger facts differ from Lean's",
                            b.case, e.lean
                        ));
                    }
                }
            }
            rows.push((
                b.case.as_str(),
                e.lean.as_str(),
                e.lean_kind.as_str(),
                &e.lean_levels,
                e.lean_type.as_str(),
                &e.lean_axioms,
            ));
        }
    }
    assert!(
        drift.is_empty(),
        "the ledger drifted from the Lean oracle:\n  {}",
        drift.join("\n  ")
    );
    assert_eq!(
        elaborated, ELABORATED_DECLARATION_COUNT,
        "Lean {LEAN_VERSION} elaborated {elaborated} declarations from the corpus, pinned at \
         {ELABORATED_DECLARATION_COUNT} — the figure this file's header states"
    );
    rows.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
    let canonical: String = rows
        .iter()
        .map(|r| {
            format!(
                "{}\n",
                serde_json::to_string(r).expect("oracle facts serialize")
            )
        })
        .collect();
    assert_eq!(
        sha256_hex(canonical.as_bytes()),
        ORACLE_FACTS_SHA256,
        "a Lean oracle fact (type, universe parameters, kind or axioms) changed for some ledger row. \
         Re-run scripts/lean/conformance-lean-oracle.sh under the pinned toolchain and explain the change; the \
         expected value of an obligation never moves silently"
    );
}

#[test]
fn every_ledger_row_is_authored_by_its_own_case() {
    let bad: Vec<String> = bridge()
        .iter()
        .flat_map(|b| {
            let prefix = format!("{}.", b.case.replace('-', "_"));
            b.declarations
                .iter()
                .filter(move |e| !e.lean.starts_with(&prefix))
                .map(move |e| format!("{} {}", b.case, e.lean))
        })
        .collect();
    assert!(
        bad.is_empty(),
        "a ledger row outside its case's namespace is a renamed duplicate waiting to happen:\n  {}",
        bad.join("\n  ")
    );
}

/// The generated harvest test binary.
const HARVEST_BINARY: &str = "original_harvest_bridge";

/// A harvested bridge is one of the roots its cited harvest test decides, and no dedicated bridge test
/// also claims the row: the harvest only promotes rows nobody earned, it never re-cites them.
#[test]
fn harvested_rows_are_decided_by_their_harvest_test_and_claimed_by_no_dedicated_test() {
    let kernel_tests = root().join("crates/logicaffeine_tests/tests");
    let harvest =
        fs::read_to_string(kernel_tests.join(format!("{HARVEST_BINARY}.rs"))).unwrap_or_default();
    let mut decided: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
    let mut rest = harvest.as_str();
    while let Some(at) = rest.find("fn harvest_") {
        let body = &rest[at + 3..];
        let name = &body[..body.find('(').unwrap_or(0)];
        let end = body.find("\n}").unwrap_or(body.len());
        let roots = body[..end]
            .split("lean: \"")
            .skip(1)
            .filter_map(|s| s.split('"').next())
            .map(str::to_string)
            .collect();
        decided.insert(name, roots);
        rest = &body[end..];
    }
    let dedicated: Vec<(String, String)> = fs::read_dir(&kernel_tests)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("rs"))
        .filter(|p| p.file_stem().and_then(|s| s.to_str()) != Some(HARVEST_BINARY))
        .map(|p| {
            (
                p.display().to_string(),
                fs::read_to_string(&p).unwrap_or_default(),
            )
        })
        .collect();
    let mut bad = Vec::new();
    let mut harvested = 0;
    for b in bridge() {
        for e in b
            .declarations
            .iter()
            .filter(|e| e.status == Status::Bridged)
        {
            let Some(test) = e.test.strip_prefix(&format!("{HARVEST_BINARY}::")) else {
                continue;
            };
            harvested += 1;
            if !decided
                .get(test)
                .is_some_and(|roots| roots.contains(&e.lean))
            {
                bad.push(format!("{} cites {test}, which does not decide it", e.lean));
            }
            let short = e.lean.split_once('.').map_or(e.lean.as_str(), |(_, s)| s);
            let ledger_path = format!("fixtures/lean_conformance/bridge/{}.json", b.case);
            for (path, source) in &dedicated {
                if source.contains(&ledger_path) && source.contains(&format!("\"{short}\"")) {
                    bad.push(format!("{} is claimed by the dedicated test file {path}; the harvest must not re-cite it", e.lean));
                }
            }
        }
    }
    assert!(
        bad.is_empty(),
        "harvested bridge ownership:\n  {}",
        bad.join("\n  ")
    );
    eprintln!("  {harvested} harvested bridges, each decided by its own harvest test and claimed by no dedicated test");
}

#[test]
fn earned_bridges_are_never_lost() {
    let status: BTreeMap<(&str, &str), Status> = bridge()
        .iter()
        .flat_map(|b| {
            b.declarations
                .iter()
                .map(move |e| ((b.case.as_str(), e.lean.as_str()), e.status))
        })
        .collect();
    let lost: Vec<String> = EARNED_BRIDGES
        .iter()
        .filter(|id| status.get(id) != Some(&Status::Bridged))
        .map(|(c, l)| format!("{c} {l} is now {:?}", status.get(&(*c, *l))))
        .collect();
    assert!(
        lost.is_empty(),
        "an earned bridge was lost:\n  {}",
        lost.join("\n  ")
    );
    let earned: BTreeSet<(&str, &str)> = EARNED_BRIDGES.iter().copied().collect();
    let unpinned: Vec<String> = status
        .iter()
        .filter(|(id, s)| **s == Status::Bridged && !earned.contains(*id))
        .map(|((c, l), _)| format!("(\"{c}\", \"{l}\"),"))
        .collect();
    assert!(
        unpinned.is_empty(),
        "newly bridged declarations must be pinned in EARNED_BRIDGES (and OPEN_CEILING lowered):\n  {}",
        unpinned.join("\n  ")
    );
}

/// Resolve a ledger `test` citation — `path/to/file.rs::name` or `module::name` — to the source file
/// that defines it.
fn cited_test_source(citation: &str) -> Result<(PathBuf, String), String> {
    let (file, name) = citation
        .trim()
        .rsplit_once("::")
        .ok_or_else(|| format!("{citation}: not file::test"))?;
    let candidates: Vec<PathBuf> = if file.ends_with(".rs") {
        vec![root().join(file)]
            .into_iter()
            .filter(|p| p.is_file())
            .collect()
    } else {
        ["crates", "apps"]
            .iter()
            .flat_map(|top| {
                fs::read_dir(root().join(top))
                    .into_iter()
                    .flatten()
                    .flatten()
            })
            .map(|member| member.path().join("tests").join(format!("{file}.rs")))
            .filter(|p| p.is_file())
            .collect()
    };
    match candidates.as_slice() {
        [one] => Ok((one.clone(), name.to_string())),
        [] => Err(format!("{citation}: no such test file")),
        many => Err(format!("{citation}: ambiguous module, found {many:?}")),
    }
}

/// Split a source line into its leading `#[...]` attributes and the remainder, so `#[test] fn f()`
/// on one line reads the same as the attribute on the line above.
fn leading_attributes(line: &str) -> (Vec<&str>, &str) {
    let mut attributes = Vec::new();
    let mut rest = line.trim_start();
    while rest.starts_with("#[") {
        let mut depth = 0usize;
        let Some(end) = rest.char_indices().find_map(|(i, c)| {
            match c {
                '[' => depth += 1,
                ']' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i + 1);
                    }
                }
                _ => {}
            }
            None
        }) else {
            break;
        };
        attributes.push(&rest[..end]);
        rest = rest[end..].trim_start();
    }
    (attributes, rest)
}

/// A ledger `test` citation names a function its file defines, marked `#[test]` and not `#[ignore]`d.
fn cited_running_test(citation: &str) -> Result<(), String> {
    let (path, name) = cited_test_source(citation)?;
    let source = fs::read_to_string(&path).unwrap_or_default();
    let lines: Vec<&str> = source.lines().collect();
    let defines = |l: &&str| {
        let (_, rest) = leading_attributes(l);
        let rest = rest.strip_prefix("pub ").unwrap_or(rest);
        rest.starts_with(&format!("fn {name}(")) || rest.starts_with(&format!("fn {name}<"))
    };
    let Some(at) = lines.iter().position(defines) else {
        return Err(format!("{} defines no fn {name}", path.display()));
    };
    let mut attributes: Vec<&str> = leading_attributes(lines[at]).0;
    attributes.extend(
        lines[..at]
            .iter()
            .rev()
            .map(|l| l.trim())
            .take_while(|l| l.starts_with("#[") || l.starts_with("//"))
            .flat_map(|l| leading_attributes(l).0),
    );
    if !attributes.contains(&"#[test]") {
        return Err(format!("fn {name} is not a #[test]"));
    }
    if attributes.iter().any(|a| a.starts_with("#[ignore")) {
        return Err(format!(
            "fn {name} is #[ignore]d — an ignored test is not evidence"
        ));
    }
    Ok(())
}

#[test]
fn bridged_rows_cite_existing_unignored_tests() {
    let bad: Vec<String> = all_entries()
        .filter(|e| e.status == Status::Bridged)
        .filter_map(|e| {
            cited_running_test(&e.test)
                .err()
                .map(|why| format!("{}: {why}", e.lean))
        })
        .collect();
    assert!(
        bad.is_empty(),
        "a bridge must cite a real, running test:\n  {}",
        bad.join("\n  ")
    );
}

// ============================================================================================
// REFUSALS — a row closes as `refused` only by a complete, named, correct refusal
// ============================================================================================

/// The generated refusal test binary.
const REFUSAL_BINARY: &str = "original_refusal_bridge";

/// Lean's `#print axioms` names one of these, or a row cannot be refused: refusing a declaration that
/// rests on none of them would refuse valid mathematics. `Lean.ofReduceBool` alone is not one — a
/// `native_decide` proof of a true decidable statement is re-proved by the kernel and bridged.
const REFUSED_ESCAPES: [&str; 3] = ["sorryAx", "lcProof", "Lean.trustCompiler"];

/// Lean's three standard axioms. Every other axiom a refused row rests on is an escape and has its
/// taint path.
const ALLOWED_AXIOMS: [&str; 3] = ["propext", "Quot.sound", "Classical.choice"];

/// What one generated refusal test states.
#[derive(Debug, Clone, PartialEq, Eq)]
struct GeneratedRefusal {
    lean: String,
    axioms: Vec<String>,
    taint: Vec<Vec<String>>,
}

/// One ledger row with its case.
type Row<'a> = (&'a str, &'a BridgeEntry);

fn ledger_rows() -> Vec<Row<'static>> {
    bridge()
        .iter()
        .flat_map(|b| b.declarations.iter().map(move |e| (b.case.as_str(), e)))
        .collect()
}

/// Lean's `#print axioms` for every declaration the oracle elaborated, by (case, name).
fn oracle_axioms() -> BTreeMap<(String, String), Vec<String>> {
    truth()
        .iter()
        .flat_map(|g| {
            g.declarations
                .iter()
                .map(move |d| ((g.case.clone(), d.name.clone()), d.axioms.clone()))
        })
        .collect()
}

/// Parse the refusal binary's source: test name → what it states, and every `fn refusal_…` it cannot
/// read with the reason — a test the lock cannot read is a test it cannot hold to its row.
fn generated_refusals(source: &str) -> (BTreeMap<String, GeneratedRefusal>, Vec<String>) {
    let mut read = BTreeMap::new();
    let mut unreadable = Vec::new();
    let mut rest = source;
    while let Some(at) = rest.find("fn refusal_") {
        let body = &rest[at + 3..];
        let name = body[..body.find('(').unwrap_or(0)].to_string();
        let end = body.find("\n}").unwrap_or(body.len());
        match read_refusal(&body[..end]) {
            Ok(g) => {
                read.insert(name, g);
            }
            Err(why) => unreadable.push(format!("{REFUSAL_BINARY}::{name}: {why}")),
        }
        rest = &body[end..];
    }
    (read, unreadable)
}

/// What one generated test's `Refusal { … }` literal states, in whatever layout it is written.
fn read_refusal(block: &str) -> Result<GeneratedRefusal, String> {
    let fields = literal_fields(block)?;
    let field = |key: &str| {
        fields
            .get(key)
            .copied()
            .ok_or_else(|| format!("no `{key}` field"))
    };
    Ok(GeneratedRefusal {
        lean: serde_json::from_str::<String>(field("lean")?)
            .map_err(|_| "`lean` is not one string literal".to_string())?,
        axioms: string_slices(field("axioms")?)
            .and_then(Slice::flat)
            .ok_or("`axioms` is not a `&[…]` of string literals")?,
        taint: string_slices(field("taint")?)
            .and_then(Slice::nested)
            .ok_or("`taint` is not a `&[&[…], …]` of string literals")?,
    })
}

/// The `key: value` fields of the first `Refusal { … }` literal in `block`, split at the commas that
/// stand outside every string and bracket, so a value reads the same on one line or across many.
fn literal_fields(block: &str) -> Result<BTreeMap<&str, &str>, String> {
    let open = block
        .find("Refusal {")
        .ok_or("no `Refusal { … }` literal")?
        + "Refusal {".len();
    let body = &block[open..];
    let mut pieces = Vec::new();
    let (mut depth, mut start, mut in_string, mut escaped) = (0usize, 0, false, false);
    let mut close = None;
    for (i, c) in body.char_indices() {
        if in_string {
            match c {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '[' | '(' | '{' => depth += 1,
            ']' | ')' | '}' if depth > 0 => depth -= 1,
            '}' => {
                pieces.push(&body[start..i]);
                close = Some(i);
                break;
            }
            ']' | ')' => return Err(format!("an unopened `{c}` in the `Refusal` literal")),
            ',' if depth == 0 => {
                pieces.push(&body[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if close.is_none() {
        return Err("the `Refusal { … }` literal never closes".into());
    }
    let mut fields = BTreeMap::new();
    for piece in pieces.into_iter().map(str::trim).filter(|p| !p.is_empty()) {
        let Some((key, value)) = piece.split_once(':') else {
            return Err(format!("{piece:?} is not a `key: value` field"));
        };
        let key = key.trim();
        if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(format!("{piece:?} is not a `key: value` field"));
        }
        if fields.insert(key, value.trim()).is_some() {
            return Err(format!("the field `{key}` twice"));
        }
    }
    Ok(fields)
}

/// A Rust `&[…]` literal of string literals, nested to any depth.
#[derive(Debug)]
enum Slice {
    Str(String),
    List(Vec<Slice>),
}

impl Slice {
    fn flat(self) -> Option<Vec<String>> {
        let Slice::List(items) = self else {
            return None;
        };
        items
            .into_iter()
            .map(|i| if let Slice::Str(s) = i { Some(s) } else { None })
            .collect()
    }
    fn nested(self) -> Option<Vec<Vec<String>>> {
        let Slice::List(items) = self else {
            return None;
        };
        items.into_iter().map(Slice::flat).collect()
    }
}

fn string_slices(text: &str) -> Option<Slice> {
    let mut stack: Vec<Vec<Slice>> = vec![Vec::new()];
    let mut rest = text.trim_start();
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("&[") {
            stack.push(Vec::new());
            rest = after;
        } else if let Some(after) = rest.strip_prefix(']') {
            let done = stack.pop()?;
            stack.last_mut()?.push(Slice::List(done));
            rest = after;
        } else if rest.starts_with('"') {
            let mut stream = serde_json::Deserializer::from_str(rest).into_iter::<String>();
            let s = stream.next()?.ok()?;
            let used = stream.byte_offset();
            stack.last_mut()?.push(Slice::Str(s));
            rest = &rest[used..];
        } else if let Some(after) = rest.strip_prefix(',') {
            rest = after;
        } else {
            return None;
        }
        rest = rest.trim_start();
    }
    let mut top = stack.pop()?;
    (stack.is_empty() && top.len() == 1).then(|| top.remove(0))
}

/// A refused row carries its evidence and nothing else claims one: a named test, no LOGOS artifact (a
/// refusal publishes nothing), a stated argument, and the refusal record — outcome
/// `disallowed_assumptions`, code `disallowed_assumption`, and one taint path per escape, from the row
/// to that escape.
fn refusal_record_violations(rows: &[Row<'_>]) -> Vec<String> {
    const MIN_ARGUMENT: usize = 40;
    let mut bad = Vec::new();
    for (case, e) in rows {
        let who = format!("{case} {}", e.lean);
        if e.status != Status::Refused {
            if e.refusal.is_some() {
                bad.push(format!("{who}: a refusal record on a {:?} row", e.status));
            }
            continue;
        }
        if e.test.trim().is_empty() {
            bad.push(format!(
                "{who}: refused without the named test that shows it"
            ));
        }
        if !e.logos.trim().is_empty() {
            bad.push(format!(
                "{who}: a refusal publishes nothing, yet the row claims a LOGOS artifact"
            ));
        }
        if e.why.trim().len() < MIN_ARGUMENT {
            bad.push(format!(
                "{who}: a refusal states its argument in at least {MIN_ARGUMENT} characters"
            ));
        }
        let Some(r) = &e.refusal else {
            bad.push(format!("{who}: refused without its refusal record"));
            continue;
        };
        if r.outcome != "disallowed_assumptions" || r.code != "disallowed_assumption" {
            bad.push(format!("{who}: outcome {:?} / code {:?}, a trust-escape refusal is disallowed_assumptions / disallowed_assumption", r.outcome, r.code));
        }
        let escapes: BTreeSet<&str> = r
            .axioms
            .iter()
            .map(String::as_str)
            .filter(|a| !ALLOWED_AXIOMS.contains(a))
            .collect();
        let mut reached: BTreeMap<&str, usize> = BTreeMap::new();
        for path in &r.taint {
            let (Some(first), Some(last)) = (path.first(), path.last()) else {
                bad.push(format!("{who}: an empty taint path"));
                continue;
            };
            if first != &e.lean {
                bad.push(format!(
                    "{who}: a taint path starts at {first}, not at the refused declaration"
                ));
            }
            if !escapes.contains(last.as_str()) {
                bad.push(format!("{who}: a taint path ends at {last}, which is not one of its escapes {escapes:?}"));
            }
            if path.iter().collect::<BTreeSet<_>>().len() != path.len() {
                bad.push(format!(
                    "{who}: the taint path {path:?} visits a declaration twice"
                ));
            }
            *reached.entry(last.as_str()).or_default() += 1;
        }
        for escape in &escapes {
            match reached.get(escape) {
                Some(1) => {}
                None => bad.push(format!("{who}: no taint path to {escape}")),
                Some(n) => bad.push(format!(
                    "{who}: {n} taint paths to {escape}; the report gives one"
                )),
            }
        }
    }
    bad
}

/// A refused row's axioms are exactly Lean's `#print axioms`, and Lean names a refused escape.
fn refusal_axiom_violations(
    rows: &[Row<'_>],
    oracle: &BTreeMap<(String, String), Vec<String>>,
) -> Vec<String> {
    let mut bad = Vec::new();
    for (case, e) in rows.iter().filter(|(_, e)| e.status == Status::Refused) {
        let who = format!("{case} {}", e.lean);
        let Some(lean) = oracle.get(&(case.to_string(), e.lean.clone())) else {
            bad.push(format!("{who}: the Lean oracle never elaborated it, so no refusal can be held against Lean"));
            continue;
        };
        if !lean.iter().any(|a| REFUSED_ESCAPES.contains(&a.as_str())) {
            bad.push(format!(
                "{who}: Lean's #print axioms {lean:?} names none of {REFUSED_ESCAPES:?}; refusing it refuses valid mathematics"
            ));
        }
        let Some(r) = &e.refusal else { continue };
        let stated: BTreeSet<&String> = r.axioms.iter().collect();
        if stated.len() != r.axioms.len() {
            bad.push(format!(
                "{who}: the refusal states an axiom twice: {:?}",
                r.axioms
            ));
        }
        if stated != lean.iter().collect::<BTreeSet<_>>() {
            bad.push(format!(
                "{who}: the refusal names {:?}, Lean's #print axioms names {lean:?}",
                r.axioms
            ));
        }
    }
    bad
}

/// A refused row cites a running refusal test that decides exactly that row, stating the row's axioms
/// and taint paths, and no two refused rows cite one test.
fn refusal_citation_violations(
    rows: &[Row<'_>],
    generated: &BTreeMap<String, GeneratedRefusal>,
    running: &dyn Fn(&str) -> Result<(), String>,
) -> Vec<String> {
    let mut bad = Vec::new();
    let mut cited: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for (case, e) in rows.iter().filter(|(_, e)| e.status == Status::Refused) {
        let who = format!("{case} {}", e.lean);
        cited.entry(e.test.trim()).or_default().push(who.clone());
        if let Err(why) = running(&e.test) {
            bad.push(format!("{who}: {why}"));
        }
        let Some(name) = e.test.trim().strip_prefix(&format!("{REFUSAL_BINARY}::")) else {
            bad.push(format!("{who}: a refusal is decided by its generated test in {REFUSAL_BINARY}, not by {:?}", e.test));
            continue;
        };
        let Some(g) = generated.get(name) else {
            bad.push(format!("{who}: {REFUSAL_BINARY} generates no test {name}"));
            continue;
        };
        if g.lean != e.lean {
            bad.push(format!("{who}: {name} decides {}, not this row", g.lean));
        }
        if let Some(r) = &e.refusal {
            if g.axioms != r.axioms {
                bad.push(format!(
                    "{who}: {name} asserts the axioms {:?}, the ledger records {:?}",
                    g.axioms, r.axioms
                ));
            }
            if g.taint != r.taint {
                bad.push(format!(
                    "{who}: {name} asserts the taint paths {:?}, the ledger records {:?}",
                    g.taint, r.taint
                ));
            }
        }
    }
    for (test, rows) in cited.into_iter().filter(|(_, rows)| rows.len() > 1) {
        bad.push(format!(
            "{test} is cited by {} refused rows ({}); each refused row has its own test",
            rows.len(),
            rows.join(", ")
        ));
    }
    bad
}

/// Every pinned refusal is still refused, and every refused row is pinned.
fn earned_refusal_violations(rows: &[Row<'_>], earned: &[(&str, &str)]) -> Vec<String> {
    let status: BTreeMap<(&str, &str), Status> = rows
        .iter()
        .map(|(c, e)| ((*c, e.lean.as_str()), e.status))
        .collect();
    let pinned: BTreeSet<(&str, &str)> = earned.iter().copied().collect();
    let mut bad: Vec<String> = earned
        .iter()
        .filter(|id| status.get(id) != Some(&Status::Refused))
        .map(|(c, l)| {
            format!(
                "{c} {l}: an earned refusal is now {:?}",
                status.get(&(*c, *l))
            )
        })
        .collect();
    if pinned.len() != earned.len() {
        bad.push("EARNED_REFUSALS lists a row twice".to_string());
    }
    bad.extend(
        status
            .iter()
            .filter(|(id, s)| **s == Status::Refused && !pinned.contains(*id))
            .map(|((c, l), _)| format!("newly refused, pin it in EARNED_REFUSALS (and lower OPEN_CEILING): (\"{c}\", \"{l}\"),")),
    );
    bad
}

/// Everything wrong between the refused rows and the refusal binary's source: each test the lock
/// cannot read, then each citation that does not hold.
fn refusal_binary_violations(
    source: &str,
    rows: &[Row<'_>],
    running: &dyn Fn(&str) -> Result<(), String>,
) -> Vec<String> {
    let (generated, mut bad) = generated_refusals(source);
    bad.extend(refusal_citation_violations(rows, &generated, running));
    bad
}

fn refusal_source() -> String {
    fs::read_to_string(root().join(format!(
        "crates/logicaffeine_tests/tests/{REFUSAL_BINARY}.rs"
    )))
    .unwrap_or_default()
}

#[test]
fn refused_rows_carry_their_refusal_and_a_taint_path_per_escape() {
    let bad = refusal_record_violations(&ledger_rows());
    assert!(
        bad.is_empty(),
        "a refused row is a complete refusal:\n  {}",
        bad.join("\n  ")
    );
}

#[test]
fn a_refused_rows_axioms_are_leans_print_axioms() {
    let bad = refusal_axiom_violations(&ledger_rows(), &oracle_axioms());
    assert!(
        bad.is_empty(),
        "a refusal names exactly the axioms Lean names:\n  {}",
        bad.join("\n  ")
    );
}

#[test]
fn refused_rows_cite_existing_running_tests() {
    let bad = refusal_binary_violations(&refusal_source(), &ledger_rows(), &cited_running_test);
    assert!(
        bad.is_empty(),
        "a refusal must cite the real, running test that decides it:\n  {}",
        bad.join("\n  ")
    );
}

#[test]
fn earned_refusals_are_never_lost() {
    let bad = earned_refusal_violations(&ledger_rows(), EARNED_REFUSALS);
    assert!(
        bad.is_empty(),
        "refusals are banked by identity:\n  {}",
        bad.join("\n  ")
    );
}

#[test]
fn bridged_refused_and_open_partition_the_corpus() {
    let mut by_status: BTreeMap<Status, usize> = BTreeMap::new();
    for (_, e) in ledger_rows() {
        *by_status.entry(e.status).or_default() += 1;
    }
    let count = |s: Status| by_status.get(&s).copied().unwrap_or(0);
    let (bridged, refused, open, impossible) = (
        count(Status::Bridged),
        count(Status::Refused),
        count(Status::Open),
        count(Status::Impossible),
    );
    eprintln!("  bridged {bridged} + refused {refused} + open {open} + impossible {impossible} = {DECLARATION_COUNT}");
    assert_eq!(
        bridged + refused + open + impossible,
        DECLARATION_COUNT,
        "every authored declaration is bridged, refused, open or impossible — and counted once"
    );
    assert_eq!(
        bridged,
        EARNED_BRIDGES.len(),
        "every bridged row is pinned in EARNED_BRIDGES and none twice"
    );
    assert_eq!(
        refused,
        EARNED_REFUSALS.len(),
        "every refused row is pinned in EARNED_REFUSALS and none twice"
    );
}

/// A well-formed refused row over a tiny ledger, and one broken variant per way a refusal could lie.
#[test]
fn the_refusal_checks_catch_every_dishonest_row() {
    let row =
        |status: Status, test: &str, logos: &str, why: &str, refusal: Option<RefusalRecord>| {
            BridgeEntry {
                lean: "T99_001.user".into(),
                lean_kind: "theorem".into(),
                lean_levels: vec![],
                lean_type: "0 = 1".into(),
                lean_axioms: vec!["sorryAx".into()],
                status,
                test: test.into(),
                logos: logos.into(),
                why: why.into(),
                refusal,
            }
        };
    let record = || RefusalRecord {
        outcome: "disallowed_assumptions".into(),
        code: "disallowed_assumption".into(),
        axioms: vec!["sorryAx".into()],
        taint: vec![vec![
            "T99_001.user".into(),
            "T99_001.lemma".into(),
            "sorryAx".into(),
        ]],
    };
    let why = "Lean's #print axioms names sorryAx; the pipeline translates it, both kernels check it, and the policy refuses it.";
    let citation = format!("{REFUSAL_BINARY}::refusal_t99_001_user");
    let good = || row(Status::Refused, &citation, "", why, Some(record()));
    let oracle = || {
        BTreeMap::from([(
            ("T99-001".to_string(), "T99_001.user".to_string()),
            vec!["sorryAx".to_string()],
        )])
    };
    let generated = || {
        BTreeMap::from([(
            "refusal_t99_001_user".to_string(),
            GeneratedRefusal {
                lean: "T99_001.user".into(),
                axioms: vec!["sorryAx".into()],
                taint: vec![vec![
                    "T99_001.user".into(),
                    "T99_001.lemma".into(),
                    "sorryAx".into(),
                ]],
            },
        )])
    };
    let running = |c: &str| {
        if c == citation {
            Ok(())
        } else {
            Err(format!("{c}: no such running test"))
        }
    };
    let earned: &[(&str, &str)] = &[("T99-001", "T99_001.user")];

    let all = |e: &BridgeEntry,
               oracle: &BTreeMap<(String, String), Vec<String>>,
               generated: &BTreeMap<String, GeneratedRefusal>,
               running: &dyn Fn(&str) -> Result<(), String>,
               earned: &[(&str, &str)]| {
        let rows = [("T99-001", e)];
        let mut bad = refusal_record_violations(&rows);
        bad.extend(refusal_axiom_violations(&rows, oracle));
        bad.extend(refusal_citation_violations(&rows, generated, running));
        bad.extend(earned_refusal_violations(&rows, earned));
        bad
    };
    let clean = all(&good(), &oracle(), &generated(), &running, earned);
    assert!(
        clean.is_empty(),
        "the well-formed refusal passes every check: {clean:?}"
    );

    let caught = |what: &str, bad: Vec<String>| assert!(!bad.is_empty(), "not caught: {what}");
    let with = |edit: &dyn Fn(&mut BridgeEntry)| {
        let mut e = good();
        edit(&mut e);
        all(&e, &oracle(), &generated(), &running, earned)
    };
    caught("a refused row without a test", with(&|e| e.test.clear()));
    caught(
        "a refused row that claims a LOGOS artifact",
        with(&|e| e.logos = "published anyway".into()),
    );
    caught(
        "a refused row without its argument",
        with(&|e| e.why = "sorry".into()),
    );
    caught(
        "a refused row without its refusal record",
        with(&|e| e.refusal = None),
    );
    caught(
        "a refusal record on a bridged row",
        with(&|e| e.status = Status::Bridged),
    );
    caught(
        "a refusal record on an open row",
        with(&|e| e.status = Status::Open),
    );
    caught(
        "another outcome",
        with(&|e| e.refusal.as_mut().unwrap().outcome = "unsupported_feature".into()),
    );
    caught(
        "another diagnostic code",
        with(&|e| e.refusal.as_mut().unwrap().code = "unsupported_feature".into()),
    );
    caught(
        "an axiom Lean does not name",
        with(&|e| e.refusal.as_mut().unwrap().axioms.push("propext".into())),
    );
    caught(
        "an axiom Lean names left out",
        with(&|e| e.refusal.as_mut().unwrap().axioms.clear()),
    );
    caught(
        "an axiom stated twice",
        with(&|e| e.refusal.as_mut().unwrap().axioms.push("sorryAx".into())),
    );
    caught(
        "an escape without a taint path",
        with(&|e| e.refusal.as_mut().unwrap().taint.clear()),
    );
    caught(
        "a taint path from another declaration",
        with(&|e| e.refusal.as_mut().unwrap().taint[0][0] = "T99_001.other".into()),
    );
    caught(
        "a taint path that ends elsewhere",
        with(&|e| {
            e.refusal.as_mut().unwrap().taint[0].pop();
        }),
    );
    caught(
        "two taint paths to one escape",
        with(&|e| {
            let path = e.refusal.as_ref().unwrap().taint[0].clone();
            e.refusal.as_mut().unwrap().taint.push(path);
        }),
    );
    caught(
        "a taint path that repeats a declaration",
        with(&|e| {
            e.refusal.as_mut().unwrap().taint[0].insert(1, "T99_001.user".into());
        }),
    );
    caught(
        "a cited test that does not run",
        with(&|e| e.test = format!("{REFUSAL_BINARY}::refusal_missing")),
    );
    caught(
        "a refusal cited from another binary",
        with(&|e| e.test = "original_harvest_bridge::refusal_t99_001_user".into()),
    );

    caught(
        "Lean names no refused escape (a native_decide row must be re-proved, not refused)",
        all(
            &{
                let mut e = good();
                e.lean_axioms = vec!["Lean.ofReduceBool".into()];
                let r = e.refusal.as_mut().unwrap();
                r.axioms = vec!["Lean.ofReduceBool".into()];
                r.taint = vec![vec!["T99_001.user".into(), "Lean.ofReduceBool".into()]];
                e
            },
            &BTreeMap::from([(
                ("T99-001".to_string(), "T99_001.user".to_string()),
                vec!["Lean.ofReduceBool".to_string()],
            )]),
            &BTreeMap::from([(
                "refusal_t99_001_user".to_string(),
                GeneratedRefusal {
                    lean: "T99_001.user".into(),
                    axioms: vec!["Lean.ofReduceBool".into()],
                    taint: vec![vec!["T99_001.user".into(), "Lean.ofReduceBool".into()]],
                },
            )]),
            &running,
            earned,
        ),
    );
    caught(
        "the recorded axioms differ from Lean's oracle",
        all(
            &good(),
            &BTreeMap::from([(
                ("T99-001".to_string(), "T99_001.user".to_string()),
                vec!["lcProof".to_string()],
            )]),
            &generated(),
            &running,
            earned,
        ),
    );
    caught(
        "a row the Lean oracle never elaborated",
        all(&good(), &BTreeMap::new(), &generated(), &running, earned),
    );
    caught("the cited test decides another row", {
        let mut g = generated();
        g.get_mut("refusal_t99_001_user").unwrap().lean = "T99_001.other".into();
        all(&good(), &oracle(), &g, &running, earned)
    });
    caught("the cited test states other axioms", {
        let mut g = generated();
        g.get_mut("refusal_t99_001_user").unwrap().axioms = vec!["lcProof".into()];
        all(&good(), &oracle(), &g, &running, earned)
    });
    caught("the cited test asserts another taint path", {
        let mut g = generated();
        g.get_mut("refusal_t99_001_user").unwrap().taint =
            vec![vec!["T99_001.user".into(), "sorryAx".into()]];
        all(&good(), &oracle(), &g, &running, earned)
    });
    caught(
        "the cited test is not in the generated binary",
        all(&good(), &oracle(), &BTreeMap::new(), &running, earned),
    );
    caught(
        "a refusal that was never pinned",
        all(&good(), &oracle(), &generated(), &running, &[]),
    );
    caught(
        "a pinned refusal that was lost",
        all(
            &row(Status::Open, "", "", "", None),
            &oracle(),
            &generated(),
            &running,
            earned,
        ),
    );
    {
        let (a, b) = (good(), {
            let mut e = good();
            e.lean = "T99_001.twin".into();
            e
        });
        let rows = [("T99-001", &a), ("T99-001", &b)];
        caught(
            "two refused rows citing one test",
            refusal_citation_violations(&rows, &generated(), &running),
        );
    }
}

/// The scanner reads what a generated test states, not how it is laid out: the generator's one-line
/// fields, rustfmt's multi-line slices and a packed single line read the same, and a string's own
/// commas, brackets and escaped quotes never split a field.
#[test]
fn the_refusal_scanner_reads_every_layout() {
    let shape = || GeneratedRefusal {
        lean: "T06_014.bridgeShape".into(),
        axioms: vec!["Lean.trustCompiler".into(), "Lean.ofReduceBool".into()],
        taint: vec![
            vec![
                "T06_014.bridgeShape".into(),
                "Lean.reduceBool".into(),
                "Lean.trustCompiler".into(),
            ],
            vec!["T06_014.bridgeShape".into(), "Lean.ofReduceBool".into()],
        ],
    };
    let generator = r#"
#[test]
fn refusal_t06_014_bridge_shape() {
    check_refusal(&Refusal {
        fixture: "T06-014.bridgeShape",
        lean: "T06_014.bridgeShape",
        kind: "theorem",
        axioms: &["Lean.trustCompiler", "Lean.ofReduceBool"],
        taint: &[&["T06_014.bridgeShape", "Lean.reduceBool", "Lean.trustCompiler"], &["T06_014.bridgeShape", "Lean.ofReduceBool"]],
    });
}
"#;
    let rustfmt = r#"
#[test]
fn refusal_t06_014_bridge_shape() {
    check_refusal(&Refusal {
        fixture: "T06-014.bridgeShape",
        lean: "T06_014.bridgeShape",
        kind: "theorem",
        axioms: &["Lean.trustCompiler", "Lean.ofReduceBool"],
        taint: &[
            &[
                "T06_014.bridgeShape",
                "Lean.reduceBool",
                "Lean.trustCompiler",
            ],
            &["T06_014.bridgeShape", "Lean.ofReduceBool"],
        ],
    });
}
"#;
    let packed = r#"#[test] fn refusal_t06_014_bridge_shape() { check_refusal(&Refusal { fixture: "T06-014.bridgeShape", taint: &[ &[ "T06_014.bridgeShape", "Lean.reduceBool", "Lean.trustCompiler", ], &[ "T06_014.bridgeShape", "Lean.ofReduceBool" ] ], kind: "theorem", axioms: &[ "Lean.trustCompiler" , "Lean.ofReduceBool" ], lean: "T06_014.bridgeShape" }); }"#;
    for (layout, source) in [
        ("the generator's", generator),
        ("rustfmt's", rustfmt),
        ("a packed", packed),
    ] {
        assert_eq!(
            generated_refusals(source),
            (
                BTreeMap::from([("refusal_t06_014_bridge_shape".to_string(), shape())]),
                vec![]
            ),
            "{layout} layout reads as the test it states"
        );
    }

    let awkward = r#"
#[test]
fn refusal_t99_002_awkward() {
    check_refusal(&Refusal {
        lean: "T99_002.«a, [b] \"c, [d]\" e»",
        axioms: &["sorryAx"],
        taint: &[&[
            "T99_002.«a, [b] \"c, [d]\" e»",
            "sorryAx",
        ]],
        fixture: "T99-002.awkward",
        kind: "def",
    });
}
"#;
    assert_eq!(
        generated_refusals(awkward),
        (
            BTreeMap::from([(
                "refusal_t99_002_awkward".to_string(),
                GeneratedRefusal {
                    lean: "T99_002.«a, [b] \"c, [d]\" e»".into(),
                    axioms: vec!["sorryAx".into()],
                    taint: vec![vec![
                        "T99_002.«a, [b] \"c, [d]\" e»".into(),
                        "sorryAx".into()
                    ]],
                }
            )]),
            vec![]
        ),
        "a string's commas, brackets and escaped quotes belong to the string"
    );
}

/// A generated test the scanner cannot read is named with its reason, never dropped: the lock holds a
/// refused row to its test only through what the scanner reads, so a silent drop would hide the test.
#[test]
fn the_refusal_scanner_names_every_test_it_cannot_read() {
    let readable = r#"
#[test]
fn refusal_t99_001_user() {
    check_refusal(&Refusal {
        fixture: "T99-001.user",
        lean: "T99_001.user",
        kind: "theorem",
        axioms: &["sorryAx"],
        taint: &[&["T99_001.user", "sorryAx"]],
    });
}
"#;
    let broken = [
        (
            "no `taint` field",
            r#"lean: "T99_002.x", axioms: &["sorryAx"],"#,
        ),
        (
            "`taint` is not a `&[&[…], …]` of string literals",
            r#"lean: "T99_002.x", axioms: &["sorryAx"], taint: &["T99_002.x", "sorryAx"],"#,
        ),
        (
            "`axioms` is not a `&[…]` of string literals",
            r#"lean: "T99_002.x", axioms: &[sorryAx], taint: &[&["T99_002.x", "sorryAx"]],"#,
        ),
        (
            "`lean` is not one string literal",
            r#"lean: T99_002.x, axioms: &["sorryAx"], taint: &[&["T99_002.x", "sorryAx"]],"#,
        ),
        (
            "the field `lean` twice",
            r#"lean: "T99_002.x", lean: "T99_002.y", axioms: &["sorryAx"], taint: &[&["T99_002.x", "sorryAx"]],"#,
        ),
        (
            "an unopened `]`",
            r#"lean: "T99_002.x"], axioms: &["sorryAx"], taint: &[&["T99_002.x", "sorryAx"]],"#,
        ),
        (
            "is not a `key: value` field",
            r#"lean: "T99_002.x", axioms: &["sorryAx"], taint: &[&["T99_002.x", "sorryAx"]], "stray","#,
        ),
        (
            "is not a `key: value` field",
            r#"lean: "T99_002.x", axioms: &["sorryAx"], taint: &[&["T99_002.x", "sorryAx"]], "stray: field","#,
        ),
    ];
    for (why, fields) in broken {
        let source = format!(
            "{readable}\n#[test]\nfn refusal_t99_002_x() {{\n    check_refusal(&Refusal {{ {fields} }});\n}}\n"
        );
        let (read, unreadable) = generated_refusals(&source);
        assert_eq!(
            read.keys().collect::<Vec<_>>(),
            ["refusal_t99_001_user"],
            "{why}: the readable neighbour still reads, the broken test does not"
        );
        assert!(
            matches!(unreadable.as_slice(), [one] if one.starts_with(&format!("{REFUSAL_BINARY}::refusal_t99_002_x: ")) && one.contains(why)),
            "{why}: named with its reason, got {unreadable:?}"
        );
        assert_eq!(
            refusal_binary_violations(&source, &[], &|_| Ok(())),
            unreadable,
            "{why}: the live check fails on the test it cannot read"
        );
    }

    for (why, source) in [
        (
            "the `Refusal { … }` literal never closes",
            "#[test]\nfn refusal_t99_003_open() {\n    check_refusal(&Refusal {\n        lean: \"T99_003.open\",\n",
        ),
        (
            "no `Refusal { … }` literal",
            "#[test]\nfn refusal_t99_004_none() {\n    check_refusal(&OTHER);\n}\n",
        ),
    ] {
        let (read, unreadable) = generated_refusals(source);
        assert!(read.is_empty(), "{why}: nothing reads");
        assert!(
            matches!(unreadable.as_slice(), [one] if one.contains(why)),
            "{why}: named with its reason, got {unreadable:?}"
        );
    }
}

// ============================================================================================
// THE RATCHET
// ============================================================================================

#[test]
fn the_open_count_only_falls() {
    let open = all_entries().filter(|e| e.status == Status::Open).count();
    assert!(
        open <= OPEN_CEILING,
        "OPEN rose to {open}, ceiling is {OPEN_CEILING}. Lower the ceiling as you bridge; never \
         raise it to go green — that converts \"we cannot do this\" into \"we no longer track that \
         we cannot do this\"."
    );
    if open < OPEN_CEILING {
        eprintln!(
            "\n  ✓ OPEN is {open}, ceiling {OPEN_CEILING} — lower OPEN_CEILING to {open} to bank it.\n"
        );
    }
}

// ============================================================================================
// THE WORKLIST — this is the self-driving part. `-- --nocapture` to read it.
// ============================================================================================

#[test]
fn worklist() {
    let entries: Vec<&BridgeEntry> = all_entries().collect();
    let total = entries.len();
    let open: Vec<&&BridgeEntry> = entries
        .iter()
        .filter(|e| e.status == Status::Open)
        .collect();
    let bridged = entries
        .iter()
        .filter(|e| e.status == Status::Bridged)
        .count();
    let refused = entries
        .iter()
        .filter(|e| e.status == Status::Refused)
        .count();
    let impossible = entries
        .iter()
        .filter(|e| e.status == Status::Impossible)
        .count();

    let mut by_blocker: BTreeMap<Blocker, Vec<&BridgeEntry>> = BTreeMap::new();
    for e in &open {
        by_blocker.entry(Blocker::of(e)).or_default().push(e);
    }
    let mut ranked: Vec<(&Blocker, &Vec<&BridgeEntry>)> = by_blocker.iter().collect();
    ranked.sort_by_key(|(_, v)| std::cmp::Reverse(v.len()));

    eprintln!("\n  ══ LEAN BRIDGE ══  show me the LOGOS that proves this Lean");
    eprintln!(
        "  Lean {LEAN_VERSION}, {} cases, {total} authored declarations",
        truth().len()
    );
    eprintln!(
        "  bridged {bridged} · refused {refused} · impossible {impossible} · OPEN {}",
        open.len()
    );
    let mut bridged_by_kind: BTreeMap<&str, usize> = BTreeMap::new();
    for e in entries.iter().filter(|e| e.status == Status::Bridged) {
        *bridged_by_kind.entry(e.lean_kind.as_str()).or_default() += 1;
    }
    eprintln!(
        "  bridged by kind (definitions and theorems counted separately): {bridged_by_kind:?}"
    );
    eprintln!("\n  what is blocking, ranked by how many declarations it unlocks:");
    for (b, ds) in &ranked {
        eprintln!("    {:>5}  {}", ds.len(), b.label());
    }

    if let Some((top, ds)) = ranked.first() {
        eprintln!(
            "\n  biggest lever — {} ({} declarations). First ten:",
            top.label(),
            ds.len()
        );
        for e in ds.iter().take(10) {
            let lv = if e.lean_levels.is_empty() {
                String::new()
            } else {
                format!(".{{{}}}", e.lean_levels.join(","))
            };
            let ty = e.lean_type.replace('\n', " ");
            let ty = if ty.len() > 88 {
                format!("{}…", &ty[..88])
            } else {
                ty
            };
            eprintln!("    {}{lv}  [{}]  {ty}", e.lean, e.lean_kind);
        }
    }
    eprintln!();
}
