# Logicaffeine 1.0: English-to-Logic Transpiler Implementation Plan

## Overview

Create a standalone Rust tool at `/logos/` that transpiles natural English sentences into formal logical notation (Logicaffeine WFFs - Well-Formed Formulas).

**Architecture**: Compiler Pipeline
- **Lexer** → Token Stream
- **Parser** → Abstract Syntax Tree (AST)
- **Transpiler** → Logicaffeine Notation (LaTeX/Gensler format)

---

## Project Structure

```
/logos/
├── Cargo.toml           # Standalone workspace (like glb-generator)
├── src/
│   ├── main.rs          # CLI entry point and REPL
│   ├── lib.rs           # Public API exports
│   ├── lexer.rs         # Tokenization layer
│   ├── token.rs         # Token types and definitions
│   ├── parser.rs        # Recursive descent parser
│   ├── ast.rs           # AST node definitions
│   └── transpile.rs     # Logicaffeine code generation
├── generate-docs.sh     # Documentation generator
└── IMPLEMENTATION_PLAN.md
```

---

## Implementation Steps

### Step 1: Create Project Skeleton

Create the `/logos/` directory with standalone `Cargo.toml`:

```toml
[package]
name = "logos"
version = "0.1.0"
edition = "2021"
authors = ["Logicaffeine Team"]
license = "BSL"
description = "English-to-Logic Transpiler targeting Logicaffeine notation"

[workspace]

[dependencies]
# Minimal dependencies for the MVP
```

### Step 2: Implement Token Types (`token.rs`)

Define the atomic units of the logical language:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenType {
    // Quantifiers
    All, No, Some,

    // Connectives
    And, Or, If, Then, Not,

    // Modals (Alethic)
    Necessary, Possible,

    // Content Words
    Noun(String),
    Adjective(String),
    Verb(String),
    ProperName(String),

    // Functional Words
    Is, Are, That,

    // Punctuation
    LParen, RParen, Comma, Period,
    EOF,
}

pub struct Token {
    pub kind: TokenType,
    pub lexeme: String,
}
```

### Step 3: Implement Lexer (`lexer.rs`)

Build the tokenizer with a dictionary-based approach:

**Core responsibilities:**
- Skip whitespace
- Handle punctuation
- Classify words via dictionary lookup
- Apply heuristic fallbacks for unknown words:
  - Capitalized → ProperName
  - Ends in "s"/"ing" → Verb
  - Ends in "ian"/"er" → Noun
  - Default → Adjective
- **Strict mode**: Panic with clear error message if word classification fails completely

**Dictionary entries:**
- Quantifiers: "all", "every", "no", "some"
- Connectives: "and", "or", "if", "then", "not"
- Copulas: "is", "are"
- Modals: "necessary", "possible"
- Subordinators: "that"

### Step 4: Implement AST (`ast.rs`)

Define the deep structure of logical expressions:

```rust
pub enum Expr {
    // Set A: Syllogistic
    Categorical {
        quantifier: TokenType,
        subject: String,
        copula_negative: bool,
        predicate: String,
    },

    // Set C: Propositional
    BinaryOp {
        left: Box<Expr>,
        op: TokenType,
        right: Box<Expr>,
    },

    // Set J: Modal / Negation
    UnaryOp {
        op: TokenType,
        operand: Box<Expr>,
    },

    // Atomic propositions
    Atom(String),
}
```

### Step 5: Implement Parser (`parser.rs`)

Recursive descent parser with the following grammar:

```
Sentence      → IfExpr | ModalExpr | Quantified | Conjunction
IfExpr        → "if" Sentence ","? "then"? Sentence
ModalExpr     → ("necessary" | "possible") "that"? Sentence
Quantified    → Quantifier ContentWord Copula "not"? ContentWord
Conjunction   → Atom (("and" | "or") Atom)*
Atom          → "(" Sentence ")" | ContentWord
```

**Key methods:**
- `parse_sentence()` - entry point with precedence dispatch
- `parse_conditional()` - handles If-Then structures
- `parse_modal()` - handles necessity/possibility operators
- `parse_categorical()` - handles All/Some/No quantified statements
- `parse_conjunction()` - handles And/Or chains
- `parse_atom()` - handles parentheses and atomic propositions

### Step 6: Implement Transpiler (`transpile.rs`)

Convert AST to Logicaffeine LaTeX notation:

| AST Node | Output Format |
|----------|---------------|
| `Categorical(All, S, P)` | `All S is P` |
| `Categorical(No, S, P)` | `No S is P` |
| `Categorical(Some, S, P, neg=true)` | `Some S is not P` |
| `BinaryOp(And)` | `(L \cdot R)` |
| `BinaryOp(Or)` | `(L \vee R)` |
| `BinaryOp(If)` | `(L \supset R)` |
| `UnaryOp(Not)` | `\sim O` |
| `UnaryOp(Necessary)` | `\square O` |
| `UnaryOp(Possible)` | `\lozenge O` |
| `Atom(word)` | First letter uppercase |

### Step 7: Implement CLI (`main.rs`)

Simple demonstration runner:

```rust
fn main() {
    let inputs = vec![
        "All men are mortal.",
        "If it is raining, then it is pouring.",
        "It is necessary that if logic is fun, then students are happy.",
        "Some logicians are not boring.",
    ];

    for input in inputs {
        // Lex → Parse → Transpile pipeline
    }
}
```

### Step 8: Create generate-docs.sh

Script to generate documentation from source files:

```bash
#!/bin/bash
# Generate documentation for the Logicaffeine transpiler

OUTPUT="LOGOS_DOCUMENTATION.md"
echo "# Logicaffeine 1.0 Source Documentation" > "$OUTPUT"
echo "" >> "$OUTPUT"

# Include each source file with syntax highlighting
for file in src/*.rs; do
    echo "## $(basename $file)" >> "$OUTPUT"
    echo '```rust' >> "$OUTPUT"
    cat "$file" >> "$OUTPUT"
    echo '```' >> "$OUTPUT"
    echo "" >> "$OUTPUT"
done
```

---

## Test Cases

The transpiler must correctly handle:

| Input | Expected Output |
|-------|-----------------|
| "All men are mortal." | `All M is M` |
| "If it is raining, then it is pouring." | `(R \supset P)` |
| "It is necessary that logic is fun." | `\square L` |
| "Some logicians are not boring." | `Some L is not B` |
| "P and Q" | `(P \cdot Q)` |
| "Not P" | `\sim P` |

---

## Future Expansions

1. **Error Recovery**: Replace `panic!` with `Result<T, ParseError>` for graceful error handling and suggestions

2. **Scope Resolution**: Handle quantifier ambiguity like "All boys love a girl" (universal vs existential scope)

3. **Relative Clauses**: Add recursive `NounPhrase` with optional embedded `Sentence`:
   ```rust
   Quantified {
       quantifier: TokenType,
       subject: String,
       relative_clause: Option<Box<Expr>>,
       predicate: String,
   }
   ```

4. **Extended Dictionary**: Integration with WordNet or POS tagger for robust word classification

5. **Deontic Logic**: Add `Ought`, `Permissible` operators for ethical reasoning

6. **Belief Logic**: Add `Believes(agent, proposition)` for epistemic statements

---

## Files to Create

1. `/logos/Cargo.toml` - Project manifest
2. `/logos/src/token.rs` - Token type definitions
3. `/logos/src/lexer.rs` - Tokenization implementation
4. `/logos/src/ast.rs` - AST node definitions
5. `/logos/src/parser.rs` - Recursive descent parser
6. `/logos/src/transpile.rs` - Logicaffeine code generation
7. `/logos/src/lib.rs` - Public API exports
8. `/logos/src/main.rs` - CLI entry point
9. `/logos/generate-docs.sh` - Documentation generator
10. `/logos/IMPLEMENTATION_PLAN.md` - This document (copied to project)
