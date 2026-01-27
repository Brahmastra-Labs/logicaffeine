# Homepage Enhancement

**Branch:** `homepage-redesign-collin`
**Status:** Implemented
**File:** `src/ui/pages/landing.rs`

---

## What Changed

| Component | Before | After |
|-----------|--------|-------|
| Hero subheadline | "Turn everyday English into rigorous First-Order Logic…" | "Write Code, Logic, and Math in plain English. LOGOS compiles your words into programs, proofs, and formal systems — no symbols required." |
| Hero KPI pills | Plain English in / Formal logic out / Zero guesswork | Code in English / Logic in English / Math in English |
| Hero microcopy | "Built for people who take thinking seriously: students, researchers…" | "Students, engineers, researchers, and attorneys — anyone who thinks for a living." |
| Hero demo | Static 2-column English → FOL | Interactive mini-studio with mode toggle, file explorer, 5s auto-cycling |
| How It Works | 3 generic steps | 3 mode user stories with input→output demos |
| What You Get cards 1 & 4 | Logic-only descriptions | Multi-mode descriptions |
| New section | — | Security & Policies (4 cards + code demo) |
| Comparison table | 4 columns | 5 columns (added Elixir) |
| Bottom CTA | "Start with the Curriculum, or jump into the Studio." | "Start with the Curriculum, or explore any mode in the Studio. Code, Logic, Math — your call." |
| Footer | © 2025 | © 2026 |

## What Stayed

- Headline: "Debug Your Thoughts."
- Badge, CTAs, tech-stack badges
- Hello World, Who Uses, FAQ, comparison structure
- Background orbs and animations

---

## Mini-Studio

Replaces the static hero demo with an interactive panel that cycles through curated examples per mode.

### Layout

```
┌─────────────────────────────────────────────┐
│ 🔴🟡🟢          [λ Code] [∀ Logic] [π Math] │  header + mode toggle
├──────────────┬──────────────────────────────┤
│ FILES        │ hello-world.logos        λ   │
│              │                              │
│ ● hello…    │ ## Main                      │
│   fibonacci… │                              │
│   counters…  │ Let greeting be "Hello…".    │
│   tally…     │ Show greeting.              │
│   client…    │ …                           │
├──────────────┴──────────────────────────────┤
│ Your ideas, any mode. Try it in the Studio. │  footer
└─────────────────────────────────────────────┘
```

### Files Per Mode

| Mode | Files |
|------|-------|
| Code (6) | hello-world.logos, fibonacci.logos, counters.logos, tally.logos, client.logos, policies.logos |
| Logic (4) | simple-sentences.logic, quantifiers.logic, prover-demo.logic, modus-tollens.logic |
| Math (3) | natural-numbers.logos, boolean-logic.logos, prop-logic.logos |

### Behavior

- Starts in Code mode, first file selected
- Auto-cycles every 5 seconds with blue highlight on active file
- Hover pauses cycling, mouse leave resumes
- Click selects file and pauses cycling
- Mode switch resets to first file and restarts timer
- Mobile: hides file explorer at 980px, toggle labels at 768px

### Footer Copy

"Your ideas, any mode. Try it in the Studio."

### Implementation

**Data:** `DemoExample` struct with `filename`, `icon`, `content` fields. Three const arrays (`CODE_DEMO_EXAMPLES`, `LOGIC_DEMO_EXAMPLES`, `MATH_DEMO_EXAMPLES`) hold trimmed example content (~10-15 lines each). Helper `examples_for_mode()` returns the slice for a given `StudioMode`.

**State signals:**
- `demo_mode: Signal<StudioMode>` — active mode
- `active_index: Signal<usize>` — selected file index
- `cycling_paused: Signal<bool>` — hover/click pause state
- `timer_started: Signal<bool>` — ensures timer spawns once

**Timer:** `use_effect` + `spawn` with `gloo_timers::future::TimeoutFuture::new(5_000)` in an async loop, guarded by `#[cfg(target_arch = "wasm32")]`.

**CSS classes:** `.mini-studio`, `.mini-header`, `.mini-mode-toggle`, `.mini-toggle-btn`, `.mini-explorer`, `.mini-file-item`, `.mini-code-panel`, `.mini-code-content`, `.mini-footer`

---

## Mode User Stories (How It Works)

Three cards replacing the previous 3 generic steps. Each card has a mode icon, title, mini input→output demo, and description.

### Code (λ) — "Write a program"

```
Input:  Let x be 10. Show x + 5.
Output: 15
```

"Type readable definitions. Get compiled programs — Rust under the hood, English on the surface."

### Logic (∀) — "Formalize an argument"

```
Input:  Every cat sleeps.
Output: ∀x(Cat(x) → Sleep(x))
```

"Turn plain language into First-Order Logic. Every reading surfaced — no guessing."

### Math (π) — "Prove a theorem"

```
Input:  Theorem: ∀n, n + 0 = n.
Output: Proof: by induction. ✓
```

"Define types, state theorems, and prove them with automated tactics."

**CSS classes:** `.mode-stories`, `.mode-story-card`, `.story-header`, `.story-demo`, `.story-io`

---

## Security & Policies Section

New section after "What you get."

**Title:** "Security & Policies"
**Subtitle:** "Capability-based security with policy blocks. Define who can do what in plain English."

### Feature Cards

| Card | Icon | Title | Copy |
|------|------|-------|------|
| 1 | Shield | Policy Blocks | Define security rules as readable policy sections. Who can access what — stated plainly. |
| 2 | Lock | Capabilities | Role-based access control expressed in English. No annotation soup. |
| 3 | Beaker | Check Guards | Runtime guard checks that enforce your policies. "Check that the user is admin." |
| 4 | Brain | Predicates | Define custom predicates: "A User is admin if the user's role equals 'admin'." |

### Code Demo

```
## Policy                              fn can_publish(
A User can publish the Document    →       user: &User,
    if user's role equals "editor".        doc: &Document
                                       ) -> bool
```

**CSS classes:** `.security-demo`, `.security-cards`, `.security-card`, `.security-code-demo`, `.demo-side`

---

## Comparison Table

| Feature | LOGOS | Lean 4 | Rust | Python | Elixir |
|---------|-------|--------|------|--------|--------|
| Syntax | English prose | Lean DSL | Symbols | Symbols | Symbols |
| File Format | Markdown (.md) | .lean | .rs | .py | .ex |
| Performance | Native (via Rust) | Native | Native | Interpreted | BEAM VM |
| Proofs | Built-in | Required | Optional | None | None |
| Memory | Ownership (English) | GC | Ownership | GC | GC |

Grid uses `repeat(5, 1fr)` for the 5 comparison columns (6 total including feature label). Mobile hides the 6th column (Elixir) below 768px.

---

## Pricing Page: Feature Showcase

**File:** `src/ui/pages/pricing.rs`

New section inserted between the free license banner and the lifetime section on the pricing page. Highlights everything LOGOS ships with, organized into 6 category groups.

### Placement

1. Active license banner (if applicable)
2. Pricing header
3. Free license banner
4. **Feature showcase section**
5. Lifetime section
6. Paid tier cards
7. Rest of page

### Layout

- Glass card container (`.features-showcase`) with centered header
- 3-column grid (`.features-grid`), collapses to 1 column at 700px
- 6 category cards (`.feature-group`) with staggered `fadeInUp` animations
- Each card has a colored icon box, title, and 5 checkmark bullets

### Categories

| # | Title | Icon | Color |
|---|-------|------|-------|
| 1 | English-Native Programming | Lightning | Cyan (#00d4ff) |
| 2 | Formal Logic & Semantics | Brain | Indigo (#818cf8) |
| 3 | Type Theory & Proofs | Beaker | Green (#22c55e) |
| 4 | Distributed Systems & CRDTs | Sparkles | Amber (#fbbf24) |
| 5 | Security & Concurrency | Shield | Pink (#ec4899) |
| 6 | Verification & Tooling | Lock | Purple (#8b5cf6) |

### CSS Classes

- `.features-showcase` — section container, glass card background
- `.features-showcase-header` — centered title and subtitle
- `.features-grid` — 3-column grid layout
- `.feature-group` — individual category card with hover effect
- `.feature-group-header` — icon + title row
- `.feature-group-icon` — colored icon box (inline `style` per card)
- `.feature-group-list` — checkmark bullet list, matches `.tier-features` styling
