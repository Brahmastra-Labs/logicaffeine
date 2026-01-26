# Homepage Redesign & Product Tour Proposal

**Branch:** `homepage-redesign-collin`
**Status:** Draft — Awaiting Approval
**Author:** Collin Pounds
**Date:** 2026-01-26

---

## Table of Contents

1. [Executive Summary](#executive-summary)
2. [Problem Statement](#problem-statement)
3. [Current State Analysis](#current-state-analysis)
4. [Positioning Strategy](#positioning-strategy)
5. [Homepage Copy — All Sections](#homepage-copy)
6. [Product Tour Script](#product-tour-script)
7. [Architecture & Component Plan](#architecture--component-plan)
8. [Implementation Plan](#implementation-plan)
9. [Success Metrics](#success-metrics)
10. [Open Questions](#open-questions)

---

## Executive Summary

The current homepage positions LOGOS as an educational logic tool ("Debug Your Thoughts"). This undersells the product. CRDTs, P2P mesh networking, and the proof engine are fully functional in the codebase but invisible on the site.

This proposal replaces the homepage and adds a 7-step product tour. The new positioning: **LOGOS is a DSL for distributed systems with readable syntax that compiles to production Rust.** Lead with distributed state, not "English programming."

### What Changes

| Component | Current | Proposed |
|-----------|---------|----------|
| Hero headline | "Debug Your Thoughts." | Problem-oriented distributed systems headline |
| Hero subheadline | Logic translation focus | Concrete distributed systems value prop |
| Hero CTAs | Start Learning / Open Studio / See Pricing | Try in Browser / See Generated Rust |
| Hero demo | English → FOL transpilation | Distributed counter with live Rust output |
| Trust block | None | 2227+ tests, libp2p, tokio, rayon badges |
| Feature blocks | 6 generic cards (Transpilation, Tutor, etc.) | 5 problem→solution blocks with code |
| How It Works | 3 vague steps | Pipeline diagram (Lexer→Parser→AST→Codegen) |
| Code examples | Hello World only | CRDT counter, scope ambiguity, compile-time proof |
| Comparison table | vs Lean 4, Rust, Python | Retooled: vs Rust+libp2p, Elixir/OTP, Automerge |
| Who section | Students, Law, Engineering | Removed (audience implied by content) |
| FAQ | 6 generic questions | 6 developer-specific questions addressing skepticism |
| Product tour | None | 7-step spotlight tour in Studio |
| Visual: gradient orbs | Animated blurs on landing | Removed site-wide |
| Visual: typography | System sans-serif everywhere | Inter Display for headlines, system sans for body |
| Visual: color | Blue-to-purple gradient on buttons/badges/cards | Single accent `#818cf8`, no gradients on UI elements |
| Visual: spacing | 18px gaps, 18px card padding, 74px section padding | 28px gaps, 28px card padding, 96px section padding |
| Visual: borders | `rgba(255,255,255,0.10)` | Softened to `0.08`, cards to `0.02` background |
| Visual: motion | Decorative orb float/pulse | Functional: hero type-on, code glow, scroll reveals, tab slides |

### What Stays

- Dioxus/Rust framework (no framework changes)
- Existing component library (reuse cards, demos, icons)
- Dark theme (gradient orbs removed — see below)
- Responsive breakpoints (980px, 768px, 640px)
- MainNav and Footer components
- CSS custom properties and design tokens
- SEO schemas (updated content)

### Visual Overhaul: Design Upgrades

This redesign includes five visual upgrades alongside the content rewrite. These are scoped to CSS changes and minor RSX adjustments — no framework or build changes.

#### 1. Remove Gradient Orbs (site-wide)

The animated gradient orbs (`.bg-orb`, `.orb1`, `.orb2`, `.orb3`) are removed from the landing page and all other pages. The floating blur animations add visual noise without communicating product value and compete with code demos for attention. The dark background stays (subtle radial gradients baked into the `.landing` background property), but the animated overlay orbs are cut.

**Scope:** Remove the three `div { class: "bg-orb orb*" }` elements from `landing.rs`. Remove `.bg-orb`, `.orb1`, `.orb2`, `.orb3`, and the `float` / `pulse-glow` keyframes from CSS. Audit all other pages for orb usage.

#### 2. Typography: Display Typeface for Headlines

Add a distinctive display typeface for headlines only. Body text stays system sans-serif. Monospace stays `SF Mono` / `Fira Code`.

**Font:** Inter Display (variable, WOFF2). Self-hosted for performance — no Google Fonts dependency.

**Where it applies:**
- `.h-title` (hero headline)
- `.section-title` (all section headings)
- `.card h3` (feature block titles)
- `.faq-q` (FAQ questions)
- Tour modal headlines

**CSS changes:**
```css
@font-face {
  font-family: 'Inter Display';
  src: url('/assets/fonts/InterDisplay-Variable.woff2') format('woff2');
  font-weight: 400 900;
  font-display: swap;
}

:root {
  --font-display: 'Inter Display', var(--font-sans);
}

.h-title,
.section-title,
.card h3,
.faq-q {
  font-family: var(--font-display);
}
```

**Asset:** Download Inter Display variable WOFF2 (~100KB). Place at `assets/fonts/InterDisplay-Variable.woff2`. Preload in `index.html` for zero FOIT:
```html
<link rel="preload" href="/assets/fonts/InterDisplay-Variable.woff2"
      as="font" type="font/woff2" crossorigin>
```

#### 3. One Accent Color, Not a Gradient

Replace the blue-to-purple gradient used on buttons, badges, step numbers, and card hovers with a single strong accent: **`#818cf8`** (the existing purple). Blue (`#00d4ff`) becomes a secondary highlight used only for inline code tokens and the LOGOS column in the comparison table.

**What changes:**

| Element | Current | New |
|---------|---------|-----|
| `.btn-primary` | `linear-gradient(135deg, rgba(96,165,250,0.95), rgba(167,139,250,0.95))` | `background: #818cf8` |
| `.btn-primary:hover` | Gradient | `background: #9ba3fb` (lighter) |
| `.btn-primary` box-shadow | `rgba(96,165,250,0.18)` | `rgba(129,140,248,0.25)` |
| `.step-num` | `linear-gradient(135deg, blue, purple)` | `background: #818cf8` |
| `.card:hover` border | `rgba(167,139,250,0.28)` | `rgba(129,140,248,0.3)` |
| `.card::before` hover gradient | `linear-gradient(135deg, blue/12%, purple/12%)` | `background: rgba(129,140,248,0.08)` (flat) |
| `.compare-cell.highlight` | `rgba(167,139,250,0.08)` | `rgba(129,140,248,0.10)` |
| KPI pill active state | None | Subtle `#818cf8` left border |
| `.badge .dot` | `var(--color-success)` green | Keep green (status indicator, not accent) |

**New CSS variable:**
```css
:root {
  --color-accent: #818cf8;
  --color-accent-hover: #9ba3fb;
  --color-accent-subtle: rgba(129, 140, 248, 0.10);
  --color-accent-glow: rgba(129, 140, 248, 0.25);
}
```

#### 4. Breathing Room: Spacing Increase

Increase spacing throughout to create visual confidence. Every section gets more vertical padding, cards get more internal padding, and gaps widen.

**Changes:**

| Token | Current | New |
|-------|---------|-----|
| `.section` padding | `74px 0` | `96px 0` |
| `.card` padding | `18px` | `28px` |
| `.grid3` / `.grid2` gap | `18px` | `28px` |
| `.hero` padding | `84px 0 30px` | `96px 0 48px` |
| `.container` max-width | `1120px` | `1120px` (unchanged) |
| `.h-sub` max-width | `580px` | `600px` |
| `.section-sub` max-width | `760px` | `760px` (unchanged) |
| `.faq-item` padding | `18px 18px 14px` | `24px 24px 20px` |
| `.demo-col` padding | `18px 18px 22px` | `24px 24px 28px` |
| `.demo-col` min-height | `240px` | `280px` |
| `.hero-ctas` gap | `12px` | `16px` |
| `.kpi` gap | `14px` | `16px` |

These are pure CSS token adjustments. No RSX changes needed.

#### 5. Functional Motion

Replace decorative animation (orbs) with purposeful motion tied to content comprehension.

**A. Hero Code Type-On**

The hero demo's left column (LOGOS source) types itself in on page load. The right column (Generated Rust) fades in after the type-on completes.

**Implementation:** CSS `@keyframes` with `steps()` for the typing effect. The source text is already static in the RSX — wrap each line in a `span` with staggered `animation-delay`. No JavaScript needed.

```css
.hero-type-line {
  overflow: hidden;
  white-space: nowrap;
  width: 0;
  animation: type-in 0.6s steps(40, end) forwards;
}
.hero-type-line:nth-child(1) { animation-delay: 0.3s; }
.hero-type-line:nth-child(2) { animation-delay: 0.6s; }
.hero-type-line:nth-child(3) { animation-delay: 0.9s; }
/* ... stagger per line */

@keyframes type-in {
  to { width: 100%; }
}

.hero-rust-pane {
  opacity: 0;
  animation: fadeIn 0.5s ease forwards;
  animation-delay: 2.5s; /* after type-on completes */
}
```

Respects `prefers-reduced-motion` — all lines visible immediately, no animation.

**B. Code Block Glow Focus**

Active/hovered code blocks get a subtle glow border. This draws the eye to the code — the product itself — rather than surrounding chrome.

```css
.demo:hover,
.demo:focus-within {
  border-color: rgba(129, 140, 248, 0.3);
  box-shadow:
    0 0 0 1px rgba(129, 140, 248, 0.15),
    0 0 40px rgba(129, 140, 248, 0.08),
    0 30px 80px rgba(0, 0, 0, 0.55);
  transition: border-color 0.3s ease, box-shadow 0.3s ease;
}

.hello-code:hover,
.hello-result:hover {
  border-color: rgba(129, 140, 248, 0.25);
  box-shadow: 0 0 30px rgba(129, 140, 248, 0.06);
}
```

Feature block code examples get the same treatment — subtle purple glow on hover/focus.

**C. Scroll-Triggered Section Reveals**

Feature blocks, comparison table rows, and FAQ items fade in as they enter the viewport. Uses `IntersectionObserver` via `web-sys` (WASM-safe).

```rust
// Lightweight scroll reveal — add "reveal" class on intersection
// CSS handles the animation:
```

```css
.reveal {
  opacity: 0;
  transform: translateY(16px);
  transition: opacity 0.5s ease, transform 0.5s ease;
}
.reveal.visible {
  opacity: 1;
  transform: translateY(0);
}
```

Each feature block, FAQ item, and comparison row gets the `.reveal` class. A single `IntersectionObserver` instance (threshold: 0.15) toggles `.visible` on entry. One-shot — once visible, stays visible.

**D. Tab Switch Animation**

When switching between the 3 code example tabs, the outgoing code slides/fades left and the incoming code slides/fades in from the right.

```css
.tab-content-enter {
  animation: slideInRight 0.3s ease forwards;
}
.tab-content-exit {
  animation: slideOutLeft 0.2s ease forwards;
}

@keyframes slideInRight {
  from { opacity: 0; transform: translateX(16px); }
  to { opacity: 1; transform: translateX(0); }
}
@keyframes slideOutLeft {
  from { opacity: 1; transform: translateX(0); }
  to { opacity: 0; transform: translateX(-16px); }
}
```

**E. Comparison Table Row Hover**

Rows highlight on hover with a subtle background shift. The LOGOS column stays accented.

```css
.compare-row:not(.header):hover {
  background: rgba(255, 255, 255, 0.03);
}
.compare-row:not(.header):hover .compare-cell.highlight {
  background: rgba(129, 140, 248, 0.15);
}
```

**Motion Budget:** All animations complete within 500ms. Nothing loops. Everything respects `prefers-reduced-motion: reduce` via the existing media query that disables all transitions/animations.

#### Design Reference: Linear.app

Linear is the north star for this redesign. Specific patterns to adopt:

| Linear Pattern | How We Apply It |
|---------------|----------------|
| **Restrained color** — One purple accent (`#5E6AD2`), everything else is neutral grays on dark | We use `#818cf8` as our single accent. No gradients on UI elements. |
| **Typography weight** — Heavy display headlines (700-800), light body text (400). Strong contrast. | Inter Display at 800-900 weight for headlines, system sans at 400 for body. Large size gap between headline and body. |
| **Generous whitespace** — Sections separated by 120px+, cards have 32px+ padding | Our spacing bump (sections to 96px, cards to 28px) moves in this direction. |
| **Subtle borders** — `rgba(255,255,255,0.08)` borders, not `0.10`. Barely visible. | Drop our border opacity from `0.10` to `0.08` across all cards, demo panels, and dividers. |
| **No decorative elements** — No orbs, no blobs, no particles. Content IS the design. | Orbs removed. Code demos are the visual centerpiece. |
| **Functional hover states** — Elements lift slightly, border brightens. No color explosions. | `translateY(-2px)` on hover, border goes from `0.08` to `0.15`. Glow on code blocks only. |
| **Scroll-triggered reveals** — Content fades up as you scroll. Staggered, not simultaneous. | Our `.reveal` pattern with `IntersectionObserver`. Stagger children by 80ms. |
| **Monochrome + one accent** — Screenshots/demos are grayscale or dark, accent pops. | Hero demo panel is dark (`rgba(0,0,0,0.18)` right column). Purple accent only on LOGOS column in comparison table and primary CTA. |

**Additional Linear-inspired CSS adjustments:**

```css
/* Softer borders (Linear uses barely-visible dividers) */
:root {
  --border-subtle: rgba(255, 255, 255, 0.08);
  --border-hover: rgba(255, 255, 255, 0.15);
}

/* Letter-spacing on section titles (Linear uses tight tracking) */
.section-title {
  letter-spacing: -1.2px;  /* tighter than current -0.8px */
  font-weight: 800;
}

/* Muted section subtitles (Linear keeps these very understated) */
.section-sub {
  color: rgba(255, 255, 255, 0.45);  /* more muted than current --text-secondary */
  font-size: var(--font-body-md);     /* not body-lg — keep understated */
}

/* Card backgrounds almost invisible (Linear cards barely exist) */
.card {
  background: rgba(255, 255, 255, 0.02);  /* down from 0.04 */
  border-color: var(--border-subtle);
}
```

---

## Problem Statement

### Reddit Feedback (Verbatim)

> "It's crazy that every couple of years someone comes along and thinks this is a good idea" — r/rust

> "'write literal English that compiles to rust' is very misleading" — r/Programming

### The Fix (u/One_Measurement_8866)

> "The main thing that makes this interesting isn't 'English → Rust' but that you've picked a very opinionated domain: distributed state + mesh networking + CRDTs... The win is: nail a tight, transparent semantic core first, not a magic English layer."

### Root Cause

The current homepage triggers the "AppleScript objection" immediately. The hero says "Debug Your Thoughts" and the demo shows English → FOL. A Rust developer sees this and thinks: another natural language programming gimmick.

The actual product has:
- 6 production CRDTs with automatic journaling and GossipSub replication
- P2P mesh networking in 3 lines (generates libp2p, QUIC, mDNS)
- Curry-Howard proofs with automated tactics and Z3 integration
- Deterministic Montague grammar (not LLM guessing)

None of this is visible on the homepage.

---

## Current State Analysis

### Current Landing Page (`src/ui/pages/landing.rs`)

1,054 lines of Dioxus RSX. Sections in order:

1. **Hero** — "Debug Your Thoughts." with English → FOL demo
2. **How It Works** — 3 generic steps (Write → Get logic → Validate)
3. **Hello World** — `hello.md` compiling to native binary
4. **What You Get** — 6 feature cards (Transpilation, Tutor, Assumptions, Consistency, Studio, Commercial)
5. **Who Uses It** — Students, Law, Engineering
6. **Comparison Table** — vs Lean 4, Rust, Python
7. **FAQ** — 6 questions
8. **Final CTA** — Start Learning / View Licenses
9. **Footer** — Copyright, links

### What's Missing From the Current Page

| Feature | Status in Codebase | Visible on Homepage |
|---------|-------------------|-------------------|
| CRDTs (6 types: GCounter, PNCounter, ORSet, RGA, YATA, ORMap) | Implemented, examples exist in `examples.rs` | No |
| P2P mesh networking (Listen/Connect/Send) | Implemented, examples in `examples.rs` | No |
| `Shared` trait + `ConvergentCount` / `Tally` types | Implemented | No |
| GossipSub replication | Implemented | No |
| Proof engine (Curry-Howard) | Implemented | No |
| Automated tactics (ring, lia, omega, cc, simp, induction) | Implemented | No |
| Z3 integration | Implemented (feature-gated) | No |
| Imperative mode (English → Rust) | Implemented | Partially (Hello World) |
| Logic mode (English → FOL) | Implemented | Yes (hero demo) |
| Parse forests (up to 12 readings) | Implemented | No |
| AST viewer | Component exists (`ast_tree.rs`) | No |
| Generated Rust viewer | Available in Studio | No |
| `Portable` trait for serialized messaging | Implemented | No |
| `PeerAgent` for remote node interaction | Implemented | No |
| `Merge` for CRDT replica convergence | Implemented | No |

### Existing Components We Can Reuse

- `ast_tree.rs` — AST visualization (for transparency section)
- `code_editor.rs` — Code editor with syntax highlighting
- `logic_output.rs` — FOL expression display
- `repl_output.rs` — REPL execution results
- `mode_selector.rs` — Mode switching UI
- Icon library (Lightning, Brain, Shield, Lock, Tools, GraduationCap, etc.)
- Card, badge, pill, demo panel patterns from current landing CSS

---

## Positioning Strategy

### Core Message

**"Distributed apps in 3 lines. Compiles to production Rust."**

### Positioning Rules

1. **Never lead with "English programming"** — triggers immediate skepticism
2. **Lead with distributed systems** — the actual differentiator
3. **Show transparency always** — AST, generated Rust, parse forests
4. **Address the license early** — free for most, open source 2029
5. **Developer-first** — no dumbing down, no SMB messaging
6. **WASM-safe** — all interactive demos must work in browser

### Target Audiences

| Audience | Hook | What They Care About |
|----------|------|---------------------|
| Rust devs building distributed systems | "Batteries-included CRDTs + P2P" | Correctness, performance, no runtime overhead |
| Multiplayer/collab app devs | "P2P state sync without the PhD" | Ease of use, eventual consistency, offline support |
| DeFi/blockchain devs | "Prove invariants at compile time" | Formal verification, safety guarantees |
| Logic/PL enthusiasts | "Real Montague semantics, not LLM guessing" | Determinism, soundness, linguistics |

### What LOGOS Actually Is (Internal Reference)

Three modes, one language:

- **Imperative Mode:** English → Executable Rust (LLVM compiled)
- **Logic Mode:** English → First-Order Logic (∀, ∃, →, ∧)
- **Proof Mode:** Curry-Howard proofs with automated tactics

---

## Homepage Copy

All copy below is final. No placeholders.

---

### A. Hero Section

**Badge:**

```
2227+ tests passing · Free for individuals
```

**Headline:**

```
Distributed State.
Three Lines of Code.
```

**Subheadline:**

```
LOGOS is a typed DSL for distributed systems. Write readable definitions,
get production Rust with CRDTs, P2P networking, and formal verification
built in. No runtime. No magic.
```

**Primary CTA:** `Try in Browser` → links to Studio with preloaded CRDT example

**Secondary CTA:** `See Generated Rust` → anchor scrolls to transparency section

**KPI Pills:**

```
Compiles to Rust  ·  6 Native CRDTs  ·  P2P in 3 Lines  ·  Prove at Compile Time
```

**Hero Demo Panel:**

Left column header: "LOGOS Source"

```
## Definition
A Counter is Shared and has:
    points: ConvergentCount.

## Main
Let mutable c be a new Counter.

Listen on "/ip4/0.0.0.0/tcp/8080".
Sync c on "game-room".

Increase c's points by 10.
Show c's points.
```

Right column header: "Generated Rust"

```rust
use libp2p::{gossipsub, mdns, quic};
use logos_runtime::{Shared, GCounter, Journal};

struct Counter {
    points: GCounter,
}

impl Shared for Counter { /* auto-derived */ }

fn main() -> Result<(), Box<dyn Error>> {
    let transport = quic::Transport::new(/*...*/);
    let mut c = Counter {
        points: GCounter::new(),
    };
    c.sync("game-room", &transport)?;
    c.points.increment(10);
    println!("{}", c.points.value());
    Ok(())
}
```

Footer bar: `Deterministic compilation · No LLM · See the AST →`

---

### B. Trust Block

Positioned directly below the hero. Single row, minimal.

```
Built on: libp2p · tokio · rayon · LLVM
2227+ tests passing · BSL 1.1 (MIT in 2029)
```

---

### C. Feature Blocks

Five blocks. Each follows the pattern: **pain → solution → code**.

---

#### Block 1: Distributed State Without the Boilerplate

**Pain:**

CRDTs require manual conflict resolution, journaling infrastructure, and network sync code. A simple counter becomes 400 lines of Rust.

**Solution:**

Mark a struct `Shared` and use convergent types. LOGOS generates the merge logic, journaling, and GossipSub replication. One `Sync` call gives you eventual consistency that survives restarts, offline nodes, and network partitions.

**Code:**

```logos
## Definition
A Score is Shared and has:
    points: Tally.

## Main
Let mutable s be a new Score.
Increase s's points by 100.
Decrease s's points by 30.
Show "Final: " + s's points.
```

**Footnote:** `Supports ConvergentCount (G-Counter), Tally (PN-Counter), ORSet (AddWins/RemoveWins), RGA, YATA, ORMap`

---

#### Block 2: P2P Networking in Plain Text

**Pain:**

libp2p requires understanding transports, protocols, peer discovery, and message routing. The learning curve is weeks, not hours.

**Solution:**

Three statements. LOGOS generates production libp2p with QUIC transport, mDNS discovery, and GossipSub pub/sub. The same networking stack that IPFS uses.

**Code:**

```logos
## Definition
A Greeting is Portable and has:
    message: Text.

## Main
Listen on "/ip4/0.0.0.0/tcp/8000".
Connect to "/ip4/127.0.0.1/tcp/8000".

Let remote be a PeerAgent at "/ip4/127.0.0.1/tcp/8000".
Let msg be a new Greeting with message "Hello, peer!".
Send msg to remote.
```

**Footnote:** `Generates: QUIC transport, mDNS peer discovery, GossipSub pub/sub, connection management`

---

#### Block 3: Logic Mode — English to Formal Logic

**Pain:**

First-order logic is powerful but inaccessible. Existing tools require you to already know the notation. LLMs guess at formalization and miss scope ambiguities.

**Solution:**

Type English, get standard FOL with deterministic Montague grammar. LOGOS returns ALL valid readings — not one guess. Scope ambiguities are surfaced, not hidden.

**Code:**

```
"Every student read a book."
```

**Output (both readings):**

```
∀x(Student(x) → ∃y(Book(y) ∧ Read(x,y)))    — different books
∃y(Book(y) ∧ ∀x(Student(x) → Read(x,y)))    — same book
```

**Footnote:** `Neo-Davidsonian events, Montague λ-calculus, DRS, parse forests (up to 12 readings)`

---

#### Block 4: Proofs That Compile

**Pain:**

Testing catches bugs in code you thought to test. Formal verification is theoretically better but practically impractical for most projects.

**Solution:**

Curry-Howard correspondence at the language level. Write invariants as policy definitions. LOGOS proves them at compile time using automated tactics. Z3 integration for satisfiability checking.

**Code:**

```logos
## Definition
A User has:
    a role, which is Text.

## Policy
A User is admin if the user's role equals "admin".

## Main
Let u be a new User with role "admin".
Check that the u is admin.
Show "Admin check passed!".
```

**Footnote:** `Tactics: ring, lia, omega, cc, simp, induction · Z3 integration for compile-time verification`

---

#### Block 5: See Everything

**Pain:**

"Magic" languages surprise you. You don't know what they generate, how they parse, or why something broke. Trust erodes.

**Solution:**

Every compilation step is visible. See the AST, the generated Rust, the parse forest, and the proof obligations. If LOGOS can't prove something, it tells you exactly why.

**Visual:** Side-by-side three-panel layout:

| English Source | AST (Tree View) | Generated Rust |
|---------------|-----------------|----------------|
| Live editor input | `ast_tree.rs` component rendering | Syntax-highlighted Rust output |

**Footnote:** `AST viewer, generated Rust, parse forests, proof traces — nothing hidden`

---

### D. How It Works

Pipeline diagram rendered as a visual flow:

```
English ──→ Lexer ──→ Parser ──→ AST
                                  │
                    ┌─────────────┴──────────────┐
                    ▼                             ▼
               Semantics                      Codegen
              (λ-calculus)                     (Rust)
                    ▼                             ▼
                  FOL                        Executable
            (∀, ∃, →, ∧)               (LLVM native binary)
```

**Caption:** `Deterministic pipeline. No neural networks. No probabilistic inference. Every step inspectable.`

This will be rendered as styled HTML/CSS boxes with connecting lines, not a text diagram. The existing step/arrow pattern from the current landing page can be adapted.

---

### E. Code Examples Section

Three tabs. Users switch between them. Default is the distributed counter.

#### Tab 1: Distributed Counter (Default)

```logos
## Definition
A Counter is Shared and has:
    points: ConvergentCount.

## Main
Let mutable c be a new Counter.
Increase c's points by 10.
Increase c's points by 5.
Increase c's points by 3.
Show "Total points:".
Show c's points.
```

Caption: `Automatic journaling. GossipSub replication. Crash-safe. Offline-capable.`

#### Tab 2: Scope Ambiguity

```
"Every student read a book."
```

Output:

```
Reading 1: ∀x(Student(x) → ∃y(Book(y) ∧ Read(x,y)))
Reading 2: ∃y(Book(y) ∧ ∀x(Student(x) → Read(x,y)))
```

Caption: `Both readings surfaced. Deterministic Montague grammar — not LLM guessing.`

#### Tab 3: Compile-Time Proof

```logos
## Definition
A User has:
    a role, which is Text.

## Policy
A User is admin if the user's role equals "admin".

## Main
Let u be a new User with role "admin".
Check that the u is admin.
Show "Admin check passed!".
```

Caption: `Proven at compile time. If it compiles, the invariant holds.`

---

### F. Comparison Table — "How LOGOS Compares"

Retooled for the distributed systems audience. Replaces the current Lean 4 / Rust / Python matrix with competitors a distributed-systems developer actually weighs.

**Section title:** `How LOGOS Compares`

**Section subtitle:** `What you'd need without it.`

#### Matrix (exact cell copy)

| Feature | **LOGOS** | Rust + libp2p | Elixir / OTP | Automerge |
|---------|-----------|---------------|--------------|-----------|
| CRDTs | 6 built-in (GCounter, PNCounter, ORSet, RGA, YATA, ORMap) | Manual or crate | None built-in | 4 built-in |
| P2P Networking | 3 lines (QUIC, mDNS, GossipSub) | Manual (~200 LoC) | Distributed Erlang | None |
| Formal Verification | Compile-time (Curry-Howard + Z3) | External (Kani, Prusti) | None | None |
| Conflict Resolution | Automatic (merge semantics per CRDT) | Manual | Manual (GenServer) | Automatic |
| Journaling / Persistence | Built-in (`Mount` statement) | Manual (sled, RocksDB) | Manual (Mnesia, ETS) | Manual |
| Syntax | Readable English DSL | Rust symbols | Elixir DSL | JavaScript / Rust API |
| Output | Native binary (LLVM via Rust) | Native binary | BEAM VM | Library (no standalone binary) |
| Offline Support | Built-in (journal replay on reconnect) | Manual | Limited | Built-in |
| Lines for a P2P counter | ~10 | ~400 | ~80 | ~60 (JS, no P2P) |

#### Column footnotes

- **Rust + libp2p:** The manual approach. You write everything LOGOS generates. Production-grade but high effort.
- **Elixir / OTP:** Battle-tested distribution via BEAM, but no CRDTs, no formal verification, no native compilation.
- **Automerge:** Excellent CRDT library. No networking, no verification, no standalone compilation. JavaScript-first.

#### Visual notes

- LOGOS column uses the existing `.highlight` class (purple accent background)
- Reuse the existing `.compare-table` / `.compare-row` / `.compare-cell` CSS structure from the current landing page
- On mobile (< 700px): collapse to LOGOS + Rust + libp2p only (most relevant comparison), hide Elixir and Automerge columns

---

### G. FAQ

Six questions in a 2-column grid. Each addresses a specific developer skepticism.

---

**Q: "Isn't this just AppleScript?"**

No. LOGOS has a deterministic Montague grammar that compiles to Rust via LLVM. It generates the same libp2p, tokio, and rayon code you'd write by hand. The syntax is readable, but the compilation is rigorous — every step from English to AST to Rust is inspectable.

---

**Q: "Why not just use ChatGPT to generate Rust?"**

LLMs are probabilistic. They guess at one interpretation and miss edge cases. LOGOS uses deterministic Montague grammar and returns ALL valid readings of an ambiguous statement. "Every student read a book" yields both scope readings — an LLM picks one and hopes.

---

**Q: "What can the proof engine actually prove?"**

Polynomial equality (ring), linear arithmetic (lia), Presburger arithmetic (omega), congruence closure (cc), term simplification (simp), and structural induction. Z3 integration handles satisfiability checking. It won't prove everything — but it proves the things that matter for distributed system invariants.

---

**Q: "Is this production-ready?"**

2227+ tests passing. The compiler generates standard Rust that links against libp2p, tokio, and rayon. Licensed under BSL 1.1 — free for individuals and educators, commercial licenses available, converts to MIT in 2029.

---

**Q: "How is this different from Solidity?"**

LOGOS compiles to native Rust, not EVM bytecode. It has built-in CRDTs and P2P networking that Solidity doesn't. Verification happens at compile time, not through external audits. And you can run it anywhere — not just on a blockchain.

---

**Q: "What does the generated code look like?"**

Standard Rust. No custom runtime, no interpreter. The generated code uses libp2p for networking, serde for serialization, and standard error handling. Click "See Generated Rust" on any example to inspect the output.

---

### G. Final CTA

**Headline:** `Start Building`

**Subtext:**

```
LOGOS is free for individuals.
Write distributed apps in readable code. See exactly what compiles.
```

**Primary CTA:** `Open Studio` → Route::Studio with CRDT example preloaded

**Secondary CTA:** `Read the Docs` → Route::Guide

**Tertiary (text link):** `View Pricing` → Route::Pricing

---

## Product Tour Script

### Overview

- 7 steps, mobile-first, spotlight UI pattern
- **Trigger:** First visit to Studio (check OPFS for `tour_completed` flag)
- **UI Pattern:** Anchored tooltip with dimmed background overlay. Mobile: bottom sheet cards with swipe.
- **Progress:** Dot indicators (1-7) + "Skip tour" button
- **Persistence:** Tour completion state stored in OPFS via raw `web-sys` OPFS API. Fallback to `localStorage` via `gloo-storage` if OPFS unavailable.

### Tour State Management

New component at `src/ui/components/product_tour.rs`:

```rust
struct TourState {
    current_step: usize,  // 0-6
    completed: bool,
}
```

- Check on Studio mount: if `!tour_completed`, show tour
- Skip button sets `tour_completed = true` in OPFS
- Completing step 7 sets `tour_completed = true`

---

### Step 1: Welcome Modal

**Highlight:** None (centered modal over dimmed background)

**Content:**

```
LOGOS Studio

LOGOS compiles readable code to production Rust
with built-in distributed systems primitives.

This tour takes 60 seconds.
```

**CTA:** `Show me` / `Skip`

---

### Step 2: Editor Panel

**Highlight:** Editor pane (spotlight border glow)

**Preloaded content:** Hello World example from `examples.rs`

**Tooltip:**

```
This is a LOGOS source file.

Plain English with defined structure.
It compiles — deterministically — to Rust.
```

**CTA:** `Next`

---

### Step 3: Compile → Rust Pane

**Highlight:** Compile button, then Generated Rust output pane

**Action:** Auto-trigger compilation. Rust output appears in right pane.

**Tooltip:**

```
Hit Compile. The generated Rust appears here.

Every token maps 1-to-1. No hidden transformations.
What you see is what runs.
```

**CTA:** `Next`

---

### Step 4: AST Viewer

**Highlight:** AST tree panel

**Action:** AST visualization renders from the compiled example.

**Tooltip:**

```
This is the Abstract Syntax Tree.

Every parse decision is visible — how LOGOS read
your input, what it inferred, and what it chose.

No magic.
```

**CTA:** `Next`

---

### Step 5: CRDT Example

**Highlight:** Editor pane

**Action:** Replace editor content with `CODE_CRDT_COUNTERS` from `examples.rs`:

```logos
## Definition
A Counter is Shared and has:
    points: ConvergentCount.

## Main
Let mutable c be a new Counter.
Increase c's points by 10.
Increase c's points by 5.
Increase c's points by 3.
Show "Total points:".
Show c's points.
```

**Tooltip:**

```
This is what makes LOGOS different.

A Shared struct with a ConvergentCount field.
Three increments. LOGOS generates the CRDT merge
logic, journaling, and GossipSub replication.

This is production libp2p code.
```

**CTA:** `Compile it` (triggers compilation)

---

### Step 6: Run → Output

**Highlight:** Run button, then output pane

**Action:** Execute the compiled WASM. Show output.

**Tooltip:**

```
It runs.

LOGOS compiled your readable source to Rust,
then to WASM. The CRDT is live.
```

**CTA:** `Next`

---

### Step 7: Next Steps

**Highlight:** None (centered modal)

**Content:**

```
You're ready.

  Docs     Full language reference
  Examples Distributed apps, logic, proofs
  Learn    Structured curriculum with exercises
```

**CTAs:**

- `Open Docs` → Route::Guide
- `Browse Examples` → Studio with example picker open
- `Start Learning` → Route::Learn

**Dismiss:** Sets `tour_completed = true` in OPFS

---

### Tour UI Specifications

**Desktop:**

- Tooltip: 360px max-width, anchored to highlighted element
- Background: `rgba(0, 0, 0, 0.75)` overlay with cutout around spotlighted element
- Cutout: 8px padding, 12px border-radius, subtle border glow (`rgba(96, 165, 250, 0.4)`)
- Animation: Fade in 200ms, spotlight slides between elements over 300ms
- Progress: 7 dots at tooltip bottom, current dot filled with accent gradient

**Mobile (< 640px):**

- Bottom sheet: Full-width card, 280px max-height, swipeable left/right
- Background: Same dimmed overlay
- Spotlight: Page scrolls to ensure highlighted element is visible above the sheet
- Progress: Same 7 dots

**Accessibility:**

- Focus trap within tooltip
- Escape key dismisses tour
- `aria-describedby` on spotlighted elements
- `prefers-reduced-motion`: No animation, instant transitions
- Keyboard: Tab through CTAs, Enter to advance

---

## Architecture & Component Plan

### New Files

```
assets/fonts/
└── InterDisplay-Variable.woff2   # Display typeface (~100KB, self-hosted)

src/ui/components/
├── product_tour.rs          # Tour controller, step management, tooltip rendering
├── tour_state.rs            # OPFS persistence for tour_completed flag
├── spotlight.rs             # Reusable spotlight overlay with element cutout
├── pipeline_diagram.rs      # How It Works pipeline visualization
└── code_tabs.rs             # Tabbed code example switcher
```

### Modified Files

```
src/ui/pages/landing.rs      # Complete rewrite of page content + remove orbs
src/ui/pages/studio.rs       # Add tour trigger on first visit
src/ui/components/mod.rs     # Register new components
src/ui/seo.rs                # Update JSON-LD schemas for new content
assets/style.css             # Design system overhaul: accent color, spacing, borders,
                             #   font-face, glow states, scroll reveals, type-on animation,
                             #   tour overlay styles, pipeline styles
index.html                   # Add font preload link
```

### Component Tree

```
Landing (page)
├── MainNav
├── HeroSection
│   ├── Badge (2227+ tests · Free)
│   ├── Headline + Subheadline
│   ├── CTAs (Try in Browser / See Generated Rust)
│   ├── KPI Pills
│   └── HeroDemoPanel
│       ├── LOGOSSource (left column)
│       └── GeneratedRust (right column, syntax-highlighted)
├── TrustBlock (libp2p · tokio · rayon · LLVM)
├── FeatureBlocks (×5)
│   ├── PainStatement
│   ├── SolutionStatement
│   └── CodeExample (reuses existing .code / .demo-col styles)
├── PipelineDiagram (new component)
├── CodeTabs (new component)
│   ├── Tab: Distributed Counter
│   ├── Tab: Scope Ambiguity
│   └── Tab: Compile-Time Proof
├── ComparisonTable (retooled, reuses .compare-table styles)
├── FAQ (×6, reuses .faq-item styles)
├── FinalCTA
└── Footer

Studio (page, modified)
├── ProductTour (conditional, new)
│   ├── Spotlight overlay
│   ├── TooltipCard (positioned relative to target element)
│   ├── ProgressDots (7 dots)
│   └── TourState (OPFS read/write)
└── [existing Studio components unchanged]
```

### WASM Considerations

- All demo code in hero and feature blocks is **static display** (no WASM execution on landing page)
- Studio tour uses existing WASM compilation pipeline already in `studio.rs`
- Tour state persistence via OPFS (`web-sys::FileSystemDirectoryHandle`) — WASM-safe
- Fallback: `localStorage` via `gloo-storage` if OPFS unavailable
- No new external API calls for tour functionality
- Hero "Try in Browser" CTA navigates to Studio route with query param for preloaded example

---

## Implementation Plan

### Phase 1: Design Foundation + Content Rewrite

Establish the new visual system first, then rewrite all landing page content on top of it.

**1a. Design Foundation (CSS + assets)**

| Task | Files |
|------|-------|
| Remove gradient orbs (`.bg-orb`, `.orb1-3`) from landing and all other pages | `landing.rs`, `style.css`, other pages |
| Add Inter Display font: download WOFF2, add `@font-face`, preload in `index.html` | `assets/fonts/`, `style.css`, `index.html` |
| Consolidate accent color: replace all blue-purple gradients with `#818cf8` flat accent | `style.css`, `landing.rs` |
| Add new CSS variables: `--font-display`, `--color-accent`, `--color-accent-hover`, `--color-accent-subtle`, `--color-accent-glow`, `--border-subtle`, `--border-hover` | `style.css` |
| Apply display font to `.h-title`, `.section-title`, `.card h3`, `.faq-q` | `style.css` |
| Increase spacing: section padding to 96px, card padding to 28px, grid gaps to 28px, hero padding to 96px/48px | `style.css` |
| Soften borders site-wide from `0.10` opacity to `0.08` | `style.css` |
| Tighten headline letter-spacing to `-1.2px`, mute `.section-sub` color to `rgba(255,255,255,0.45)` | `style.css` |
| Reduce card background opacity from `0.04` to `0.02` | `style.css` |
| Add code block glow-on-hover: `.demo:hover`, `.hello-code:hover` get purple glow border + box-shadow | `style.css` |
| Add comparison table row hover state | `style.css` |

**1b. Content Rewrite**

| Task | Files |
|------|-------|
| Rewrite hero: new headline, subheadline, CTAs, KPI pills | `landing.rs` |
| Replace hero demo: LOGOS source left, Generated Rust right | `landing.rs` |
| Wrap hero source lines in spans for type-on animation (CSS-only, staggered delays) | `landing.rs`, `style.css` |
| Add trust block below hero | `landing.rs` |
| Replace 6 feature cards with 5 problem→solution blocks | `landing.rs` |
| Replace "How It Works" 3-step with pipeline text (styled in Phase 2) | `landing.rs` |
| Replace Hello World with static code example section | `landing.rs` |
| Retool comparison table: new columns (LOGOS, Rust+libp2p, Elixir/OTP, Automerge), 9 new rows | `landing.rs` |
| Rewrite FAQ with 6 developer-focused questions | `landing.rs` |
| Update final CTA copy and button targets | `landing.rs` |
| Remove "Who uses LOGICAFFEINE" section | `landing.rs` |
| Update SEO JSON-LD schemas for new messaging | `seo.rs` |

### Phase 2: Interactive Components + Motion

Build new components and layer in functional motion.

| Task | Files |
|------|-------|
| Build `code_tabs.rs` — tabbed switcher with 3 examples + tab-switch slide animation | `code_tabs.rs`, `mod.rs`, `style.css` |
| Build `pipeline_diagram.rs` — visual pipeline with hover states | `pipeline_diagram.rs`, `mod.rs`, `style.css` |
| Add "See Generated Rust" scroll-anchor behavior | `landing.rs` |
| Build transparency section with 3-panel layout (English → AST → Rust) | `landing.rs`, `style.css` |
| Wire "Try in Browser" to Studio with `?example=crdt` param | `landing.rs`, `studio.rs` |
| Implement hero type-on animation: CSS `steps()` keyframes, staggered line delays, Rust pane fade-in after completion | `landing.rs`, `style.css` |
| Implement scroll-triggered reveals: `IntersectionObserver` via `web-sys`, `.reveal` / `.visible` classes on feature blocks, FAQ items, comparison rows | `landing.rs`, `style.css` |
| Add `prefers-reduced-motion` bypass for all new animations (instant visibility, no transforms) | `style.css` |

### Phase 3: Product Tour

Build the 7-step tour overlay for Studio.

| Task | Files |
|------|-------|
| Build `spotlight.rs` — overlay with element cutout + purple glow border | `spotlight.rs`, `style.css` |
| Build `tour_state.rs` — OPFS persistence (fallback: localStorage) | `tour_state.rs` |
| Build `product_tour.rs` — step controller + tooltip (Inter Display for headlines) | `product_tour.rs` |
| Add tour trigger to Studio on first visit | `studio.rs` |
| Implement tooltip positioning (anchor to DOM elements) | `product_tour.rs` |
| Add mobile bottom sheet variant | `product_tour.rs`, `style.css` |
| Add progress dots (accent color) and skip button | `product_tour.rs` |
| Step 5: auto-replace editor with CRDT example | `product_tour.rs` |
| Steps 3, 6: auto-trigger compilation | `product_tour.rs` |

### Phase 4: Polish & Responsiveness

| Task | Files |
|------|-------|
| Mobile responsiveness pass on all new landing sections (spacing scales down at breakpoints) | `landing.rs`, `style.css` |
| Mobile comparison table: collapse to LOGOS + Rust+libp2p columns only at < 700px | `style.css` |
| Mobile bottom sheet tour testing | `product_tour.rs`, `style.css` |
| Verify Inter Display font loads with `font-display: swap` (no FOIT) | `index.html`, browser testing |
| Verify all animations respect `prefers-reduced-motion` | `style.css`, browser testing |
| Accessibility audit (focus traps, ARIA, keyboard nav, color contrast with new accent) | All new components |
| Performance audit: hero < 3s including font load, demos < 5s | Lighthouse |
| Cross-browser testing (Chrome, Firefox, Safari, mobile Safari) | All |

---

## Success Metrics

| Metric | Target | Measurement |
|--------|--------|-------------|
| Hero load time | < 3s | Lighthouse |
| Interactive demo load | < 5s | Custom performance mark |
| Tour completion rate | 50%+ | OPFS `tour_completed` flag |
| Value prop comprehension | Rust dev understands in < 10s | Qualitative user testing |
| Bounce rate | 20% reduction vs current | Analytics (post-launch) |
| "Try in Browser" click rate | 15%+ of visitors | Click tracking (post-launch) |
| "See Generated Rust" engagement | 10%+ of visitors | Scroll tracking (post-launch) |

---

## Open Questions

These should be resolved before implementation begins.

### 1. Hero Demo: Static vs Live?

- **Option A: Static display** — Pre-rendered LOGOS source and Rust output. Fastest load, simplest to build.
- **Option B: Live compilation** — User edits LOGOS source, Rust output updates in real time. More impressive, requires WASM compilation on page load.

**Recommendation:** Option A for initial launch. Option B as fast-follow.

### 2. Studio Preloaded Example

When "Try in Browser" is clicked, which example preloads?

- **Option A:** Distributed counter (`CODE_CRDT_COUNTERS` from `examples.rs`) — matches hero demo
- **Option B:** P2P networking example (`CODE_NETWORK_SERVER`) — more interactive
- **Option C:** Example picker modal — user chooses

**Recommendation:** Option A for consistency with homepage messaging.

### 3. Tour Trigger Timing

- **Option A:** First visit to Studio only
- **Option B:** First visit + after clicking "Try in Browser"
- **Option C:** Always available via a "Tour" button in Studio nav

**Recommendation:** Option A with Option C as persistent access point.

### 4. Comparison Table

The current table (vs Lean 4, Rust, Python) is retooled for distributed systems. New columns:

- **LOGOS** (highlighted)
- **Rust + libp2p** (the manual approach — most relevant comparison)
- **Elixir / OTP** (distribution-native alternative)
- **Automerge** (CRDT library, no networking/verification)

Full copy is in [Section F of Homepage Copy](#f-comparison-table--how-logos-compares). Mobile collapses to LOGOS vs Rust+libp2p only.

**Resolved:** Retool with distributed-systems competitors. See Section F for exact cell values.

### 5. Existing Audience Sections

Current site has Students, Law/Policy, Engineering sections. Should we:

- **Option A:** Remove entirely — content speaks to developers implicitly
- **Option B:** Lightweight "Who builds with LOGOS" with developer personas
- **Option C:** Move to a separate page

**Recommendation:** Option A for homepage. Audience content can live in the Guide.

### 6. Analytics

- **Option A:** Lightweight custom events via OPFS
- **Option B:** Plausible Analytics (privacy-friendly)
- **Option C:** Defer to post-launch

**Recommendation:** Option C. Ship the redesign, add measurement after.

---

## Appendix: Content Diff Summary

### Sections Removed

| Section | Reason |
|---------|--------|
| "Debug Your Thoughts" hero | Triggers "AppleScript objection" |
| "How it works" (3 steps) | Too vague, replaced with pipeline diagram |
| "Hello World in LOGOS" | Replaced with distributed systems examples |
| "Who uses LOGICAFFEINE" (3 audience cards) | Developer audience implied by content |
| Comparison table (vs Lean 4, Rust, Python) | Retooled with distributed-systems competitors |
| 6 generic feature cards | Replaced by 5 problem→solution blocks with code |

### Sections Added

| Section | Purpose |
|---------|---------|
| Trust block (tests, deps, license) | Immediate credibility signal |
| 5 feature blocks with pain/solution/code | Show distributed systems capabilities |
| Pipeline diagram | Technical transparency |
| Tabbed code examples (3 tabs) | Demonstrate all three modes |
| Transparency section (English → AST → Rust) | Address "magic language" skepticism |
| 7-step product tour | Onboard new Studio users |

### Sections Modified

| Section | What Changed |
|---------|-------------|
| Hero | New headline, subheadline, CTAs, demo content |
| FAQ | New questions targeting developer skepticism |
| Final CTA | New copy, "Start Building" framing |
| Footer | Same structure, updated copyright year |
