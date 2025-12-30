# LOGOS Language Guide

**Write English. Get Logic. Run Code.**

Welcome to LOGOS, where natural language becomes executable programs. This guide will take you from your first line of code to writing sophisticated applications—all in plain English.

---

## Table of Contents

### Part I: Programming in LOGOS

**Basics**

1. [Introduction](#1-introduction)
   - [What is LOGOS?](#what-is-logos)
   - [The Vision](#the-vision)
   - [How to Read This Guide](#how-to-read-this-guide)

2. [Getting Started](#2-getting-started)
   - [Hello World](#hello-world)
   - [Running Your Programs](#running-your-programs)
   - [Program Structure](#program-structure)

3. [Variables and Types](#3-variables-and-types)
   - [Creating Variables](#creating-variables)
   - [Changing Values](#changing-values)
   - [Primitive Types](#primitive-types)
   - [Type Annotations](#type-annotations)

4. [Operators and Expressions](#4-operators-and-expressions)
   - [Arithmetic](#arithmetic)
   - [Comparisons](#comparisons)
   - [Logical Operators](#logical-operators)
   - [Precedence and Grouping](#precedence-and-grouping)

5. [Control Flow](#5-control-flow)
   - [Conditionals](#conditionals)
   - [While Loops](#while-loops)
   - [For-Each Loops](#for-each-loops)
   - [Early Returns](#early-returns)

6. [Functions](#6-functions)
   - [Defining Functions](#defining-functions)
   - [Parameters](#parameters)
   - [Return Values](#return-values)
   - [Calling Functions](#calling-functions)
   - [Recursion](#recursion)

7. [Collections](#7-collections)
   - [Creating Lists](#creating-lists)
   - [Accessing Elements](#accessing-elements)
   - [Modifying Collections](#modifying-collections)
   - [Slicing](#slicing)
   - [Iterating](#iterating)

8. [User-Defined Types](#8-user-defined-types)
   - [Structs](#structs)
   - [Enums](#enums)
   - [Pattern Matching](#pattern-matching)

9. [Generics](#9-generics)
   - [Generic Types](#generic-types)
   - [Generic Collections](#generic-collections)

**Advanced**

10. [Memory and Ownership](#10-memory-and-ownership)
    - [The Three Verbs: Give, Show, Let Modify](#the-three-verbs)
    - [Ownership Rules](#ownership-rules)
    - [Common Patterns](#common-patterns)

11. [The Zone System](#11-the-zone-system)
    - [Creating Zones](#creating-a-zone)
    - [The Hotel California Rule](#the-hotel-california-rule)
    - [Escaping with Copy](#escaping-with-copy)
    - [Nested Zones](#nested-zones)

12. [Concurrency](#12-concurrency)
    - [Attempt All (Async I/O)](#attempt-all-async-io)
    - [Simultaneously (Parallel CPU)](#simultaneously-parallel-cpu)
    - [Race and Timeout](#race-first-to-finish)
    - [Streams and Channels](#streams-and-channels)

13. [Error Handling](#13-error-handling)
    - [Socratic Error Messages](#the-philosophy)
    - [The Failure Type](#the-failure-type)

14. [Advanced Features](#14-advanced-features)
    - [Refinement Types](#refinement-types)
    - [Assertions](#assertions)
    - [Modules](#modules)

### Part II: Project Structure

15. [Modules](#15-modules)
    - [Importing Modules](#importing-modules)
    - [Creating Modules](#creating-modules)
    - [Visibility](#visibility)

16. [The CLI: largo](#16-the-cli-largo)
    - [Creating a Project](#creating-a-project)
    - [Build Commands](#build-commands)
    - [Project Manifest](#project-manifest)

17. [Standard Library](#17-standard-library)
    - [File Operations](#file-operations)
    - [Time Operations](#time-operations)
    - [Random Numbers](#random-numbers)
    - [Environment](#environment)
    - [Native Functions](#native-functions)

### Part III: Logic Mode

18. [Logic Mode](#18-logic-mode)
    - [Introduction to Formal Logic](#introduction-to-formal-logic)
    - [Quantifiers](#quantifiers)
    - [Logical Connectives](#logical-connectives)
    - [Modal Operators](#modal-operators)
    - [Tense and Aspect](#tense-and-aspect)
    - [Wh-Questions](#wh-questions)
    - [Ambiguity and Multiple Readings](#ambiguity-and-multiple-readings)

### Part IV: Proofs and Verification

19. [Assertions and Trust](#19-assertions-and-trust)
    - [Assert](#assert)
    - [Trust with Justification](#trust-with-justification)
    - [Proof Blocks](#proof-blocks-advanced)

20. [Z3 Static Verification](#20-z3-static-verification)
    - [Refinement Types with Static Checking](#refinement-types-with-static-checking)
    - [Variable Tracking](#variable-tracking)
    - [Function Preconditions](#function-preconditions)

### Part V: Reference

21. [Complete Examples](#21-complete-examples)
    - [Mergesort](#mergesort)
    - [Factorial](#factorial)
    - [Working with Structs](#working-with-structs)
    - [Collection Processing](#collection-processing)

22. [Quick Reference](#22-quick-reference)
    - [Syntax Cheat Sheet](#syntax-cheat-sheet)
    - [Type Reference](#type-reference)
    - [Operator Reference](#operator-reference)
    - [Ownership Reference](#ownership-reference)
    - [Concurrency Reference](#concurrency-reference)

---

# Part I: Programming in LOGOS

---

## 1. Introduction

### What is LOGOS?

LOGOS is a programming language where you write code in natural English. Instead of cryptic symbols and arcane syntax, you express your ideas in sentences that read like plain prose—and those sentences compile into efficient, executable programs.

LOGOS has two modes:

| Mode | What It Does | Output |
|------|--------------|--------|
| **Imperative Mode** | Write executable programs | Rust code (compiled to native binaries) |
| **Logic Mode** | Translate English to formal logic | First-Order Logic notation |

This guide focuses primarily on **Imperative Mode**—using LOGOS as a programming language. Part II covers Logic Mode for those interested in formal semantics.

### The Vision

The name LOGOS comes from the Greek λόγος, meaning "word," "reason," and "principle." In LOGOS, these concepts unify:

- **Words** become executable code
- **Reason** becomes verifiable logic
- **Principles** become formal proofs

When you write LOGOS, you're not writing comments that describe code—you're writing sentences that *are* the code. Every statement you write is both human-readable documentation and machine-executable instruction.

### How to Read This Guide

This guide is designed to serve readers at all levels:

**If you're new to programming:**
- Read each section in order
- Try every example yourself
- Don't skip ahead—each concept builds on the previous

**If you're an experienced programmer:**
- Use the Table of Contents to jump to what interests you
- The Quick Reference section provides rapid lookup
- The Complete Examples show real-world patterns

Throughout this guide, code examples appear in blocks like this:

```logos
## Main
Let message be "Hello, World!".
Show message.
```

When you see `## Main`, that marks the entry point of a program—where execution begins.

---

## 2. Getting Started

### Hello World

Every programming journey begins with Hello World. In LOGOS:

```logos
## Main
Show "Hello, World!".
```

That's it. Two lines. The first line (`## Main`) declares the entry point of your program. The second line displays text to the screen.

Notice how the code reads like a simple instruction: "Show 'Hello, World!'" It's a command, and the computer obeys.

### Running Your Programs

To run a LOGOS program:

```bash
# Build the project
cargo build

# Run the program
cargo run
```

For development, you can also use the interactive web interface or REPL mode.

### Program Structure

A LOGOS program is organized into sections marked by headers:

```logos
## Definition
A Point has:
    an x: Int.
    a y: Int.

## To greet (name: Text) -> Text:
    Return "Hello, " + name + "!".

## Main
Let p be a new Point with x 10 and y 20.
Let message be greet("World").
Show message.
```

The three main section types are:

| Header | Purpose |
|--------|---------|
| `## Definition` | Define new types (structs, enums) |
| `## To [verb]` | Define functions |
| `## Main` | The entry point—where your program starts |

Sections can appear in any order. The compiler reads all definitions before executing `## Main`.

### Your First Real Program

Let's write something slightly more interesting:

```logos
## Main
Let name be "Alice".
Let age be 25.
Show "Name: " + name.
Show "Age: " + age.
```

This program:
1. Creates a variable `name` containing the text "Alice"
2. Creates a variable `age` containing the number 25
3. Displays both values

Run it, and you'll see:
```
Name: Alice
Age: 25
```

---

## 3. Variables and Types

Variables are containers that hold values. In LOGOS, you create and modify variables using natural English sentences.

### Creating Variables

Use `Let` to create a new variable:

```logos
## Main
Let x be 5.
Let name be "Bob".
Let is_active be true.
Let temperature be 98.6.
```

The word `be` assigns a value to the variable. Reading these aloud:
- "Let x be 5" — create a variable called `x` and set it to 5
- "Let name be Bob" — create a variable called `name` and set it to "Bob"

Variable names can be:
- Single words: `x`, `count`, `name`
- Multi-word phrases: `user count`, `total items`, `is valid`

```logos
## Main
Let user count be 0.
Let total price be 99.99.
Let is logged in be true.
```

Multi-word variable names make your code read even more naturally.

### Changing Values

Use `Set` to change an existing variable:

```logos
## Main
Let x be 5.
Show x.          # Shows: 5

Set x to 10.
Show x.          # Shows: 10

Set x to x + 1.
Show x.          # Shows: 11
```

The difference between `Let` and `Set`:
- `Let` creates a *new* variable
- `Set` modifies an *existing* variable

If you try to `Set` a variable that doesn't exist, you'll get an error. If you try to `Let` a variable that already exists in the same scope, you'll shadow (hide) the previous one.

### Primitive Types

LOGOS has four primitive types:

| Type | Description | Examples |
|------|-------------|----------|
| `Int` | Whole numbers | `5`, `-10`, `0`, `1000000` |
| `Bool` | True or false | `true`, `false` |
| `Text` | Strings of characters | `"Hello"`, `"LOGOS"`, `""` |
| `Float` | Decimal numbers | `3.14`, `-0.5`, `98.6` |

#### Integers (Int)

Integers are whole numbers, positive or negative:

```logos
## Main
Let count be 42.
Let negative be -17.
Let zero be 0.
Let big be 1000000.
```

You can perform arithmetic on integers:

```logos
## Main
Let a be 10.
Let b be 3.

Let sum be a + b.       # 13
Let diff be a - b.      # 7
Let product be a * b.   # 30
Let quotient be a / b.  # 3 (integer division)
```

#### Booleans (Bool)

Booleans represent true or false:

```logos
## Main
Let is_valid be true.
Let is_empty be false.
```

Booleans are essential for making decisions (we'll see this in Control Flow).

#### Text

Text (strings) holds sequences of characters:

```logos
## Main
Let greeting be "Hello".
Let name be "World".
Let empty be "".
```

Concatenate text with `+`:

```logos
## Main
Let first be "Hello".
Let second be "World".
Let message be first + ", " + second + "!".
Show message.  # Shows: Hello, World!
```

#### Floats

Floats are decimal numbers:

```logos
## Main
Let pi be 3.14159.
Let temperature be 98.6.
Let negative be -0.5.
```

### Type Annotations

Usually, LOGOS infers the type from the value you assign. But you can be explicit:

```logos
## Main
Let x: Int be 5.
Let name: Text be "Alice".
Let flag: Bool be true.
Let rate: Float be 3.14.
```

Type annotations are useful when:
- You want to be explicit for documentation
- The type can't be inferred from context
- You want to catch errors early

```logos
## Main
Let count: Int be 0.        # Explicit: count is an integer
Let items be [1, 2, 3].     # Inferred: items is Seq of Int
```

### Working with Text

Text has several useful operations:

```logos
## Main
Let greeting be "Hello, World!".

# Length
Let len be length of greeting.
Show len.  # Shows: 13

# Concatenation
Let part1 be "Hello".
Let part2 be "World".
Let full be part1 + ", " + part2 + "!".
```

### Converting Between Types

Sometimes you need to convert between types:

```logos
## Main
Let num be 42.
Let text_num be num as Text.  # "42"

Let pi_text be "3.14".
Let pi be pi_text as Float.   # 3.14
```

---

## 4. Operators and Expressions

Operators let you combine values into expressions. LOGOS supports both symbolic operators (like `+`) and English words (like `plus`).

### Arithmetic

LOGOS supports standard arithmetic operations:

| Operation | Symbol | English |
|-----------|--------|---------|
| Addition | `+` | `plus` |
| Subtraction | `-` | `minus` |
| Multiplication | `*` | `times` |
| Division | `/` | `divided by` |
| Modulo | `%` | `modulo` |

You can use either form:

```logos
## Main
Let a be 10.
Let b be 3.

# Using symbols
Let sum1 be a + b.
Let diff1 be a - b.
Let prod1 be a * b.
Let quot1 be a / b.
Let rem1 be a % b.

# Using English
Let sum2 be a plus b.
Let diff2 be a minus b.
Let prod2 be a times b.
Let quot2 be a divided by b.
Let rem2 be a modulo b.
```

Both forms compile to identical code. Choose whichever reads better in context.

#### Combining Operations

You can chain operations:

```logos
## Main
Let result be 2 + 3 + 4.      # 9
Let product be 2 * 3 * 4.     # 24
```

When mixing different operators, use parentheses to be clear:

```logos
## Main
Let result be (2 + 3) * 4.    # 20
Let other be 2 + (3 * 4).     # 14
```

### Comparisons

Comparison operators produce boolean values (true or false):

| Operation | Symbol | English |
|-----------|--------|---------|
| Less than | `<` | `is less than` |
| Greater than | `>` | `is greater than` |
| Less or equal | `<=` | `is at most` |
| Greater or equal | `>=` | `is at least` |
| Equal | `==` | `equals` |
| Not equal | `!=` | `is not` |

```logos
## Main
Let x be 5.
Let y be 10.

# Using symbols
Let a be x < y.     # true
Let b be x > y.     # false
Let c be x == 5.    # true
Let d be x != y.    # true

# Using English
Let e be x is less than y.        # true
Let f be x is greater than y.     # false
Let g be x equals 5.              # true
Let h be x is not y.              # true
Let i be x is at most 5.          # true (5 <= 5)
Let j be x is at least 5.         # true (5 >= 5)
```

The English forms read more naturally in context:

```logos
## Main
Let age be 25.
If age is at least 18:
    Show "Adult".
```

### Logical Operators

Combine boolean expressions with logical operators:

| Operation | Keyword | Meaning |
|-----------|---------|---------|
| AND | `and` | Both must be true |
| OR | `or` | At least one must be true |
| NOT | `not` | Inverts true/false |

```logos
## Main
Let a be true.
Let b be false.

Let both be a and b.       # false
Let either be a or b.      # true
Let not_a be not a.        # false
```

Combine comparisons with logical operators:

```logos
## Main
Let age be 25.
Let has_id be true.

If age is at least 18 and has_id:
    Show "Entry allowed".

Let temperature be 72.
If temperature is less than 60 or temperature is greater than 80:
    Show "Uncomfortable".
Otherwise:
    Show "Comfortable".
```

### Precedence and Grouping

When you combine multiple operators, LOGOS follows standard precedence rules:

1. Parentheses `()` — highest priority
2. `not`
3. `*`, `/`, `%` (multiplication, division, modulo)
4. `+`, `-` (addition, subtraction)
5. Comparisons (`<`, `>`, `<=`, `>=`, `==`, `!=`)
6. `and`
7. `or` — lowest priority

When in doubt, use parentheses to make your intent clear:

```logos
## Main
# These are equivalent:
Let a be 2 + 3 * 4.         # 14 (multiplication first)
Let b be 2 + (3 * 4).       # 14 (explicit)

# These are equivalent:
Let c be true or false and false.   # true (and before or)
Let d be true or (false and false). # true (explicit)
```

---

## 5. Control Flow

Control flow determines which code runs and in what order. LOGOS provides conditionals and loops using natural English syntax.

### Conditionals

Use `If` to execute code only when a condition is true:

```logos
## Main
Let x be 10.

If x is greater than 5:
    Show "x is big".
```

The colon (`:`) after the condition opens an indented block. Everything indented under the `If` runs only when the condition is true.

#### If/Otherwise

Use `Otherwise` to handle the false case:

```logos
## Main
Let temperature be 72.

If temperature is greater than 80:
    Show "It's hot!".
Otherwise:
    Show "It's comfortable.".
```

#### Multiple Conditions

Chain conditions with additional `If` statements:

```logos
## Main
Let score be 85.

If score is at least 90:
    Show "Grade: A".
Otherwise:
    If score is at least 80:
        Show "Grade: B".
    Otherwise:
        If score is at least 70:
            Show "Grade: C".
        Otherwise:
            Show "Grade: F".
```

Or use a cleaner nested structure:

```logos
## Main
Let score be 85.

If score is at least 90:
    Show "Grade: A".
If score is at least 80 and score is less than 90:
    Show "Grade: B".
If score is at least 70 and score is less than 80:
    Show "Grade: C".
If score is less than 70:
    Show "Grade: F".
```

#### Compound Conditions

Combine multiple conditions with `and` and `or`:

```logos
## Main
Let age be 25.
Let has_license be true.
Let is_insured be true.

If age is at least 18 and has_license and is_insured:
    Show "You can rent a car.".
Otherwise:
    Show "Sorry, you cannot rent a car.".
```

```logos
## Main
Let day be "Saturday".

If day equals "Saturday" or day equals "Sunday":
    Show "It's the weekend!".
Otherwise:
    Show "It's a weekday.".
```

### While Loops

Use `While` to repeat code as long as a condition is true:

```logos
## Main
Let count be 1.

While count is at most 5:
    Show count.
    Set count to count + 1.
```

Output:
```
1
2
3
4
5
```

#### Loop Variables

A common pattern is to use a counter variable:

```logos
## Main
Let i be 1.
Let sum be 0.

While i is at most 100:
    Set sum to sum + i.
    Set i to i + 1.

Show "Sum of 1 to 100: " + sum.  # Shows: 5050
```

#### Careful with Infinite Loops

Make sure your loop condition eventually becomes false:

```logos
## Main
Let x be 10.

# This loop will terminate
While x is greater than 0:
    Show x.
    Set x to x - 1.

# DON'T do this - infinite loop!
# While true:
#     Show "Forever!".
```

### For-Each Loops

Use `Repeat for` to iterate over collections:

```logos
## Main
Let numbers be [1, 2, 3, 4, 5].

Repeat for n in numbers:
    Show n.
```

Output:
```
1
2
3
4
5
```

#### Processing Each Item

```logos
## Main
Let names be ["Alice", "Bob", "Charlie"].

Repeat for name in names:
    Show "Hello, " + name + "!".
```

Output:
```
Hello, Alice!
Hello, Bob!
Hello, Charlie!
```

#### Accumulating Results

```logos
## Main
Let numbers be [10, 20, 30, 40, 50].
Let total be 0.

Repeat for n in numbers:
    Set total to total + n.

Show "Total: " + total.  # Shows: 150
```

### Early Returns

Use `Return` to exit a function (or the main block) early:

```logos
## To find_first_negative (numbers: Seq of Int) -> Int:
    Repeat for n in numbers:
        If n is less than 0:
            Return n.
    Return 0.  # No negative found

## Main
Let nums be [5, 3, -2, 8, -4].
Let first_neg be find_first_negative(nums).
Show first_neg.  # Shows: -2
```

#### Guard Clauses

Use early returns at the start of functions to handle special cases:

```logos
## To divide (a: Int) and (b: Int) -> Int:
    If b equals 0:
        Show "Error: Cannot divide by zero".
        Return 0.
    Return a / b.

## Main
Let result be divide(10, 2).
Show result.  # Shows: 5
```

---

## 6. Functions

Functions are reusable blocks of code. In LOGOS, you define functions using natural English headers that describe what the function does.

### Defining Functions

A function definition starts with `## To` followed by the function name:

```logos
## To greet:
    Show "Hello!".

## Main
greet.
```

The function name should be a verb or verb phrase that describes what the function does.

### Parameters

Functions can accept parameters—values passed in when the function is called:

```logos
## To greet (name: Text):
    Show "Hello, " + name + "!".

## Main
greet("Alice").
greet("Bob").
```

Output:
```
Hello, Alice!
Hello, Bob!
```

#### Multiple Parameters

Use `and` to separate multiple parameters:

```logos
## To add (a: Int) and (b: Int) -> Int:
    Return a + b.

## Main
Let sum be add(3, 5).
Show sum.  # Shows: 8
```

Or with more descriptive names:

```logos
## To calculate_area (width: Int) and (height: Int) -> Int:
    Return width * height.

## Main
Let area be calculate_area(10, 5).
Show "Area: " + area.  # Shows: Area: 50
```

#### Many Parameters

For functions with many parameters:

```logos
## To create_user (name: Text) and (age: Int) and (email: Text) -> Text:
    Return name + " (" + age + ") - " + email.

## Main
Let info be create_user("Alice", 25, "alice@example.com").
Show info.
```

### Return Values

Use `-> Type` to specify what the function returns:

```logos
## To double (n: Int) -> Int:
    Return n * 2.

## To is_adult (age: Int) -> Bool:
    Return age is at least 18.

## To format_greeting (name: Text) -> Text:
    Return "Hello, " + name + "!".

## Main
Show double(5).              # Shows: 10
Show is_adult(25).           # Shows: true
Show format_greeting("Bob"). # Shows: Hello, Bob!
```

#### Functions Without Return Values

If a function doesn't return anything, omit the `-> Type`:

```logos
## To print_separator:
    Show "-------------------".

## Main
print_separator.
Show "Content here".
print_separator.
```

Output:
```
-------------------
Content here
-------------------
```

### Calling Functions

Call functions by name, passing arguments in parentheses:

```logos
## To square (n: Int) -> Int:
    Return n * n.

## Main
Let x be square(5).
Let y be square(square(2)).  # Nested calls: square(4) = 16
Show x.  # Shows: 25
Show y.  # Shows: 16
```

#### Using Results in Expressions

Function results can be used anywhere a value is expected:

```logos
## To max (a: Int) and (b: Int) -> Int:
    If a is greater than b:
        Return a.
    Return b.

## Main
Let biggest be max(10, max(5, 20)).  # max(10, 20) = 20
Show biggest.

If max(3, 7) is greater than 5:
    Show "The maximum exceeds 5".
```

### Recursion

Functions can call themselves. This is called recursion:

```logos
## To factorial (n: Int) -> Int:
    If n is at most 1:
        Return 1.
    Return n * factorial(n - 1).

## Main
Show factorial(5).  # Shows: 120
```

Let's trace through `factorial(5)`:
- `factorial(5)` = 5 × `factorial(4)`
- `factorial(4)` = 4 × `factorial(3)`
- `factorial(3)` = 3 × `factorial(2)`
- `factorial(2)` = 2 × `factorial(1)`
- `factorial(1)` = 1 (base case)
- Working back up: 2 × 1 = 2, 3 × 2 = 6, 4 × 6 = 24, 5 × 24 = 120

#### Recursive Structure

Every recursive function needs:
1. A **base case** — when to stop recursing
2. A **recursive case** — calling itself with a "smaller" problem

```logos
## To countdown (n: Int):
    If n is at most 0:
        Show "Liftoff!".
        Return.
    Show n.
    countdown(n - 1).

## Main
countdown(5).
```

Output:
```
5
4
3
2
1
Liftoff!
```

---

## 7. Collections

Collections hold multiple values. The primary collection type in LOGOS is `Seq` (sequence), similar to lists or arrays in other languages.

### Creating Lists

Create a list with square brackets:

```logos
## Main
Let numbers be [1, 2, 3, 4, 5].
Let names be ["Alice", "Bob", "Charlie"].
Let flags be [true, false, true].
```

Create an empty list with a type:

```logos
## Main
Let items be a new Seq of Int.
Let words be a new Seq of Text.
```

### Accessing Elements

LOGOS uses **1-based indexing**. The first element is at position 1, not 0.

Why? Because that's how humans count. When you have a list of items, the first item is item 1, not item 0.

```logos
## Main
Let fruits be ["apple", "banana", "cherry"].

Let first be item 1 of fruits.   # "apple"
Let second be item 2 of fruits.  # "banana"
Let third be item 3 of fruits.   # "cherry"

Show first.
Show second.
Show third.
```

#### Getting the Length

```logos
## Main
Let numbers be [10, 20, 30, 40, 50].
Let len be length of numbers.
Show "List has " + len + " items".  # Shows: List has 5 items
```

#### Accessing the Last Element

```logos
## Main
Let items be [1, 2, 3, 4, 5].
Let n be length of items.
Let last be item n of items.
Show last.  # Shows: 5
```

### Modifying Collections

#### Adding Elements

Use `Push` to add an element to the end:

```logos
## Main
Let numbers be [1, 2, 3].
Push 4 to numbers.
Push 5 to numbers.
Show numbers.  # Shows: [1, 2, 3, 4, 5]
```

#### Removing Elements

Use `Pop` to remove and get the last element:

```logos
## Main
Let numbers be [1, 2, 3, 4, 5].
Pop from numbers into last.
Show last.     # Shows: 5
Show numbers.  # Shows: [1, 2, 3, 4]
```

#### Copying Collections

Use `copy of` to create a deep copy:

```logos
## Main
Let original be [1, 2, 3].
Let duplicate be copy of original.

Push 4 to duplicate.

Show original.   # Shows: [1, 2, 3]
Show duplicate.  # Shows: [1, 2, 3, 4]
```

Without `copy of`, both variables would reference the same list.

### Slicing

Extract a portion of a list with `through`:

```logos
## Main
Let numbers be [10, 20, 30, 40, 50].

Let first_three be numbers 1 through 3.
Show first_three.  # Shows: [10, 20, 30]

Let middle be numbers 2 through 4.
Show middle.  # Shows: [20, 30, 40]

Let last_two be numbers 4 through 5.
Show last_two.  # Shows: [40, 50]
```

Slicing is **inclusive** on both ends. `numbers 1 through 3` includes items at positions 1, 2, and 3.

### Iterating

Use `Repeat for` to process each element:

```logos
## Main
Let names be ["Alice", "Bob", "Charlie"].

Repeat for name in names:
    Show "Hello, " + name + "!".
```

Output:
```
Hello, Alice!
Hello, Bob!
Hello, Charlie!
```

#### Building a New List

```logos
## Main
Let numbers be [1, 2, 3, 4, 5].
Let doubled be a new Seq of Int.

Repeat for n in numbers:
    Push n * 2 to doubled.

Show doubled.  # Shows: [2, 4, 6, 8, 10]
```

#### Filtering

```logos
## Main
Let numbers be [1, 2, 3, 4, 5, 6, 7, 8, 9, 10].
Let evens be a new Seq of Int.

Repeat for n in numbers:
    If n modulo 2 equals 0:
        Push n to evens.

Show evens.  # Shows: [2, 4, 6, 8, 10]
```

#### Finding Items

```logos
## To contains (items: Seq of Int) and (target: Int) -> Bool:
    Repeat for item in items:
        If item equals target:
            Return true.
    Return false.

## Main
Let numbers be [5, 10, 15, 20].
Show contains(numbers, 15).  # Shows: true
Show contains(numbers, 7).   # Shows: false
```

---

## 8. User-Defined Types

Beyond primitive types and collections, LOGOS lets you define your own types to model your problem domain.

### Structs

A struct (structure) groups related values together. Define one in a `## Definition` block:

```logos
## Definition
A Point has:
    an x: Int.
    a y: Int.

## Main
Let p be a new Point with x 10 and y 20.
Show p's x.  # Shows: 10
Show p's y.  # Shows: 20
```

#### Struct Syntax

The definition syntax is:
```
A [TypeName] has:
    a [field1]: [Type1].
    a [field2]: [Type2].
    ...
```

Use `a` or `an` based on what sounds natural in English.

#### Creating Instances

Use `a new [Type] with [fields]`:

```logos
## Definition
A Person has:
    a name: Text.
    an age: Int.
    an email: Text.

## Main
Let alice be a new Person with name "Alice" and age 25 and email "alice@example.com".
Show alice's name.   # Shows: Alice
Show alice's age.    # Shows: 25
Show alice's email.  # Shows: alice@example.com
```

#### Accessing Fields

Use `'s` (possessive) to access fields:

```logos
## Main
Let p be a new Point with x 5 and y 10.

Let x_coord be p's x.
Let y_coord be p's y.

Show "(" + x_coord + ", " + y_coord + ")".  # Shows: (5, 10)
```

#### Modifying Fields

Use `Set` with the possessive:

```logos
## Main
Let p be a new Point with x 0 and y 0.
Show p's x.  # Shows: 0

Set p's x to 100.
Show p's x.  # Shows: 100
```

#### Structs with Functions

Functions can work with your custom types:

```logos
## Definition
A Rectangle has:
    a width: Int.
    a height: Int.

## To area (r: Rectangle) -> Int:
    Return r's width * r's height.

## To perimeter (r: Rectangle) -> Int:
    Return 2 * (r's width + r's height).

## Main
Let rect be a new Rectangle with width 10 and height 5.
Show "Area: " + area(rect).           # Shows: Area: 50
Show "Perimeter: " + perimeter(rect). # Shows: Perimeter: 30
```

### Enums

An enum (enumeration) defines a type that can be one of several variants:

```logos
## Definition
A Shape is either:
    a Circle with radius: Int.
    a Rectangle with width: Int and height: Int.
    a Square with side: Int.

## Main
Let c be a new Circle with radius 5.
Let r be a new Rectangle with width 10 and height 3.
Let s be a new Square with side 4.
```

Each variant can have different fields (or no fields at all).

#### Variants Without Data

```logos
## Definition
A Direction is either:
    North.
    South.
    East.
    West.

## Main
Let heading be North.
```

#### Common Enum Patterns

**Option/Maybe** — a value that might not exist:

```logos
## Definition
A Maybe of [T] is either:
    a Just with value: T.
    Nothing.

## Main
Let found be a new Just with value 42.
Let missing be Nothing.
```

**Result** — success or failure:

```logos
## Definition
A Result is either:
    an Ok with value: Int.
    an Error with message: Text.

## Main
Let success be a new Ok with value 100.
Let failure be a new Error with message "Something went wrong".
```

### Pattern Matching

Use `Inspect` to handle different enum variants:

```logos
## Definition
A Shape is either:
    a Circle with radius: Int.
    a Rectangle with width: Int and height: Int.

## To area (s: Shape) -> Int:
    Inspect s:
        When Circle:
            Return 3 * s's radius * s's radius.
        When Rectangle:
            Return s's width * s's height.

## Main
Let c be a new Circle with radius 5.
Let r be a new Rectangle with width 10 and height 3.

Show area(c).  # Shows: 75
Show area(r).  # Shows: 30
```

#### Pattern Matching Syntax

```
Inspect [expression]:
    When [Variant1]:
        [code for Variant1]
    When [Variant2]:
        [code for Variant2]
    ...
```

Each `When` clause handles one variant. The code under it executes only if the value matches that variant.

#### Accessing Variant Fields

Inside a `When` clause, access the variant's fields using `'s`:

```logos
## Definition
A Message is either:
    a Text with content: Text.
    a Number with value: Int.
    Empty.

## To describe (m: Message) -> Text:
    Inspect m:
        When Text:
            Return "Text message: " + m's content.
        When Number:
            Return "Number: " + m's value.
        When Empty:
            Return "Empty message".

## Main
Let m1 be a new Text with content "Hello".
Let m2 be a new Number with value 42.
Let m3 be Empty.

Show describe(m1).  # Shows: Text message: Hello
Show describe(m2).  # Shows: Number: 42
Show describe(m3).  # Shows: Empty message
```

---

## 9. Generics

Generics let you write types and functions that work with any type, not just specific ones.

### Generic Types

Define a generic type with `[T]` in the type name:

```logos
## Definition
A Box of [T] has:
    a contents: T.

## Main
Let int_box be a new Box of Int with contents 42.
Let text_box be a new Box of Text with contents "Hello".

Show int_box's contents.   # Shows: 42
Show text_box's contents.  # Shows: Hello
```

The `[T]` is a placeholder that gets replaced with a real type when you use it:
- `Box of Int` — a box containing an integer
- `Box of Text` — a box containing text

#### Multiple Type Parameters

```logos
## Definition
A Pair of [A] and [B] has:
    a first: A.
    a second: B.

## Main
Let p be a new Pair of Int and Text with first 1 and second "one".
Show p's first.   # Shows: 1
Show p's second.  # Shows: one
```

### Generic Collections

Collections are generic types. `Seq of Int` is a sequence of integers:

```logos
## Main
Let numbers be a new Seq of Int.
Push 1 to numbers.
Push 2 to numbers.

Let names be a new Seq of Text.
Push "Alice" to names.
Push "Bob" to names.
```

#### Nested Generics

You can nest generic types:

```logos
## Main
Let matrix be a new Seq of (Seq of Int).

Let row1 be [1, 2, 3].
Let row2 be [4, 5, 6].

Push row1 to matrix.
Push row2 to matrix.

Let first_row be item 1 of matrix.
Show first_row.  # Shows: [1, 2, 3]
```

### Generic Functions

Functions can be generic too:

```logos
## To first_of (items: Seq of [T]) -> T:
    Return item 1 of items.

## To last_of (items: Seq of [T]) -> T:
    Let n be length of items.
    Return item n of items.

## Main
Let numbers be [10, 20, 30].
Let names be ["Alice", "Bob", "Charlie"].

Show first_of(numbers).  # Shows: 10
Show last_of(names).     # Shows: Charlie
```

---

## 10. Memory and Ownership

LOGOS provides memory safety through an ownership system expressed in natural English. Instead of cryptic symbols, you use verbs that describe what you're doing with data.

### The Three Verbs

| Verb | Meaning | What Happens |
|------|---------|--------------|
| `Give` | Transfer ownership | The original variable can no longer be used |
| `Show` | Temporary read access | The function can look but not modify |
| `Let modify` | Temporary write access | The function can change the data |

### Give (Move)

Use `Give` to transfer ownership of data to another function:

```logos
## To process (data: Text):
    Show "Processing: " + data.

## Main
Let profile be "User Profile Data".
Give profile to process.

Show profile.  # ERROR: You already gave 'profile' away
```

When you `Give` something, you no longer own it. The compiler will stop you from using it afterward.

**Error message:**
```
Error at line 7: You already gave 'profile' away.

  5 │ Give profile to process.
  7 │ Show profile.
    │      ^^^^^^^
    │
  = You gave 'profile' to 'process' on line 5.
  = Once you give something away, you can no longer use it.
  = Did you mean to show it first, then give it?
```

### Show (Immutable Borrow)

Use `Show` to let a function read your data without giving up ownership:

```logos
## To display (data: Text):
    Show "Displaying: " + data.

## Main
Let profile be "User Profile Data".
Show profile to display.

Show profile.  # OK! We still own 'profile'
```

After `Show`, you still own the data. You can use it again.

### Let Modify (Mutable Borrow)

Use `Let [function] modify [data]` to let a function change your data:

```logos
## To increment (counter: Int):
    Set counter to counter + 1.

## Main
Let count be 0.
Let increment modify count.
Show count.  # Shows: 1
```

The function can change the data, but you get it back when it's done.

### Ownership Rules

LOGOS enforces these rules to prevent bugs:

1. **Single Owner:** Every value has exactly one owner at a time
2. **Move Semantics:** `Give` transfers ownership—you can't use it after
3. **Borrow Checking:** References (`Show`) can't outlive the owner
4. **Exclusive Mutation:** Only one `Let modify` at a time

### Common Patterns

**Copy first, then give:**
```logos
## Main
Let original be "Important data".
Let backup be copy of original.
Give original to processor.
Show backup.  # OK! backup is independent
```

**Show multiple times:**
```logos
## Main
Let data be "Shared info".
Show data to logger.
Show data to validator.
Show data to reporter.  # All OK - just reading
```

**Sequential mutation:**
```logos
## Main
Let items be [1, 2, 3].
Let sorter modify items.
Let reverser modify items.  # OK - mutations don't overlap
Show items.
```

---

## 11. The Zone System

For high-performance scenarios, LOGOS provides **Zones**—memory regions where allocations are fast and cleanup is instant.

### Why Zones?

Normal memory allocation is flexible but has overhead. Zones trade flexibility for speed:

| Operation | Normal Heap | Zone |
|-----------|-------------|------|
| Allocate | O(log n) | O(1) |
| Deallocate individual | O(log n) | N/A |
| Free everything | O(n) | O(1) |

Zones are perfect for temporary data that you'll discard all at once.

### Creating a Zone

```logos
## Main
Inside a zone called "WorkSpace":
    Let temp_data be [1, 2, 3, 4, 5].
    Let result be process(temp_data).
    Show result.
# Zone freed instantly when block exits
```

Everything allocated inside the zone is freed automatically when the block ends.

### The Hotel California Rule

**"What happens in the Zone, stays in the Zone."**

References to zone-allocated data cannot escape:

```logos
## Main
Let result be nothing.
Inside a zone called "Temp":
    Let calculation be heavy_computation().
    Set result to calculation.  # ERROR!
```

**Error:**
```
Zone safety violation: 'calculation' cannot escape zone 'Temp'.
The object will be deallocated when the zone exits.
```

### Escaping with Copy

To get data out of a zone, make an explicit copy:

```logos
## Main
Let result be nothing.
Inside a zone called "Temp":
    Let calculation be heavy_computation().
    Set result to a copy of calculation.  # OK! Deep copy escapes
Show result.  # OK
```

### What Can Escape?

| Type | Can Escape? | Why |
|------|-------------|-----|
| Primitives (Int, Bool) | Yes | Copied automatically |
| References | No | Would point to freed memory |
| Explicit copies | Yes | Creates independent data |

### Zone with Size

Specify the zone size for predictable memory use:

```logos
## Main
Inside a zone called "Buffer" of size 64 KB:
    # Allocations happen in pre-allocated 64KB
    Let data be process_chunk(input).
```

### Memory-Mapped Zones

For reading large files without loading them into memory, use **mapped zones**:

```logos
## Main
Inside a zone called "BigData" mapped from "dataset.bin":
    Let bytes be zone's as_slice().
    Show "File size: " + length of bytes + " bytes".
    # Process file contents without copying into memory
```

Mapped zones provide:
- **Zero-copy I/O:** File contents accessed directly from disk via OS page faults
- **Read-only access:** You can read the data, but not modify it
- **Ideal for large files:** Handle gigabyte files without memory issues

| Zone Type | Allocation | Access | Use Case |
|-----------|------------|--------|----------|
| Heap | O(1) bump | Read/Write | Temporary data |
| Mapped | OS page fault | Read-only | Large file processing |

```logos
## Main
# Process a massive log file without loading it all
Inside a zone called "Logs" mapped from "server.log":
    Let content be zone's as_slice().
    Repeat for line in split(content, "\n"):
        If contains(line, "ERROR"):
            Show line.
```

### Nested Zones

Zones can nest. Inner zone data can't escape to outer zones:

```logos
## Main
Inside a zone called "Outer":
    Let x be 1.
    Inside a zone called "Inner":
        Let y be 2.
        Set x to y.  # ERROR: 'y' has shorter lifetime than 'x'
```

### When to Use Zones

Use zones when:
- Processing large amounts of temporary data
- Performance is critical (games, simulations)
- Memory allocation patterns are predictable
- You want instant cleanup

Don't use zones when:
- Data needs to persist beyond a function
- You need to free individual items
- Allocation patterns are unpredictable

---

## 12. Concurrency

LOGOS provides safe concurrency through structured patterns. No data races, no deadlocks.

### Two Kinds of Concurrent Work

| Pattern | Keyword | Use For | Compiles To |
|---------|---------|---------|-------------|
| **Async** | `Attempt all` | I/O-bound tasks (network, files) | tokio::join! |
| **Parallel** | `Simultaneously` | CPU-bound tasks (computation) | rayon/threads |

### Attempt All (Async I/O)

Use `Attempt all of the following:` for I/O operations that wait on external resources:

```logos
## Main
Attempt all of the following:
    Let profile be fetch_user_profile().
    Let settings be fetch_user_settings().
    Let history be fetch_user_history().

Show "Profile: " + profile.
Show "Settings: " + settings.
Show "History: " + history.
```

All three fetches run concurrently. The program waits until all complete before continuing.

### Simultaneously (Parallel CPU)

Use `Simultaneously:` for CPU-intensive work:

```logos
## Main
Let data be [1, 2, 3, 4, 5, 6, 7, 8].
Let mid be 4.

Simultaneously:
    Let left_sum be sum(data 1 through mid).
    Let right_sum be sum(data (mid + 1) through 8).

Let total be left_sum + right_sum.
Show "Total: " + total.
```

Both computations run in parallel on different CPU cores.

### Race: First to Finish

Use `Await the first success of:` when you want the fastest response:

```logos
## Main
Await the first success of:
    Query primary server.
    Query backup server.

Use whichever responded first.
```

Both queries start. The first to succeed wins; the other is cancelled.

### Timeout

Add timeout to prevent waiting forever:

```logos
## Main
Await the result of fetch_data() or timeout after 30 seconds.
```

### Ownership and Concurrency

The ownership system prevents data races:

```logos
## Main
Let data be "shared".

# OK: Multiple reads
Attempt all of the following:
    Show data to analyzer_a.
    Show data to analyzer_b.

# ERROR: Concurrent writes
Attempt all of the following:
    Let modifier_a modify data.
    Let modifier_b modify data.  # ERROR!
```

**Error:**
```
Concurrent mutable borrows: 'data' cannot be modified by both
'modifier_a' and 'modifier_b' simultaneously.
```

### Streams and Channels

For producer-consumer patterns:

```logos
## Main
Create a stream called "LogStream".

# Producer
Spawn a task:
    Repeat for i from 1 to 100:
        Pour "Log entry " + i into "LogStream".

# Consumer
Spawn a task consuming "LogStream":
    Repeat for message in "LogStream":
        Write message to log file.
```

---

## 13. Error Handling

LOGOS uses **Socratic error messages**—friendly, educational feedback that teaches while it corrects.

### The Philosophy

Instead of cryptic compiler errors, LOGOS explains:
1. **What** went wrong
2. **Where** it happened
3. **Why** it's a problem
4. **How** to fix it

### Example Error

```logos
## Main
Let data be "important".
Give data to processor.
Show data.  # Error!
```

**LOGOS says:**
```
I notice: You gave ownership of 'data' to the processor on line 3.

  2 │ Give data to processor.
  3 │ Show data.
    │      ^^^^
    │
This is a problem because:
  Once you give something away, you no longer own it.
  Line 3 tries to use 'data', but it belongs to 'processor' now.

You might want to:
  1. Copy it first: "Let backup be copy of data. Give data..."
  2. Show it before giving: "Show data. Give data to processor."
  3. Only show it: "Show data to processor" (borrow, not give)
```

### The Failure Type

Functions that might fail return a `Result`:

```logos
## To divide (a: Int) and (b: Int) -> Result of Int:
    If b equals 0:
        Return Failure with message "Cannot divide by zero".
    Return Success with value a / b.

## Main
Let result be divide(10, 0).
Inspect result:
    When Success (value):
        Show "Result: " + value.
    When Failure (message):
        Show "Error: " + message.
```

### Error Propagation

Errors propagate naturally through return values. Handle them where appropriate.

---

## 14. Advanced Features

### Refinement Types

Refinement types add constraints to base types. The constraint is checked at runtime:

```logos
## Main
Let positive: Int where it > 0 be 5.
Let percentage: Int where it >= 0 and it <= 100 be 85.
```

If you try to assign a value that violates the constraint, you'll get an error.

### Assertions

Use `Assert` to verify conditions in your code:

```logos
## To divide_safe (a: Int) and (b: Int) -> Int:
    Assert that b is not 0.
    Return a / b.

## Main
Let result be divide_safe(10, 2).
Show result.  # Shows: 5
```

If the assertion fails, the program stops with an error message.

#### Trust with Reason

Use `Trust` when you know something is true but the compiler can't verify it:

```logos
## Main
Let user_input be 42.  # Assume this came from user
Trust that user_input is greater than 0 because "validated by UI".
```

The `because` clause documents why you believe the condition holds.

### Modules

Organize code across multiple files with `Use`:

```logos
# In main file:
Use Math.

## Main
Let result be Math's square(5).
Show result.
```

The module system allows you to:
- Split large programs into manageable pieces
- Reuse code across projects
- Control what's public and private

---

# Part II: Project Structure

---

## 15. Modules

Organize large programs across multiple files using the module system.

### Importing Modules

Use `Use` to import a module:

```logos
Use Math.
Use Utils.
Use Data.Structures.

## Main
Let result be Math's square(5).
Let formatted be Utils's format(result).
Show formatted.
```

### Qualified Access

Access module contents with the possessive `'s`:

```logos
Use Math.

## Main
Let x be Math's pi.
Let y be Math's sqrt(16).
Let z be Math's sin(x).
```

### Creating Modules

Each `.md` file is a module. The filename becomes the module name:

**Math.md:**
```logos
# Math

## Definition
Let pi be 3.14159.

## To square (n: Int) -> Int:
    Return n * n.

## To sqrt (n: Float) -> Float:
    # Implementation
```

**Main.md:**
```logos
# Main

Use Math.

## Main
Show Math's square(5).
```

### Visibility

By default, all definitions are public. Mark fields private with no `public` modifier:

```logos
## Definition
A Counter has:
    a public value: Int.      # Accessible from other modules
    a internal_state: Int.    # Only accessible within this module
```

---

## 16. The CLI: largo

LOGOS projects are built with `largo`, the LOGOS build tool.

### Creating a Project

```bash
largo new myproject
cd myproject
```

This creates:
```
myproject/
├── Largo.toml
└── src/
    └── main.md
```

### Project Manifest

**Largo.toml:**
```toml
[package]
name = "myproject"
version = "1.0.0"
author = "Your Name"

[dependencies]
std = "1.0"
```

### Build Commands

| Command | Description |
|---------|-------------|
| `largo build` | Compile the project |
| `largo build --release` | Compile with optimizations |
| `largo run` | Build and run |
| `largo check` | Type check without compiling |
| `largo test` | Run tests |

### Example Workflow

```bash
# Create new project
largo new calculator
cd calculator

# Edit src/main.md
# ... write your code ...

# Check for errors
largo check

# Run in development mode
largo run

# Build release version
largo build --release
```

### Adding Dependencies

Edit `Largo.toml`:
```toml
[dependencies]
std = "1.0"
json = "2.1"
http = "1.0"
```

Then import in your code:
```logos
Use Json.
Use Http.

## Main
Let data be Json's parse(text).
```

---

## 17. Standard Library

> **Note:** The standard library is not yet fully implemented. The examples below show the planned API, but most of these modules are not yet available. This section documents the intended design.

LOGOS includes a standard library with common functionality.

### File Operations

```logos
Use File.

## Main
# Read entire file
Let contents be File's read("input.txt").

# Write to file
File's write("output.txt", "Hello, World!").

# Check if file exists
If File's exists("config.json"):
    Let config be File's read("config.json").
```

### Time Operations

```logos
Use Time.

## Main
# Get current timestamp
Let now be Time's now().

# Sleep (pause execution)
Time's sleep(1000).  # milliseconds

# Measure duration
Let start be Time's now().
do_work().
Let end be Time's now().
Show "Elapsed: " + (end - start) + "ms".
```

### Random Numbers

```logos
Use Random.

## Main
# Random integer in range [min, max]
Let dice be Random's randomInt(1, 6).

# Random float in range [0, 1)
Let probability be Random's randomFloat().

# Random element from list
Let items be ["apple", "banana", "cherry"].
Let pick be Random's choice(items).
```

### Environment

```logos
Use Env.

## Main
# Get environment variable
Let home be Env's get("HOME").
Let path be Env's get("PATH").

# Get command-line arguments
Let args be Env's args().
Repeat for arg in args:
    Show "Argument: " + arg.
```

### Console I/O

```logos
## Main
# Output (you've seen this)
Show "Hello!".

# Input
Let name be read_line("Enter your name: ").
Show "Hello, " + name + "!".
```

### Native Functions

Define bindings to external code:

```logos
## To native sqrt (n: Float) -> Float
## To native sin (angle: Float) -> Float
## To native cos (angle: Float) -> Float

## Main
Let x be sqrt(16.0).  # 4.0
Let y be sin(3.14159 / 2).  # ~1.0
```

---

# Part III: Logic Mode

---

## 18. Logic Mode

LOGOS can translate English sentences into First-Order Logic (FOL). This is useful for formal verification, knowledge representation, and understanding the logical structure of natural language.

### Introduction to Formal Logic

Logic Mode takes English sentences and produces formal logical notation:

| Input | Output |
|-------|--------|
| "All cats are mammals." | `∀x(Cat(x) → Mammal(x))` |
| "Some dogs bark." | `∃x(Dog(x) ∧ Bark(x))` |
| "John runs." | `Run(j)` |

The symbols:
- `∀` — "for all" (universal quantifier)
- `∃` — "there exists" (existential quantifier)
- `→` — "implies" (if...then)
- `∧` — "and" (conjunction)
- `∨` — "or" (disjunction)
- `¬` — "not" (negation)

### Quantifiers

Quantifiers express claims about all or some members of a group.

#### Universal Quantifier (All)

```
Input:  "All birds fly."
Output: ∀x(Bird(x) → Fly(x))
```

Read as: "For all x, if x is a bird, then x flies."

Trigger words: `all`, `every`, `each`

```
"Every student studies."     → ∀x(Student(x) → Study(x))
"Each dog barks."            → ∀x(Dog(x) → Bark(x))
"All philosophers think."    → ∀x(Philosopher(x) → Think(x))
```

#### Existential Quantifier (Some)

```
Input:  "Some cats sleep."
Output: ∃x(Cat(x) ∧ Sleep(x))
```

Read as: "There exists an x such that x is a cat and x sleeps."

Trigger words: `some`, `a`, `an`

```
"Some dogs run."         → ∃x(Dog(x) ∧ Run(x))
"A student passed."      → ∃x(Student(x) ∧ Pass(x))
"An apple fell."         → ∃x(Apple(x) ∧ Fall(x))
```

#### Negative Quantifier (No)

```
Input:  "No fish fly."
Output: ∀x(Fish(x) → ¬Fly(x))
```

Read as: "For all x, if x is a fish, then x does not fly."

Or equivalently: `¬∃x(Fish(x) ∧ Fly(x))` — "There does not exist a fish that flies."

```
"No cats bark."          → ∀x(Cat(x) → ¬Bark(x))
"No student failed."     → ∀x(Student(x) → ¬Fail(x))
```

#### Generic Quantifier (Bare Plurals)

Bare plurals (nouns without determiners) express generalizations:

```
Input:  "Birds fly."
Output: Gen x(Bird(x) → Fly(x))
```

The `Gen` (generic) quantifier captures law-like generalizations that admit exceptions. Birds fly as a general rule, even though penguins don't.

```
"Dogs bark."             → Gen x(Dog(x) → Bark(x))
"Cats hunt mice."        → Gen x(Cat(x) → Hunt(x, mice))
```

### Logical Connectives

#### Conjunction (And)

```
Input:  "John runs and Mary walks."
Output: Run(j) ∧ Walk(m)
```

Both parts must be true.

#### Disjunction (Or)

```
Input:  "John runs or Mary walks."
Output: Run(j) ∨ Walk(m)
```

At least one part must be true.

#### Negation (Not)

```
Input:  "John does not run."
Output: ¬Run(j)
```

The statement is false.

#### Implication (If...Then)

```
Input:  "If John runs, then Mary walks."
Output: Run(j) → Walk(m)
```

When the first part is true, the second must also be true.

#### Biconditional (If and Only If)

```
Input:  "John runs if and only if Mary walks."
Output: Run(j) ↔ Walk(m)
```

Both parts have the same truth value.

### Modal Operators

Modals express possibility, necessity, and obligation.

#### Possibility (Can, Might, May)

```
Input:  "John can swim."
Output: ◇Swim(j)
```

The diamond `◇` means "it is possible that."

```
"John might leave."      → ◇Leave(j)
"Mary may enter."        → ◇Enter(m)
```

#### Necessity (Must)

```
Input:  "John must leave."
Output: □Leave(j)
```

The box `□` means "it is necessary that."

```
"All men must die."      → ∀x(Man(x) → □Die(x))
```

### Tense and Aspect

LOGOS tracks temporal information:

#### Past Tense

```
Input:  "John ran."
Output: PAST(Run(j))
```

#### Future Tense

```
Input:  "John will run."
Output: FUT(Run(j))
```

#### Progressive Aspect

```
Input:  "John is running."
Output: PROG(Run(j))
```

Indicates ongoing action.

#### Perfect Aspect

```
Input:  "John has run."
Output: PERF(Run(j))
```

Indicates completed action with current relevance.

#### Complex Tenses

Tenses can combine:

```
Input:  "John had been running."
Output: PAST(PERF(PROG(Run(j))))
```

### Wh-Questions

Questions produce lambda expressions:

```
Input:  "Who loves Mary?"
Output: λx.Love(x, m)
```

Read as: "The set of x such that x loves Mary."

```
Input:  "What does John love?"
Output: λx.Love(j, x)

Input:  "Where did John go?"
Output: λx.Go(j, x)
```

### Ambiguity and Multiple Readings

Natural language is often ambiguous. LOGOS can produce multiple readings.

#### Scope Ambiguity

```
Input: "Every woman loves a man."

Reading 1 (Surface Scope):
∀x(Woman(x) → ∃y(Man(y) ∧ Love(x, y)))
"Each woman loves some man (possibly different men)"

Reading 2 (Inverse Scope):
∃y(Man(y) ∧ ∀x(Woman(x) → Love(x, y)))
"There is one specific man whom every woman loves"
```

Use `compile_all_scopes()` to get all readings.

#### Structural Ambiguity

```
Input: "I saw the man with the telescope."

Reading 1 (Instrument):
∃e(See(e) ∧ Agent(e, i) ∧ Theme(e, m) ∧ Instrument(e, t))
"I used the telescope to see him"

Reading 2 (Modifier):
∃e(See(e) ∧ Agent(e, i) ∧ Theme(e, m)) ∧ With(m, t)
"I saw the man who has the telescope"
```

#### Lexical Ambiguity

```
Input: "I saw her duck."

Reading 1 (Noun):
See(i, duck_of_her)
"I saw her pet duck"

Reading 2 (Verb):
See(i, [Duck(her)])
"I saw her perform a ducking motion"
```

---

# Part IV: Proofs and Verification

---

## 19. Assertions and Trust

LOGOS bridges imperative programming with formal verification through assertions and proof statements.

### Assert

Use `Assert` to verify conditions at runtime:

```logos
## To withdraw (amount: Int) from (balance: Int) -> Int:
    Assert that amount is greater than 0.
    Assert that amount is at most balance.
    Return balance - amount.

## Main
Let result be withdraw(50, 100).
Show result.  # Shows: 50
```

If an assertion fails, the program stops with a clear error message.

### Trust with Justification

Use `Trust` for conditions the compiler can't verify automatically:

```logos
## To process_positive (n: Int) -> Int:
    Trust that n is greater than 0 because "caller guarantees positive input".
    Return n * 2.
```

The `because` clause is **mandatory**. It documents your reasoning:

```logos
Trust that the list is sorted because "we just called sort()".
Trust that pointer is valid because "allocated in parent scope".
Trust that n > 0 because "validated at API boundary".
```

### Trust Generates Debug Assertions

In development builds, `Trust` becomes a `debug_assert!`:

```logos
Trust that x > 0 because "loop invariant".
# Compiles to: debug_assert!(x > 0, "loop invariant");
```

In release builds, it generates no code—the trust is assumed.

### Auditing Trust Statements

Find all trust statements in your codebase:

```bash
largo audit    # Lists all Trust statements with justifications
```

This helps review assumptions during code review.

### Proof Blocks (Advanced)

For formal verification, use theorem blocks:

```logos
> **Theorem:** For all natural numbers n, factorial(n) > 0.
> *Proof:* By induction on n.
> Base case: factorial(0) = 1 > 0. ✓
> Inductive case: factorial(n+1) = (n+1) * factorial(n).
> By IH, factorial(n) > 0, and n+1 > 0, so their product > 0. ✓
```

Proof blocks are documentation that the compiler can optionally verify.

---

## 20. Z3 Static Verification

LOGOS can use the Z3 SMT solver to verify refinement types at compile time.

### What is Z3?

Z3 is a theorem prover. Instead of checking constraints at runtime, Z3 proves (or disproves) them at compile time:

| Approach | When Checked | If Violated |
|----------|--------------|-------------|
| Runtime assertion | When code runs | Program crashes |
| Z3 verification | At compile time | Compilation fails |

### Refinement Types with Static Checking

```logos
## Main
Let positive: Int where it > 0 be 5.      # OK: 5 > 0 ✓
Let negative: Int where it > 0 be -3.     # ERROR at compile time!
```

**Compile error:**
```
Refinement type violation: Cannot prove (-3 > 0).

  2 │ Let negative: Int where it > 0 be -3.
    │                              ^^^^
    │
  The constraint 'it > 0' is not satisfied by the value -3.
```

### Variable Tracking

Z3 tracks constraints through variable assignments:

```logos
## Main
Let x be 10.
Let y: Int where it > 5 be x.  # OK: Z3 knows x = 10 > 5

Set x to 3.
Let z: Int where it > 5 be x.  # ERROR: 3 is not > 5
```

### Compound Predicates

Multiple constraints:

```logos
## Main
Let percentage: Int where it >= 0 and it <= 100 be 85.  # OK
Let invalid: Int where it >= 0 and it <= 100 be 150.    # ERROR
```

### Function Preconditions

Z3 verifies function contracts:

```logos
## To divide (a: Int) and (b: Int where it is not 0) -> Int:
    Return a / b.

## Main
Let x be divide(10, 2).   # OK: 2 ≠ 0
Let y be divide(10, 0).   # ERROR: Precondition violated
```

### Enabling Z3 Verification

Z3 verification is opt-in. Enable it in your build:

```bash
largo build --verify    # Enable Z3 checking
```

Or in Largo.toml:
```toml
[build]
verify = true
```

### What Z3 Can Prove

| Constraint Type | Example | Z3 Support |
|-----------------|---------|------------|
| Integer bounds | `it > 0`, `it < 100` | Full |
| Equality | `it == 5` | Full |
| Arithmetic | `it * 2 < 100` | Full |
| Boolean logic | `it > 0 and it < 10` | Full |
| Array bounds | `index < length` | Partial |
| Complex invariants | Custom properties | Varies |

### Graceful Degradation

When Z3 can't prove something, you can use `Trust`:

```logos
## Main
Let value be complex_computation().
Trust that value > 0 because "algorithm guarantees positive result".
Let result: Int where it > 0 be value.
```

---

# Part V: Reference

---

## 21. Complete Examples

### Mergesort

A complete, recursive sorting algorithm in LOGOS:

```logos
## To Merge (left: Seq of Int) and (right: Seq of Int) -> Seq of Int:
    Let result be a new Seq of Int.
    Let i be 1.
    Let j be 1.
    Let n_left be length of left.
    Let n_right be length of right.

    While i is at most n_left and j is at most n_right:
        Let l_val be item i of left.
        Let r_val be item j of right.

        If l_val is less than r_val:
            Push l_val to result.
            Set i to i + 1.
        Otherwise:
            Push r_val to result.
            Set j to j + 1.

    While i is at most n_left:
        Let v be item i of left.
        Push v to result.
        Set i to i + 1.

    While j is at most n_right:
        Let v be item j of right.
        Push v to result.
        Set j to j + 1.

    Return result.

## To MergeSort (items: Seq of Int) -> Seq of Int:
    Let n be length of items.
    If n is less than 2:
        Return copy of items.

    Let mid be n / 2.
    Let left_slice be items 1 through mid.
    Let right_slice be items (mid + 1) through n.

    Let sorted_left be MergeSort(copy of left_slice).
    Let sorted_right be MergeSort(copy of right_slice).

    Return Merge(sorted_left, sorted_right).

## Main
    Let numbers be a new Seq of Int.
    Push 3 to numbers.
    Push 1 to numbers.
    Push 4 to numbers.
    Push 1 to numbers.
    Push 5 to numbers.

    Let sorted be MergeSort(numbers).
    Show sorted.
```

Output: `[1, 1, 3, 4, 5]`

### Factorial

Classic recursive example:

```logos
## To factorial (n: Int) -> Int:
    If n is at most 1:
        Return 1.
    Return n * factorial(n - 1).

## Main
Let result be factorial(5).
Show result.  # Shows: 120
```

### Working with Structs

A complete example with custom types:

```logos
## Definition
A Person has:
    a name: Text.
    an age: Int.

## To create_person (name: Text) and (age: Int) -> Person:
    Return a new Person with name name and age age.

## To birthday (p: Person) -> Person:
    Return a new Person with name p's name and age (p's age + 1).

## To introduce (p: Person) -> Text:
    Return "Hi, I'm " + p's name + " and I'm " + p's age + " years old.".

## Main
Let alice be create_person("Alice", 25).
Show introduce(alice).

Let older_alice be birthday(alice).
Show introduce(older_alice).
```

Output:
```
Hi, I'm Alice and I'm 25 years old.
Hi, I'm Alice and I'm 26 years old.
```

### Collection Processing

Common patterns for working with collections:

```logos
## To sum (numbers: Seq of Int) -> Int:
    Let total be 0.
    Repeat for n in numbers:
        Set total to total + n.
    Return total.

## To filter_positive (numbers: Seq of Int) -> Seq of Int:
    Let result be a new Seq of Int.
    Repeat for n in numbers:
        If n is greater than 0:
            Push n to result.
    Return result.

## To map_double (numbers: Seq of Int) -> Seq of Int:
    Let result be a new Seq of Int.
    Repeat for n in numbers:
        Push n * 2 to result.
    Return result.

## Main
Let data be [-2, 5, -1, 8, 3, -4, 7].

Let positives be filter_positive(data).
Show "Positives: " + positives.

Let doubled be map_double(positives).
Show "Doubled: " + doubled.

Let total be sum(doubled).
Show "Sum: " + total.
```

Output:
```
Positives: [5, 8, 3, 7]
Doubled: [10, 16, 6, 14]
Sum: 46
```

---

## 22. Quick Reference

### Syntax Cheat Sheet

#### Variables
```logos
Let x be 5.                    # Create variable
Set x to 10.                   # Change variable
Let x: Int be 5.               # With type annotation
```

#### Control Flow
```logos
If condition:                  # Conditional
    ...
Otherwise:
    ...

While condition:               # While loop
    ...

Repeat for item in items:      # For-each loop
    ...

Return value.                  # Return from function
```

#### Functions
```logos
## To name (param: Type) -> ReturnType:
    ...

## To name (a: Int) and (b: Int) -> Int:
    ...
```

#### Structs
```logos
## Definition
A TypeName has:
    a field1: Type1.
    a field2: Type2.

Let x be a new TypeName with field1 value1 and field2 value2.
Let v be x's field1.
Set x's field1 to new_value.
```

#### Enums
```logos
## Definition
A TypeName is either:
    a Variant1 with field: Type.
    a Variant2.

Inspect x:
    When Variant1:
        ...
    When Variant2:
        ...
```

#### Collections
```logos
Let items be [1, 2, 3].        # List literal
Let items be a new Seq of Int. # Empty list

Push value to items.           # Add to end
Pop from items into x.         # Remove from end

Let v be item 1 of items.      # Access (1-indexed)
Let len be length of items.    # Get length
Let slice be items 2 through 4.# Slice (inclusive)
Let dup be copy of items.      # Deep copy
```

### Type Reference

| Type | Description | Example Values |
|------|-------------|----------------|
| `Int` | Integer | `5`, `-10`, `0` |
| `Bool` | Boolean | `true`, `false` |
| `Text` | String | `"Hello"` |
| `Float` | Decimal | `3.14`, `-0.5` |
| `Seq of T` | List/Sequence | `[1, 2, 3]` |

### Operator Reference

#### Arithmetic
| Operator | Symbol | English |
|----------|--------|---------|
| Add | `+` | `plus` |
| Subtract | `-` | `minus` |
| Multiply | `*` | `times` |
| Divide | `/` | `divided by` |
| Modulo | `%` | `modulo` |

#### Comparison
| Operator | Symbol | English |
|----------|--------|---------|
| Less than | `<` | `is less than` |
| Greater than | `>` | `is greater than` |
| Less or equal | `<=` | `is at most` |
| Greater or equal | `>=` | `is at least` |
| Equal | `==` | `equals` |
| Not equal | `!=` | `is not` |

#### Logical
| Operator | Keyword |
|----------|---------|
| AND | `and` |
| OR | `or` |
| NOT | `not` |

### Logic Mode Reference

#### Quantifiers
| English | Symbol | Output |
|---------|--------|--------|
| All X are Y | `∀` | `∀x(X(x) → Y(x))` |
| Some X is Y | `∃` | `∃x(X(x) ∧ Y(x))` |
| No X is Y | `¬∃` | `¬∃x(X(x) ∧ Y(x))` |

#### Connectives
| English | Symbol |
|---------|--------|
| and | `∧` |
| or | `∨` |
| not | `¬` |
| if...then | `→` |
| if and only if | `↔` |

#### Modals
| English | Symbol |
|---------|--------|
| can, may, might | `◇` |
| must | `□` |

### Ownership Reference

#### Ownership Verbs
| Verb | Meaning | Result |
|------|---------|--------|
| `Give x to f.` | Move | Transfers ownership, `x` unusable after |
| `Show x to f.` | Borrow | Temporary read access, `x` still yours |
| `Let f modify x.` | Mutable Borrow | Temporary write access |
| `copy of x` | Clone | Creates independent copy |

#### Ownership Patterns
```logos
Give data to processor.        # Move - can't use 'data' after
Show data to logger.           # Borrow - 'data' still usable
Let sorter modify items.       # Mutable borrow - temporary change
Let backup be copy of data.    # Clone - independent copy
```

### Zone Reference

#### Zone Types
| Type | Syntax | Access |
|------|--------|--------|
| Heap | `Inside a zone called "X":` | Read/Write |
| Heap sized | `Inside a zone called "X" of size 64 KB:` | Read/Write |
| Mapped | `Inside a zone called "X" mapped from "file":` | Read-only |

#### Zone Syntax
```logos
Inside a zone called "Name":
    # Heap zone - freed when block exits

Inside a zone called "Buffer" of size 64 KB:
    # Pre-allocated heap zone

Inside a zone called "Data" mapped from "file.bin":
    Let bytes be zone's as_slice().  # Read-only access
```

#### Zone Rules
| Rule | Effect |
|------|--------|
| References can't escape | Compile error if you try |
| Primitives can escape | Copied automatically |
| Explicit copy can escape | `a copy of x` works |
| Inner → outer forbidden | Nested zones are strict |
| Mapped zones read-only | Cannot allocate or modify |

### Concurrency Reference

#### Concurrent Patterns
```logos
Attempt all of the following:     # Async I/O (tokio::join!)
    Task 1.
    Task 2.

Simultaneously:                    # Parallel CPU (rayon/threads)
    Compute A.
    Compute B.

Await the first success of:        # Race - first wins
    Query A.
    Query B.

Await result or timeout after N seconds.  # Timeout
```

#### Streams
```logos
Create a stream called "Name".
Pour value into "Name".
Spawn a task consuming "Name":
    Repeat for item in "Name":
        Process item.
```

### Project Reference

#### largo Commands
| Command | Description |
|---------|-------------|
| `largo new name` | Create project |
| `largo build` | Compile |
| `largo build --release` | Optimized build |
| `largo run` | Build and run |
| `largo check` | Type check only |
| `largo test` | Run tests |
| `largo audit` | List Trust statements |

#### Module Syntax
```logos
Use ModuleName.                    # Import module
Let x be ModuleName's function().  # Qualified access
```

### Verification Reference

#### Assertions
```logos
Assert that condition.                              # Runtime check
Trust that P because "reason".                      # Assumed true
Let x: Int where it > 0 be 5.                      # Refinement type
Let x: Int where it >= 0 and it <= 100 be 50.     # Compound
```

#### Z3 Constraints
| Constraint | Syntax | Z3 Support |
|------------|--------|------------|
| Bounds | `it > 0`, `it < 100` | Full |
| Equality | `it == 5` | Full |
| Arithmetic | `it * 2 < 100` | Full |
| Logic | `it > 0 and it < 10` | Full |

---

*LOGOS: Where natural language meets formal logic meets executable code.*
