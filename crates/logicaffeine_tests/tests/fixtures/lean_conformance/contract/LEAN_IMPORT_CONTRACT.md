# Lean → LOGOS import contract (draft 2, 2026-09-23)

Owner: LOGOS maintainers. Consumers: `largo lean`, the MaCSad worker (W08) and frontend (W09).
This is the LOGOS side of the Lean import contract. Every field listed here has a consumer; nothing is added
"because a workflow framework usually has one". Machine-checked by `lean_contract_lock.rs` against the fixtures in
`crates/logicaffeine_tests/tests/fixtures/lean_conformance/contract/fixtures/`.

## 1. Inputs

| Mode | Command | Status |
|---|---|---|
| Pinned Lean project + target | `largo lean import --project <dir> --target <Qualified.Name> --out <dir>` | public experience |
| Pre-exported kernel artifact | `largo lean import <export.ndjson> --target <Qualified.Name> --out <dir>` | advanced interchange |
| Benchmark compatibility | `largo lean import <export.ndjson> --arena-protocol` | prints `ACCEPT`, exit 0/1/2, unchanged |

Supported profile (Stage A): Lean `4.14.0`, lean4export `20c4a64a`, export format `3.1.0`, no Lake dependencies beyond
Lean core, allowed axioms `propext`, `Quot.sound`, `Classical.choice`. A project is compiled in isolation; the
target's declaration/type/dependency closure is resolved and `#print axioms` decides taint — exit code 0 alone never
establishes freedom from `sorry`.

Compatibility rule: an unknown export format version, an unknown mandatory record field or an unknown record kind is
refused with a distinct code. Unknown optional fields in *our* JSON are refused on read (`deny_unknown_fields`);
additions bump the schema version.

## 2. Result (`logos-lean-result/1`, stdout, one JSON document)

```
{ "schema": "logos-lean-result/1",
  "outcome": "verified" | "invalid_source" | "unsupported_feature" | "disallowed_assumptions"
           | "resource_limit" | "infrastructure_failure",
  "stages": { "lean_accepted": S, "translated": S, "primary_check": S, "independent_recheck": S,
              "assumption_policy": S, "emission_reparsed": S, "fresh_replay": S },
  "target": { "lean": "Qualified.Name", "logos": "Qualified.Name" },
  "assumptions": ["propext", ...],
  "information_losses": [ { "category": "...", "detail": "..." } ],
  "diagnostics": [ { "code": "...", "stage": "...", "declaration": "..."|null, "line": n|null, "message": "..." } ],
  "audit": { "schema": "logos-lean-import-report/1", ... },
  "package": { "path": "...", "manifest_sha256": "..." } | null }
```

`audit` is the importer's own report (`logos-lean-import-report/1`): internal helpers, quarantined runtime
declarations, the declaration audit with each declaration's transitive axioms, and metadata losses.

**Only the project mode can be `verified`.** Lean acceptance is a stage: a pre-exported artifact was not observed
being accepted by Lean in this run, so `lean_accepted` is `not_run`, the outcome is `unsupported_feature` with
code `lean_acceptance_not_observed`, and no package is published. A pre-exported artifact can still be imported and
checked (every other stage runs and is reported); it cannot be labelled verified, because nothing attests that its
source was valid Lean.

`S` is `"passed" | "failed" | "not_run" | "unsupported"`. `verified` requires every stage `passed`; if the
independent recheck cannot run, the stage says `unsupported` and the outcome is not `verified`.
Diagnostics are written to stderr as NDJSON; stdout carries only the result.

Exit codes: `0` verified · `1` invalid source (a real rejection) · `2` unsupported feature · `3` disallowed
assumptions · `4` resource limit · `5` infrastructure failure · `64` usage.

A failure of LOGOS's own artifact after the user's input passed every check (the assembled package does not
replay, the emitted document does not reparse) is `internal_inconsistency` → `infrastructure_failure`, never
`invalid_source`: the user's mathematics is never blamed for our defect.

## 3. Proof package (`logos-lean-package/1`, a directory)

```
manifest.json          schema, result id, sha256 of every file, source identity, target (Lean name + type,
                       LOGOS name + type), toolchain (LOGOS version, prelude `logos-coc-4`, Lean, exporter),
                       transitive assumptions of statement and proof, information losses, receipts
<Target>.lg            readable LOGOS: literate document, reparsed and rechecked before publication
certificate.json       replayable machine representation (EnvironmentCertificate)
correspondence.json    Lean declaration/span ↔ LOGOS declaration/span; generated helpers with origin
source/export.ndjson   the exact export the result was produced from
source/*.lean          original source when the project mode was used
```

Published atomically: built in a staging directory beside `--out`, digested, replayed by a new `largo` process,
and renamed into place only after every stage passed. A failed run publishes nothing. An earlier package at the
same path is moved aside to `<name>.superseded-<digest>` (kept, never under the failed run's name); a
non-package directory at `--out` is refused and left untouched. The `.lg` is one file of the package, digested in
its manifest and never written separately, so a document cannot exist without the package that verifies it.

## 4. Replay

`largo lean replay <package>` runs in a fresh process with no importer state: digest and limit checks before any
work, environment rebuilt with every declaration kernel-validated, primary check, independent recheck, exact
conversion (not cumulative subtyping), statement binding against an independent reference translation of the
exported Lean type, assumption policy. It prints a `logos-lean-replay/1` receipt naming each stage.

A hash identifies bytes; it does not prove semantic correspondence. The trusted computing base (kernel term
representation shared by both checkers) is documented in the manifest.
