# LOGOS Code Editor / Playground

**Status:** Draft
**Version:** 0.2.0
**Last Updated:** December 2024

---

## 0. Language Implementation Status

> **This section documents what's currently working in the LOGOS programming language.**

### 0.1 Fully Implemented (901+ tests passing, 54 phases)

#### Statements

| Statement | Syntax | Generated Rust | Status |
|-----------|--------|----------------|--------|
| **Let** | `Let x be 5.` | `let x = 5;` | ✅ |
| **Let (typed)** | `Let x: Int be 5.` | `let x: i64 = 5;` | ✅ |
| **Let (mutable)** | `Let mutable x be 5.` | `let mut x = 5;` | ✅ |
| **Set** | `Set x to 10.` | `x = 10;` | ✅ |
| **SetField** | `Set p's x to 10.` | `p.x = 10;` | ✅ |
| **Return** | `Return 42.` | `return 42;` | ✅ |
| **Return (void)** | `Return.` | `return;` | ✅ |
| **If** | `If x equals 5: ...` | `if x == 5 { ... }` | ✅ |
| **If/Otherwise** | `If x equals 5: ... Otherwise: ...` | `if x == 5 { ... } else { ... }` | ✅ |
| **While** | `While x less than 10: ...` | `while x < 10 { ... }` | ✅ |
| **Repeat (collection)** | `Repeat for x in items: ...` | `for x in items { ... }` | ✅ |
| **Repeat (range)** | `Repeat for i from 1 to 10: ...` | `for i in 1..=10 { ... }` | ✅ |
| **Call** | `Call function.` | `function();` | ✅ |
| **Assert** | `Assert that x is greater than 0.` | `debug_assert!(x > 0);` | ✅ |
| **Trust** | `Trust that x > 0 because "positive".` | `debug_assert!(x > 0, "positive");` | ✅ |
| **Show** | `Show x.` | `show(x);` | ✅ |
| **Give** | `Give data to processor.` | Move semantics | ✅ |
| **Function Def** | `## To verb (x: T):` | `fn verb(x: T) { }` | ✅ |
| **Struct Def** | `## Definition` block | `struct Name { ... }` | ✅ |
| **Inspect** | `Inspect x: If it is a Variant: ...` | `match x { ... }` | ✅ |
| **Push** | `Push x to items.` | `items.push(x);` | ✅ |
| **Pop** | `Pop from items.` / `Pop from items into y.` | `items.pop();` / `let y = items.pop();` | ✅ |

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
| **Less/Greater or equal** | `x <= 5`, `x >= 5` | `(x <= 5)`, `(x >= 5)` | ✅ |
| **Logical And/Or** | `x and y`, `x or y` | `(x && y)`, `(x \|\| y)` | ✅ |
| **List literals** | `[1, 2, 3]` | `vec![1, 2, 3]` | ✅ |
| **Empty list** | `[]` | `vec![]` | ✅ |
| **Indexing** | `item 1 of list` | `list[0]` (1→0 indexed) | ✅ |
| **Dynamic Index** | `items at i` | `items[i - 1]` | ✅ |
| **Slice** | `items 1 through 3 of list` | `list[0..3].to_vec()` | ✅ |
| **Length** | `length of items` | `items.len()` | ✅ |
| **Copy** | `copy of items` | `items.clone()` | ✅ |
| **Ranges** | `1 to 10` | `1..=10` | ✅ |
| **Field Access** | `p's x` / `the x of p` | `p.x` | ✅ |
| **Constructor** | `a new Point` | `Point::default()` | ✅ |
| **Generic Constructor** | `a new Box of Int` | `Box::<i64>::default()` | ✅ |
| **Variant Constructor** | `a new Circle with radius 10` | `Shape::Circle { radius: 10 }` | ✅ |
| **Function Call** | `func(x, y)` | `func(x, y)` | ✅ |

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

#### Type System Features

| Feature | Syntax | Status |
|---------|--------|--------|
| **Struct definitions** | `## Definition` block with fields | ✅ |
| **Enum definitions** | `A Shape is either: A Circle. A Point.` | ✅ |
| **Payload variants** | `A Circle with a radius, which is Int.` | ✅ |
| **Concise variants** | `A Success (value: Int).` | ✅ |
| **Generic structs** | `A Box of T with a value, which is T.` | ✅ |
| **Pattern matching** | `Inspect x: If it is a Circle (radius): ...` | ✅ |
| **Otherwise clause** | `Otherwise: ...` in Inspect | ✅ |
| **Refinement types** | `Int where it > 0` (AST ready) | ⚠️ |

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
| **Refinement types** | AST variant exists | Parsing of `where` clauses |
| **Give/Show ownership** | Parser and AST | Full codegen for ownership transfer |

### 0.3 Not Yet Implemented

| Feature | Notes |
|---------|-------|
| String interpolation | `"Hello, {name}!"` |
| Concurrency | Spawn, channels, agents |
| Higher-order functions | Lambdas, closures |
| Traits/Interfaces | Polymorphism |

---

## 1. Vision & Goals

### 1.1 What We're Building

A web-based IDE for writing, compiling, and running LOGOS programs. Think "REPL meets IDE" - simple enough to experiment quickly, powerful enough for real development.

### 1.2 Core Principles

| Principle | Description |
|-----------|-------------|
| **Immediate Feedback** | See generated Rust code as you type |
| **Run Anywhere** | Execute code in-browser via WASM |
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

### 2.3 Data Flow (WASM Sandbox)

```
User Types → Editor → compile_to_rust() → Rust Panel
                                       ↓
User Clicks Run → WASM Interpreter → In-browser execution → Console
```

**Key insight:** No backend server required for code execution. The entire pipeline runs in the browser:

1. LOGOS source → Rust code (already works via `compile_to_rust()`)
2. Rust code → Interpreted/executed in WASM sandbox
3. Output captured and displayed in Console

**Benefits:**
- No server costs for execution
- Instant feedback (no network latency)
- Works offline
- Matches existing Dioxus WASM stack

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

---

## 4. Component Breakdown

### 4.1 Existing Components to Reuse

| Component | File | Usage |
|-----------|------|-------|
| **LiveEditor** | `src/ui/components/editor.rs` | CodeMirror FFI binding for code input |
| **LogicOutput** | `src/ui/components/logic_output.rs` | Syntax-highlighted output display |
| **SocraticGuide** | `src/ui/components/socratic_guide.rs` | Error hints and guidance |

### 4.2 New Components to Create

| Component | File | Description |
|-----------|------|-------------|
| **Playground** | `src/ui/pages/playground.rs` | Main page component |
| **FileTree** | `src/ui/components/file_tree.rs` | File explorer sidebar |
| **RustOutput** | `src/ui/components/rust_output.rs` | Rust code display (similar to LogicOutput) |
| **Console** | `src/ui/components/console.rs` | Execution output |
| **RunButton** | `src/ui/components/run_button.rs` | Execute button with loading state |

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
│   │   └── LiveEditor (existing)
│   └── RustPanel
│       ├── RustOutput (new)
│       └── CompileStatus
├── ConsolePanel
│   ├── Console (new)
│   └── RunButton (new)
└── Footer (optional)
    └── SocraticGuide (existing)
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
```

### 5.2 Files to Modify

| File | Changes |
|------|---------|
| `src/ui/router.rs` | Add `#[route("/playground")] Playground {}` |
| `src/ui/pages/mod.rs` | Add `pub mod playground; pub use playground::Playground;` |
| `src/ui/pages/home.rs` | Add portal card linking to Playground |
| `src/ui/components/mod.rs` | Export new components |

### 5.3 Dioxus 0.6 Patterns (Match Existing Style)

```rust
use dioxus::prelude::*;

const PLAYGROUND_STYLE: &str = r#"
    .playground-container {
        display: flex;
        height: 100vh;
        background: linear-gradient(135deg, #1a1a2e 0%, #16213e 50%, #0f3460 100%);
    }
    /* ... */
"#;

#[component]
pub fn Playground() -> Element {
    let source = use_signal(|| String::from("## Main\n\nLet x be 5.\nReturn x.\n"));
    let rust_code = use_signal(|| None::<String>);
    let console_output = use_signal(|| Vec::<ConsoleEntry>::new());
    let is_running = use_signal(|| false);

    // Live compilation on source change
    use_effect(move || {
        let src = source.read().clone();
        match logos::compile_to_rust(&src) {
            Ok(code) => rust_code.set(Some(code)),
            Err(e) => rust_code.set(None),
        }
    });

    rsx! {
        style { "{PLAYGROUND_STYLE}" }
        div { class: "playground-container",
            // ... components
        }
    }
}
```

### 5.4 Route Definition

```rust
// src/ui/router.rs
#[derive(Clone, Routable, Debug, PartialEq)]
pub enum Route {
    // ... existing routes ...

    #[route("/playground")]
    Playground {},
}
```

---

## 6. Execution Architecture (WASM Sandbox)

### 6.1 Approach

Since the entire LOGOS compiler already runs in WASM (via Dioxus), execution can happen client-side. Options:

| Approach | Pros | Cons |
|----------|------|------|
| **Interpret Rust AST** | Simpler, no external deps | Limited stdlib support |
| **WASM-based Rust interpreter** | Full Rust support | Complex, large binary |
| **Compile to JS** | Native browser execution | Transpilation complexity |

**Recommended:** Start with a lightweight interpreter for the generated Rust subset:
- Variable bindings
- Arithmetic
- Conditionals and loops
- `show()` / `println()` captured to console

### 6.2 Execution Flow

```rust
// src/ui/components/run_button.rs
async fn execute_code(rust_code: &str) -> Vec<ConsoleEntry> {
    let mut output = Vec::new();

    // Parse generated Rust (subset)
    // Execute with captured stdout
    // Return console entries

    output
}
```

### 6.3 Future: Full Rust Execution

For complete Rust support, consider:
- WebAssembly compilation of generated code
- Playground API service (Cloudflare Worker)
- Integration with Rust Playground API

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

### 7.2 Persistence (localStorage)

```rust
use gloo_storage::LocalStorage;

fn save_files(files: &[FileEntry]) {
    let json = serde_json::to_string(files).unwrap();
    LocalStorage::set("playground_files", json).unwrap();
}

fn load_files() -> Vec<FileEntry> {
    LocalStorage::get("playground_files")
        .unwrap_or_else(|_| vec![default_file()])
}

fn default_file() -> FileEntry {
    FileEntry {
        id: uuid(),
        name: "main.lg".into(),
        content: "## Main\n\nLet message be \"Hello, LOGOS!\".\nShow message.\nReturn.\n".into(),
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

### Phase 2: In-Browser Execution

**Goal:** Run code and see output (no backend)

- [ ] Create lightweight Rust interpreter for generated code subset
- [ ] Add Console component
- [ ] Add Run button
- [ ] Capture `show()`/`println()` output

**Deliverable:** Can click Run and see program output in browser

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

### Deployment

Uses existing Cloudflare Pages infrastructure:

```yaml
# .github/workflows/deploy-frontend.yml pattern
- dx build --release
- wrangler pages deploy target/dx/logos/release/web/public --project-name=logicaffeine
```

---

## 9. Styling Guide

### 9.1 Theme Colors (from `src/ui/app.rs`)

```css
/* Backgrounds */
--bg-gradient: linear-gradient(135deg, #1a1a2e 0%, #16213e 50%, #0f3460 100%);
--bg-panel: rgba(0, 0, 0, 0.3);
--bg-input: rgba(255, 255, 255, 0.08);

/* Borders */
--border-subtle: rgba(255, 255, 255, 0.1);
--border-accent: rgba(102, 126, 234, 0.5);

/* Text */
--text-primary: #e8e8e8;
--text-secondary: #94a3b8;
--text-muted: #666;

/* Accent */
--accent-primary: #667eea;
--accent-secondary: #764ba2;
--accent-gradient: linear-gradient(135deg, #667eea, #764ba2);
--accent-cyan: #00d4ff;

/* Semantic */
--color-success: #4ade80;
--color-error: #ff6b6b;
--color-warning: #fbbf24;
--color-info: #60a5fa;
```

### 9.2 Panel Styling

```css
.playground-panel {
    background: rgba(0, 0, 0, 0.3);
    border-right: 1px solid rgba(255, 255, 255, 0.1);
    display: flex;
    flex-direction: column;
}

.panel-header {
    padding: 12px 16px;
    border-bottom: 1px solid rgba(255, 255, 255, 0.1);
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
    color: #ff6b6b;
}

.console-entry.success {
    color: #4ade80;
}

.console-entry.system {
    color: #00d4ff;
    text-shadow: 0 0 20px rgba(0, 212, 255, 0.3);
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

### 10.2 Integration with Curriculum

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

### Collection Operations (Phase 43)

```markdown
## Main

Let mutable items be [].
Push 1 to items.
Push 2 to items.
Push 3 to items.
Let n be length of items.
Show n.
Pop from items into last.
Show last.
Return.
```
**Output:**
```
3
3
```

---

### Struct Definition and Field Access

```markdown
## Definition

A Point has:
    An x, which is Int.
    A y, which is Int.

## Main

Let p be a new Point.
Set p's x to 10.
Set p's y to 20.
Show p's x.
Show p's y.
Return.
```
**Output:**
```
10
20
```

---

### Enum Definition and Pattern Matching

```markdown
## Definition

A Shape is either:
    A Circle with a radius, which is Int.
    A Rectangle with a width, which is Int, and a height, which is Int.
    A Point.

## Main

Let s be a new Circle with radius 5.
Inspect s:
    If it is a Circle (radius: r):
        Show r.
    If it is a Rectangle (width: w, height: h):
        Show w times h.
    If it is a Point:
        Show "point".
Return.
```
**Output:** `5`

---

### Pattern Matching with Otherwise

```markdown
## Definition

A Color is either:
    A Red.
    A Green.
    A Blue.

## Main

Let c be a new Green.
Inspect c:
    If it is a Red:
        Show "red".
    Otherwise:
        Show "not red".
Return.
```
**Output:** `not red`

---

### Trust Statements (Documented Assertions)

```markdown
## Main

Let age be 25.
Trust that age is greater than 0 because "Age must be positive".
Trust that age is less than 150 because "Human lifespan limit".
Show "Age is valid!".
Return age.
```
**Output:** `Age is valid!`

---

### Function Definition

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
    Return x plus x.

## Main

Let result be double(5).
Show result.
```
**Output:** `10`

---

### Multi-parameter Function

```markdown
## To add (a: Int) and (b: Int):
    Return a plus b.

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
