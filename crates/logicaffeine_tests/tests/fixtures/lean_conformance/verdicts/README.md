# Capability verdicts — one file per corpus tier

`lean_parity_matrix.rs` reads `manifest.json` (all 248 corpus cases) and every `t*.json` in this
directory, and fails unless the two agree exactly. One file per tier means thirteen agents can
write verdicts in parallel without ever touching the same file.

## Why our verdicts are not the corpus's

The corpus tags each case `translate` / `degrade` / `reject`. Those are **translator** verdicts,
and a translator that refuses 77 of 248 cases is perfect. We are a **competitor**: every refusal
is mathematics Lean can express and we cannot. So each case is re-tagged for what *we* owe.

## Schema

```json
{
  "tier": "t00_terms",
  "test_binary": "lean_conformance_t00_terms",
  "cases": [
    {
      "id": "T00-001",
      "verdict": "MUST",
      "status": "Gap",
      "test": "de_bruijn_lowers_to_a_named_var_with_freshening",
      "why": ""
    }
  ]
}
```

| Field | Meaning |
|---|---|
| `tier` | the corpus directory name, exactly (`crates/logicaffeine_tests/tests/fixtures/lean_conformance/cases/<tier>/`) |
| `test_binary` | the test file that decides this tier, without `.rs` |
| `id` | the corpus case id, exactly as `manifest.json` spells it |
| `verdict` | `MUST` · `DIVERGE` · `OUT` — see below |
| `status` | `Native` · `Gap` · `Refused` — where we stand **today** |
| `test` | the `#[test]` fn in `test_binary` that decides this case |
| `why` | the argument. **Required for `DIVERGE` and `OUT`**, ignored for `MUST` |

### `verdict` — what we owe

- **`MUST`** — Lean can do it and we should too. The test asserts the capability and fails today
  if `status` is `Gap`. This is the work queue.
- **`DIVERGE`** — we deliberately differ and ours is defensible or better. The test asserts **our**
  behaviour, and `why` carries the argument. *A `DIVERGE` whose argument cannot be written in one
  paragraph is a `MUST` in disguise* — the matrix records the argument precisely so this is
  reviewable rather than a place to hide.
- **`OUT`** — genuinely out of scope (replaying arbitrary `MetaM`, `IO`/`do` notation, environment
  extensions). The test asserts we **refuse cleanly**; never that we half-work.

### `status` — where we stand today

- **`Native`** — works now, and the test proves it. Counts toward the tier's floor.
- **`Gap`** — we cannot do it yet. The test is RED and names the work.
- **`Refused`** — we decline, deliberately and loudly. Correct for `OUT`, and for a `MUST` we have
  decided to refuse rather than answer wrongly (an unsound answer is worse than no answer).

`Native` is the only status that counts toward `TIER_FLOORS` in `lean_parity_matrix.rs`. Those
floors may only rise. Raising one is a deliberate edit by a human, with the wave that earned it
named in a comment — copy the style of `parity_catalog_lock.rs`.

## Rules the lock enforces

1. Every one of the 248 corpus cases has exactly one verdict. No case classified twice, none left out.
2. No verdict names a case that is not in the corpus.
3. Every case names the test that decides it.
4. Every `DIVERGE` and `OUT` states its argument.
5. Each tier file's `tier` matches a real corpus directory, and its case count matches that
   directory's.
6. Per-tier `Native` counts never fall below `TIER_FLOORS`.

An empty directory is the correct starting state: 248 unclassified, the lock RED, and the number
of cases we owe a verdict on is visible rather than assumed.
