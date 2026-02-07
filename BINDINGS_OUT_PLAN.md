# Bindings Out: Expression-Level Escape Hatches

Design document for returning values from `Escape to Rust:` blocks back into Logos scope.

---

## 1. The Problem

Escape hatches today are one-directional: Logos variables flow *into* escape blocks, but nothing flows back. Variables defined inside `{ }` braces are trapped by scope hygiene. This makes escape hatches statement-only — useful for side effects (printing, I/O, mutation of pre-existing mutable variables) but unable to *produce* new values.

### 1a. The Trap

```logos
## Main
Escape to Rust:
    let result = (10_i64).pow(3);
Show result.
```

This fails at the Rust level. Codegen wraps the escape in braces:

```rust
fn main() {
    {
        let result = (10_i64).pow(3);
    }
    println!("{}", result);  // ERROR: `result` not found in this scope
}
```

The brace hygiene that protects Logos from escape block pollution also prevents escape blocks from introducing bindings.

### 1b. The Mutable Workaround

```logos
## Main
Let mut result be 0.
Escape to Rust:
    result = (10_i64).pow(3);
Show result.
```

This works — `result` is declared in Logos scope, so it's visible both inside and after the escape block. But it's clunky:

1. You must pre-declare a mutable variable with a throwaway initial value
2. You must know the right "zero value" for the type (`0`, `""`, `false`, `Vec::new()`, ...)
3. It reads as mutation when the intent is initialization
4. For struct types or complex values, the throwaway initial value may be expensive or impossible to construct

### 1c. The `return` Workaround (Functions Only)

```logos
## To cube (n: Int) -> Int:
    Escape to Rust:
        return n * n * n;

## Main
Show cube(5).
```

This works because `return` in Rust returns from the enclosing *function*, jumping over the brace boundary entirely. But it only works in functions, not in `Main`, and it couples "compute a value" with "exit the function" — you can't compute a value via escape and then do more work afterwards.

### 1d. The FRIEND_PLANS.md `compress` Example

From the spec:

```logos
## To compress (data: Seq of Int) -> Seq of Int:
    Let header be [1, 0, 0].

    Escape to Rust:
        use flate2::write::ZlibEncoder;
        use flate2::Compression;
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&data.as_bytes()).unwrap();
        encoder.finish().unwrap()

    Push header to result.
    Return result.
```

Where does `result` come from? It's never declared with `Let`. The spec implies the escape block's last expression (`encoder.finish().unwrap()`) becomes bound to `result` somehow. But the mechanism isn't specified — is `result` a magic variable? Is it declared by the block? The spec's AST includes `bindings_out: Option<Symbol>` which suggests a named output binding, but the surface syntax doesn't show how the user requests it.

This ambiguity is what this document resolves.

---

## 2. The Spec

### 2a. Verbatim from FRIEND_PLANS.md

**AST** (lines 56–63):

```rust
Escape {
    language: Symbol,    // "Rust" for now, could support others later
    bindings_in: Vec<Symbol>,   // LOGOS variables available inside the block
    bindings_out: Option<Symbol>, // optional return value
    code: String,        // raw foreign code
    span: Span,
}
```

**CodeGen** (line 68):

> Emit the raw string directly into the generated Rust function. Wrap it in a block `{ ... }` with variable bindings from the enclosing scope. The last expression in the block becomes the return value if the escape is used in expression position.

**Analysis** (line 70):

> Escape blocks are opaque to ownership and escape analysis. Conservative approach: any variable referenced in the escape block is considered consumed (moved).

### 2b. Analysis

The spec envisions two fields we haven't implemented:

1. **`bindings_in: Vec<Symbol>`** — tracks which Logos variables are referenced inside the block. We kept this implicit (all in-scope variables are available via Rust scoping rules) and this decision is correct. Making it explicit would be noisy for users and provide no safety benefit — rustc already catches use of undeclared variables.

2. **`bindings_out: Option<Symbol>`** — the escape block optionally produces a value bound to a named variable. The spec's `compress` example shows this in action but doesn't pin down the surface syntax. That's what we need to design.

The key phrase is "if the escape is used in **expression position**." This tells us the spec envisions escape-as-expression, not just escape-as-statement. The question is how to surface it.

---

## 3. Design Space

Each option is analyzed on five axes:

| Axis | Meaning |
|------|---------|
| **Purity** | Does it extend existing semantics or add new ones? |
| **Ergonomics** | How does it read in English? |
| **Composability** | Can it be used everywhere expressions appear? |
| **Parser Complexity** | How hard to implement? |
| **Consistency** | Does it follow patterns already in the language? |

### Option A: Escape Becomes an Expression

The parser recognizes `Escape to Rust:` in expression position (anywhere an expression is expected).

```logos
Let x: Int be Escape to Rust:
    let a = 10_i64;
    let b = 20_i64;
    a + b
```

Generated Rust:

```rust
let x: i64 = {
    let a = 10_i64;
    let b = 20_i64;
    a + b
};
```

The type annotation on `Let` is required — Logos can't infer types through opaque Rust code. But this is consistent with how Logos already works: `Let x: Int be 5.` puts the type on the variable, not on the value.

| Axis | Rating | Rationale |
|------|--------|-----------|
| Purity | **Highest** | Removes an artificial restriction rather than adding syntax. The escape block already generates `{ code }` which is a valid Rust block expression. We're just letting the parser accept it where expressions are expected. |
| Ergonomics | **Excellent** | `Let x: Type be <expr>` is a pattern every Logos user already knows. Combining it with `Escape to Rust:` is obvious. |
| Composability | **Excellent** | Works in `Let`, `Set`, `Return`, function arguments — anywhere expressions go. |
| Parser Complexity | **Low-Medium** | Handle `TokenType::Escape` in `parse_primary_expr()`. The indented block is already lexed as `Indent EscapeBlock Dedent`. The period issue (see below) needs care. |
| Consistency | **High** | Follows the `Let x: Type be <compound-expr>` pattern. Block-terminated like `If`/`While`, not period-terminated like `Let x be 5.` |

**The Period Question.** Logos statements end with periods. `Let x be 5.` But escape blocks end with dedent, not periods. Today this is fine because `Escape to Rust:` is a top-level statement and the statement loop at `parser/mod.rs:1108` *optionally* consumes a period after each statement. For expression-level escape in `Let`, the same approach works: `parse_let_statement()` calls `parse_imperative_expr()` which would return the escape expression, and the statement loop optionally consumes the period. No period needed after an indented block — the dedent signals the end unambiguously.

### Option B: Statement-Level with Named Binding

The escape statement itself declares a variable as its output.

```logos
Escape to Rust as result: Int:
    let a = 10_i64;
    let b = 20_i64;
    a + b
Show result.
```

Generated Rust:

```rust
let result: i64 = {
    let a = 10_i64;
    let b = 20_i64;
    a + b
};
println!("{}", result);
```

This maps directly to FRIEND_PLANS.md's `bindings_out: Option<Symbol>` — the symbol is `result`, declared inline.

| Axis | Rating | Rationale |
|------|--------|-----------|
| Purity | Medium | Extends an existing statement with new fields. Doesn't add new syntax categories. |
| Ergonomics | OK | Reads well enough: "escape to Rust as result (an Int)." But introduces a **second** way to declare variables alongside `Let`. Two paths to variable introduction is a language design smell. |
| Composability | **None** | It's a statement. Can't use it in `Set`, can't nest it, can't pass it as a function argument, can't use it in `Return`. |
| Parser Complexity | **Low** | Extend `parse_escape_statement()` to optionally consume `as <name>: <type>` before the colon. |
| Consistency | **Low** | Duplicates `Let`'s job. Now there are two places where variables are born: `Let x be ...` and `Escape ... as x: Type:`. |

### Option C: "Giving" Keyword

A new keyword marks that the escape block produces a value and specifies its type.

```logos
Let x be Escape to Rust giving Int:
    let a = 10_i64;
    let b = 20_i64;
    a + b
```

| Axis | Rating | Rationale |
|------|--------|-----------|
| Purity | **Low** | New keyword (`giving`) that appears nowhere else in the language. |
| Ergonomics | Nice | Reads well in English: "escape to Rust, giving back an Int." But verbose. |
| Composability | Same as A | If implemented as expression, works everywhere expressions go. |
| Parser Complexity | Low | New keyword after "Rust" before ":". |
| Consistency | **Low** | `giving` is unique — no other expression uses it. Type annotation is on the escape mechanism rather than the variable, which is conceptually backwards. The *variable* has the type, not the escape. |

### Option D: Implicit `result` Variable

The escape block magically introduces a variable named `result` bound to its last expression.

```logos
Escape to Rust:
    let a = 10_i64;
    let b = 20_i64;
    a + b
Show result.
```

This matches the FRIEND_PLANS.md `compress` example literally — `result` appears after the block without being declared.

| Axis | Rating | Rationale |
|------|--------|-----------|
| Purity | **Lowest** | Magic variable name materializing from nowhere. |
| Ergonomics | Minimal typing | But maximum confusion. Where does `result` come from? What if I already have a variable called `result`? What type is it? |
| Composability | **None** | Statement-only. The value is only accessible via the magic name. |
| Parser Complexity | Low | Minimal parser changes but high semantic complexity — need to type-infer or annotate the invisible variable. |
| Consistency | **Lowest** | Breaks the fundamental principle that variables are always introduced by `Let`. |

### Can Multiple Options Coexist?

Yes. Statement-level escape (side effects, no binding) and expression-level escape (value capture) are dispatched from different parse contexts. The parser already knows whether it's in statement position (`parse_statement()`) or expression position (`parse_imperative_expr()` → `parse_primary_expr()`). Today's `Escape to Rust:` statement continues to work unchanged for fire-and-forget blocks. Expression-level `Escape to Rust:` is simply a new entry in `parse_primary_expr()`. They never conflict.

---

## 4. Recommendation: Option A

**Option A is the right choice because it's not adding a feature — it's removing a restriction.**

The escape block already generates `{ code }` which is a valid Rust block expression. The only reason it can't return values today is that the parser won't accept `Escape` in expression position. Option A fixes this by recognizing `TokenType::Escape` in `parse_primary_expr()` — the exact same entry point that handles `a new Point`, function calls, and other compound expressions. The generated code doesn't even change shape:

| Statement escape | Expression escape |
|---|---|
| `{ code; }` | `let x: T = { code };` |

The braces are already there. We're just assigning them to something.

### Why Not the Others

**Option B** (statement-level `as name: Type`) introduces a second variable declaration mechanism. This is a real cost — every user must now learn that there are two ways to introduce variables, and know when to use which. For a language that prizes readability and consistency, this is the wrong trade-off. Additionally, statement-level binding can't compose. You can't write `Return Escape to Rust: ...` with Option B.

**Option C** (`giving`) adds a keyword that exists solely for this one feature. Keywords are expensive — they occupy cognitive space, they collide with identifiers, and they set precedents for future features. The type annotation also ends up in the wrong place (on the escape mechanism, not the variable).

**Option D** (implicit `result`) is the most dangerous. Magic variables are the opposite of Logos's design philosophy. Every variable should be traceable to a `Let` declaration. The `compress` example in FRIEND_PLANS.md reads naturally in a spec document but would be deeply confusing in real code.

### The Type Annotation Requirement

Expression-level escape requires a type annotation on the `Let`:

```logos
Let x: Int be Escape to Rust:
    42_i64
```

This is non-negotiable. Logos can't look inside opaque Rust code to infer types. But this isn't a burden — it's *correct*. The type annotation tells both the Logos compiler and the human reader what `x` is. It's the same pattern as `Let x: Int be 5.` and `Let items: Seq of Int be [1, 2, 3].`

Omitting the type annotation is an error:

```logos
Let x be Escape to Rust:
    42_i64
```

This should fail with a clear message: "Escape expressions require a type annotation. Write `Let x: Int be Escape to Rust:` so Logos knows the type of `x`."

### The `return` Footgun

Inside an expression-level escape block, Rust's `return` returns from the **enclosing function**, not from the block. This is standard Rust behavior but worth documenting explicitly:

```logos
## To compute (n: Int) -> Int:
    Let x: Int be Escape to Rust:
        if n < 0 { return -1; }  // returns from compute(), NOT from the block
        n * n                     // this is the block's value when n >= 0
```

Both patterns are useful, but they serve different purposes:
- **Last expression** (no semicolon): the block's value, assigned to the variable
- **`return`**: early exit from the enclosing function, skipping the assignment entirely

---

## 5. Implementation Sketch

Seven files to touch — the same set as the original escape implementation.

### 5a. AST (`crates/logicaffeine_language/src/ast/stmt.rs`)

Add an `Escape` variant to `Expr`:

```rust
/// Escape hatch expression: embed raw foreign code that produces a value.
/// `Escape to Rust:` followed by an indented block whose last expression
/// becomes the value. Used in expression position: `Let x: Int be Escape to Rust:`
Escape {
    /// Target language ("Rust" for now).
    language: Symbol,
    /// Raw foreign code, captured verbatim with base indentation stripped.
    code: Symbol,
},
```

Note: no `span` field needed on the expression — spans are tracked at the statement level. This matches other expression variants like `BinaryOp`, `Call`, `Copy`, etc.

The existing `Stmt::Escape` remains unchanged — it handles statement-level (side-effect) escape blocks.

### 5b. Parser — Expression Dispatch (`crates/logicaffeine_language/src/parser/mod.rs`)

Add a branch in `parse_primary_expr()` (currently at line 4464):

```rust
TokenType::Escape => {
    return self.parse_escape_expr();
}
```

The new `parse_escape_expr()` function mirrors `parse_escape_statement()` but returns an `Expr` instead of a `Stmt`:

```rust
fn parse_escape_expr(&mut self) -> ParseResult<&'a Expr<'a>> {
    self.advance(); // consume "Escape"

    // Expect "to"
    // ... (same validation as parse_escape_statement)
    self.advance(); // consume "to"

    // Parse and validate language name ("Rust")
    // ... (same as parse_escape_statement)

    // Expect colon
    // ... (same as parse_escape_statement)
    self.advance(); // consume ":"

    // Expect Indent
    // ... (same as parse_escape_statement)
    self.advance(); // consume Indent

    // Expect EscapeBlock token
    let code = match &self.peek().kind {
        TokenType::EscapeBlock(sym) => { let s = *sym; self.advance(); s }
        _ => return Err(/* ... */),
    };

    // Expect Dedent
    if self.check(&TokenType::Dedent) { self.advance(); }

    Ok(self.ctx.alloc_imperative_expr(Expr::Escape { language, code }))
}
```

Common validation logic between `parse_escape_statement()` and `parse_escape_expr()` can be extracted into a shared helper, or left duplicated — it's only ~30 lines of straightforward token matching.

### 5c. Parser — Period Handling

The statement loop in `parse_body()` (line 1108) already optionally consumes a period:

```rust
let stmt = self.parse_statement()?;
statements.push(stmt);

if self.check(&TokenType::Period) {
    self.advance();
}
```

When `parse_let_statement()` parses a `Let x: Int be Escape to Rust: ...` expression, the escape block consumes through the `Dedent` token. The trailing period is not present (escape blocks end with dedent, not period), and the `if self.check(&TokenType::Period)` simply doesn't fire. This already works correctly — no changes needed.

### 5d. Codegen (`crates/logicaffeine_compile/src/codegen.rs`)

Add expression-level codegen for `Expr::Escape`. The expression form generates a `{ code }` block *as a string* (since expressions produce strings in the codegen, not direct writes):

```rust
Expr::Escape { language: _, code } => {
    let raw_code = interner.resolve(*code);
    let mut block = String::from("{\n");
    for line in raw_code.lines() {
        block.push_str("    ");
        block.push_str(line);
        block.push('\n');
    }
    block.push('}');
    block
}
```

When used in `Let x: Int be Escape to Rust:`, the codegen produces:

```rust
let x: i64 = {
    let a = 10_i64;
    let b = 20_i64;
    a + b
};
```

The `= { ... }` assignment is syntactically valid Rust. The semicolon after `}` comes from the Let statement's codegen, not from the escape expression.

### 5e. Escape Analysis (`crates/logicaffeine_compile/src/analysis/escape.rs`)

Add a match arm for `Expr::Escape`:

```rust
Expr::Escape { .. } => {
    // Opaque — the Rust compiler handles zone safety for raw code
}
```

Same treatment as `Stmt::Escape`: the block is opaque and defers to rustc.

### 5f. Ownership Analysis (`crates/logicaffeine_compile/src/analysis/ownership.rs`)

Add a match arm for `Expr::Escape`:

```rust
Expr::Escape { .. } => {
    // Opaque to ownership analysis — rustc catches use-after-move
}
```

Same treatment as `Stmt::Escape`.

### 5g. Interpreter (`crates/logicaffeine_compile/src/interpreter.rs`)

Add a match arm for `Expr::Escape`:

```rust
Expr::Escape { .. } => {
    Err("Escape expressions contain raw Rust code and cannot be interpreted. \
         Use `largo build` or `largo run` to compile and run this program.".to_string())
}
```

Same rejection as `Stmt::Escape`.

---

## 6. Test Matrix

### Group A: The Problem (codegen-only, demonstrating the gap)

**A1: Variable trapped in braces**

```rust
#[test]
fn escape_expr_problem_variable_trapped_in_braces() {
    // This demonstrates WHY expression-level escape is needed.
    // A variable defined inside an escape block can't be used after it.
    let source = r#"## Main
Escape to Rust:
    let result = 42_i64;
Show result.
"#;
    let result = compile_logos(source);
    assert!(!result.success, "Should fail: `result` is trapped inside {{ }} braces");
}
```

**A2: The mutable workaround works but is clunky**

```rust
#[test]
fn escape_expr_workaround_mutable_preallocation() {
    assert_exact_output(
        r#"## Main
Let mut result be 0.
Escape to Rust:
    result = (10_i64).pow(3);
Show result.
"#,
        "1000",
    );
}
```

### Group B: The Solution (E2E, demonstrating the feature)

**B1: Basic integer**

```rust
#[test]
fn e2e_escape_expr_basic_int() {
    assert_exact_output(
        r#"## Main
Let x: Int be Escape to Rust:
    42_i64
Show x.
"#,
        "42",
    );
}
```

**B2: Multi-step computation**

```rust
#[test]
fn e2e_escape_expr_multi_step() {
    assert_exact_output(
        r#"## Main
Let answer: Int be Escape to Rust:
    let a = 10_i64;
    let b = 32_i64;
    a + b
Show answer.
"#,
        "42",
    );
}
```

**B3: Text value**

```rust
#[test]
fn e2e_escape_expr_text() {
    assert_exact_output(
        r#"## Main
Let msg: Text be Escape to Rust:
    format!("hello {}", "world")
Show msg.
"#,
        "hello world",
    );
}
```

**B4: Struct construction**

```rust
#[test]
fn e2e_escape_expr_struct() {
    assert_exact_output(
        r#"## A Point has:
    An x: Int.
    A y: Int.

## Main
Let p: Point be Escape to Rust:
    Point { x: 10, y: 20 }
Show p's x plus p's y.
"#,
        "30",
    );
}
```

**B5: With Logos variable access (bindings_in)**

```rust
#[test]
fn e2e_escape_expr_accesses_logos_variable() {
    assert_exact_output(
        r#"## Main
Let items be [10, 20, 30].
Let total: Int be Escape to Rust:
    items.iter().sum::<i64>()
Show total.
"#,
        "60",
    );
}
```

**B6: In function body**

```rust
#[test]
fn e2e_escape_expr_in_function() {
    assert_exact_output(
        r#"## To isqrt (n: Int) -> Int:
    Let result: Int be Escape to Rust:
        (n as f64).sqrt() as i64
    Return result.

## Main
Show isqrt(49).
"#,
        "7",
    );
}
```

**B7: Set with expression escape**

```rust
#[test]
fn e2e_escape_expr_in_set() {
    assert_exact_output(
        r#"## Main
Let mut x: Int be 0.
Set x to Escape to Rust:
    42 * 2
Show x.
"#,
        "84",
    );
}
```

### Group C: Edge Cases

**C1: Missing type annotation produces an error**

```rust
#[test]
fn escape_expr_missing_type_annotation_error() {
    let source = r#"## Main
Let x be Escape to Rust:
    42_i64
Show x.
"#;
    let result = compile_to_rust(source);
    assert!(result.is_err(), "Should require type annotation for escape expressions");
}
```

**C2: Expression escape in Otherwise branch**

```rust
#[test]
fn e2e_escape_expr_in_otherwise() {
    assert_exact_output(
        r#"## Main
Let x be 0.
Let label: Text be Escape to Rust:
    if x > 0 { "positive".to_string() } else { "non-positive".to_string() }
Show label.
"#,
        "non-positive",
    );
}
```

**C3: Return inside expression escape returns from function**

```rust
#[test]
fn e2e_escape_expr_return_exits_function() {
    assert_exact_output(
        r#"## To safe_div (a: Int, b: Int) -> Int:
    Let result: Int be Escape to Rust:
        if b == 0 { return -1; }
        a / b
    Return result.

## Main
Show safe_div(10, 0).
"#,
        "-1",
    );
}
```

---

## 7. What We're NOT Doing (and Why)

### `bindings_in` — Keep Implicit

The FRIEND_PLANS.md spec includes `bindings_in: Vec<Symbol>` to track which Logos variables are available inside the escape block. We keep this implicit: all in-scope variables are available because Rust scoping handles it. The escape block's `{ }` braces are emitted inside the function body where all local variables are in scope. Explicit `bindings_in` would be noisy (`Escape to Rust with x and y and z:`) and provide no safety benefit — rustc already catches references to undeclared variables.

If we ever need `bindings_in` for correctness (e.g., cross-compiling to a language without lexical scoping), it can be added as `Escape to Rust with x and y:` without conflicting with expression-level escape.

### Conservative Ownership — Keep Current Opaque Approach

The FRIEND_PLANS.md spec says "any variable referenced in the escape block is considered consumed (moved)." Our current implementation is more permissive: ownership analysis simply skips escape blocks (`Stmt::Escape { .. } => {}`), deferring entirely to rustc. This is the right choice today:

1. **Ergonomic**: Users don't get false "variable moved" errors from Logos when rustc would accept the code
2. **Correct**: Any actual use-after-move is caught by rustc during compilation
3. **Simple**: No need to parse Rust code to determine which variables are referenced

The conservative approach would reject valid programs like:

```logos
Let items be [1, 2, 3].
Escape to Rust:
    println!("{}", items.len());  // borrows, does NOT move
Show items.  // Logos would reject this under conservative analysis
```

Our opaque approach correctly compiles this — `items.len()` only borrows, and rustc allows the subsequent use.

### Top-Level Escape (Outside `{ }`)

We're not adding escape blocks that bypass brace hygiene (e.g., injecting code directly into the function without wrapping `{ }`). The braces serve a critical purpose: they prevent escape code from shadowing Logos variables and prevent Logos code from depending on escape-internal variables (the exact problem that expression-level escape solves cleanly). Removing the braces would create a bidirectional pollution channel that's impossible to reason about.
