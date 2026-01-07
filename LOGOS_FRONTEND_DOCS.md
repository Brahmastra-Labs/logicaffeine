# LOGOS - Frontend Documentation

## Overview
This document covers the **Frontend Layer** of the LOGOS system: the Dioxus web application, UI components, and gamification engine.

## Table of Contents
1. [Architecture](#architecture)
2. [Web Application](#web-application)
3. [Problem Generator](#problem-generator)
4. [Gamification](#gamification)
5. [Relevant Tests](#relevant-tests)

## Architecture

**Stack:**
*   **Framework:** Dioxus 0.6 (React-like Rust framework)
*   **Routing:** Client-side routing (`src/ui/router.rs`)
*   **State:** Signal-based reactivity
*   **Platform:** WASM (Browser) / Desktop (WebView)

### Entry Point
**File:** `src/main.rs`

App launch configuration.

\n
---
### App Component
**File:** `src/ui/app.rs`

Root component and layout.

\n
---
### Router
**File:** `src/ui/router.rs`

Route definitions.

\n
---
### Global State
**File:** `src/ui/state.rs`

Signal management.

\n
---
### Workspace
**File:** `src/ui/pages/workspace.rs`

Main IDE interface.

\n
---
### Lesson
**File:** `src/ui/pages/lesson.rs`

Interactive problem solving.

\n
---
### Content Engine
**File:** `src/content.rs`

Curriculum loading.

\n
---
### Generator
**File:** `src/generator.rs`

Problem template instantiation.

\n
---
### Grader
**File:** `src/grader.rs`

Semantic answer checking.

\n
---
### Game State
**File:** `src/game.rs`

XP, Level, and Streak tracking.

\n
---
### Achievements
**File:** `src/achievements.rs`

Achievement system logic.

\n
---
### SRS
**File:** `src/srs.rs`

Spaced Repetition System.

\n
---
### Audio
**File:** `src/audio.rs`

Sound effects.

\n
---
## Relevant Tests
#### Learning State
**File:** `tests/learn_state_tests.rs`
Progress tracking tests.
---
#### Unlock Logic
**File:** `tests/unlock_logic_tests.rs`
Lesson unlocking tests.
---
#### E2E Collections
**File:** `tests/e2e_collections.rs`
Runtime verification (relevant for UI feedback).
---
