# LOGOS Code Editor / Playground

**Status:** Draft
**Version:** 0.1.0
**Last Updated:** December 2024

---

## 0. Language Implementation Status

> **This section documents what's currently working in the LOGOS programming language.**

### 0.1 Fully Implemented (848 tests passing)

#### Statements

| Statement | Syntax | Generated Rust | Status |
|-----------|--------|----------------|--------|
| **Let** | `Let x be 5.` | `let x = 5;` | ✅ |
| **Let (typed)** | `Let x: Int be 5.` | `let x: i64 = 5;` | ✅ |
| **Let (mutable)** | `Let mutable x be 5.` | `let mut x = 5;` | ✅ |
| **Set** | `Set x to 10.` | `x = 10;` | ✅ |
| **Return** | `Return 42.` | `return 42;` | ✅ |
| **Return (void)** | `Return.` | `return;` | ✅ |
| **If** | `If x equals 5: ...` | `if x == 5 { ... }` | ✅ |
| **If/Otherwise** | `If x equals 5: ... Otherwise: ...` | `if x == 5 { ... } else { ... }` | ✅ |
| **While** | `While x less than 10: ...` | `while x < 10 { ... }` | ✅ |
| **Repeat (collection)** | `Repeat for x in items: ...` | `for x in items { ... }` | ✅ |
| **Repeat (range)** | `Repeat for i from 1 to 10: ...` | `for i in 1..=10 { ... }` | ✅ |
| **Call** | `Call function.` | `function();` | ✅ |
| **Assert** | `Assert that x is greater than 0.` | `debug_assert!(x > 0);` | ✅ |
| **Show** | `Show x.` | `show(x);` | ✅ |
| **Function Def** | `## To verb (x: T):` | `fn verb(x: T) { }` | ✅ |
| **Call (expr)** | `func(x, y)` | `func(x, y)` | ✅ |

#### Expressions

| Expression | Example | Generated Rust | Status |
|------------|---------|----------------|--------|
| **Numbers** | `42`, `3.14` | `42`, `3.14` | ✅ |
| **Text** | `"hello"` | `"hello".to_string()` | ✅ |
| **Booleans** | `true`, `false` | `true`, `false` | ✅ |
| **Nothing** | `nothing` | `()` | ✅ |
| **Addition** | `x plus y` | `(x + y)` | ✅ |
| **Subtraction** | `x minus y` | `(x - y)` | ✅ |
| **Multiplication** | `x times y` | `(x * y)` | ✅ |
| **Division** | `x divided by y` | `(x / y)` | ✅ |
| **Equality** | `x equals 5` | `(x == 5)` | ✅ |
| **Inequality** | `x does not equal 5` | `(x != 5)` | ✅ |
| **Less than** | `x less than 10` | `(x < 10)` | ✅ |
| **Greater than** | `x greater than 0` | `(x > 0)` | ✅ |
| **List literals** | `[1, 2, 3]` | `vec![1, 2, 3]` | ✅ |
| **Empty list** | `[]` | `vec![]` | ✅ |
| **Indexing** | `item 1 of list` | `list[0]` (1→0 indexed) | ✅ |
| **Ranges** | `1 to 10` | `1..=10` | ✅ |

#### Types

| LOGOS Type | Rust Type | Status |
|------------|-----------|--------|
| `Int` | `i64` | ✅ |
| `Nat` | `u64` | ✅ |
| `Real` | `f64` | ✅ |
| `Text` | `String` | ✅ |
| `Bool` | `bool` | ✅ |
| `Unit` | `()` | ✅ |
| `List of X` | `Vec<X>` | ✅ |
| `Seq of X` | `Vec<X>` | ✅ |
| `Option of X` | `Option<X>` | ✅ |
| `Result of X and Y` | `Result<X, Y>` | ✅ |

#### Runtime Library (logos_core)

The compiler embeds a runtime library with:
- `show(x)` - Display value to console
- `print(x)` - Print without newline
- `println(x)` - Print with newline
- `read_line()` - Read user input
- Type aliases for all LOGOS types

### 0.2 Partially Implemented

| Feature | What Works | What's Missing |
|---------|------------|----------------|
| **Structs** | Definition, constructor (`a new Point`) | Field access (`p's x`) |
| **Give/Show ownership** | Parser recognizes verbs | Codegen for ownership transfer |

### 0.3 Not Yet Implemented

| Feature | Notes |
|---------|-------|
| Pattern matching | `Inspect` / `Match` statements |
| Error handling | Try/catch, Result propagation |
| Concurrency | Spawn, channels, agents |
| String interpolation | `"Hello, {name}!"` |
| Higher-order functions | Lambdas, closures |

---

## 1. Vision & Goals

### 1.1 What We're Building

A web-based IDE for writing, compiling, and running LOGOS programs. Think "REPL meets IDE" - simple enough to experiment quickly, powerful enough for real development.

### 1.2 Core Principles

| Principle | Description |
|-----------|-------------|
| **Immediate Feedback** | See generated Rust code as you type |
| **Run Anywhere** | Execute code without local toolchain |
| **Minimal Friction** | No setup, no accounts, just code |
| **Progressive Complexity** | Simple by default, powerful when needed |

### 1.3 Target Users

1. **Learners** - Students exploring LOGOS syntax
2. **Developers** - Building real applications
3. **Experimenters** - Testing language features
4. **Educators** - Demonstrating concepts

---

## 2. Architecture

### 2.1 Layout Diagram

```
┌─────────────────────────────────────────────────────────────────────────┐
│  Header: λ LOGOS Playground                    [Home] [Learn] [Studio]  │
├────────────┬────────────────────────────┬───────────────────────────────┤
│            │                            │                               │
│  File Tree │  LOGOS Editor              │  Rust Output                  │
│            │                            │                               │
│  📁 src/   │  ## Main                   │  fn main() {                  │
│    main.lg │                            │      let x = 5;               │
│    lib.lg  │  Let x be 5.               │      return x;                │
│            │  Return x.                 │  }                            │
│            │                            │                               │
│  + New     │                            │  ─────────────────────────    │
│            │                            │  Compile: ✓ Success           │
├────────────┴────────────────────────────┴───────────────────────────────┤
│  Console                                                       [▶ Run]  │
│  > Program output will appear here...                                   │
│                                                                         │
└─────────────────────────────────────────────────────────────────────────┘
```

### 2.2 Panel Responsibilities

| Panel | Purpose |
|-------|---------|
| **File Tree** | Navigate and manage project files |
| **Editor** | Write LOGOS source code |
| **Rust Output** | View generated Rust code + compilation status |
| **Console** | Execution output, errors, and REPL history |

### 2.3 Data Flow

```
User Types → Editor → compile_to_rust() → Rust Panel
                                      ↓
User Clicks Run → POST /api/run → Server → cargo build+run → Console
```

---

## 3. Feature Requirements

### 3.1 Must Have (MVP)

| Feature | Description |
|---------|-------------|
| **Code Editor** | Syntax-aware text editing for LOGOS |
| **Live Rust Preview** | Real-time Rust codegen as you type |
| **Run Button** | Execute code and see output |
| **Error Display** | Show compilation/runtime errors clearly |
| **Single File Mode** | Basic editing without file management |

### 3.2 Should Have (v1.0)

| Feature | Description |
|---------|-------------|
| **File Tree** | Create, rename, delete, organize files |
| **localStorage Persistence** | Files saved in browser |
| **Multiple Tabs** | Work on multiple files |
| **Keyboard Shortcuts** | Cmd+S save, Cmd+Enter run, etc. |
| **Resizable Panels** | Adjust layout to preference |

### 3.3 Nice to Have (Future)

| Feature | Description |
|---------|-------------|
| **Syntax Highlighting** | Rich LOGOS syntax colors |
| **Autocomplete** | Keyword and variable suggestions |
| **Share Links** | URL-encoded snippets for sharing |
| **Dark/Light Theme** | Theme toggle |
| **Export to Cargo** | Download as full Cargo project |
| **Import Examples** | Load pre-built examples |
| **Collaborative Editing** | Real-time multiplayer |

---

## 4. Component Breakdown

### 4.1 New Components to Create

| Component | File | Description |
|-----------|------|-------------|
| **Playground** | `src/ui/pages/playground.rs` | Main page component |
| **FileTree** | `src/ui/components/file_tree.rs` | File explorer sidebar |
| **RustOutput** | `src/ui/components/rust_output.rs` | Rust code display |
| **Console** | `src/ui/components/console.rs` | Execution output |
| **RunButton** | `src/ui/components/run_button.rs` | Execute button with loading state |

### 4.2 Existing Components to Reuse

| Component | From | Usage |
|-----------|------|-------|
| **LiveEditor** | `src/ui/components/editor.rs` | Code input textarea |
| **SocraticGuide** | `src/ui/components/socratic_guide.rs` | Error hints |

### 4.3 Component Hierarchy

```
Playground
├── Header
│   ├── Logo
│   └── NavLinks
├── MainLayout (resizable)
│   ├── FileTree
│   │   ├── FileList
│   │   └── NewFileButton
│   ├── EditorPanel
│   │   ├── TabBar (if multiple files)
│   │   └── LiveEditor
│   └── RustPanel
│       ├── RustOutput
│       └── CompileStatus
├── ConsolePanel
│   ├── Console
│   └── RunButton
└── Footer (optional)
    └── SocraticGuide
```

---

## 5. Technical Implementation

### 5.1 Files to Create

```
src/
├── ui/
│   ├── pages/
│   │   └── playground.rs      # ~400 lines
│   └── components/
│       ├── file_tree.rs       # ~150 lines
│       ├── rust_output.rs     # ~100 lines
│       ├── console.rs         # ~80 lines
│       └── run_button.rs      # ~50 lines
└── server.rs                  # ~150 lines (backend)
```

### 5.2 Files to Modify

| File | Changes |
|------|---------|
| `src/ui/router.rs` | Add `#[route("/playground")] Playground {}` |
| `src/ui/pages/mod.rs` | Add `pub mod playground; pub use playground::Playground;` |
| `src/ui/pages/home.rs` | Add 5th portal card linking to Playground |
| `src/ui/components/mod.rs` | Export new components |
| `Cargo.toml` | Add `axum`, `tokio` as optional deps for server |

### 5.3 Route Definition

```rust
// src/ui/router.rs
#[derive(Clone, Routable, Debug, PartialEq)]
pub enum Route {
    // ... existing routes ...

    #[route("/playground")]
    Playground {},
}
```

### 5.4 Portal Card Addition

```rust
// src/ui/pages/home.rs - add to portal-grid
Link {
    to: Route::Playground {},
    class: "portal-card",
    div { class: "icon", "💻" }
    h2 { "Playground" }
    p { "Write, compile, and run LOGOS programs in your browser." }
    div { class: "arrow", "→" }
}
```

---

## 6. API Specification

### 6.1 Endpoints

#### POST /api/run

Execute LOGOS source code.

**Request:**
```json
{
  "source": "## Main\nLet x be 5.\nReturn x."
}
```

**Response (success):**
```json
{
  "success": true,
  "rust_code": "fn main() {\n    let x = 5;\n    return x;\n}\n",
  "output": "5\n",
  "execution_time_ms": 127
}
```

**Response (compile error):**
```json
{
  "success": false,
  "error": {
    "type": "parse",
    "message": "Unknown verb 'Xyz' at line 2",
    "line": 2,
    "column": 1
  }
}
```

**Response (runtime error):**
```json
{
  "success": false,
  "rust_code": "fn main() { panic!(\"oops\"); }",
  "error": {
    "type": "runtime",
    "message": "thread 'main' panicked at 'oops'"
  }
}
```

### 6.2 Server Implementation

```rust
// src/server.rs
use axum::{Router, Json, routing::post};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct RunRequest {
    source: String,
}

#[derive(Serialize)]
struct RunResponse {
    success: bool,
    rust_code: Option<String>,
    output: Option<String>,
    error: Option<RunError>,
    execution_time_ms: Option<u64>,
}

async fn run_handler(Json(req): Json<RunRequest>) -> Json<RunResponse> {
    // 1. Compile to Rust
    // 2. Write to temp directory
    // 3. cargo build + run with timeout
    // 4. Return output or error
}

pub async fn run_server() {
    let app = Router::new()
        .route("/api/run", post(run_handler));

    axum::serve(listener, app).await.unwrap();
}
```

### 6.3 Security Considerations

| Concern | Mitigation |
|---------|------------|
| Infinite loops | 5 second execution timeout |
| Resource exhaustion | Memory limit (256MB) |
| File system access | Sandboxed temp directory |
| Network access | Block outbound connections |
| Malicious code | Run in isolated container (future) |

---

## 7. State Management

### 7.1 Playground State

```rust
struct PlaygroundState {
    // File Management
    files: Signal<Vec<FileEntry>>,
    active_file_idx: Signal<usize>,

    // Editor Content (current file)
    source: Signal<String>,

    // Compilation Result
    rust_code: Signal<Option<String>>,
    compile_error: Signal<Option<String>>,

    // Execution
    console_entries: Signal<Vec<ConsoleEntry>>,
    is_running: Signal<bool>,

    // Layout
    file_tree_width: Signal<f64>,
    editor_width: Signal<f64>,
}

struct FileEntry {
    id: String,
    name: String,
    content: String,
}

struct ConsoleEntry {
    timestamp: String,
    content: String,
    entry_type: ConsoleEntryType,
}

enum ConsoleEntryType {
    Output,
    Error,
    System,
}
```

### 7.2 Persistence

```rust
// Save to localStorage on file change
fn save_files(files: &[FileEntry]) {
    let json = serde_json::to_string(files).unwrap();
    gloo_storage::LocalStorage::set("playground_files", json).unwrap();
}

// Load from localStorage on mount
fn load_files() -> Vec<FileEntry> {
    gloo_storage::LocalStorage::get("playground_files")
        .unwrap_or_else(|_| vec![default_file()])
}

fn default_file() -> FileEntry {
    FileEntry {
        id: uuid(),
        name: "main.lg".into(),
        content: "## Main\n\nLet message be \"Hello, LOGOS!\".\nReturn message.\n".into(),
    }
}
```

---

## 8. Phased Roadmap

### Phase 1: Basic Playground (MVP)

**Goal:** Working code editor with Rust output

- [ ] Add `/playground` route
- [ ] Create `Playground` page with 2-panel layout (Editor | Rust)
- [ ] Wire up `compile_to_rust()` to show generated code
- [ ] Add portal card to home page
- [ ] Basic error display

**Deliverable:** Can write LOGOS, see Rust output, see errors

### Phase 2: Execution Backend

**Goal:** Run code and see output

- [ ] Create `src/server.rs` with Axum
- [ ] Implement `/api/run` endpoint
- [ ] Add Console component
- [ ] Add Run button
- [ ] Wire up execution flow

**Deliverable:** Can click Run and see program output

### Phase 3: File Management

**Goal:** Multiple files with persistence

- [ ] Create FileTree component
- [ ] Implement localStorage persistence
- [ ] Add file CRUD operations
- [ ] Tab bar for multiple files
- [ ] 3-panel layout (FileTree | Editor | Rust)

**Deliverable:** Full file management experience

### Phase 4: Polish

**Goal:** Production-ready experience

- [ ] Keyboard shortcuts (Cmd+S, Cmd+Enter)
- [ ] Resizable panels
- [ ] Loading states and animations
- [ ] Mobile responsive layout
- [ ] Socratic error hints

**Deliverable:** Polished, delightful experience

---

## 9. Styling Guide

### 9.1 Theme Colors (from existing UI)

```css
/* Backgrounds */
--bg-gradient: linear-gradient(135deg, #1a1a2e 0%, #16213e 50%, #0f3460 100%);
--bg-panel: rgba(0, 0, 0, 0.3);
--bg-header: rgba(0, 0, 0, 0.2);

/* Borders */
--border-subtle: rgba(255, 255, 255, 0.08);
--border-accent: rgba(102, 126, 234, 0.5);

/* Text */
--text-primary: #e8e8e8;
--text-secondary: #94a3b8;
--text-muted: #888;

/* Accent */
--accent-primary: #667eea;
--accent-secondary: #764ba2;
--accent-gradient: linear-gradient(135deg, #667eea, #764ba2);

/* Semantic */
--color-success: #4ade80;
--color-error: #f87171;
--color-warning: #fbbf24;
--color-info: #60a5fa;
```

### 9.2 Panel Styling

```css
.playground-panel {
    background: rgba(0, 0, 0, 0.3);
    border-right: 1px solid rgba(255, 255, 255, 0.08);
    display: flex;
    flex-direction: column;
}

.panel-header {
    padding: 12px 16px;
    border-bottom: 1px solid rgba(255, 255, 255, 0.08);
    font-size: 12px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: #888;
}
```

### 9.3 Console Styling

```css
.console-output {
    font-family: 'SF Mono', 'Fira Code', 'Consolas', monospace;
    font-size: 13px;
    line-height: 1.5;
    padding: 12px;
    overflow-y: auto;
}

.console-entry.error {
    color: #f87171;
}

.console-entry.success {
    color: #4ade80;
}
```

---

## 10. Future Considerations

### 10.1 After MVP

| Feature | Priority | Complexity |
|---------|----------|------------|
| Syntax highlighting for LOGOS | High | Medium |
| Autocomplete | High | High |
| Share snippets via URL | Medium | Low |
| Download as Cargo project | Medium | Low |
| Import example programs | Medium | Low |
| Vim/Emacs keybindings | Low | Medium |

### 10.2 Infrastructure

| Consideration | Notes |
|---------------|-------|
| **Server deployment** | Fly.io, Railway, or similar |
| **Execution sandboxing** | Firecracker microVMs for production |
| **Rate limiting** | Prevent abuse of execution endpoint |
| **Caching** | Cache Rust compilation for identical inputs |
| **Telemetry** | Track usage patterns (opt-in) |

### 10.3 Integration with Curriculum

The Playground could integrate with Learn/Curriculum:
- "Try it yourself" links open Playground with pre-loaded code
- Exercises can be attempted in Playground
- Progress syncs between Curriculum and Playground

---

## Appendix A: Example LOGOS Programs

> **All examples below are fully working and tested.**

### Hello World

```markdown
## Main

Let greeting be "Hello, LOGOS!".
Show greeting.
Return.
```
**Output:** `Hello, LOGOS!`

---

### Variables and Math

```markdown
## Main

Let x be 5.
Let y be 10.
Let sum be x plus y.
Let product be x times y.
Show sum.
Show product.
Return sum.
```
**Output:**
```
15
50
```

---

### Type Annotations

```markdown
## Main

Let count: Nat be 0.
Let name: Text be "Alice".
Let numbers: List of Int be [1, 2, 3].
Show name.
Return.
```

---

### Mutable Variables

```markdown
## Main

Let mutable counter be 0.
Set counter to counter plus 1.
Set counter to counter plus 1.
Set counter to counter plus 1.
Show counter.
Return counter.
```
**Output:** `3`

---

### Conditionals

```markdown
## Main

Let x be 42.
If x equals 42:
    Show "The answer to everything!".
Otherwise:
    Show "Just a number.".
Return.
```
**Output:** `The answer to everything!`

---

### Nested Conditionals

```markdown
## Main

Let score be 85.
If score greater than 90:
    Show "A".
Otherwise:
    If score greater than 80:
        Show "B".
    Otherwise:
        Show "C".
Return.
```
**Output:** `B`

---

### While Loops

```markdown
## Main

Let mutable i be 1.
While i less than 6:
    Show i.
    Set i to i plus 1.
Return.
```
**Output:**
```
1
2
3
4
5
```

---

### Repeat Over Collection

```markdown
## Main

Let fruits be ["apple", "banana", "cherry"].
Repeat for fruit in fruits:
    Show fruit.
Return.
```
**Output:**
```
apple
banana
cherry
```

---

### Repeat Over Range

```markdown
## Main

Let mutable sum be 0.
Repeat for i from 1 to 5:
    Set sum to sum plus i.
Show sum.
Return sum.
```
**Output:** `15` (1+2+3+4+5)

---

### List Operations

```markdown
## Main

Let numbers be [10, 20, 30, 40, 50].
Let first be item 1 of numbers.
Let third be item 3 of numbers.
Show first.
Show third.
Return.
```
**Output:**
```
10
30
```
> Note: LOGOS uses 1-based indexing!

---

### Assertions (Debug Checks)

```markdown
## Main

Let age be 25.
Assert that age is greater than 0.
Assert that age is less than 150.
Show "Age is valid!".
Return age.
```
**Output:** `Age is valid!`

---

### FizzBuzz

```markdown
## Main

Repeat for i from 1 to 20:
    If i modulo 15 equals 0:
        Show "FizzBuzz".
    Otherwise:
        If i modulo 3 equals 0:
            Show "Fizz".
        Otherwise:
            If i modulo 5 equals 0:
                Show "Buzz".
            Otherwise:
                Show i.
Return.
```

---

### Sum of List

```markdown
## Main

Let numbers be [1, 2, 3, 4, 5, 6, 7, 8, 9, 10].
Let mutable total be 0.
Repeat for n in numbers:
    Set total to total plus n.
Show "Sum:".
Show total.
Return total.
```
**Output:**
```
Sum:
55
```

---

### Factorial (Iterative)

```markdown
## Main

Let n be 5.
Let mutable result be 1.
Let mutable i be 1.
While i less than n plus 1:
    Set result to result times i.
    Set i to i plus 1.
Show "5! =".
Show result.
Return result.
```
**Output:**
```
5! =
120
```

---

### Basic Function (Procedure)

```markdown
## To greet (name: Text):
    Show name.

## Main
Call greet with "World".
```
**Output:** `World`

---

### Function with Return Value

```markdown
## To double (x: Int):
    Return x + x.

## Main
Let result be double(5).
Show result.
```
**Output:** `10`

---

### Multi-parameter Function

```markdown
## To add (a: Int) and (b: Int):
    Return a + b.

## Main
Let sum be add(3, 4).
Show sum.
```
**Output:** `7`

---

## Appendix B: Keyboard Shortcuts

| Shortcut | Action |
|----------|--------|
| `Cmd/Ctrl + Enter` | Run code |
| `Cmd/Ctrl + S` | Save file |
| `Cmd/Ctrl + N` | New file |
| `Cmd/Ctrl + W` | Close tab |
| `Cmd/Ctrl + P` | Quick file open |
| `Cmd/Ctrl + /` | Toggle comment |
| `Escape` | Close dialogs |

---

*This document is a living specification. Update as implementation progresses.*
