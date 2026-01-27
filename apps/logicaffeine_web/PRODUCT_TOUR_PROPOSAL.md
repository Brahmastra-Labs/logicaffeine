# Product Tour

**Branch:** TBD
**Status:** Not Started
**Date:** 2026-01-26

---

## Summary

A 7-step spotlight tour for first-time Studio visitors. Guides users from "what is this?" to compiling and running a CRDT example.

---

## Overview

- 7 steps, mobile-first, spotlight UI pattern
- **Trigger:** First visit to Studio (check OPFS for `tour_completed` flag)
- **UI Pattern:** Anchored tooltip with dimmed background overlay. Mobile: bottom sheet cards with swipe.
- **Progress:** Dot indicators (1-7) + "Skip tour" button
- **Persistence:** OPFS via raw `web-sys` API. Fallback to `localStorage` via `gloo-storage`.

---

## Tour Steps

### Step 1: Welcome Modal

**Highlight:** None (centered modal over dimmed background)

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
**Preloaded content:** Hello World example

```
This is a LOGOS source file.

Plain English with defined structure.
It compiles — deterministically — to Rust.
```

**CTA:** `Next`

---

### Step 3: Compile → Rust Pane

**Highlight:** Compile button, then Generated Rust output pane
**Action:** Auto-trigger compilation.

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
**Action:** Replace editor content with CRDT counter example.

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

```
It runs.

LOGOS compiled your readable source to Rust,
then to WASM. The CRDT is live.
```

**CTA:** `Next`

---

### Step 7: Next Steps

**Highlight:** None (centered modal)

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

## UI Specifications

**Desktop:**
- Tooltip: 360px max-width, anchored to highlighted element
- Background: `rgba(0, 0, 0, 0.75)` overlay with cutout around spotlighted element
- Cutout: 8px padding, 12px border-radius, border glow `rgba(96, 165, 250, 0.4)`
- Animation: Fade in 200ms, spotlight slides between elements over 300ms
- Progress: 7 dots at tooltip bottom, current dot filled with accent

**Mobile (< 640px):**
- Bottom sheet: Full-width card, 280px max-height, swipeable left/right
- Spotlight: Page scrolls to ensure highlighted element is visible above the sheet

**Accessibility:**
- Focus trap within tooltip
- Escape key dismisses tour
- `aria-describedby` on spotlighted elements
- `prefers-reduced-motion`: No animation, instant transitions
- Keyboard: Tab through CTAs, Enter to advance

---

## Architecture

### New Files

```
src/ui/components/
├── product_tour.rs    # Tour controller, step management, tooltip rendering
├── tour_state.rs      # OPFS persistence for tour_completed flag
└── spotlight.rs       # Reusable spotlight overlay with element cutout
```

### Modified Files

```
src/ui/pages/studio.rs       # Add tour trigger on first visit
src/ui/components/mod.rs     # Register new components
```

### State Management

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

## Implementation Plan

| Task | Files |
|------|-------|
| Build `spotlight.rs` — overlay with element cutout + glow border | `spotlight.rs`, CSS |
| Build `tour_state.rs` — OPFS persistence (fallback: localStorage) | `tour_state.rs` |
| Build `product_tour.rs` — step controller + tooltip | `product_tour.rs` |
| Add tour trigger to Studio on first visit | `studio.rs` |
| Implement tooltip positioning (anchor to DOM elements) | `product_tour.rs` |
| Add mobile bottom sheet variant | `product_tour.rs`, CSS |
| Add progress dots and skip button | `product_tour.rs` |
| Step 5: auto-replace editor with CRDT example | `product_tour.rs` |
| Steps 3, 6: auto-trigger compilation | `product_tour.rs` |
