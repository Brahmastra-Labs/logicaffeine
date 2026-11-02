# Infinite scroll — parked pending G27

Loads a page of todos, then loads the next page automatically whenever the
visitor scrolls to the bottom, until the server says there is nothing more.

```bash
largo test    # runs the ## Test block against the tree-walker and the VM — GREEN
largo check   # fails — see G27 below
```

## Status: parked in `work/lgx-ease/gauntlet-scratch/`, not in `examples/`

`largo test` is fully green (`vm_agreed=3`, every test passes on both the
tree-walker and the lowered bytecode VM). `largo check`/`largo build
--static` — the WASM AOT path a real browser needs — fails with an unlocated
"cannot instantiate a written type: a type the checker could not determine."

## What it shows (on the tiers that work)

**`When the user scrolls to the bottom:`**, driven in tests by the
UNDOCUMENTED `Raise "scrolledtobottom".` (`docs/frontend.md`'s Testing
reference lists no drive verb for it at all — found only by reading
`crates/logicaffeine_tests/tests/lgx_vm_parity_lock.rs`'s own
`scrolling_to_the_bottom_reaches_its_handler_on_both_tiers`). Recorded as
finding **G26** (a documentation gap, no RED test — the mechanism works once
found).

**A derived `Int` counting the loaded rows**, `keyed by` avoided only because
this list is append-only (nothing is ever reordered or removed) — every
`{for each}` loop is a plain, non-derived `todos` field, never a derived one
or a direct call, per G6.

## Limits

**`docs/frontend.md`'s own canonical Fetching example — copied byte for
byte, confirmed independently at the `compile_to_wasm` API level, bypassing
the CLI entirely — fails to compile for the browser.** `examples/
fullstack_todo`'s own, more elaborate typed-fetch page compiles cleanly with
the identical `Result of Seq of <record> and HttpError` shape, so typed
fetch itself is not broken outright. Careful ONE-CHANGE-AT-A-TIME
bisection, starting from `fullstack_todo`'s own working file and never
reconstructing by hand, found no single ingredient whose removal alone
explains the gap — the SAME reduced content flipped between compiling and
failing at different points while this lane was working, with no rebuild of
the `largo` binary in between. Consistent with the checker area itself
(`analysis/{check,unify}.rs`'s generic/`InferType::Instance` resolution)
being live, in-progress work per `work/lgx-ease/LANES.md`'s grant to the
release-prep session — recorded rather than root-caused, since it is not
this lane's file. Finding **G27** in `work/lgx-ease/gauntlet2.md`; RED test
`gauntlet_findings_lock::the_canonical_fetching_example_compiles_for_the_browser`.

**A record type reached through `Import` (rather than declared in the same
file) never gets its JSON-decode codec generated for a typed fetch — "Unknown
function: `fromJsonSeq_<Type>`" — on the reference tree-walker itself.**
Confirmed by isolating the ONE change (`Import Shared.` naming an external
`## A Todo has:` vs. the identical declaration written inline in `pages/
app.lg`): inline compiles and runs; imported fails with the missing
function, even though `examples/fullstack_todo` also uses `Import Shared.`
successfully — the working case there also declares a `## Route` returning
that same `Seq of <record>` type, which this app (frontend-only, no
backend) does not. Recorded as finding **G28** (a documentation/behavior
gap — no RED test: reproducing it needs the multi-file `Import`/loader
machinery, which the single-source-string test APIs this lane's RED tests
use do not exercise). Worked around here by declaring `## A Todo has:`
inline in `pages/app.lg` rather than importing it.
