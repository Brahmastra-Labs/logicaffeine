# Competition Discoveries — what world-class guides do that ours doesn't (yet)

Cross-referenced from `COMPETITION_GUIDES/` against the LOGOS Syntax Guide
(`apps/logicaffeine_web/src/ui/pages/guide/`, 25 sections / 92 examples). These are
**recommendations**, not changes — guide structure/prose edits await your sign-off (see
`SYNTAX_GUIDE_WORK.md`). Ordered by impact.

## What our guide already does well (keep)

- **Every example is editable + runnable in-browser** via `GuideCodeBlock` (WASM interpreter / FOL
  compile). This is the Go-Tour / Gleam-Tour superpower, and we already have it. Lean into it harder.
- **Dual mode** (Imperative → run, Logic → FOL) is genuinely novel; no comparable guide has it.
- **Sticky sidebar + scroll-spy + collapsible example groups** — solid navigation ergonomics.
- **"How to Read This Guide"** splits novice vs experienced paths up front (Gleam only does the latter).

## High impact

### 1. A goal-indexed "Recipe Book" axis (Inform 7)
Inform ships **two** books: a feature tutorial *and* a goal-organized cookbook ("how do I model money /
conversation / liquids"). For a **natural-language** language this matters most — users arrive with a
goal phrased in English, not a feature name. Our guide is feature-organized only.
**Suggestion:** a "Recipes / How do I…?" section (or sibling page) — "read a file and count lines",
"a CLI that takes args", "a REST-ish request handler", "a shared counter across peers", each a complete
runnable program. Reuses the existing example engine; additive, no language work.

### 2. Build-one-complete-program tutorial (Rust ch.2/12/20)
The Rust Book teaches a *whole* Guessing Game in chapter 2 before any reference, then a grep clone, then
a web server. Our §24 "Complete Examples" is only factorial/fib/filter snippets — and its prose even
promises a Mergesort / "Working with Structs" / "Collection Processing" example that **don't exist**
(logged in `SYNTAX_GUIDE_WORK.md`). **Suggestion:** one narrated end-to-end build early (a tiny game or
a todo CLI), assembled step by step from the features as they're introduced.

### 3. Exercises with self-check (Go Tour)
Go's tour makes you *write* code ("Exercise: Slices", "Exercise: Web Crawler") and verifies it. Our
examples are passive (run-only). We already have a grader on the Learn page — the machinery exists.
**Suggestion:** end key sections with a "Now you try" challenge that runs the user's code and checks
output, reusing the Learn grader.

## Medium impact

### 4. Exhaustive reference appendices (Rust appendices A–E)
Rust has full tables of **every keyword**, **every operator/symbol**, derivable traits, and dev tools.
Our Quick Reference (§25) is good but partial — no single exhaustive keyword/operator index, and no
catalogue of the Socratic **error messages** (which are a selling point — document them as a feature).
**Suggestion:** an appendix with a complete keyword table, an operator/English-synonym table (we have
`plus`/`+`, `is at most`/`<=` etc. — make it exhaustive), and an error-message gallery.

### 5. Inline comprehension checks (Brown U. Rust Book)
The Brown edition inserts **quizzes** after sections and **visualizes ownership**. We have none.
**Suggestion:** a lightweight "predict the output" toggle on some examples (hide output, ask, reveal) —
cheap given the run engine, and it turns reading into active recall. A LOGOS analogue to ownership
visualization: show the FOL/AST side-by-side for an Imperative snippet (we already render AST in Studio).

### 6. Document the natural-language design principles (Inform 7 "least surprise")
Inform treats "the phrasing you guess is the phrasing that works" as an explicit, documented tenet, and
teaches the exceptions. LOGOS has known surface footguns (see the `logos-sugaring-campaign` notes:
1-based bracket indexing, `is` vs `equals`, possessive `'s`, no map literal). **Suggestion:** a short
"How LOGOS reads English" section stating the design rules and the deliberate exceptions, so users build
the right mental model instead of hitting ambiguities blind.

## Lower impact / ergonomics

### 7. In-guide search + "everything on one page" view (Rust `S`-to-search; Gleam single-page)
We have a sidebar but (as far as the audit saw) no in-guide full-text search, and no single-page view for
Ctrl-F power users. **Suggestion:** a client-side search over `SECTIONS`, and/or an "expand all" view.

### 8. More examples per feature (Inform's ~500 vs our ~92)
Several documented features have **zero** examples: `Inspect`/`When` pattern matching, `Pop`, set/map
edge cases. (Tracked in `SYNTAX_GUIDE_WORK.md`.) **Suggestion:** at minimum one runnable example per
documented construct — the harness (`guide_examples.rs`) already guarantees they'd be kept honest.

---

_Source notes for each guide live in `COMPETITION_GUIDES/`. Sources: The Rust Book
(doc.rust-lang.org/book), A Tour of Go (go.dev/tour), Gleam Tour (tour.gleam.run), Inform 7
(ganelson.github.io/inform-website)._
