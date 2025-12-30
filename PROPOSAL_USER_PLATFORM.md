# Logicaffeine User Platform Proposal
## Cloudflare-Native Architecture with Rust Workers

**Version:** 3.0 | **Date:** December 30, 2025

---

## Table of Contents

1. [Executive Summary](#1-executive-summary)
2. [User Experience: Integrated Learn Textbook](#2-user-experience-integrated-learn-textbook)
3. [Progress & Gamification](#3-progress--gamification)
4. [Curriculum Review Process](#4-curriculum-review-process)
5. [Technical: Infrastructure & Schema](#5-technical-infrastructure--schema)
6. [Technical: API & Authentication](#6-technical-api--authentication)
7. [Multiplayer Battles & Tournaments](#7-multiplayer-battles--tournaments)
8. [AAA Quality Requirements](#8-aaa-quality-requirements)
9. [Implementation Checklist](#9-implementation-checklist)
10. [Glossary](#10-glossary)
11. [Appendix A: Curriculum Mapping (Gensler)](#appendix-a-curriculum-mapping-genslers-introduction-to-logic)

---

## 1. Executive Summary

Logicaffeine is a gamified platform for learning First-Order Logic through translation exercises. This proposal defines the complete architecture for transforming it into a connected, competitive learning platform.

### Architecture Overview

```
┌─────────────────────────────────────────────────────────────────────┐
│                 LOGICAFFEINE CLOUDFLARE ARCHITECTURE                │
├─────────────────────────────────────────────────────────────────────┤
│                                                                     │
│  ┌───────────┐   ┌───────────┐   ┌───────────┐   ┌───────────┐    │
│  │ Cloudflare│   │ Cloudflare│   │ Cloudflare│   │ Cloudflare│    │
│  │   Pages   │   │  Workers  │   │    D1     │   │    R2     │    │
│  │ (Frontend)│   │  (Rust)   │   │ (SQLite)  │   │ (Storage) │    │
│  └─────┬─────┘   └─────┬─────┘   └─────┬─────┘   └─────┬─────┘    │
│        │               │               │               │           │
│        └───────────────┴───────────────┴───────────────┘           │
│                              │                                      │
│                    ┌─────────┴─────────┐                           │
│                    │     API Layer     │                           │
│                    │  • Auth (OAuth)   │                           │
│                    │  • Progress Sync  │                           │
│                    │  • Leaderboards   │                           │
│                    │  • Battles        │                           │
│                    │  • Payments       │                           │
│                    └───────────────────┘                           │
└─────────────────────────────────────────────────────────────────────┘
```

### What We're Building

1. **Integrated Learn textbook** — Single scrollable page with lessons, examples, infinite practice, and 17-question tests per module
2. **Symbol dictionary + auto-hints** — Every logic output shows symbol meanings and Socratic hints automatically
3. **Duolingo-style progress tracking** — XP, streaks, achievements, module mastery, visual progress
4. **Multiplayer competition** — Time-based battles and tournaments with lives system
5. **Cloud sync** — Progress saved to D1, accessible from any device

---

## 2. User Experience: Integrated Learn Textbook

### 2.1 Simplified Navigation

Everything happens in the Learn page — no separate routes for practice, read, or test modes.

```
┌─────────────────────────────────────────────────────────────────────┐
│                        ROUTES (SIMPLIFIED)                          │
├─────────────────────────────────────────────────────────────────────┤
│                                                                     │
│  /              → Landing page                                      │
│  /learn         → Textbook with all modules (scrollable)           │
│  /learn#atomic  → Jump to specific module section                   │
│  /profile       → Stats, achievements, activity                     │
│  /battle        → Multiplayer battles                               │
│                                                                     │
│  NO LONGER NEEDED:                                                  │
│  ✗ /learn/:era/:module  (now just anchor links)                    │
│  ✗ /lesson/:era/:module/:mode  (practice/test built into Learn)   │
│  ✗ /review  (SRS integrated into Practice component)               │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
```

### 2.2 Learn Page Structure

The Learn page is a scrollable textbook. Each module section contains:
1. **Lesson** — Reading content with symbol dictionary
2. **Interactive Examples** — Run logic, see output with auto-hints
3. **Practice** — Infinite flashcard mode for XP
4. **Test** — 17-question scored assessment

**Focus Mode:** When a student clicks into a module, other eras collapse/recede to minimize cognitive clutter. The environment shrinks to fit the task.

```
┌─────────────────────────────────────────────────────────────────────┐
│ LEARN                                          🔥 14  ⚡ 2,450 XP   │
├─────────────────────────────────────────────────────────────────────┤
│                                                                     │
│  [Era 0: Practice] [Era 1: Basics ▼] [Era 2: Quantifiers] ...      │
│                                                                     │
│  ════════════════════════════════════════════════════════════════  │
│  ERA 1: BASICS (expanded, others collapsed)                        │
│  ════════════════════════════════════════════════════════════════  │
│                                                                     │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │ § 1.1 THE ATOMIC WORLD                      ⭐⭐░ 67% complete│   │
│  ├─────────────────────────────────────────────────────────────┤   │
│  │                                                              │   │
│  │ [LESSON] [EXAMPLES] [PRACTICE ∞] [TEST 📝]                  │   │
│  │                                                              │   │
│  │ ─────────────────────────────────────────────────────────── │   │
│  │                                                              │   │
│  │ In logic, we start with the simplest building blocks:       │   │
│  │ INDIVIDUALS and PROPERTIES.                                  │   │
│  │ ...                                                          │   │
│  │                                                              │   │
│  └─────────────────────────────────────────────────────────────┘   │
│                                                                     │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │ § 1.2 RELATIONS                             ░░░ not started │   │
│  └─────────────────────────────────────────────────────────────┘   │
│                                                                     │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │ § 1.3 NEGATION                              🔒 locked       │   │
│  └─────────────────────────────────────────────────────────────┘   │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
```

### 2.3 Module Section Tabs

Each module section has 4 tabs that switch content in-place:

| Tab | Description |
|-----|-------------|
| **LESSON** | Reading content from `lesson.md` |
| **EXAMPLES** | Interactive code blocks with symbol dictionary |
| **PRACTICE ∞** | Infinite flashcard mode, earn XP per correct answer |
| **TEST 📝** | 17-question assessment with final score |

### 2.4 Symbol Dictionary (Auto-Generated)

When logic code runs, the output includes a **Symbol Dictionary** that explains every symbol used:

```
┌─────────────────────────────────────────────────────────────────────┐
│ EXAMPLES                                                            │
├─────────────────────────────────────────────────────────────────────┤
│                                                                     │
│  Try it: "All cats are mammals"                                    │
│  ┌───────────────────────────────────────────────────────────────┐ │
│  │ > All cats are mammals                                        │ │
│  │                                                                │ │
│  │ OUTPUT:                                                        │ │
│  │ ∀x(Cat(x) → Mammal(x))                                        │ │
│  │                                                                │ │
│  │ ┌─────────────────────────────────────────────────────────┐   │ │
│  │ │ 📖 SYMBOL DICTIONARY                                     │   │ │
│  │ ├─────────────────────────────────────────────────────────┤   │ │
│  │ │ ∀    "for all" — universal quantifier                   │   │ │
│  │ │ x    variable representing any individual               │   │ │
│  │ │ →    "implies" / "if...then"                            │   │ │
│  │ │ Cat  predicate: "is a cat"                              │   │ │
│  │ │ Mammal  predicate: "is a mammal"                        │   │ │
│  │ └─────────────────────────────────────────────────────────┘   │ │
│  │                                                                │ │
│  │ 💡 HINT: "All X are Y" always becomes ∀x(X(x) → Y(x)).       │ │
│  │    The arrow (→) captures the "if...then" relationship.      │ │
│  │    If something is a cat, THEN it is a mammal.               │ │
│  │                                                                │ │
│  └───────────────────────────────────────────────────────────────┘ │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
```

**Symbol Dictionary Generation:**
- Automatically extract all symbols from the FOL output
- Look up each symbol in a master dictionary
- Display only the symbols used in this specific output
- Include predicates/constants with their English meanings

### 2.5 Socratic Hints (Struggle-Triggered)

Hints appear **when the user struggles** — not automatically after showing the answer (which kills discovery).

**Trigger conditions:**
- After 5 seconds of inactivity on a problem
- After a wrong attempt
- Before the final answer is revealed (not after)

In Test mode, hints are completely disabled. In the Review screen, hints become **Socratic questions** that force reflection:
- Instead of: "The answer is ∀x(Bird(x) → Fly(x))"
- Ask: "You used ∧ (and). But does being a bird GUARANTEE flying, or just coincide with it?"

```rust
struct LogicOutput {
    fol: String,                    // "∀x(Cat(x) → Mammal(x))"
    symbol_dictionary: Vec<SymbolEntry>,
    socratic_hint: Option<String>,  // Shown on struggle, not automatically
}

struct SymbolEntry {
    symbol: String,     // "∀"
    name: String,       // "for all"
    meaning: String,    // "applies to EVERYTHING in the universe"
}
```

### 2.6 Practice Mode (Infinite Flashcards)

Click **PRACTICE ∞** to enter infinite flashcard mode within the module section:

```
┌─────────────────────────────────────────────────────────────────────┐
│ § 1.1 THE ATOMIC WORLD                              [✕ Exit Practice]│
├─────────────────────────────────────────────────────────────────────┤
│                                                                     │
│  ┌───────────────────────────────────────────────────────────────┐ │
│  │                                                                │ │
│  │  🔥 COMBO: 7                              +15 XP this card    │ │
│  │                                                                │ │
│  │  ─────────────────────────────────────────────────────────── │ │
│  │                                                                │ │
│  │  Translate: "Socrates is mortal"                              │ │
│  │                                                                │ │
│  │  ┌─────────────────────────────────────────────────────────┐  │ │
│  │  │ A) Mortal(s)                                            │  │ │
│  │  │ B) s(Mortal)                                            │  │ │
│  │  │ C) Socrates → Mortal                                    │  │ │
│  │  │ D) ∀x(Socrates(x))                                      │  │ │
│  │  └─────────────────────────────────────────────────────────┘  │ │
│  │                                                                │ │
│  │  Session: 23 correct | 2 wrong | 89% accuracy                 │ │
│  │                                                                │ │
│  └───────────────────────────────────────────────────────────────┘ │
│                                                                     │
│  [Keep Going]                              [End Session → +345 XP] │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
```

**Practice Mode Features:**
- Infinite questions drawn from module's exercise pool
- **Priority queue**: Wrong answers re-inserted at position `current + 3` for immediate correction
- Combo multiplier for consecutive correct answers
- Session stats shown (correct/wrong/accuracy)
- Exit anytime, XP banked immediately
- SRS-due exercises prioritized

**Diminishing Returns (Anti-Grind):**
After a module reaches "Mastered" (100% complete), XP from that module decays:
- First week after mastery: 50% XP
- After first week: 25% XP
- This forces players to tackle harder modules for full rewards

### 2.7 Test Mode (17 Questions)

Click **TEST 📝** to start a scored assessment:

```
┌─────────────────────────────────────────────────────────────────────┐
│ § 1.1 THE ATOMIC WORLD — TEST                      Question 8 / 17 │
├─────────────────────────────────────────────────────────────────────┤
│                                                                     │
│  ████████░░░░░░░░░                                                 │
│                                                                     │
│  ─────────────────────────────────────────────────────────────────│
│                                                                     │
│  Translate: "Paris is beautiful and ancient"                       │
│                                                                     │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │ A) Beautiful(p) ∧ Ancient(p)                                │   │
│  │ B) Beautiful ∧ Ancient(Paris)                               │   │
│  │ C) p(Beautiful, Ancient)                                    │   │
│  │ D) Beautiful(Paris) → Ancient(Paris)                        │   │
│  └─────────────────────────────────────────────────────────────┘   │
│                                                                     │
│  ⚠️ No hints in Test mode. Answers revealed at the end.           │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
```

**Test Complete Screen:**

```
┌─────────────────────────────────────────────────────────────────────┐
│ § 1.1 THE ATOMIC WORLD — TEST COMPLETE                             │
├─────────────────────────────────────────────────────────────────────┤
│                                                                     │
│                         🎉 SCORE: 15 / 17                          │
│                            88% — GREAT!                            │
│                                                                     │
│                          +170 XP earned                            │
│                                                                     │
│  ─────────────────────────────────────────────────────────────────│
│                                                                     │
│  REVIEW MISTAKES:                                                  │
│                                                                     │
│  Q3: "The cat is sleeping"                                         │
│      Your answer: Sleep(cat)                                       │
│      Correct: Sleeping(c)  ← use lowercase for individuals        │
│                                                                     │
│  Q12: "All birds can fly"                                          │
│       Your answer: ∀x(Bird(x) ∧ Fly(x))                            │
│       Correct: ∀x(Bird(x) → Fly(x))  ← use → not ∧ for "all...are"│
│                                                                     │
│  ─────────────────────────────────────────────────────────────────│
│                                                                     │
│  [Retake Test]                              [Continue to § 1.2 →] │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
```

**Test Mode Features:**
- Fixed 17 questions per test
- No hints, no immediate feedback
- Full XP multiplier (1.0x)
- Review all mistakes at the end with explanations
- Retake available anytime
- Score saved to progress (best score tracked)

### 2.8 Lesson File Structure

```
assets/curriculum/01_trivium/01_atomic/
├── meta.json       # Module metadata
├── lesson.md       # Lesson content (AI-generated, committed)
├── symbols.json    # Symbol dictionary for this module
└── exercises/
    ├── ex_01.json
    ├── ex_02.json
    └── ...
```

**symbols.json example:**
```json
{
  "symbols": [
    { "symbol": "∀", "name": "for all", "meaning": "universal quantifier" },
    { "symbol": "∃", "name": "there exists", "meaning": "existential quantifier" },
    { "symbol": "→", "name": "implies", "meaning": "if...then" },
    { "symbol": "∧", "name": "and", "meaning": "conjunction" },
    { "symbol": "∨", "name": "or", "meaning": "disjunction" },
    { "symbol": "¬", "name": "not", "meaning": "negation" }
  ],
  "predicates": [
    { "symbol": "Cat", "meaning": "is a cat" },
    { "symbol": "Mammal", "meaning": "is a mammal" }
  ]
}
```

### 2.9 Lesson Generation Prompt

```
You are writing a textbook lesson for "{module_title}" in Logicaffeine.

Learning objective: {pedagogy from meta.json}

Write a lesson (300-500 words) with these sections:

## Concept
Plain English explanation of the core idea.

## Notation
The logical symbols used and what they mean. Include a reference table.

## Pattern
The translation pattern from English to logic. Show the template.

## Examples
2-3 worked examples with step-by-step explanations.
For each example, show:
- The English sentence
- The logic translation
- Why each symbol is used

## Common Mistakes
List 2-3 pitfalls students often encounter.

Requirements:
- Use simple language, assume no prior logic knowledge
- Use concrete, memorable examples
- Connect to real-world reasoning
```

---

## 3. Progress & Gamification

### 3.1 Module States & Unlocking

**Unlock Rule:** The last 2 modules in each era are locked until you complete 100% of ANY one unlocked module.

```
ERA 0: PRACTICE (6 modules)
┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────────┐
│Syllog.  │ │Propos.  │ │ Modal   │ │Deontic  │ │ Belief  │ │Informal │
│ ⭐⭐░    │ │ ░░░     │ │ ░░░     │ │ ░░░     │ │ 🔒      │ │ 🔒      │
│ 67/98   │ │ 0/114   │ │ 0/34    │ │ 0/38    │ │ locked  │ │ locked  │
└─────────┘ └─────────┘ └─────────┘ └─────────┘ └─────────┘ └─────────┘
                                                     ▲           ▲
                                        Unlock by completing 100% of
                                        ANY module above (e.g., Syllogistic)
```

**Module States:**

| State | Icon | Meaning |
|-------|------|---------|
| Locked | 🔒 | Complete one module to unlock |
| Available | ░░░ | Can start anytime |
| Started | ⭐░░ | 1-49% complete |
| Progressing | ⭐⭐░ | 50-99% complete |
| Mastered | ⭐⭐⭐ | 100% complete |
| Perfected | 👑 | 100% complete + 90%+ accuracy |

### 3.2 XP System

**Base XP Calculation:**
```
base = 10 + (difficulty - 1) × 5
```

**Bonuses:**
| Bonus | Calculation | Max |
|-------|-------------|-----|
| Combo | base × (1 + combo × 0.1) | 2x at 10-combo |
| Streak | streak_days × 2 | +14 at 7-day streak |
| First Try | +5 flat | +5 |
| Critical | 10% chance for +base | +base |

**Level Formula:**
```
level = floor(√xp / 10) + 1
```

### 3.3 Streaks & Combos

**Streak:**
- Increments daily when you answer at least one exercise correctly
- Lost after 2 days of inactivity (unless frozen)
- Earn "freeze" tokens from achievements to skip a day

**Combo:**
- Increments on each correct answer
- Resets to 0 on wrong answer
- Multiplies XP up to 2x at 10-combo

### 3.4 Achievements

| Achievement | Trigger | Reward |
|-------------|---------|--------|
| First Blood | 1 correct answer | 50 XP |
| On Fire | 5-combo | 100 XP |
| Unstoppable | 10-combo | 250 XP, "Logic Machine" title |
| Week Warrior | 7-day streak | 200 XP, +1 freeze |
| Monthly Master | 30-day streak | 1000 XP, +1 freeze |
| Century | 100 correct | 500 XP, "Scholar" title |
| Millennium | 1000 correct | 2000 XP, "Sage" title |
| Perfectionist | 100% accuracy on module | 300 XP, "Precise" title |

### 3.5 Profile Dashboard

```
┌────────────────────────────────────────────────────────────────────┐
│ [Avatar]  Username                           Level 12 Scholar      │
│           ████████████░░░░░░░░                2,450 / 3,000 XP     │
├────────────────────────────────────────────────────────────────────┤
│ 🔥 14-day streak     ⚡ Best: 25-combo     🎯 847 XP today         │
│ 🛡️ 2 freezes         📚 12/18 modules      ✓ 89% accuracy         │
├────────────────────────────────────────────────────────────────────┤
│ MODULE PROGRESS                                                    │
│ ┌────────────┬────────────┬────────────┬────────────┬───────────┐ │
│ │ Syllogistic│ Propos.    │ Modal      │ Deontic    │ Belief    │ │
│ │ ⭐⭐⭐ 100%  │ ⭐⭐░ 67%   │ ⭐░░ 23%   │ ░░░ 0%     │ 🔒        │ │
│ └────────────┴────────────┴────────────┴────────────┴───────────┘ │
├────────────────────────────────────────────────────────────────────┤
│ ACTIVITY (last 30 days)                                            │
│ ░░█░█████░░░██████░░░█████████░░                                   │
│        ▲                                                           │
│   practiced                                                        │
├────────────────────────────────────────────────────────────────────┤
│ RECENT ACHIEVEMENTS                                                │
│ 🏆 Week Warrior (7-day streak) — 2 days ago                        │
│ 🎖️ Century (100 correct) — 5 days ago                             │
└────────────────────────────────────────────────────────────────────┘
```

---

## 4. Curriculum Review Process

### 4.1 Exercise Audit with AI

Run each exercise through an AI with this prompt to identify confusing content:

```
Pretend you are a student seeing this exercise for the first time.
You have just read the lesson but are NOT an expert in logic.

Exercise: {exercise JSON}

Answer:
1. Is the prompt clear? What might confuse you?
2. Are the answer options unambiguous?
3. Is the hint helpful or too vague?
4. What would make this clearer?

Flag as: CLEAR / NEEDS WORK / CONFUSING
```

Store audit results in `assets/curriculum/.audit/{exercise_id}.json`.

### 4.2 Pedagogical Transformation Patterns

When improving exercises, apply these patterns:

| Pattern | Before | After |
|---------|--------|-------|
| **Socratic Hint** | "Use uppercase" | "Does 'cat' refer to one specific cat, or cats in general?" |
| **Debug Thinking** | "Check your syntax" | "If this were true, would a talking cat exist?" |
| **Real Stakes** | "P implies Q" | "If you study (S), you'll pass (P). S → P" |
| **Contrast Pairs** | Single explanation | "Compare: 'Some cat' (∃) vs 'All cats' (∀)" |

### 4.3 Hint Templates by Module Type

| Module | Hint Pattern |
|--------|-------------|
| Syllogistic | "Does X refer to a specific person, or a category?" |
| Propositional | "How many connectives do you see? Does each part get its own?" |
| Modal | "Is the sentence saying X IS true, or X COULD BE true?" |
| Quantifiers | "If this logic were true, would [counterexample] exist?" |
| Definitions | "Can you think of something that fits but ISN'T a [term]?" |

---

## 5. Technical: Infrastructure & Schema

### 5.1 Cloudflare Services

| Service | Purpose | Key Limits |
|---------|---------|------------|
| **Pages** | Static frontend (Dioxus WASM) | — |
| **Workers** | Rust API (`workers-rs`) | 10ms CPU (free), 30s (paid) |
| **D1** | SQLite database | 10GB, 5M reads/day (free) |
| **R2** | Object storage | 1 write/sec per key |

### 5.2 Crate Structure

```
logicaffeine/
├── logos/                 # Core compiler (no UI deps)
├── logos_core/            # Runtime library
├── logos_verification/    # Z3 verification (Pro+)
├── logos_web/             # Dioxus frontend
├── logos_cli/             # CLI tool
└── workers/
    ├── api/               # api.logicaffeine.com
    └── registry/          # registry.logicaffeine.com
```

### 5.3 D1 Database Schema

```sql
-- USERS & AUTH
CREATE TABLE users (
    id TEXT PRIMARY KEY,
    email TEXT,
    display_name TEXT NOT NULL,
    avatar_url TEXT,
    github_id TEXT UNIQUE,
    google_id TEXT UNIQUE,
    created_at TEXT DEFAULT (datetime('now')),
    last_login_at TEXT
);

CREATE TABLE sessions (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    refresh_token_hash TEXT NOT NULL,
    created_at TEXT DEFAULT (datetime('now')),
    expires_at TEXT NOT NULL,
    revoked_at TEXT
);

-- PROGRESS & GAMIFICATION
CREATE TABLE user_progress (
    user_id TEXT PRIMARY KEY,
    xp INTEGER DEFAULT 0,
    level INTEGER DEFAULT 1,
    current_streak INTEGER DEFAULT 0,
    best_streak INTEGER DEFAULT 0,
    streak_freezes INTEGER DEFAULT 0,
    last_activity_date TEXT,
    current_combo INTEGER DEFAULT 0,
    best_combo INTEGER DEFAULT 0,
    total_correct INTEGER DEFAULT 0,
    total_attempts INTEGER DEFAULT 0,
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
    next_review TEXT,
    last_attempt_at TEXT,
    PRIMARY KEY (user_id, exercise_id)
);

CREATE TABLE module_progress (
    user_id TEXT NOT NULL,
    module_id TEXT NOT NULL,
    exercises_completed INTEGER DEFAULT 0,
    exercises_total INTEGER NOT NULL,
    accuracy REAL DEFAULT 0,
    completed_at TEXT,
    PRIMARY KEY (user_id, module_id)
);

CREATE TABLE user_achievements (
    user_id TEXT NOT NULL,
    achievement_id TEXT NOT NULL,
    unlocked_at TEXT DEFAULT (datetime('now')),
    PRIMARY KEY (user_id, achievement_id)
);

-- LICENSES & PAYMENTS
CREATE TABLE licenses (
    id TEXT PRIMARY KEY,
    license_key TEXT UNIQUE NOT NULL,
    customer_email TEXT NOT NULL,
    plan TEXT NOT NULL,
    stripe_subscription_id TEXT,
    status TEXT DEFAULT 'active',
    created_at TEXT DEFAULT (datetime('now')),
    expires_at TEXT
);

-- BATTLES & TOURNAMENTS
CREATE TABLE battles (
    id TEXT PRIMARY KEY,
    player_a_id TEXT NOT NULL,
    player_b_id TEXT,
    exercise_set TEXT NOT NULL,
    player_a_score INTEGER DEFAULT 0,
    player_b_score INTEGER DEFAULT 0,
    status TEXT DEFAULT 'waiting',
    created_at TEXT DEFAULT (datetime('now')),
    finished_at TEXT,
    winner_id TEXT
);

CREATE TABLE tournaments (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    mode TEXT NOT NULL,
    max_players INTEGER DEFAULT 8,
    current_players INTEGER DEFAULT 0,
    status TEXT DEFAULT 'waiting',
    timer_per_move INTEGER,
    timer_total INTEGER,
    lives_per_player INTEGER DEFAULT 3,
    is_private INTEGER DEFAULT 0,
    invite_code TEXT UNIQUE,
    created_at TEXT DEFAULT (datetime('now')),
    started_at TEXT,
    finished_at TEXT
);

CREATE TABLE tournament_players (
    tournament_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    lives_remaining INTEGER DEFAULT 3,
    score INTEGER DEFAULT 0,
    status TEXT DEFAULT 'active',
    final_placement INTEGER,
    PRIMARY KEY (tournament_id, user_id)
);

CREATE TABLE matchmaking_queue (
    user_id TEXT PRIMARY KEY,
    skill_ranking INTEGER DEFAULT 1000,
    queued_at TEXT DEFAULT (datetime('now')),
    preferred_mode TEXT
);

-- ANALYTICS (for AI audit and data science)
CREATE TABLE analytics_events (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    event_type TEXT NOT NULL,           -- 'answer', 'hint_shown', 'module_start', etc.
    exercise_id TEXT,
    user_input TEXT,                    -- What they typed/selected
    correct_answer TEXT,                -- Expected answer
    is_correct INTEGER,
    time_to_answer_ms INTEGER,          -- Latency tracking
    hint_was_shown INTEGER DEFAULT 0,
    created_at TEXT DEFAULT (datetime('now'))
);

-- INDEXES
CREATE INDEX idx_users_github ON users(github_id);
CREATE INDEX idx_users_google ON users(google_id);
CREATE INDEX idx_progress_xp ON user_progress(xp DESC);
CREATE INDEX idx_battles_status ON battles(status);
CREATE INDEX idx_tournaments_status ON tournaments(status);
CREATE INDEX idx_queue_skill ON matchmaking_queue(skill_ranking);
```

---

## 6. Technical: API & Authentication

### 6.1 API Routes

```rust
// Health
GET  /health

// Auth (OAuth only)
GET  /auth/github              → Redirect to GitHub OAuth
GET  /auth/github/callback     → Handle callback, create session
GET  /auth/google              → Redirect to Google OAuth
GET  /auth/google/callback     → Handle callback, create session
POST /auth/refresh             → Refresh access token
GET  /auth/me                  → Get current user

// Progress
GET  /progress                 → Get user progress
POST /progress/sync            → Sync progress from client
POST /progress/xp              → Record XP event

// Leaderboard
GET  /leaderboard              → Global leaderboard
GET  /leaderboard/weekly       → Weekly leaderboard
GET  /leaderboard/:era         → Era-specific leaderboard

// Battles
POST /battles/create           → Create battle
GET  /battles/:id              → Get battle state
POST /battles/:id/action       → Submit answer

// Tournaments
GET  /tournaments              → List active tournaments
POST /tournaments/create       → Create tournament
POST /tournaments/:id/join     → Join tournament
GET  /tournaments/:id          → Get tournament state
POST /tournaments/:id/submit   → Submit answer
POST /tournaments/queue/join   → Join matchmaking queue

// Payments
POST /session                  → Handle Stripe session
POST /validate                 → Validate license key
POST /webhook/stripe           → Stripe webhook
```

### 6.2 OAuth Flow

```
┌─────────────────────────────────────────────────────────────────────┐
│                         OAUTH FLOW                                   │
├─────────────────────────────────────────────────────────────────────┤
│                                                                     │
│  1. User clicks "Sign in with GitHub"                              │
│     └─▶ Frontend redirects to /auth/github                         │
│                                                                     │
│  2. Worker redirects to GitHub OAuth                               │
│     └─▶ github.com/login/oauth/authorize?client_id=...             │
│                                                                     │
│  3. User authorizes, GitHub redirects back                         │
│     └─▶ /auth/github/callback?code=...                             │
│                                                                     │
│  4. Worker exchanges code for token                                │
│     ├─▶ Fetch GitHub access token                                  │
│     ├─▶ Fetch user profile from GitHub API                         │
│     ├─▶ Upsert user in D1                                          │
│     └─▶ Create JWT tokens (access + refresh)                       │
│                                                                     │
│  5. Redirect to frontend with token                                │
│     └─▶ logicaffeine.com/?token=...                                │
│                                                                     │
│  6. Frontend stores tokens in localStorage                         │
│     └─▶ Includes tokens in Authorization header for API calls      │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
```

### 6.3 Payment Flow

```
┌─────────────────────────────────────────────────────────────────────┐
│                        PAYMENT FLOW                                  │
├─────────────────────────────────────────────────────────────────────┤
│                                                                     │
│  1. User clicks "Buy" → Redirect to Stripe Checkout                │
│                                                                     │
│  2. After payment, Stripe redirects to /success?session_id=...     │
│                                                                     │
│  3. Frontend calls POST /session { sessionId }                     │
│     Worker:                                                        │
│     ├─▶ Verify session with Stripe API                             │
│     ├─▶ Generate license key: LGC-XXXX-XXXX-XXXX-XXXX             │
│     ├─▶ Store in D1 licenses table                                 │
│     ├─▶ Send email with license key                                │
│     └─▶ Return { licenseKey, plan, email }                         │
│                                                                     │
│  4. Frontend stores license key locally                            │
│                                                                     │
│  5. License validation: POST /validate { licenseKey }              │
│     └─▶ Check D1, return { valid, plan }                           │
│                                                                     │
│  6. Stripe webhooks update license status on cancel/renewal        │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
```

---

## 7. Multiplayer Battles & Tournaments

### 7.1 Architecture: Durable Objects (Not R2 Polling)

**Critical:** R2's 1-write-per-second limit creates unacceptable latency (~1000ms) for Blitz mode. AAA standard is <100ms feedback.

**Solution:** Use **Cloudflare Durable Objects** for real-time game state:
- Strong consistency with low-latency WebSockets
- Each battle/tournament gets its own Durable Object instance
- State persists in-memory with automatic persistence to storage

```
┌─────────────────────────────────────────────────────────────────────┐
│                 BATTLE SYSTEM (DURABLE OBJECTS)                     │
├─────────────────────────────────────────────────────────────────────┤
│                                                                     │
│  ┌─────────┐                                     ┌─────────┐       │
│  │Player A │                                     │Player B │       │
│  └────┬────┘                                     └────┬────┘       │
│       │                                               │            │
│       │  WebSocket connect                            │            │
│       │───────────────────┐         ┌────────────────│            │
│       │                   ▼         ▼                │            │
│       │            ┌─────────────────────┐           │            │
│       │            │   DURABLE OBJECT    │           │            │
│       │            │   (Battle State)    │           │            │
│       │            │                     │           │            │
│       │            │ • scores            │           │            │
│       │            │ • current_question  │           │            │
│       │            │ • timer             │           │            │
│       │            │ • lives             │           │            │
│       │            └──────────┬──────────┘           │            │
│       │                       │                      │            │
│       │◀──────────────────────┴─────────────────────▶│            │
│       │        Real-time broadcasts (<50ms)          │            │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
```

**Flow:**
1. Player connects via WebSocket to `/battle/:id/ws`
2. Worker routes to the battle's Durable Object
3. Durable Object maintains authoritative state
4. Actions processed immediately, broadcast to all connected clients
5. Results stored to D1 after match ends (for history/analytics)

### 7.3 Tournament Modes

| Mode | Timer | Lives | Description |
|------|-------|-------|-------------|
| **Blitz** | 10-30s per move | 3 | Fast-paced, timer resets each answer |
| **Classic** | 5 min total | 3 | Chess-clock style, time bank depletes |
| **Marathon** | 30 min total | 5 | Endurance, survive the longest |
| **Sprint** | 1 min rounds | 1 | Race format, 1 life per round |
| **Co-op** | 8 min shared | 5 | Two players solve together |

### 7.4 Lives System with Ghost Mode

When a player loses all lives, they enter **Ghost Mode** instead of being fully eliminated:
- Can continue solving problems
- Earn partial XP (25% rate)
- Can earn "Redemption Tokens" (3 correct = 1 token)
- Spend 2 tokens to revive with 1 life
- Keeps eliminated players engaged instead of idle

```rust
pub fn record_answer(&mut self, correct: bool) -> AnswerResult {
    if correct {
        self.score += 1;
        if self.status == PlayerStatus::Ghost {
            self.redemption_progress += 1;
            if self.redemption_progress >= 3 {
                self.redemption_tokens += 1;
                self.redemption_progress = 0;
            }
        }
        AnswerResult::Correct
    } else {
        if self.status == PlayerStatus::Active {
            self.lives_remaining = self.lives_remaining.saturating_sub(1);
            if self.lives_remaining == 0 {
                self.status = PlayerStatus::Ghost;  // Not eliminated!
                AnswerResult::EnteredGhostMode
            } else {
                AnswerResult::Wrong { lives_left: self.lives_remaining }
            }
        } else {
            AnswerResult::GhostWrong  // No penalty in ghost mode
        }
    }
}
```

### 7.5 Tournament Rewards

| Placement | XP Multiplier | Title |
|-----------|---------------|-------|
| 1st | 5x | "Tournament Champion" |
| 2nd | 3x | "Silver Logician" |
| 3rd | 2x | "Bronze Reasoner" |
| Top 8 | 1.5x | — |

### 7.6 Matchmaking

- Skill-based using ELO ranking (default 1000)
- Match players within 200 ranking points
- Queue fills to 8 players, then tournament auto-starts
- Private lobbies via invite code (e.g., "LOGIC-ABCD")

---

## 8. AAA Quality Requirements

### 8.1 The "Flow State" Requirement

**No interaction shall take longer than 100ms to provide visual feedback.**

- Clicking a tab, submitting an answer, or any button press must show immediate visual response
- Network requests happen in the background with optimistic UI updates
- Use loading skeletons, not spinners

### 8.2 The "Juice" (UX Feel)

| Element | Requirement |
|---------|-------------|
| **Optimistic UI** | When user clicks "Check," UI reacts immediately (sound + visual) before server response |
| **Micro-animations** | Progress bars fill with glow, combo counter shakes on high numbers |
| **Sound design** | Correct/wrong sounds, combo milestone sounds, achievement unlocks |
| **Keyboard shortcuts** | `Enter` to submit, `1-4` for multiple choice, `Tab` to navigate |

### 8.3 Mobile Requirements

- **Symbol Dictionary**: Bottom-sheet modal on mobile, not inline (clutters reading)
- **Touch targets**: Minimum 44x44px for all interactive elements
- **Swipe gestures**: Swipe between tabs, swipe to dismiss modals

### 8.4 Anti-Cheat (Server-Side)

Simple validation to prevent cheating:
- Reject submissions faster than 500ms (humanly impossible)
- Validate answer server-side, never trust client "correct" signals
- Rate limit: Max 60 submissions per minute

---

## 9. Implementation Checklist

### 9.1 TDD Workflow

For each feature, follow RED/GREEN/REFACTOR:

```bash
# 1. RED: Write failing tests
cargo test --test feature_name  # Should FAIL

# 2. GREEN: Implement minimum code
cargo test --test feature_name  # Should PASS

# 3. REFACTOR: Clean up, then verify ALL tests
cargo test                       # ALL must pass
```

### 9.2 Pre-Commit Requirements

Before ANY commit:
```bash
cargo test                  # Unit + integration tests
cargo test --test e2e       # End-to-end tests (if applicable)
```

**Zero regressions allowed.** If tests fail, fix before committing.

### 9.3 Implementation Phases

**Phase 1: Integrated Learn Page**
- [ ] Consolidate routes: remove /lesson/:era/:module/:mode, /review
- [ ] Build tabbed module sections (LESSON / EXAMPLES / PRACTICE / TEST)
- [ ] Implement Focus Mode (collapse other eras when one is expanded)
- [ ] Implement Symbol Dictionary component (auto-generated from FOL output)
- [ ] Add struggle-triggered Socratic hints (5s inactivity or wrong attempt)
- [ ] Implement module unlocking logic (last 2 locked until 1 complete)

**Phase 2: Practice & Test Components**
- [ ] Build infinite Practice flashcard with priority queue (wrong answers at +3)
- [ ] Implement combo system in Practice mode
- [ ] Implement diminishing XP returns after module mastery
- [ ] Build 17-question Test mode with score screen
- [ ] Add Socratic-style mistake review (questions, not just answers)
- [ ] Track best test scores per module

**Phase 3: AAA Polish**
- [ ] Implement optimistic UI (immediate visual feedback before server)
- [ ] Add micro-animations (progress bar glow, combo shake)
- [ ] Implement keyboard shortcuts (Enter, 1-4, Tab)
- [ ] Build mobile bottom-sheet for Symbol Dictionary
- [ ] Add sound effects (correct/wrong/combo/achievement)

**Phase 4: Curriculum Content**
- [ ] Generate `lesson.md` for each module using AI
- [ ] Create `symbols.json` for each module
- [ ] Run AI audit on all ~380 exercises
- [ ] Fix CONFUSING exercises, improve NEEDS WORK exercises

**Phase 5: Progress & Profile**
- [ ] Create /profile page with stats display
- [ ] Add activity calendar (30-day heatmap)
- [ ] Show module progress grid with stars
- [ ] Display achievements list

**Phase 6: Backend Infrastructure**
- [ ] Set up D1 database with schema (including analytics_events)
- [ ] Port Workers from JavaScript to Rust
- [ ] Implement OAuth (GitHub, Google)
- [ ] Implement progress sync API
- [ ] Add anti-cheat validation (500ms min, rate limiting)

**Phase 7: Payments**
- [ ] Implement license key generation
- [ ] Add Stripe webhook handling
- [ ] Email delivery for license keys

**Phase 8: Multiplayer**
- [ ] Set up Durable Objects for battle state
- [ ] Implement WebSocket connections for real-time updates
- [ ] Implement Ghost Mode for eliminated players
- [ ] Create tournament system with matchmaking
- [ ] Store match results to D1 after completion

### 9.4 Curriculum Audit Process

1. Run audit prompt on all ~380 exercises
2. Triage by flag: CONFUSING (fix now) → NEEDS WORK (fix soon) → CLEAR (ok)
3. Apply transformation patterns to flagged exercises
4. Re-run audit to verify improvements

---

## 10. Glossary

| Term | Definition |
|------|------------|
| **Blitz Mode** | Tournament with per-move timers (10-30s) and 3 lives |
| **Chess-Clock** | Timer mode where total time bank depletes while thinking |
| **Combo** | Consecutive correct answers; multiplies XP up to 2x |
| **D1** | Cloudflare's serverless SQLite database |
| **Diminishing Returns** | XP decay after module mastery to prevent grinding easy content |
| **Dioxus** | Rust framework for building the web UI |
| **Durable Objects** | Cloudflare's stateful edge compute; used for real-time battles |
| **Era** | Major curriculum division (e.g., Era 0: Practice, Era 1: Basics) |
| **Focus Mode** | UI state where non-active eras collapse to reduce cognitive clutter |
| **FOL** | First-Order Logic; the formal logic system taught |
| **Ghost Mode** | Tournament state after elimination; can earn redemption tokens |
| **Lives** | Tournament mechanic; wrong answers cost lives, 0 = ghost mode |
| **Module** | Section within an Era containing related exercises |
| **OAuth** | Authentication protocol for GitHub/Google sign-in |
| **Optimistic UI** | Immediate visual feedback before server confirmation |
| **Practice Mode** | Infinite flashcard mode with priority queue for wrong answers |
| **Priority Queue** | Practice ordering where wrong answers reappear at position +3 |
| **R2** | Cloudflare's object storage (used for static assets) |
| **Socratic Hint** | Struggle-triggered hint that guides discovery through questioning |
| **SRS** | Spaced Repetition System; algorithm for optimal review timing |
| **Streak** | Consecutive days practiced; grants XP bonus |
| **Symbol Dictionary** | Auto-generated legend explaining all symbols in a logic output |
| **Test Mode** | 17-question scored assessment per module; full XP, no hints |
| **Workers** | Cloudflare's serverless functions (Rust via `workers-rs`) |
| **XP** | Experience Points; earned for completing exercises |

---

## Appendix A: Curriculum Mapping (Gensler's "Introduction to Logic")

The curriculum is structured around Harry Gensler's textbook "Introduction to Logic" with symbol notation conversions.

### A.1 Symbol Notation Mapping

Gensler uses different symbols than our standard. The platform auto-converts:

| Gensler | Platform | Meaning |
|---------|----------|---------|
| `~` | `¬` | negation (not) |
| `•` | `∧` | conjunction (and) |
| `⊃` | `→` | implication (if...then) |
| `≡` | `↔` | biconditional (if and only if) |
| `∨` | `∨` | disjunction (or) — same |

**Lesson files use platform symbols** but reference Gensler's notation for students using the textbook.

### A.2 Era 0: Syllogistic Practice (Gensler Chapter 2)

**Focus:** Categorical syllogisms and traditional logic forms.

| Module | Topic | Gensler Section | Key Patterns |
|--------|-------|-----------------|--------------|
| 0.1 | Categorical Propositions | §2.1-2.2 | A/E/I/O forms |
| 0.2 | Syllogism Forms | §2.3-2.4 | Barbara, Celarent, etc. |
| 0.3 | Venn Diagrams | §2.5 | Visual validity testing |
| 0.4 | Immediate Inferences | §2.6 | Conversion, obversion |
| 0.5 | Sorites | §2.7 | Chained syllogisms |
| 0.6 | Informal Fallacies | §2.8 | Common reasoning errors |

**Example Lesson Content (0.1 Categorical Propositions):**

```markdown
## Concept
Categorical propositions make claims about categories (groups). There are
exactly four types, remembered as A, E, I, O:

| Form | Pattern | Example |
|------|---------|---------|
| A | All S are P | All dogs are mammals |
| E | No S are P | No cats are reptiles |
| I | Some S are P | Some birds fly |
| O | Some S are not P | Some animals are not pets |

## Notation
- Uppercase letters (A, B, C) = categories/classes
- "All A are B" = every member of A is also in B
- "Some A are B" = at least one A is also a B

## Pattern
"All [SUBJECT] are [PREDICATE]" → A-form
"No [SUBJECT] are [PREDICATE]" → E-form
"Some [SUBJECT] are [PREDICATE]" → I-form
"Some [SUBJECT] are not [PREDICATE]" → O-form
```

### A.3 Era 1: Propositional Logic (Gensler Chapter 6)

**Focus:** Truth-functional connectives and symbolic translation.

| Module | Topic | Gensler Section | Key Patterns |
|--------|-------|-----------------|--------------|
| 1.1 | Atomic Sentences | §6.1 | Constants and predicates |
| 1.2 | Negation | §6.2 | `¬P`, double negation |
| 1.3 | Conjunction | §6.3 | `P ∧ Q` (and) |
| 1.4 | Disjunction | §6.4 | `P ∨ Q` (or) |
| 1.5 | Conditional | §6.5 | `P → Q` (if...then) |
| 1.6 | Biconditional | §6.6 | `P ↔ Q` (iff) |

**Example Lesson Content (1.3 Conjunction):**

```markdown
## Concept
A conjunction joins two statements with "and." Both parts must be true
for the whole to be true.

## Notation
- ∧ = "and" / conjunction
- P ∧ Q is true only when BOTH P is true AND Q is true

## Pattern
"[Statement 1] and [Statement 2]" → S₁ ∧ S₂

Watch for synonyms:
- "but" → ∧ (same as "and")
- "although" → ∧
- "yet" → ∧
- "while" (simultaneously) → ∧

## Examples
1. "It is raining and cold"
   - Let R = "It is raining", C = "It is cold"
   - Translation: R ∧ C

2. "Paris is beautiful but expensive"
   - Let B = "Paris is beautiful", E = "Paris is expensive"
   - Translation: B ∧ E
   - Note: "but" emphasizes contrast but logically = "and"

## Common Mistakes
- Using ∧ for "If...then" statements (should be →)
- Forgetting that "but" still means ∧ in logic
```

### A.4 Era 2: Basic Quantifiers (Gensler Chapter 8)

**Focus:** Universal and existential quantification in First-Order Logic.

| Module | Topic | Gensler Section | Key Patterns |
|--------|-------|-----------------|--------------|
| 2.1 | Predicates & Individuals | §8.1 | F(x), constants a, b, c |
| 2.2 | Universal Quantifier | §8.2 | ∀x (for all) |
| 2.3 | Existential Quantifier | §8.3 | ∃x (there exists) |
| 2.4 | Mixed Quantifiers | §8.4 | ∀x∃y, ∃x∀y scope |
| 2.5 | Quantifier Negation | §8.5 | ¬∀x ≡ ∃x¬, etc. |
| 2.6 | Relations | §8.6 | R(x,y), Loves(j,m) |

**Example Lesson Content (2.2 Universal Quantifier):**

```markdown
## Concept
The universal quantifier (∀) means "for all" or "every." It makes a claim
about EVERYTHING in the universe of discourse.

## Notation
- ∀x = "for all x" / "for every x"
- ∀x(P(x)) = "everything has property P"
- ∀x(P(x) → Q(x)) = "all P are Q"

## Pattern
"All [THINGS] are [PROPERTY]" → ∀x(Thing(x) → Property(x))

Why use → and not ∧?
- ∀x(Cat(x) ∧ Mammal(x)) means "everything is both a cat and a mammal"
- ∀x(Cat(x) → Mammal(x)) means "IF something is a cat, THEN it's a mammal"
- The second is what "All cats are mammals" actually means!

## Examples
1. "All cats are mammals"
   → ∀x(Cat(x) → Mammal(x))

2. "Every student passed"
   → ∀x(Student(x) → Passed(x))

3. "All that glitters is not gold"
   → ¬∀x(Glitters(x) → Gold(x))
   OR: ∃x(Glitters(x) ∧ ¬Gold(x))

## Common Mistakes
- Using ∧ instead of → in universal statements
- Forgetting the variable: writing ∀(Cat → Mammal) instead of ∀x(Cat(x) → Mammal(x))
```

### A.5 File Structure with Gensler References

```
assets/curriculum/00_syllogistic/
├── 01_categorical_props/
│   ├── meta.json           # Includes gensler_ref: "§2.1-2.2"
│   ├── lesson.md           # Platform symbols with Gensler notes
│   ├── symbols.json        # Includes notation_map for conversions
│   └── exercises/
│       ├── ex_01.json
│       └── ...
├── 02_syllogism_forms/
│   ├── meta.json           # gensler_ref: "§2.3-2.4"
│   └── ...
```

**meta.json with Gensler reference:**
```json
{
  "id": "categorical_props",
  "era": 0,
  "title": "Categorical Propositions",
  "pedagogy": "Introduce A/E/I/O proposition forms for categorical reasoning",
  "gensler_ref": "§2.1-2.2",
  "textbook": "Introduction to Logic, 3rd ed.",
  "exercises_count": 24
}
```

**symbols.json with notation mapping:**
```json
{
  "notation_map": {
    "gensler_to_platform": {
      "~": "¬",
      "•": "∧",
      "⊃": "→",
      "≡": "↔"
    }
  },
  "symbols": [
    { "symbol": "A", "name": "A-form", "meaning": "All S are P" },
    { "symbol": "E", "name": "E-form", "meaning": "No S are P" },
    { "symbol": "I", "name": "I-form", "meaning": "Some S are P" },
    { "symbol": "O", "name": "O-form", "meaning": "Some S are not P" }
  ]
}
```

---

*Document maintained by Engineering Team. Last updated: December 30, 2025.*
