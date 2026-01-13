# Logos Mathematical Capabilities

Logos provides comprehensive support for mathematical expressions, from basic arithmetic to cardinality quantifiers and dimensional analysis.

## Arithmetic Operators

| Operator | Symbol | English | Example |
|----------|--------|---------|---------|
| Addition | `+` | `plus` | `2 + 3` |
| Subtraction | `-` | `minus` | `10 - 4` |
| Multiplication | `*` | `times` | `3 * 4` |
| Division | `/` | `divided by` | `10 / 2` |
| Modulo | `%` | `mod` | `7 % 3` |

Division is integer division (truncates toward zero): `7 / 2 = 3`

### Operator Precedence

Standard mathematical precedence applies:

1. Parentheses `()` — highest
2. Unary minus `-x`
3. Multiplication, Division, Modulo `* / %`
4. Addition, Subtraction `+ -` — lowest

```
2 + 3 * 4     → 14   (multiplication first)
(2 + 3) * 4   → 20   (parentheses override)
10 - 5 - 2    → 3    (left-to-right associativity)
```

### Negation

Unary minus is supported via subtraction from zero:

```
-5    → 0 - 5 = -5
```

## Numeric Literals

### Integers

Standard integer literals with optional underscore separators:

```
42
1000
1_000_000
```

### Floating-Point

Decimal numbers are recognized when a digit appears on both sides of the period:

```
3.14
2.718
0.5
```

### Symbolic Constants

Mathematical constants using subscript notation:

```
aleph_0     → ℵ₀ (smallest infinite cardinal)
omega_1     → ω₁ (first uncountable ordinal)
beth_2      → ℶ₂ (beth number)
```

### English Cardinal Words

Natural language number words convert to numeric values:

| Word | Value |
|------|-------|
| `zero` | 0 |
| `one` | 1 |
| `two` | 2 |
| `three` | 3 |
| `four` | 4 |
| `five` | 5 |
| `six` | 6 |
| `seven` | 7 |
| `eight` | 8 |
| `nine` | 9 |
| `ten` | 10 |
| `twenty` | 20 |
| `hundred` | 100 |

## Comparison Operators

| Operator | Symbol | English | Description |
|----------|--------|---------|-------------|
| Less than | `<` | `is less than` | Strict inequality |
| Greater than | `>` | `is greater than` | Strict inequality |
| Less or equal | `<=` | `is at most` | Non-strict inequality |
| Greater or equal | `>=` | `is at least` | Non-strict inequality |
| Equality | `==` | `equals`, `is equal to` | Exact match |
| Inequality | `!=` | `is not` | Negated equality |

### Symbolic vs English

Both forms are interchangeable:

```
x < 5           ≡  x is less than 5
y >= 10         ≡  y is at least 10
z == 0          ≡  z equals 0
```

## Quantified Cardinals

Cardinal quantifiers specify exact counts in logical statements.

### Exact Count

```
Five students passed.
→ ∃⁵x(Student(x) ∧ Passed(x))
```

### At Least

```
At least three people attended.
→ ∃≥³x(Person(x) ∧ Attended(x))
```

### At Most

```
At most ten items remain.
→ ∃≤¹⁰x(Item(x) ∧ Remain(x))
```

## Dimensions and Units

Logos supports typed numeric values with physical dimensions.

### Dimension Types

| Dimension | Description | Examples |
|-----------|-------------|----------|
| `Length` | Spatial distance | meters, feet, miles |
| `Time` | Temporal duration | seconds, minutes, hours |
| `Weight` | Mass | kilograms, pounds |
| `Temperature` | Thermal measure | Celsius, Fahrenheit |
| `Cardinality` | Count/set size | items, members |

### Value with Unit

Numeric values can carry unit annotations:

```
10 meters
5 seconds
2.5 kilograms
98.6 Fahrenheit
```

## Group Quantifiers

Group quantifiers handle collective readings with cardinality constraints.

### Syntax

```
The three dogs bark.
```

### Semantics

Transpiles to a group-based quantification:

```
∃g(Group(g) ∧ Count(g, 3) ∧ ∀x(Member(x, g) → Dog(x)) ∧ Bark(g))
```

This captures:
- Existence of a group `g`
- The group has exactly 3 members
- All members satisfy the restriction (are dogs)
- The predicate applies to the group collectively

## Number Types in AST

The internal representation distinguishes three number kinds:

```rust
pub enum NumberKind {
    Real(f64),        // Floating-point: 3.14
    Integer(i64),     // Integer: 42
    Symbolic(Symbol), // Symbolic: aleph_0
}
```

## Output Formats

Mathematical expressions transpile to multiple formats:

### Unicode

```
∀x(x ≥ 0 → ∃y(y² = x))
```

### LaTeX

```latex
\forall x(x \geq 0 \rightarrow \exists y(y^2 = x))
```

### SimpleFOL

```
forall x (x >= 0 implies exists y (y^2 = x))
```

## Examples

### Basic Arithmetic

```
2 + 3           → 5
10 - 4          → 6
3 * 4           → 12
10 / 2          → 5
7 % 3           → 1
```

### Nested Expressions

```
(2 + 3) * 4     → 20
10 + 5 * 2 - 3  → 17
```

### Comparisons with Arithmetic

```
2 + 2 == 4      → true
5 > 3           → true
10 <= 10        → true
```

### Quantified Statements

```
Three cats sleep.
→ ∃³x(Cat(x) ∧ Sleep(x))

At least two students study logic.
→ ∃≥²x(Student(x) ∧ Study(x, logic))

At most five errors occurred.
→ ∃≤⁵x(Error(x) ∧ Occurred(x))
```
