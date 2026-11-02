# Hover

## 0:0 — the ## Note header
    **Block Header** — Documentation prose the compiler skips and the highlighter fades.
    
    ```
    ## Note
    This module parses dates.
    ```
    
    Tip: a `## Note` right above a definition becomes that definition's documentation.

## 11:0 — the Let keyword
    **Let**
    
    Declares a new variable.
    
    ```
    Let x be 5.
    Let name: Text be "Alice".
    ```
    
    Will the value change later? Then declare it `Let mutable x be 5.` — plain `Let` is immutable.
    
    [Quick Guide](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LOGOS_QUICKGUIDE.md#2-variables--mutation)

## 11:14 — the documented call to double
    To double(n: Int) -> Int
    
    Doubles a number.

## 13:0 — the Set keyword
    **Set**
    
    Updates an existing mutable variable.
    
    ```
    Set x to 10.
    ```
    
    Was `x` declared with `Let mutable`? Only mutable bindings can be `Set`.
    
    [Quick Guide](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LOGOS_QUICKGUIDE.md#2-variables--mutation)

## 14:12 — the stdlib name md5
    **md5** — standard library
    
    ```
    ## To md5 (data: Seq of Int) -> Seq of Int:
    ```
    
    The MD5 digest of a byte sequence.

## 6:5 — the Point struct name
    Point (struct)

## 18:0 — the ## Theorem header
    **Block Header** — Declares a proposition to be proved.
    
    ```
    ## Theorem: Socrates
    Given: All men are mortal. Socrates is a man.
    Prove: Socrates is mortal.
    Proof: Auto.
    ```
    
    What structure does the claim have — universal, implication, equality? The proof strategy follows it.
    
    [Quick Guide](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LOGOS_QUICKGUIDE.md#1-program-structure)
    
    ---
    
    **Proof Strategy**: Your theorem involves a universal claim. To prove it, consider an arbitrary element and show the property holds.

# Completions — statement context (after Show answer.)
- Let [Keyword] — Declares a new variable. — doc:yes
- Set [Keyword] — Updates an existing mutable variable. — doc:yes
- If [Keyword] — Runs a block only when its condition holds. — doc:yes
- While [Keyword] — Repeats a block as long as its condition stays true. — doc:yes
- Repeat [Keyword] — Walks a collection, binding each element in turn. — doc:yes
- Return [Keyword] — Hands a value back from the current function. — doc:yes
- Show [Keyword] — Displays a value while only borrowing it — you keep ownership. — doc:yes
- Give [Keyword] — Transfers ownership of a value to a new owner. — doc:yes
- Push [Keyword] — Appends a value to the end of a sequence. — doc:yes
- Call [Keyword] — Invokes a function as a statement. — doc:yes
- Inspect [Keyword] — Pattern-matches a value, running one branch per variant. — doc:yes

# Completions — expression context (after be)
- double [Function] — To double(n: Int) -> Int — doc:yes
- answer [Variable] — Let answer: double(..) (inferred) — doc:NO
- md5 [Function] — To md5 (data: Seq of Int) -> Seq of Int: — doc:yes
- Message [Class] — A Message has: — doc:yes

# Signature help — Call double with 7
label: To double(n: Int) -> Int | active: 0 | doc: Doubles a number.

# Code lenses
- line 10: Run (logicaffeine.run)
- line 18: Verify (logicaffeine.verify)
- line 18: Prove (logicaffeine.prove)

# Quickfixes

## zero-index
- Use 1-based indexing

## use-after-move
- Use 'a copy of x' instead

## unused-variable
- Remove unused 'unused'

# Diagnostic docs links (code → quickguide anchor)
- undefined-variable → #2-variables--mutation
- use-after-move → #13-output
- is-value-equality → #3-arithmetic-comparison-logic-bitwise
- zero-index → #5-collections
- type-mismatch → #2-variables--mutation
- type-mismatch → #2-variables--mutation
- arity-mismatch → #7-functions--closures
- field-not-found → #8-structs-enums--field-access
- not-a-function → #7-functions--closures
