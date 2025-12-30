# Proposal: Logicaffeine User Platform
## Cloudflare-Native Architecture with Rust Workers

**Prepared for:** CEO
**Date:** December 29, 2025
**Author:** Engineering Team
**Version:** 3.0 (Cloudflare Edition)

---

## Table of Contents

- [Executive Summary](#executive-summary)
- [Part 0: Curriculum Navigation](#part-0-curriculum-navigation-step-0)
- [Part 1: Gamification Council Session](#part-1-gamification-council-session)
- [Part 2: Curriculum Gemini Review](#part-2-curriculum-gemini-review)
- [Part 3: Cloudflare Infrastructure Analysis](#part-3-cloudflare-infrastructure-analysis)
- [Part 4: Crate Separation Strategy](#part-4-crate-separation-strategy)
- [Part 5: D1 Database Schema](#part-5-d1-database-schema)
- [Part 6: Rust Workers Implementation](#part-6-rust-workers-implementation)
- [Part 7: Authentication System](#part-7-authentication-system)
- [Part 8: Payment & License System](#part-8-payment--license-system)
- [Part 9: Multiplayer Battle System](#part-9-multiplayer-battle-system)
- [Part 10: Implementation Roadmap](#part-10-implementation-roadmap)
- [Part 11: Cost Analysis](#part-11-cost-analysis)
- [Part 12: Summary](#part-12-summary)
- [Glossary](#glossary)

---

## Executive Summary

This proposal outlines a **Cloudflare-native** platform upgrade that transforms Logicaffeine into a connected, competitive learning platform. The architecture is designed to:

- **Stay 100% on Cloudflare** - Workers, D1, R2 (object storage only), Pages
- **Port JavaScript Workers to Rust** - Using `workers-rs` for type safety and performance
- **Separate platform crates from core** - Keep the compiler/language clean
- **Enable multiplayer battles and tournaments** - Competitive learning with time battles and lives
- **Fix the payment flow** - Proper license key generation and email delivery

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                    LOGICAFFEINE CLOUDFLARE ARCHITECTURE                     │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│  ┌─────────────┐   ┌─────────────┐   ┌─────────────┐   ┌─────────────┐    │
│  │  Cloudflare │   │  Cloudflare │   │  Cloudflare │   │  Cloudflare │    │
│  │    Pages    │   │   Workers   │   │     D1      │   │     R2      │    │
│  │  (Frontend) │   │   (Rust)    │   │  (SQLite)   │   │  (Storage)  │    │
│  └─────────────┘   └─────────────┘   └─────────────┘   └─────────────┘    │
│        │                 │                 │                 │             │
│        │                 ▼                 ▼                 ▼             │
│        │         ┌─────────────────────────────────────────────────┐      │
│        │         │                  API Layer                       │      │
│        │         │  • Auth (GitHub, Google OAuth)                  │      │
│        │         │  • Progress Sync                                 │      │
│        │         │  • Leaderboards                                  │      │
│        │         │  • Multiplayer Battles & Tournaments            │      │
│        │         │  • License/Payment                               │      │
│        └────────▶│                                                  │      │
│                  └─────────────────────────────────────────────────┘      │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Implementation Priority Order

**Parts are numbered in implementation order (Part 0 = first to implement). The document is structured with foundational content first (Parts 0-4), followed by technical implementation details (Parts 5-9), then planning and summary (Parts 10-12).**

| Part | Title | Why This Order |
|------|-------|----------------|
| **Part 0** | Curriculum Navigation | CEO priority: UX before features |
| **Part 1** | Gamification Council | Design philosophy before code |
| **Part 2** | Curriculum Gemini Review | Content quality before features |
| **Part 3** | Infrastructure Analysis | Understand the platform |
| **Part 4** | Crate Separation | Clean architecture first |
| **Part 5** | D1 Database Schema | Define data models |
| **Part 6** | Rust Workers | API implementation |
| **Part 7** | Authentication | OAuth before protected features |
| **Part 8** | Payment & License | Monetization |
| **Part 9** | Multiplayer & Tournaments | Advanced features last |
| **Part 10** | Roadmap | Implementation timeline |
| **Part 11** | Cost Analysis | Budget planning |
| **Part 12** | Summary | Final review |

**Key Dependencies:**
- Curriculum Navigation (0) → enables user testing of Gamification (1) changes
- Gamification (1) → informs Curriculum Review (2) priorities
- Infrastructure (3) + Crate Separation (4) → enables all backend work
- D1 Schema (5) → required before Workers (6), Auth (7), Payments (8), Battles (9)
- Auth (7) → required before Tournaments (9) and Progress Sync

---

## Part 0: Curriculum Navigation (Step 0)

**Status: ✅ IMPLEMENTED** — Unified navigation with active tab underline

### 0.1 Implementation Summary

Created `MainNav` component (`src/ui/components/main_nav.rs`) providing:

| Feature | Implementation |
|---------|----------------|
| **Unified header** | Same nav across Landing, Guide, Pricing, Roadmap, Registry |
| **Active tab underline** | Gradient underline (`#60a5fa` → `#a78bfa`) on current page |
| **Consistent brand** | Logo + "LOGICAFFEINE" + optional subtitle |
| **GitHub + CTA buttons** | Right-aligned actions |
| **Responsive** | Links hidden on mobile, brand text collapses |

### 0.2 Pages Updated

```rust
// Usage pattern across all pages:
MainNav { active: ActivePage::Guide, subtitle: Some("Programmer's Guide") }
```

| Page | File | ActivePage |
|------|------|------------|
| Landing | `landing.rs` | `Home` |
| Guide | `guide/mod.rs` | `Guide` |
| Pricing | `pricing.rs` | `Pricing` |
| Roadmap | `roadmap.rs` | `Roadmap` |
| Registry | `registry/browse.rs` | `Registry` |

### 0.3 Navigation Links

All pages now show consistent nav: **Guide → Learn → Studio → Roadmap → Pricing**

### 0.4 Remaining Work

| Task | Status | Notes |
|------|--------|-------|
| Visual progress map on Learn page | Pending | Part 1 prerequisite |
| Breadcrumbs in Exercise view | Pending | |
| Mobile bottom nav | Pending | |

---

## Part 1: Gamification Council Session

### 1.1 The Pedagogical Council

Convene a council of master teachers to review and optimize the gamification system. The council focuses on **making learning joyful** - not just engaging, but genuinely fun.

#### Council Composition

| Council Member | Expertise | Focus Area |
|----------------|-----------|------------|
| **Jesus Christ** (Leader) | Master teacher, engagement through parables and stories | Overall vision, keeping learning joyful, connecting to meaning |
| **Maria Montessori** | Self-directed learning, prepared environment | Intrinsic motivation, removing artificial barriers |
| **Socrates** | Questioning method, dialectic | Challenge without frustration, discovery |
| **John Dewey** | Learning by doing, pragmatism | Practical application, real-world connection |
| **Lev Vygotsky** | Zone of proximal development, scaffolding | Difficulty scaling, just-right challenge |

### 1.2 Council Deliberation Note

**On Competition:** The Council considered whether competitive features (battles, tournaments, leaderboards) might undermine intrinsic motivation. After deliberation, the majority **rejected this concern**, noting that:
- Competition drives extraordinary achievement in business, athletics, and academics
- Battles and tournaments are **opt-in** - learners who prefer solo exploration can use Studio mode
- The Co-op mode provides collaborative alternatives for those who thrive in team settings
- Healthy competition and intrinsic motivation are not mutually exclusive

The proposal proceeds with full competitive features intact.

### 1.3 Council Mandate: Fun While Learning

#### Core Principles

1. **Joy over metrics** - Learning should feel like play, not work
2. **Curiosity over compliance** - Encourage exploration, not just completion
3. **Mastery over points** - Understanding matters more than XP numbers
4. **Variety over repetition** - Different reward styles, surprise elements
5. **Connection over isolation** - Learning feels better in community

#### Anti-Repetition Strategies

The current system can feel repetitive. Here are concrete fixes:

**1. Rotate Feedback Messages**
```rust
const PRAISE_MESSAGES: &[&str] = &[
    "Sharp thinking!",
    "You're getting it!",
    "Excellent logic!",
    "That's the way!",
    "Nicely reasoned!",
    "Clear as crystal!",
    "You've got this!",
    "Brilliant deduction!",
];

// Track last shown message, never repeat consecutively
fn get_praise(last_index: &mut usize) -> &'static str {
    let mut idx = rand::random::<usize>() % PRAISE_MESSAGES.len();
    while idx == *last_index {
        idx = rand::random::<usize>() % PRAISE_MESSAGES.len();
    }
    *last_index = idx;
    PRAISE_MESSAGES[idx]
}
```

**2. Contextual Feedback**
Tie feedback to the specific logic concept:
- "You've mastered universal quantification!" (not just "Correct!")
- "Negation is no match for you!"
- "Scope ambiguity resolved perfectly!"

**3. Visual Variety**
- Different particle effects per achievement type (stars, confetti, sparkles)
- Color themes that rotate by day of week
- Occasional "golden" exercises worth 3x XP (10% chance)

**4. Surprise Elements**
- Hidden bonus exercises that appear randomly
- "Mystery box" rewards after streaks
- Easter eggs for specific answer patterns

### 1.4 Session-Based Variety

Track what exercises the user has done recently to avoid monotony:

```rust
struct SessionTracker {
    recent_exercise_types: VecDeque<ExerciseType>,
    recent_difficulties: VecDeque<u8>,
    recent_modules: VecDeque<String>,
}

impl SessionTracker {
    fn suggest_next(&self, available: &[Exercise]) -> &Exercise {
        // Prefer exercise types NOT in recent history
        // Alternate difficulty to maintain flow
        // Rotate between modules when possible
    }
}
```

**Guidelines:**
- Never show same exercise twice in one session
- Alternate between exercise types (MC → translation → MC)
- Insert "breather" exercises after 2+ hard ones
- Offer "Daily Challenge" with unique constraints

### 1.5 Flow State Targeting

Based on Vygotsky's Zone of Proximal Development:

```rust
fn adjust_difficulty(state: &mut UserState, correct: bool) {
    if correct {
        state.consecutive_correct += 1;
        state.consecutive_wrong = 0;

        // After 5+ correct, bump up difficulty
        if state.consecutive_correct >= 5 {
            state.suggested_difficulty = (state.suggested_difficulty + 1).min(5);
            state.consecutive_correct = 0;
            // Scaffolding: first exercise at new difficulty gets support
            state.rank_up_bonus = true;
        }
    } else {
        state.consecutive_wrong += 1;
        state.consecutive_correct = 0;

        // After 2 wrong, offer help
        if state.consecutive_wrong >= 2 {
            state.show_hint = true;
            state.suggested_difficulty = (state.suggested_difficulty - 1).max(1);
        }
    }
}
```

**Rank Up Scaffolding:** When a learner ranks up to a new difficulty level, the first exercise includes:
- A free hint (no penalty for using it)
- +5 bonus seconds on timed exercises
- Encouraging "Rank Up!" celebration animation

This eases the transition and prevents discouragement at the difficulty spike.

### 1.6 Milestone Celebrations

Instead of constant small rewards, create memorable milestone moments:

| Milestone | Celebration | Reward |
|-----------|-------------|--------|
| Level 10 | Special "Scholar" ceremony animation | Title unlock |
| First perfect module | Confetti + achievement showcase | Badge |
| 7-day streak | Heartfelt message + freeze reward | +1 freeze, XP bonus |
| 100 correct answers | "Century" achievement with fanfare | Title unlock |
| Complete Era | Full-screen celebration, badge | Era badge |

**Implementation:**
```rust
fn check_milestones(progress: &UserProgress) -> Option<Milestone> {
    // Check for milestone conditions
    // Return milestone data for special celebration UI
    // Milestones are one-time events (stored in achievements)
}
```

### 1.7 Removing "Chore" Elements

Per council review, remove anything that feels like a chore:

**Remove or Rethink:**
- Daily login rewards (feels like obligation)
- Streak pressure (anxiety-inducing)
- XP decay (punitive)

**Replace with:**
- Discovery rewards (find something new = XP)
- Exploration bonuses (try a new module = XP)
- Comeback celebrations (haven't played in a while? Welcome back bonus!)

---

## Part 2: Curriculum Gemini Review

**Status: ✅ PHASE 1 COMPLETE** — Council feedback applied to sample exercises

### 2.1 Pedagogical Council Review

The Pedagogical Council (Part 1) reviewed 8 sample exercises and provided structured feedback. All recommendations implemented.

#### Exercises Updated

| ID | Module | Key Changes |
|----|--------|-------------|
| A_1.1 | Syllogistic | Socratic hint: "Is 'you' a specific person or a general class?" |
| C_6.1 | Propositional | Agency analogy: "Pass exam → Learn OR Practice" (goal planning) |
| J_4.1 | Modal | "Doorways to possible worlds" metaphor |
| L_12.3 | Deontic | Agent legend explaining `s:` and `u:` notation |
| N_14.1 | Belief | Fixed LaTeX, added "believer : belief-content" scaffold |
| Q_3.1 | Informal | Fixed `correct` index (3→5), neutral definition hint |
| ex_03 | Universal | Debug-style hint: "Would a Cat exist in this world?" |
| ex_01 | Scope | Mother vs President scope analogy |

#### Pedagogical Principles Applied

1. **Socratic Method** — Hints ask discovery questions, not answers
2. **Agency-Focused Analogies** — Examples connect to real decisions (no trivial food examples)
3. **Scaffolded Explanations** — Build understanding step-by-step
4. **Debug Thinking** — Encourage "testing" logic like debugging code

#### Sample Transformation

**Before:**
```json
"hint": "Think about grouping"
```

**After:**
```json
"hint": "Imagine P is 'Pass the exam', L is 'Learn the material', D is 'Do extra practice'. To pass, you need to (Learn OR Practice). Where do the parentheses go?",
"explanation": "Think of it like planning success: 'I will Pass AND (Learn OR Practice)'. The parentheses show 'Learn or Practice' is your choice of METHOD..."
```

### 2.2 Transformation Patterns (Council-Approved)

After Phase 1 & 2, we've established these reusable patterns:

| Pattern | Before | After |
|---------|--------|-------|
| **Socratic Hint** | `"hint": null` | Question that prompts discovery, not answer |
| **Debug Thinking** | "Use uppercase" | "Would a Cat exist in this world?" |
| **Function Analogy** | Technical jargon | "Like a function call in programming" |
| **Contrast Pairs** | Single explanation | "Compare to X which would mean..." |
| **Real-world Stakes** | Abstract symbols | "Like planning your success" |

**Hint Templates by Module:**

| Module | Hint Pattern |
|--------|-------------|
| Syllogistic | "Does X refer to a specific person, or a category of people?" |
| Propositional | "How many [operator]s do you see? Does each X get its own?" |
| Modal | "Is the sentence saying X IS true, or that X COULD BE true?" |
| Definitions | "Can you think of something that fits this definition but ISN'T a [term]?" |
| Quantifiers | "If this logic were true, would [counterexample] exist?" |

### 2.3 Full Curriculum Review (Remaining ~370 exercises)

**Location:** `/assets/curriculum/`
**Statistics:** ~382 JSON files, 12 updated in Phase 1 + Phase 2

### 2.4 Review Script

```bash
#!/bin/bash
# curriculum-review.sh
# Concatenate curriculum for Gemini review

OUTPUT="curriculum_bundle.txt"
> "$OUTPUT"

for era in 00_logicaffeine 01_trivium 02_quadrivium 03_metaphysics; do
    echo "=== ERA: $era ===" >> "$OUTPUT"
    cat "assets/curriculum/$era/meta.json" >> "$OUTPUT"
    echo "" >> "$OUTPUT"

    for module in assets/curriculum/$era/*/; do
        if [ -d "$module" ]; then
            echo "--- MODULE: $(basename $module) ---" >> "$OUTPUT"
            cat "$module/meta.json" >> "$OUTPUT"
            echo "" >> "$OUTPUT"

            for exercise in "$module"/*.json; do
                if [ -f "$exercise" ] && [ "$(basename $exercise)" != "meta.json" ]; then
                    cat "$exercise" >> "$OUTPUT"
                    echo "" >> "$OUTPUT"
                fi
            done
        fi
    done
done

echo "Bundle created: $OUTPUT ($(wc -l < $OUTPUT) lines)"
```

### 2.5 Gemini Review Prompt

```
You are reviewing a curriculum for Logicaffeine, an educational app that teaches
First-Order Logic through English-to-logic translation exercises.

The app has two interactive modes available on the Guide page:
1. LOGIC MODE: User writes English, system outputs First-Order Logic (FOL)
2. IMPERATIVE MODE: User writes LOGOS code, system executes it

For EACH exercise in the curriculum, provide a structured assessment:

## ACCESSIBILITY (1-5 scale)
- Is the prompt clear to someone unfamiliar with formal logic?
- Are the options/expected answers unambiguous?
- Is the difficulty appropriate for its position in the curriculum?
- Are there sufficient hints?

## INTERACTIVITY POTENTIAL (Low/Medium/High)
- Could this be converted to a code-writing exercise using Logic or Imperative mode?
- Would interactivity enhance understanding of this concept?
- What type of interactive exercise would work best?

## MAIEUTICS (Socratic Method)
- Does the hint guide through questioning rather than telling?
- Could the hint be rephrased as a question that leads to discovery?
- Does it prompt the learner to examine their reasoning?
- Example: Instead of "Use 'All'" → "Is there any instance where this is NOT true?"

## SPECIFIC FEEDBACK
- Any confusing wording that should be clarified?
- Missing hints that would help struggling learners?
- Opportunities for real-world examples or analogies?
- Is there unnecessary jargon that could be simplified?

## RECOMMENDED CHANGES
- Concrete suggestions for improvement
- Priority: High (confusing), Medium (could be better), Low (minor polish)

Output as structured JSON for easy processing:
{
  "exercise_id": "ex_01",
  "accessibility_score": 4,
  "interactivity_potential": "Medium",
  "issues": ["hint could be more specific"],
  "recommendations": [
    {"change": "Add example using 'All cats are mammals'", "priority": "Medium"}
  ]
}
```

### 2.4 Interactive Exercise Types (Hybrid Approach)

Keep existing exercise types, ADD new interactive types:

| Type | Description | Validation | Where Used |
|------|-------------|------------|------------|
| `multiple_choice` | Select correct answer | Match index | Keep everywhere |
| `translation` | Template-based translation | Pattern match | Keep everywhere |
| `code_logic` | Write English → target FOL | Compare output | Advanced modules |
| `code_imperative` | Write LOGOS code | Compare stdout | Future |
| `debug` | Fix buggy code | Output matches | Future |

### 2.5 Example Interactive Exercise

```json
{
  "id": "ex_interactive_001",
  "type": "code_logic",
  "difficulty": 2,
  "prompt": "Write an English sentence that produces the following First-Order Logic:",
  "target_fol": "∀x(Cat(x) → Mammal(x))",
  "hints": [
    "Think about a universal statement - something true for ALL of a category",
    "Use words like 'all', 'every', or 'each'",
    "The arrow (→) means 'implies' or 'if...then'"
  ],
  "starter_code": "Every cat ___",
  "validation": "exact_match",
  "acceptable_outputs": [
    "∀x(Cat(x) → Mammal(x))",
    "∀x(Cat(x) ⊃ Mammal(x))"
  ],
  "explanation": "Universal statements translate to 'for all x, if x is a Cat, then x is a Mammal'. The English phrasing 'Every cat is a mammal' captures this perfectly."
}
```

### 2.6 Curriculum Review Timeline

| Phase | Task | Effort |
|-------|------|--------|
| 1 | Export curriculum bundle | 0.5d |
| 2 | Run Gemini batch review | 1d |
| 3 | Analyze and prioritize feedback | 1d |
| 4 | Fix high-priority accessibility issues | 2d |
| 5 | Design interactive exercise schema | 1d |
| 6 | Implement `code_logic` exercise type in UI | 2d |
| 7 | Convert 10% of exercises to interactive | 3d |
| 8 | User testing + iteration | Ongoing |

### 2.7 Success Metrics

After curriculum improvements:
- **Accessibility:** Average score ≥ 4.0 across all exercises
- **Completion rate:** Increase module completion by 20%
- **User feedback:** Reduce "confusing" complaints by 50%
- **Engagement:** Users try interactive exercises when available

---

## Part 3: Cloudflare Infrastructure Analysis

### 3.1 Current State

| Service | Platform | URL | Status |
|---------|----------|-----|--------|
| Frontend | Cloudflare Pages | logicaffeine.com | Production |
| License API | Cloudflare Worker (JS) | api.logicaffeine.com | Production |
| Registry | Cloudflare Worker (JS) | registry.logicaffeine.com | Production |
| Registry DB | Cloudflare D1 | logos-registry | Production |
| Package Storage | Cloudflare R2 | logos-packages | Production |

**Current Workers are JavaScript** - we'll port to Rust using `workers-rs`.

### 3.2 D1 vs R2 SQL Comparison

| Feature | D1 | R2 SQL |
|---------|-----|--------|
| **Purpose** | Transactional database | Analytics on Iceberg tables |
| **Query Type** | SQLite (full SQL) | Distributed analytics |
| **Best For** | User data, auth, progress | Large-scale data analysis |
| **Latency** | Low (< 1ms reads) | Higher (analytics workloads) |
| **Write Speed** | Fast (sequential) | N/A (read-only on Iceberg) |
| **Max DB Size** | 10 GB (paid) | Unlimited (object storage) |
| **Free Tier** | 500 MB, 5M reads/day | Beta (no charge currently) |
| **Status** | GA | Open Beta |
| **Primary Use** | All transactional data | Package storage, game snapshots only |

**Decision: D1 is the primary database. R2 is NOT a database alternative.**

D1 handles all transactional data (users, progress, leaderboards, tournaments, sessions). R2 is used exclusively for:
- Package storage (registry)
- Game state snapshots (for client polling during battles)
- Large file storage

R2 SQL is designed for analytics workloads on Apache Iceberg tables, not real-time application data.

### 3.3 R2 Constraints for Multiplayer

From [Cloudflare R2 Limits](https://developers.cloudflare.com/r2/platform/limits/):

| Limit | Value | Impact |
|-------|-------|--------|
| **Writes per object key** | 1/second | Critical for game state |
| **Object size** | Up to 5 TiB | No issue |
| **Buckets per account** | 1,000,000 | Can use per-user buckets |
| **Bucket management ops** | 50/second | Can't create buckets per game |

**Key Insight:** R2's 1 write/sec limit means multiplayer battles must be designed with **delayed synchronization** (not real-time). R2 is used ONLY for game state snapshot distribution to clients, NOT for transactional game data (which lives in D1).

### 3.4 workers-rs Capabilities

From [workers-rs GitHub](https://github.com/cloudflare/workers-rs):

```rust
use worker::*;

#[event(fetch)]
async fn fetch(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    // D1 access
    let db = env.d1("DB")?;
    let stmt = db.prepare("SELECT * FROM users WHERE id = ?1");
    let user: Option<User> = stmt.bind(&[id])?.first().await?;

    // R2 access
    let bucket = env.bucket("PACKAGES")?;
    let object = bucket.get("key").execute().await?;

    // KV access
    let kv = env.kv("CACHE")?;
    let value = kv.get("key").text().await?;

    Response::ok("Hello from Rust!")
}
```

**D1 bindings require the `d1` feature flag.** The crate is marked "work-in-progress" but actively maintained.

---

## Part 4: Crate Separation Strategy

### 4.1 Current Structure

```
logicaffeine/
├── Cargo.toml              # Main crate (logos) - compiler + UI
├── logos_core/             # Runtime library
│   └── Cargo.toml          # tokio, rayon, serde, sha2, uuid
└── logos_verification/     # Z3-based verification (Pro+ license)
    └── Cargo.toml          # z3, ureq, dirs
```

**Problem:** The main `logos` crate includes Dioxus UI, which brings in web dependencies that shouldn't be in the core compiler/language.

### 4.2 Proposed Structure

```
logicaffeine/
├── Cargo.toml                    # Workspace root
│
├── logos/                        # Core compiler (CLEAN - no UI deps)
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs               # Lexer, parser, codegen, transpile
│       ├── lexer.rs
│       ├── parser/
│       ├── codegen.rs
│       └── ...
│
├── logos_core/                   # Runtime library (unchanged)
│   └── Cargo.toml
│
├── logos_verification/           # Z3 verification (unchanged)
│   └── Cargo.toml
│
├── logos_web/                    # Dioxus frontend (NEW)
│   ├── Cargo.toml               # dioxus, gloo-net, web-sys
│   └── src/
│       ├── main.rs
│       ├── ui/
│       │   ├── pages/
│       │   ├── components/
│       │   └── state.rs
│       ├── progress.rs
│       ├── game.rs
│       └── achievements.rs
│
├── logos_cli/                    # CLI tool (NEW)
│   ├── Cargo.toml               # clap, ureq, etc.
│   └── src/main.rs
│
└── workers/                      # Cloudflare Workers (Rust)
    ├── api/                      # api.logicaffeine.com
    │   ├── Cargo.toml           # worker, serde
    │   ├── wrangler.toml
    │   └── src/lib.rs
    │
    └── registry/                 # registry.logicaffeine.com
        ├── Cargo.toml
        ├── wrangler.toml
        └── src/lib.rs
```

### 4.3 Dependency Isolation

| Crate | Dependencies | Purpose |
|-------|--------------|---------|
| `logos` | serde, bumpalo | Core compiler - zero UI deps |
| `logos_core` | tokio, rayon, serde | Runtime for compiled programs |
| `logos_verification` | z3, ureq | SMT verification (optional) |
| `logos_web` | dioxus, gloo-net, logos | Web frontend |
| `logos_cli` | clap, logos | Command-line tool |
| `workers/api` | worker, serde | Cloudflare Worker |
| `workers/registry` | worker, serde | Cloudflare Worker |

**Result:** `cargo build -p logos` produces a clean compiler with no web dependencies.

---

## Part 6: Rust Workers Implementation

### 6.1 Porting Strategy

**Current:** JavaScript Workers in `/worker/` and `/registry/`
**Target:** Rust Workers using `workers-rs`

```toml
# workers/api/Cargo.toml
[package]
name = "logicaffeine-api"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
worker = { version = "0.4", features = ["d1", "http"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
getrandom = { version = "0.2", features = ["js"] }

[profile.release]
lto = true
opt-level = "s"
codegen-units = 1
strip = true
```

```toml
# workers/api/wrangler.toml
name = "logicaffeine-api"
main = "build/worker/shim.mjs"
compatibility_date = "2024-01-01"

[build]
command = "cargo install -q worker-build && worker-build --release"

[[d1_databases]]
binding = "DB"
database_name = "logicaffeine-users"
database_id = "..."

[[r2_buckets]]
binding = "GAME_STATES"
bucket_name = "logicaffeine-games"

[vars]
ALLOWED_ORIGIN = "https://logicaffeine.com"
```

### 6.2 Unified API Worker (Rust)

```rust
// workers/api/src/lib.rs
use worker::*;
use serde::{Deserialize, Serialize};

mod auth;
mod progress;
mod leaderboard;
mod battles;
mod payments;

#[event(fetch)]
async fn fetch(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    let router = Router::new();

    router
        // Health
        .get("/health", |_, _| Response::ok("ok"))

        // Auth routes (OAuth only - no email/password)
        .get("/auth/github", auth::github_redirect)
        .get("/auth/github/callback", auth::github_callback)
        .get("/auth/google", auth::google_redirect)
        .get("/auth/google/callback", auth::google_callback)
        .post("/auth/refresh", auth::refresh)
        .get("/auth/me", auth::me)

        // MFA stubs (future implementation for registry protection)
        .post("/auth/mfa/setup", auth::mfa_setup_stub)
        .post("/auth/mfa/verify", auth::mfa_verify_stub)

        // Progress routes
        .get("/progress", progress::get_progress)
        .post("/progress/sync", progress::sync)
        .post("/progress/xp", progress::record_xp)

        // Leaderboard routes
        .get("/leaderboard", leaderboard::global)
        .get("/leaderboard/weekly", leaderboard::weekly)
        .get("/leaderboard/:era", leaderboard::by_era)

        // Battle routes (multiplayer)
        .post("/battles/create", battles::create)
        .get("/battles/:id", battles::get_state)
        .post("/battles/:id/action", battles::submit_action)

        // Tournament routes
        .get("/tournaments", tournaments::list)
        .post("/tournaments/create", tournaments::create)
        .post("/tournaments/:id/join", tournaments::join)
        .get("/tournaments/:id", tournaments::get_state)
        .post("/tournaments/:id/submit", tournaments::submit_answer)
        .get("/tournaments/queue", tournaments::get_queue)
        .post("/tournaments/queue/join", tournaments::join_queue)
        .delete("/tournaments/queue/leave", tournaments::leave_queue)

        // Payment routes
        .post("/session", payments::handle_session)
        .post("/validate", payments::validate_license)
        .post("/webhook/stripe", payments::stripe_webhook)

        .run(req, env)
        .await
}
```

### 6.3 D1 Database Access in Rust

```rust
// workers/api/src/auth.rs
use worker::*;

#[derive(Debug, Serialize, Deserialize)]
struct User {
    id: String,
    email: Option<String>,
    display_name: String,
    github_id: Option<String>,
    google_id: Option<String>,
    password_hash: Option<String>,
    created_at: String,
}

pub async fn me(req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let user = authenticate(&req, &ctx.env).await?;

    let db = ctx.env.d1("DB")?;

    // Get user's progress
    let progress = db
        .prepare("SELECT xp, level, current_streak FROM user_progress WHERE user_id = ?1")
        .bind(&[user.id.clone().into()])?
        .first::<UserProgress>(None)
        .await?;

    // Get achievements
    let achievements = db
        .prepare("SELECT achievement_id, unlocked_at FROM user_achievements WHERE user_id = ?1")
        .bind(&[user.id.clone().into()])?
        .all()
        .await?;

    Response::from_json(&UserProfile {
        user,
        progress,
        achievements: achievements.results()?,
    })
}

async fn authenticate(req: &Request, env: &Env) -> Result<User> {
    let auth_header = req.headers().get("Authorization")?
        .ok_or_else(|| Error::from("Missing Authorization header"))?;

    if !auth_header.starts_with("Bearer ") {
        return Err(Error::from("Invalid Authorization header"));
    }

    let token = &auth_header[7..];

    // Verify JWT and get user
    let claims = verify_jwt(token, env)?;

    let db = env.d1("DB")?;
    db.prepare("SELECT * FROM users WHERE id = ?1")
        .bind(&[claims.sub.into()])?
        .first::<User>(None)
        .await?
        .ok_or_else(|| Error::from("User not found"))
}
```

---

## Part 9: Multiplayer Battle System

### 9.1 Design Constraints

**R2 Limitation:** 1 write per second per object key

**Implications:**
- No real-time synchronization possible
- All actions are delayed by at least 1 second
- Need game design that embraces latency

### 9.2 Battle Architecture

```
┌──────────────────────────────────────────────────────────────────────────┐
│                     MULTIPLAYER BATTLE SYSTEM                            │
├──────────────────────────────────────────────────────────────────────────┤
│                                                                          │
│  DESIGN: Turn-based battles with 1-second tick resolution               │
│                                                                          │
│  ┌─────────────┐         ┌─────────────┐         ┌─────────────┐        │
│  │  Player A   │         │   Worker    │         │  Player B   │        │
│  └──────┬──────┘         └──────┬──────┘         └──────┬──────┘        │
│         │                       │                       │               │
│         │  Submit answer        │                       │               │
│         │──────────────────────▶│                       │               │
│         │                       │  Store in D1          │               │
│         │                       │  (action queue)       │               │
│         │                       │                       │               │
│         │                       │◀──────────────────────│               │
│         │                       │  Submit answer        │               │
│         │                       │                       │               │
│         │        ───────────────│───────────────        │               │
│         │        │  TICK (1s)   │              │        │               │
│         │        ───────────────│───────────────        │               │
│         │                       │                       │               │
│         │                       │  Process actions      │               │
│         │                       │  Update game state    │               │
│         │                       │  Write to R2          │               │
│         │                       │                       │               │
│         │◀──────────────────────│──────────────────────▶│               │
│         │  Poll game state      │  Poll game state      │               │
│         │  (from R2 directly)   │  (from R2 directly)   │               │
│                                                                          │
└──────────────────────────────────────────────────────────────────────────┘
```

### 9.3 Game State Storage Strategy

| Data | Storage | Reason |
|------|---------|--------|
| Active games | D1 | Transactional, querying |
| Action queue | D1 | Needs ordering |
| Game snapshots | R2 | Large state, client reads directly |
| Final results | D1 | Leaderboard integration |

```sql
-- D1 Schema for battles
CREATE TABLE battles (
    id TEXT PRIMARY KEY,
    player_a_id TEXT NOT NULL,
    player_b_id TEXT NOT NULL,
    exercise_set TEXT NOT NULL,      -- JSON array of exercise IDs
    current_index INTEGER DEFAULT 0,
    player_a_score INTEGER DEFAULT 0,
    player_b_score INTEGER DEFAULT 0,
    status TEXT DEFAULT 'waiting',   -- waiting, active, finished
    created_at TEXT DEFAULT (datetime('now')),
    started_at TEXT,
    finished_at TEXT,
    winner_id TEXT
);

CREATE TABLE battle_actions (
    id TEXT PRIMARY KEY,
    battle_id TEXT NOT NULL,
    player_id TEXT NOT NULL,
    exercise_index INTEGER NOT NULL,
    answer TEXT NOT NULL,
    is_correct INTEGER,
    submitted_at TEXT DEFAULT (datetime('now')),
    processed INTEGER DEFAULT 0
);

CREATE INDEX idx_actions_unprocessed ON battle_actions(battle_id, processed);
```

### 9.4 Battle Flow

```rust
// workers/api/src/battles.rs

/// Create a new battle (matchmaking or challenge)
pub async fn create(mut req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let user = authenticate(&req, &ctx.env).await?;
    let body: CreateBattleRequest = req.json().await?;

    let db = ctx.env.d1("DB")?;

    // Generate random exercise set
    let exercises = generate_exercise_set(10)?;
    let battle_id = generate_id();

    db.prepare(r#"
        INSERT INTO battles (id, player_a_id, player_b_id, exercise_set, status)
        VALUES (?1, ?2, ?3, ?4, 'waiting')
    "#)
    .bind(&[
        battle_id.clone().into(),
        user.id.into(),
        body.opponent_id.unwrap_or_default().into(),
        serde_json::to_string(&exercises)?.into(),
    ])?
    .run()
    .await?;

    Response::from_json(&CreateBattleResponse { battle_id })
}

/// Submit an action (answer)
pub async fn submit_action(mut req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let user = authenticate(&req, &ctx.env).await?;
    let battle_id = ctx.param("id").unwrap();
    let body: SubmitActionRequest = req.json().await?;

    let db = ctx.env.d1("DB")?;

    // Verify user is in this battle
    let battle = db
        .prepare("SELECT * FROM battles WHERE id = ?1")
        .bind(&[battle_id.into()])?
        .first::<Battle>(None)
        .await?
        .ok_or_else(|| Error::from("Battle not found"))?;

    if battle.player_a_id != user.id && battle.player_b_id != user.id {
        return Err(Error::from("Not in this battle"));
    }

    // Queue the action (processed by tick worker)
    db.prepare(r#"
        INSERT INTO battle_actions (id, battle_id, player_id, exercise_index, answer)
        VALUES (?1, ?2, ?3, ?4, ?5)
    "#)
    .bind(&[
        generate_id().into(),
        battle_id.into(),
        user.id.into(),
        body.exercise_index.into(),
        body.answer.into(),
    ])?
    .run()
    .await?;

    Response::ok("Action queued")
}

/// Get current game state (clients poll this from R2)
pub async fn get_state(_req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let battle_id = ctx.param("id").unwrap();

    let bucket = ctx.env.bucket("GAME_STATES")?;
    let key = format!("battles/{}/state.json", battle_id);

    match bucket.get(&key).execute().await? {
        Some(obj) => {
            let body = obj.body().ok_or_else(|| Error::from("Empty object"))?;
            Response::from_body(ResponseBody::Stream(body))
        }
        None => {
            // Fall back to D1 for initial state
            let db = ctx.env.d1("DB")?;
            let battle = db
                .prepare("SELECT * FROM battles WHERE id = ?1")
                .bind(&[battle_id.into()])?
                .first::<Battle>(None)
                .await?
                .ok_or_else(|| Error::from("Battle not found"))?;

            Response::from_json(&battle)
        }
    }
}
```

### 9.5 Tick Worker (Scheduled)

```rust
// Runs every second via Cron Trigger
#[event(scheduled)]
async fn scheduled(_event: ScheduledEvent, env: Env, _ctx: ScheduleContext) {
    let db = env.d1("DB").unwrap();
    let bucket = env.bucket("GAME_STATES").unwrap();

    // Get active battles with pending actions
    let battles: Vec<Battle> = db
        .prepare(r#"
            SELECT DISTINCT b.* FROM battles b
            JOIN battle_actions a ON b.id = a.battle_id
            WHERE b.status = 'active' AND a.processed = 0
        "#)
        .all()
        .await
        .unwrap()
        .results()
        .unwrap();

    for battle in battles {
        process_battle_tick(&db, &bucket, &battle).await;
    }
}

async fn process_battle_tick(db: &D1Database, bucket: &Bucket, battle: &Battle) {
    // Get unprocessed actions for this battle
    let actions: Vec<BattleAction> = db
        .prepare(r#"
            SELECT * FROM battle_actions
            WHERE battle_id = ?1 AND processed = 0
            ORDER BY submitted_at ASC
        "#)
        .bind(&[battle.id.clone().into()])
        .unwrap()
        .all()
        .await
        .unwrap()
        .results()
        .unwrap();

    // Process each action
    for action in &actions {
        let is_correct = grade_answer(&battle.exercise_set, action.exercise_index, &action.answer);

        // Update action
        db.prepare("UPDATE battle_actions SET is_correct = ?1, processed = 1 WHERE id = ?2")
            .bind(&[is_correct.into(), action.id.clone().into()])
            .unwrap()
            .run()
            .await
            .unwrap();

        // Update scores
        if is_correct {
            let score_col = if action.player_id == battle.player_a_id {
                "player_a_score"
            } else {
                "player_b_score"
            };
            db.prepare(&format!("UPDATE battles SET {} = {} + 1 WHERE id = ?1", score_col, score_col))
                .bind(&[battle.id.clone().into()])
                .unwrap()
                .run()
                .await
                .unwrap();
        }
    }

    // Write updated state to R2 (1 write per tick is fine)
    let updated_battle = db
        .prepare("SELECT * FROM battles WHERE id = ?1")
        .bind(&[battle.id.clone().into()])
        .unwrap()
        .first::<Battle>(None)
        .await
        .unwrap()
        .unwrap();

    let state_json = serde_json::to_string(&BattleState::from(updated_battle)).unwrap();
    bucket
        .put(&format!("battles/{}/state.json", battle.id), state_json)
        .execute()
        .await
        .unwrap();
}
```

### 9.6 Tournament System Architecture

Tournaments extend the battle system with competitive multi-player modes, time pressure, and a lives system.

#### 9.6.1 Tournament Modes

| Mode | Timer Style | Lives | Description |
|------|-------------|-------|-------------|
| **Blitz** | Per-move (10-30s) | 3 | Fast-paced, first mistake costs a life |
| **Classic** | Chess-clock (5 min total) | 3 | Strategic, time bank depletes while thinking |
| **Marathon** | 30 min overall | 5 | Endurance, survive the longest |
| **Sprint** | 1 min rounds | 1 | Race format, 1 life per round |
| **Co-op** | Shared chess-clock (8 min) | 5 | Two players solve together against the clock |

**Co-op Mode Details:**
- Two players share a timer and lives pool
- Either player can submit an answer
- Chat/ping system for coordination ("I got this one" / "Help?")
- Designed for friends learning together or mentor-student pairing
- Unranked only (does not affect individual ranking)

#### 9.6.2 Lives System

```rust
pub struct PlayerTournamentState {
    pub lives_remaining: u8,      // Starts at 3 (configurable)
    pub score: u32,               // Points earned
    pub time_remaining_ms: u64,   // For chess-clock mode
    pub status: PlayerStatus,     // Active, Eliminated, Winner
}

pub enum PlayerStatus {
    Active,
    Eliminated { at_round: u32 },
    Winner,
}

// Game logic
impl PlayerTournamentState {
    pub fn record_answer(&mut self, correct: bool) -> AnswerResult {
        if correct {
            self.score += 1;
            AnswerResult::Correct
        } else {
            self.lives_remaining = self.lives_remaining.saturating_sub(1);
            if self.lives_remaining == 0 {
                self.status = PlayerStatus::Eliminated { at_round: current_round };
                AnswerResult::Eliminated
            } else {
                AnswerResult::Wrong { lives_left: self.lives_remaining }
            }
        }
    }
}
```

#### 9.6.3 Tournament Structure

- **Queue Size:** 8, 16, 32, or 64 players
- **Bracket Type:** Single elimination or round-robin
- **Round Duration:** 1-minute rounds within 30-minute overall timer
- **Auto-start:** Tournament begins when queue is full
- **Exercise Selection:** Random from difficulty-matched pool

#### 9.6.4 Tournament Rewards

Winners receive BOTH XP bonuses AND exclusive titles:

| Placement | XP Multiplier | Title Unlock |
|-----------|---------------|--------------|
| 1st Place | 5x base XP | "Tournament Champion" |
| 2nd Place | 3x base XP | "Silver Logician" |
| 3rd Place | 2x base XP | "Bronze Reasoner" |
| Top 8 | 1.5x base XP | - |

Mode-specific titles:
- **Blitz Master** - Win 10 Blitz tournaments
- **Marathon Runner** - Survive 5 Marathon tournaments
- **Speed Demon** - Win a Sprint tournament with 0 time remaining

Community titles:
- **Mentor** - Play 25 unranked matches against players 300+ ranks below you
- **Guide** - Help 10 unique new players complete their first tournament
- **Sage** - Earn both "Tournament Champion" and "Mentor" titles

### 9.7 Time Battle Modes

#### Per-Move Timer Mode
```rust
pub struct PerMoveTimer {
    pub time_per_move_ms: u64,    // e.g., 15000 (15 seconds)
    pub current_move_start: u64,   // Unix timestamp when move started
}

impl PerMoveTimer {
    pub fn check_timeout(&self, now_ms: u64) -> bool {
        now_ms - self.current_move_start > self.time_per_move_ms
    }

    pub fn reset(&mut self, now_ms: u64) {
        self.current_move_start = now_ms;
    }
}
```

- Each move has a fixed time limit (e.g., 15 seconds)
- Timer resets after each submission
- Timeout = automatic wrong answer (lose a life)

#### Chess-Clock Mode
```rust
pub struct ChessClock {
    pub total_time_ms: u64,       // e.g., 300000 (5 minutes)
    pub time_remaining_ms: u64,
    pub last_tick_at: u64,
    pub is_active: bool,
}

impl ChessClock {
    pub fn tick(&mut self, now_ms: u64) {
        if self.is_active {
            let elapsed = now_ms - self.last_tick_at;
            self.time_remaining_ms = self.time_remaining_ms.saturating_sub(elapsed);
            self.last_tick_at = now_ms;
        }
    }

    pub fn is_out_of_time(&self) -> bool {
        self.time_remaining_ms == 0
    }

    pub fn pause(&mut self) {
        self.is_active = false;
    }

    pub fn resume(&mut self, now_ms: u64) {
        self.is_active = true;
        self.last_tick_at = now_ms;
    }
}
```

- Total time bank per player (e.g., 5 minutes)
- Depletes while thinking
- Running out of time = elimination

### 9.8 Matchmaking System

#### Global Queue

Skill-based matching using ELO-style ranking:

```rust
pub async fn find_match(db: &D1Database, user_id: &str, ranking: i32) -> Option<String> {
    // Find players within 200 ranking points, queued in last 60 seconds
    let candidates = db
        .prepare(r#"
            SELECT user_id FROM matchmaking_queue
            WHERE user_id != ?1
            AND skill_ranking BETWEEN ?2 AND ?3
            AND queued_at > datetime('now', '-60 seconds')
            ORDER BY ABS(skill_ranking - ?4)
            LIMIT 7
        "#)
        .bind(&[
            user_id.into(),
            (ranking - 200).into(),
            (ranking + 200).into(),
            ranking.into(),
        ])?
        .all()
        .await?
        .results()?;

    if candidates.len() >= 7 {
        // Create tournament with these 8 players (including current user)
        Some(create_tournament(db, user_id, candidates).await?)
    } else {
        None
    }
}
```

**Ranking Visibility:** Optional - players can choose to show or hide their ranking on their profile.

#### Private Lobbies

```rust
pub async fn create_private_lobby(
    db: &D1Database,
    creator_id: &str,
    config: TournamentConfig,
) -> Result<PrivateLobby> {
    let invite_code = generate_invite_code(); // e.g., "LOGIC-ABCD"
    let tournament_id = generate_id();

    db.prepare(r#"
        INSERT INTO tournaments (id, name, mode, max_players, is_private, invite_code, created_by)
        VALUES (?1, ?2, ?3, ?4, 1, ?5, ?6)
    "#)
    .bind(&[
        tournament_id.clone().into(),
        config.name.into(),
        config.mode.into(),
        config.max_players.into(),
        invite_code.clone().into(),
        creator_id.into(),
    ])?
    .run()
    .await?;

    Ok(PrivateLobby { tournament_id, invite_code })
}
```

- Creator gets unique invite code (e.g., "LOGIC-ABCD")
- Share code with friends
- Start manually when ready OR auto-start when full

### 9.9 D1 Schema for Tournaments

```sql
CREATE TABLE tournaments (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    mode TEXT NOT NULL,                -- blitz, classic, marathon, sprint
    max_players INTEGER DEFAULT 8,
    current_players INTEGER DEFAULT 0,
    status TEXT DEFAULT 'waiting',     -- waiting, active, finished
    timer_per_move INTEGER,            -- seconds, NULL for chess-clock
    timer_total INTEGER,               -- seconds for chess-clock mode
    lives_per_player INTEGER DEFAULT 3,
    overall_timer INTEGER DEFAULT 1800, -- 30 minutes
    exercise_difficulty INTEGER DEFAULT 2,
    is_private INTEGER DEFAULT 0,
    invite_code TEXT UNIQUE,           -- for private lobbies
    created_by TEXT NOT NULL,
    created_at TEXT DEFAULT (datetime('now')),
    started_at TEXT,
    finished_at TEXT
);

CREATE TABLE tournament_players (
    tournament_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    lives_remaining INTEGER DEFAULT 3,
    score INTEGER DEFAULT 0,
    time_remaining INTEGER,            -- for chess-clock mode (ms)
    current_round INTEGER DEFAULT 0,
    status TEXT DEFAULT 'active',      -- active, eliminated, winner
    joined_at TEXT DEFAULT (datetime('now')),
    eliminated_at TEXT,
    final_placement INTEGER,
    PRIMARY KEY (tournament_id, user_id)
);

CREATE TABLE tournament_rounds (
    id TEXT PRIMARY KEY,
    tournament_id TEXT NOT NULL,
    round_number INTEGER NOT NULL,
    exercise_id TEXT NOT NULL,
    started_at TEXT DEFAULT (datetime('now')),
    ended_at TEXT
);

CREATE TABLE tournament_answers (
    id TEXT PRIMARY KEY,
    tournament_id TEXT NOT NULL,
    round_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    answer TEXT NOT NULL,
    is_correct INTEGER,
    time_taken INTEGER,                -- milliseconds
    submitted_at TEXT DEFAULT (datetime('now'))
);

CREATE TABLE matchmaking_queue (
    user_id TEXT PRIMARY KEY,
    skill_ranking INTEGER DEFAULT 1000,
    queued_at TEXT DEFAULT (datetime('now')),
    preferred_mode TEXT
);

CREATE TABLE player_rankings (
    user_id TEXT PRIMARY KEY,
    ranking INTEGER DEFAULT 1000,
    games_played INTEGER DEFAULT 0,
    wins INTEGER DEFAULT 0,
    ranking_visible INTEGER DEFAULT 1,  -- optional visibility
    updated_at TEXT DEFAULT (datetime('now'))
);

CREATE INDEX idx_tournaments_status ON tournaments(status);
CREATE INDEX idx_tournament_players ON tournament_players(tournament_id, status);
CREATE INDEX idx_tournament_answers ON tournament_answers(tournament_id, round_id);
CREATE INDEX idx_queue_skill ON matchmaking_queue(skill_ranking, queued_at);
```

---

## Part 8: Payment & License System

### 8.1 Current Issues

From analyzing `/worker/src/index.js`:

1. **No license key generation** - Returns Stripe subscription ID directly
2. **No persistent storage** - Licenses not stored in D1
3. **No email delivery** - Users don't receive license keys
4. **Validation depends on Stripe API** - Every validation hits Stripe

### 8.2 Improved Payment Flow

```
┌────────────────────────────────────────────────────────────────────────────┐
│                         PAYMENT FLOW (FIXED)                               │
├────────────────────────────────────────────────────────────────────────────┤
│                                                                            │
│  1. User clicks "Buy" on pricing page                                     │
│     └──▶ Redirect to Stripe Checkout                                      │
│                                                                            │
│  2. After payment, Stripe redirects to /success?session_id=cs_xxx         │
│     └──▶ Frontend calls POST /session { sessionId: "cs_xxx" }             │
│                                                                            │
│  3. Worker receives session ID                                             │
│     ├──▶ Verify with Stripe API                                           │
│     ├──▶ Generate license key: LGC-XXXX-XXXX-XXXX-XXXX                    │
│     ├──▶ Store in D1: licenses(key, customer_email, plan, stripe_sub_id)  │
│     ├──▶ Send email via Cloudflare Email Workers (or Resend API)         │
│     └──▶ Return { licenseKey, plan, email }                               │
│                                                                            │
│  4. Frontend stores license key locally                                    │
│     └──▶ Validates via POST /validate { licenseKey: "LGC-..." }           │
│                                                                            │
│  5. Validation (fast path)                                                 │
│     ├──▶ Check D1 cache first                                             │
│     ├──▶ If not cached or expired, verify with Stripe                     │
│     └──▶ Cache result in D1 for 24 hours                                  │
│                                                                            │
│  6. Stripe Webhook (/webhook/stripe)                                       │
│     ├──▶ subscription.deleted → Mark license as revoked                   │
│     ├──▶ subscription.updated → Update plan tier                          │
│     └──▶ invoice.payment_failed → Send warning email                      │
│                                                                            │
└────────────────────────────────────────────────────────────────────────────┘
```

### 8.3 D1 Schema for Licenses

```sql
CREATE TABLE licenses (
    id TEXT PRIMARY KEY,
    license_key TEXT UNIQUE NOT NULL,        -- LGC-XXXX-XXXX-XXXX-XXXX
    customer_email TEXT NOT NULL,
    plan TEXT NOT NULL,                       -- free, supporter, pro, premium, lifetime
    stripe_subscription_id TEXT,
    stripe_customer_id TEXT,
    status TEXT DEFAULT 'active',             -- active, revoked, expired
    created_at TEXT DEFAULT (datetime('now')),
    validated_at TEXT,
    expires_at TEXT                           -- NULL for lifetime
);

CREATE INDEX idx_licenses_key ON licenses(license_key);
CREATE INDEX idx_licenses_stripe ON licenses(stripe_subscription_id);
```

### 8.4 License Key Generation

```rust
// workers/api/src/payments.rs

/// Generate a license key in format: LGC-XXXX-XXXX-XXXX-XXXX
fn generate_license_key() -> String {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).expect("Failed to generate random bytes");

    let chars: String = bytes
        .iter()
        .map(|b| {
            let idx = (*b as usize) % 36;
            if idx < 10 {
                (b'0' + idx as u8) as char
            } else {
                (b'A' + (idx - 10) as u8) as char
            }
        })
        .collect();

    format!(
        "LGC-{}-{}-{}-{}",
        &chars[0..4],
        &chars[4..8],
        &chars[8..12],
        &chars[12..16]
    )
}

pub async fn handle_session(mut req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let body: SessionRequest = req.json().await?;

    // Verify with Stripe
    let stripe_key = ctx.env.secret("STRIPE_SECRET_KEY")?.to_string();
    let session = fetch_stripe_session(&body.session_id, &stripe_key).await?;

    if session.payment_status != "paid" {
        return Response::error("Payment not completed", 400);
    }

    let db = ctx.env.d1("DB")?;

    // Check if license already exists for this session
    let existing = db
        .prepare("SELECT license_key FROM licenses WHERE stripe_subscription_id = ?1")
        .bind(&[session.subscription.as_ref().unwrap().into()])?
        .first::<LicenseRow>(None)
        .await?;

    if let Some(lic) = existing {
        return Response::from_json(&SessionResponse {
            license_key: lic.license_key,
            plan: session.plan.clone(),
            email: session.customer_email.clone(),
        });
    }

    // Generate new license
    let license_key = generate_license_key();
    let license_id = generate_id();

    db.prepare(r#"
        INSERT INTO licenses (id, license_key, customer_email, plan, stripe_subscription_id, stripe_customer_id)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
    "#)
    .bind(&[
        license_id.into(),
        license_key.clone().into(),
        session.customer_email.clone().into(),
        session.plan.clone().into(),
        session.subscription.clone().unwrap_or_default().into(),
        session.customer.clone().unwrap_or_default().into(),
    ])?
    .run()
    .await?;

    // Send email (fire and forget)
    send_license_email(&ctx.env, &session.customer_email, &license_key, &session.plan).await.ok();

    Response::from_json(&SessionResponse {
        license_key,
        plan: session.plan,
        email: session.customer_email,
    })
}

pub async fn validate_license(mut req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let body: ValidateRequest = req.json().await?;

    let db = ctx.env.d1("DB")?;

    // Check D1 first
    let license = db
        .prepare("SELECT * FROM licenses WHERE license_key = ?1")
        .bind(&[body.license_key.clone().into()])?
        .first::<License>(None)
        .await?;

    match license {
        Some(lic) if lic.status == "active" => {
            // Update validated_at
            db.prepare("UPDATE licenses SET validated_at = datetime('now') WHERE id = ?1")
                .bind(&[lic.id.into()])?
                .run()
                .await?;

            Response::from_json(&ValidateResponse {
                valid: true,
                plan: lic.plan,
                email: lic.customer_email,
            })
        }
        Some(lic) => {
            Response::from_json(&ValidateResponse {
                valid: false,
                plan: lic.plan,
                email: lic.customer_email,
            })
        }
        None => {
            // Check if it's a legacy Stripe subscription ID
            if body.license_key.starts_with("sub_") {
                return validate_legacy_stripe(&ctx.env, &body.license_key).await;
            }
            Response::from_json(&ValidateResponse {
                valid: false,
                plan: "none".to_string(),
                email: String::new(),
            })
        }
    }
}
```

---

## Part 7: Authentication System

### 7.1 OAuth Implementation (Rust)

Extending the existing GitHub OAuth pattern:

```rust
// workers/api/src/auth/github.rs

const GITHUB_AUTHORIZE_URL: &str = "https://github.com/login/oauth/authorize";
const GITHUB_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
const GITHUB_USER_URL: &str = "https://api.github.com/user";

pub async fn github_redirect(_req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let client_id = ctx.env.secret("GITHUB_CLIENT_ID")?.to_string();
    let redirect_uri = format!("{}/auth/github/callback", ctx.env.var("API_URL")?.to_string());
    let state = generate_csrf_token();

    let auth_url = format!(
        "{}?client_id={}&redirect_uri={}&scope=read:user%20user:email&state={}",
        GITHUB_AUTHORIZE_URL,
        client_id,
        urlencoding::encode(&redirect_uri),
        state
    );

    Response::redirect_with_status(Url::parse(&auth_url)?, 302)
}

pub async fn github_callback(req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let url = req.url()?;
    let code = url.query_pairs()
        .find(|(k, _)| k == "code")
        .map(|(_, v)| v.to_string())
        .ok_or_else(|| Error::from("Missing code"))?;

    // Exchange code for token
    let client_id = ctx.env.secret("GITHUB_CLIENT_ID")?.to_string();
    let client_secret = ctx.env.secret("GITHUB_CLIENT_SECRET")?.to_string();

    let token_response = fetch_github_token(&code, &client_id, &client_secret).await?;

    // Fetch user info
    let github_user = fetch_github_user(&token_response.access_token).await?;

    // Upsert user in D1
    let db = ctx.env.d1("DB")?;
    let user_id = format!("gh_{}", github_user.id);

    db.prepare(r#"
        INSERT INTO users (id, display_name, github_id, avatar_url, created_at)
        VALUES (?1, ?2, ?3, ?4, datetime('now'))
        ON CONFLICT(id) DO UPDATE SET
            display_name = excluded.display_name,
            avatar_url = excluded.avatar_url,
            updated_at = datetime('now')
    "#)
    .bind(&[
        user_id.clone().into(),
        github_user.login.clone().into(),
        github_user.id.to_string().into(),
        github_user.avatar_url.clone().into(),
    ])?
    .run()
    .await?;

    // Create JWT
    let jwt_secret = ctx.env.secret("JWT_SECRET")?.to_string();
    let tokens = create_tokens(&user_id, &github_user.login, &jwt_secret)?;

    // Redirect to frontend with token
    let frontend_url = ctx.env.var("ALLOWED_ORIGIN")?.to_string();
    let redirect_url = format!(
        "{}/?token={}&login={}",
        frontend_url,
        tokens.access_token,
        github_user.login
    );

    Response::redirect_with_status(Url::parse(&redirect_url)?, 302)
}
```

### 7.2 Google OAuth

```rust
// workers/api/src/auth/google.rs

const GOOGLE_AUTHORIZE_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const GOOGLE_USERINFO_URL: &str = "https://www.googleapis.com/oauth2/v2/userinfo";

pub async fn google_redirect(_req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let client_id = ctx.env.secret("GOOGLE_CLIENT_ID")?.to_string();
    let redirect_uri = format!("{}/auth/google/callback", ctx.env.var("API_URL")?.to_string());
    let state = generate_csrf_token();

    let auth_url = format!(
        "{}?client_id={}&redirect_uri={}&response_type=code&scope=openid%20email%20profile&state={}",
        GOOGLE_AUTHORIZE_URL,
        client_id,
        urlencoding::encode(&redirect_uri),
        state
    );

    Response::redirect_with_status(Url::parse(&auth_url)?, 302)
}

// Similar callback implementation...
```

### 7.3 Session Management

Session management uses localStorage on the client with D1 on the server for persistence and security.

#### Client-Side (localStorage)

```typescript
// Client-side session storage
interface SessionData {
    accessToken: string;
    refreshToken: string;
    user: {
        id: string;
        displayName: string;
        avatarUrl: string;
        provider: 'github' | 'google';
    };
    expiresAt: number;
}

const SESSION_KEY = 'logos_session';

function saveSession(session: SessionData) {
    localStorage.setItem(SESSION_KEY, JSON.stringify(session));
}

function loadSession(): SessionData | null {
    const raw = localStorage.getItem(SESSION_KEY);
    if (!raw) return null;
    const session = JSON.parse(raw);
    if (Date.now() > session.expiresAt) {
        clearSession();
        return null;
    }
    return session;
}

function clearSession() {
    localStorage.removeItem(SESSION_KEY);
}
```

#### Server-Side (D1)

```sql
CREATE TABLE sessions (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    refresh_token_hash TEXT NOT NULL,
    ip_address TEXT,
    user_agent TEXT,
    created_at TEXT DEFAULT (datetime('now')),
    last_used_at TEXT DEFAULT (datetime('now')),
    expires_at TEXT NOT NULL,
    revoked_at TEXT
);

CREATE INDEX idx_sessions_user ON sessions(user_id);
CREATE INDEX idx_sessions_refresh ON sessions(refresh_token_hash);
```

#### Progress Linking Flow

When a user signs in for the first time, their anonymous (localStorage) progress is linked to their account:

```rust
pub async fn link_anonymous_progress(
    db: &D1Database,
    user_id: &str,
    anonymous_progress: &UserProgress,
) -> Result<()> {
    // Check if user already has server-side progress
    let existing = db
        .prepare("SELECT * FROM user_progress WHERE user_id = ?1")
        .bind(&[user_id.into()])?
        .first::<UserProgress>(None)
        .await?;

    match existing {
        Some(server) => {
            // Merge: take higher values for XP, streaks, etc.
            let merged = UserProgress {
                xp: server.xp.max(anonymous_progress.xp),
                level: server.level.max(anonymous_progress.level),
                current_streak: server.current_streak.max(anonymous_progress.current_streak),
                best_streak: server.best_streak.max(anonymous_progress.best_streak),
                best_combo: server.best_combo.max(anonymous_progress.best_combo),
                total_correct: server.total_correct + anonymous_progress.total_correct,
                ..server
            };
            update_progress(db, user_id, &merged).await?;
        }
        None => {
            // First sign-in: import anonymous progress directly
            create_progress(db, user_id, anonymous_progress).await?;
        }
    }

    Ok(())
}
```

### 7.4 MFA Stubs for Registry Access

MFA is **not implemented in v1** but the architecture supports future addition. The registry (package publishing) is a security-sensitive operation that will require MFA when implemented.

#### Stub Implementation

```rust
// workers/api/src/auth/mfa.rs

pub async fn mfa_setup_stub(_req: Request, _ctx: RouteContext<()>) -> Result<Response> {
    Response::from_json(&serde_json::json!({
        "status": "not_implemented",
        "message": "MFA will be available in a future update",
        "planned_methods": ["totp", "sms"]
    }))
}

pub async fn mfa_verify_stub(_req: Request, _ctx: RouteContext<()>) -> Result<Response> {
    Response::from_json(&serde_json::json!({
        "status": "not_implemented",
        "message": "MFA verification not yet available"
    }))
}

/// Check if MFA is required for registry operations
/// Currently returns false; will return true when MFA is implemented
pub fn require_mfa_for_registry() -> bool {
    false
}
```

#### Registry Protection (Future)

When MFA is implemented, the registry publish endpoint will require:
1. Valid session token
2. MFA verification within last 15 minutes
3. Rate limiting per user

```rust
// Future implementation pattern
pub async fn publish_package(req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let user = authenticate(&req, &ctx.env).await?;

    // MFA check (currently a no-op stub)
    if require_mfa_for_registry() {
        verify_mfa_session(&req, &ctx.env, &user.id).await?;
    }

    // ... proceed with package publishing
}
```

---

## Part 5: D1 Database Schema

### 5.1 Complete Schema

```sql
-- ============================================
-- USERS & AUTH
-- ============================================

CREATE TABLE users (
    id TEXT PRIMARY KEY,
    email TEXT,                          -- Optional, from OAuth provider
    display_name TEXT NOT NULL,
    avatar_url TEXT,

    -- OAuth providers (at least one required)
    github_id TEXT UNIQUE,
    google_id TEXT UNIQUE,

    -- MFA (future implementation - stubs only)
    mfa_enabled INTEGER DEFAULT 0,
    mfa_secret TEXT,
    mfa_phone TEXT,

    -- Status
    is_banned INTEGER DEFAULT 0,
    ban_reason TEXT,

    -- Timestamps
    created_at TEXT DEFAULT (datetime('now')),
    updated_at TEXT DEFAULT (datetime('now')),
    last_login_at TEXT
);

CREATE TABLE sessions (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    refresh_token_family TEXT NOT NULL,
    ip_address TEXT,
    user_agent TEXT,
    created_at TEXT DEFAULT (datetime('now')),
    last_used_at TEXT DEFAULT (datetime('now')),
    revoked_at TEXT
);

-- ============================================
-- PROGRESS & GAMIFICATION
-- ============================================

CREATE TABLE user_progress (
    user_id TEXT PRIMARY KEY,
    xp INTEGER DEFAULT 0,
    level INTEGER DEFAULT 1,

    -- Streaks
    current_streak INTEGER DEFAULT 0,
    best_streak INTEGER DEFAULT 0,
    streak_freezes INTEGER DEFAULT 0,
    last_activity_date TEXT,

    -- Combos
    current_combo INTEGER DEFAULT 0,
    best_combo INTEGER DEFAULT 0,

    -- Totals
    total_correct INTEGER DEFAULT 0,
    total_attempts INTEGER DEFAULT 0,

    -- Active cosmetics
    active_title TEXT,

    updated_at TEXT DEFAULT (datetime('now'))
);

CREATE TABLE exercise_progress (
    user_id TEXT NOT NULL,
    exercise_id TEXT NOT NULL,
    attempts INTEGER DEFAULT 0,
    correct_count INTEGER DEFAULT 0,
    srs_interval INTEGER DEFAULT 1,
    srs_ease REAL DEFAULT 2.5,
    srs_repetitions INTEGER DEFAULT 0,
    next_review TEXT,
    first_attempt_at TEXT,
    last_attempt_at TEXT,
    PRIMARY KEY (user_id, exercise_id)
);

CREATE TABLE user_achievements (
    user_id TEXT NOT NULL,
    achievement_id TEXT NOT NULL,
    unlocked_at TEXT DEFAULT (datetime('now')),
    PRIMARY KEY (user_id, achievement_id)
);

CREATE TABLE xp_events (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    xp_amount INTEGER NOT NULL,
    exercise_id TEXT,
    source TEXT,
    client_timestamp TEXT,
    server_timestamp TEXT DEFAULT (datetime('now')),
    session_id TEXT
);

-- ============================================
-- MULTIPLAYER BATTLES
-- ============================================

CREATE TABLE battles (
    id TEXT PRIMARY KEY,
    player_a_id TEXT NOT NULL,
    player_b_id TEXT,
    exercise_set TEXT NOT NULL,
    current_index INTEGER DEFAULT 0,
    player_a_score INTEGER DEFAULT 0,
    player_b_score INTEGER DEFAULT 0,
    status TEXT DEFAULT 'waiting',
    created_at TEXT DEFAULT (datetime('now')),
    started_at TEXT,
    finished_at TEXT,
    winner_id TEXT
);

CREATE TABLE battle_actions (
    id TEXT PRIMARY KEY,
    battle_id TEXT NOT NULL,
    player_id TEXT NOT NULL,
    exercise_index INTEGER NOT NULL,
    answer TEXT NOT NULL,
    is_correct INTEGER,
    submitted_at TEXT DEFAULT (datetime('now')),
    processed INTEGER DEFAULT 0
);

-- ============================================
-- LICENSES & PAYMENTS
-- ============================================

CREATE TABLE licenses (
    id TEXT PRIMARY KEY,
    license_key TEXT UNIQUE NOT NULL,
    customer_email TEXT NOT NULL,
    plan TEXT NOT NULL,
    stripe_subscription_id TEXT,
    stripe_customer_id TEXT,
    status TEXT DEFAULT 'active',
    created_at TEXT DEFAULT (datetime('now')),
    validated_at TEXT,
    expires_at TEXT
);

-- ============================================
-- TOURNAMENTS (see Section 9.9 for full schema)
-- ============================================

-- Tournament tables defined in Section 9.9:
-- - tournaments
-- - tournament_players
-- - tournament_rounds
-- - tournament_answers
-- - matchmaking_queue
-- - player_ratings

-- ============================================
-- INDEXES
-- ============================================

CREATE INDEX idx_users_email ON users(email);
CREATE INDEX idx_users_github ON users(github_id);
CREATE INDEX idx_users_google ON users(google_id);
CREATE INDEX idx_sessions_user ON sessions(user_id);
CREATE INDEX idx_progress_xp ON user_progress(xp DESC);
CREATE INDEX idx_xp_events_user ON xp_events(user_id, server_timestamp DESC);
CREATE INDEX idx_battles_players ON battles(player_a_id, player_b_id);
CREATE INDEX idx_battles_status ON battles(status);
CREATE INDEX idx_actions_unprocessed ON battle_actions(battle_id, processed);
CREATE INDEX idx_licenses_key ON licenses(license_key);
-- Tournament indexes defined in Section 9.9
```

---

## Part 12: Summary

### Key Changes from v2.0

| Aspect | v2.0 (Previous) | v3.0 (Cloudflare) |
|--------|-----------------|-------------------|
| Backend | Axum on Fly.io | Cloudflare Workers (Rust) |
| Database | PostgreSQL (Supabase) | D1 (SQLite) |
| Storage | Supabase Storage | R2 |
| Email | Resend/Sendgrid | Cloudflare Email Workers |
| Auth deps | argon2, jsonwebtoken | Web Crypto API, manual JWT |
| Cost | $50-180/mo | $10-135/mo |
| Infra complexity | Multiple providers | Single provider |

### Deferred Features

- **MFA** - Stubs in place for future TOTP/SMS implementation (registry protection)
- **Apple OAuth** - Low priority
- **Email/Password Auth** - Removed per CEO direction (OAuth only)

---

## Glossary

| Term | Definition |
|------|------------|
| **Blitz Mode** | Tournament format with per-move timers (10-30s) and 3 lives |
| **Chess-Clock** | Timer mode where each player has a total time bank that depletes while thinking |
| **Cloudflare D1** | Cloudflare's serverless SQLite database for transactional data |
| **Cloudflare Pages** | Cloudflare's static site hosting for the frontend |
| **Cloudflare R2** | Cloudflare's S3-compatible object storage for files and game state snapshots |
| **Cloudflare Workers** | Serverless functions running at the edge; this proposal uses Rust via `workers-rs` |
| **Crate** | A Rust package/library unit; the proposal separates code into multiple crates for clean architecture |
| **CSRF** | Cross-Site Request Forgery; a security vulnerability prevented via state tokens in OAuth |
| **Dioxus** | A Rust framework for building user interfaces (used for the frontend) |
| **ELO Rating** | A skill rating system (originally for chess) used for matchmaking |
| **Era** | A major curriculum division (e.g., Era 0: logicaffeine, Era 1: trivium) |
| **FOL** | First-Order Logic; the formal logic system taught by Logicaffeine |
| **JWT** | JSON Web Token; used for authentication and session management |
| **License Key** | A unique identifier (format: `LGC-XXXX-XXXX-XXXX-XXXX`) that grants access to paid features |
| **Lives System** | Tournament mechanic where wrong answers cost lives; elimination occurs at 0 lives |
| **LOGOS** | The programming language/compiler at the core of Logicaffeine |
| **localStorage** | Browser storage used for client-side session and progress data |
| **Marathon Mode** | Tournament format with 30-minute overall timer and 5 lives |
| **MFA** | Multi-Factor Authentication; planned for registry protection (stubs only in v1) |
| **Module** | A subdivision within an Era containing related exercises |
| **OAuth** | Open Authorization; the protocol used for GitHub and Google sign-in |
| **Per-Move Timer** | Timer mode where each answer has a fixed time limit that resets after submission |
| **SRS** | Spaced Repetition System; algorithm for optimal review scheduling |
| **Sprint Mode** | Tournament format with 1-minute rounds and 1 life per round |
| **Stripe** | Payment processing service for subscriptions and purchases |
| **Tick Worker** | Scheduled Cloudflare Worker that processes battle/tournament actions every second |
| **TOTP** | Time-based One-Time Password; a planned MFA method |
| **Tournament** | Competitive multiplayer event with brackets, lives, and time pressure |
| **workers-rs** | Rust crate for building Cloudflare Workers |
| **wrangler** | Cloudflare's CLI tool for deploying and managing Workers |
| **XP** | Experience Points; earned for completing exercises and achievements |
| **Zone of Proximal Development** | Vygotsky's educational theory; the optimal difficulty range for learning |

---

*Questions? Contact the engineering team.*
