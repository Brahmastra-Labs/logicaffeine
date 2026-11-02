# LOGOS Error Catalog

Every diagnostic the LOGOS toolchain emits, keyed by its stable
`logos-diagnostic/1` code — the `code` field of `largo check --json`,
`largo run --json`, and the MCP `logos_check` tool. Look a code up here
or with `largo explain <code>`.

Generated from `logicaffeine_language::diagnostic::catalog()`. Do not
edit by hand; run `./scripts/release/gen-error-catalog.sh` after changing
`ParseErrorKind`.

## `unexpected-token`

- **Severity**: error
- **Variant**: `UnexpectedToken`

**Example**:

```text
expected a period ('.'), found a comma (',')
```

> I was following your sentence but stumbled here: I expected a period ('.'), and found a comma (',') instead. The structure so far commits me to a period ('.') next. Is a word missing just before this, or did two thoughts run together? Splitting the sentence in two often shows which.

## `expected-content-word`

- **Severity**: error
- **Variant**: `ExpectedContentWord`

**Example**:

```text
expected a content word, found a comma (',')
```

> This spot needs a content word — a noun, a verb, or an adjective — but I found a comma (',') instead. Content words carry the meaning; the little words only arrange them. What thing, action, or quality did you mean to name here? Put that word in and the sentence grounds itself.

## `expected-copula`

- **Severity**: error
- **Variant**: `ExpectedCopula`

**Example**:

```text
expected 'is' or 'are'
```

> The subject here is waiting for 'is' or 'are' to link it to its predicate. English predication runs through a copula — without one, the two halves never connect. What is the subject supposed to BE? Write it as 'X is Y' ('are' for plurals).

## `unknown-quantifier`

- **Severity**: error
- **Variant**: `UnknownQuantifier`

**Example**:

```text
expected a quantifier, found a comma (',')
```

> I needed a quantifier here — a word like 'all', 'some', or 'no' — but found a comma (','). Quantifiers say how many things the sentence talks about, and that choice decides the whole logical shape. How many did you mean: every one, at least one, or none? Lead with the word that says so.

## `unknown-modal`

- **Severity**: error
- **Variant**: `UnknownModal`

**Example**:

```text
expected a modal, found a comma (',')
```

> I needed a modal here — 'must', 'can', 'may', 'should' — but found a comma (','). Modals set the strength of the claim: necessity, possibility, permission, obligation. Which strength did you mean? Pick the modal that carries it and the logic follows.

## `expected-verb`

- **Severity**: error
- **Variant**: `ExpectedVerb`

**Example**:

```text
expected a verb, found a comma (',')
```

> Every sentence needs a verb, and this one's verb should appear here — I found a comma (',') instead. The verb names the action or state everything else hangs on. What is the subject doing (or being)? Give the sentence that verb.

## `expected-temporal-adverb`

- **Severity**: error
- **Variant**: `ExpectedTemporalAdverb`

**Example**:

```text
expected a temporal adverb
```

> I expected a time word here — 'yesterday', 'tomorrow', 'always'. Temporal adverbs anchor the sentence on the timeline, and this construction promised one. When does this happen? Say it with a temporal adverb, or drop the construction that required one.

## `expected-presupposition-trigger`

- **Severity**: error
- **Variant**: `ExpectedPresuppositionTrigger`

**Example**:

```text
expected a presupposition trigger
```

> I expected a presupposition trigger here — a word like 'stopped', 'realized', or 'regrets'. These verbs quietly assume something was already true; that hidden assumption is what this construction works with. What background fact is being taken for granted? The trigger word is what carries it.

## `expected-focus-particle`

- **Severity**: error
- **Variant**: `ExpectedFocusParticle`

**Example**:

```text
expected a focus particle
```

> I expected a focus particle here — 'only', 'even', 'just'. Focus particles single out one part of the sentence against its alternatives. Which word should the emphasis land on? Put the particle directly before it.

## `expected-scopal-adverb`

- **Severity**: error
- **Variant**: `ExpectedScopalAdverb`

**Example**:

```text
expected a scopal adverb
```

> I expected a scopal adverb here — a word like 'necessarily' or 'possibly' that comments on the whole claim rather than the verb alone. Is the adverb meant to cover the entire sentence? If it only describes the action, it belongs next to the verb instead.

## `expected-superlative-adjective`

- **Severity**: error
- **Variant**: `ExpectedSuperlativeAdjective`

**Example**:

```text
expected a superlative adjective
```

> I expected a superlative here — 'tallest', 'fastest', the '-est' form. A superlative ranks one thing against all the others, and that is the comparison this sentence set up. Are you comparing against everything, or just one other thing? Use '-est' for everything, '-er than' for one.

## `expected-comparative-adjective`

- **Severity**: error
- **Variant**: `ExpectedComparativeAdjective`

**Example**:

```text
expected a comparative adjective
```

> I expected a comparative here — 'taller', 'faster', the '-er' form. A comparative weighs exactly two things against each other. What quality are the two compared on? Name it in its '-er' form and follow with 'than'.

## `expected-than`

- **Severity**: error
- **Variant**: `ExpectedThan`

**Example**:

```text
expected 'than' to complete the comparison
```

> A comparative opened a comparison, and 'than' introduces the other side — but it is missing here. Taller than what, exactly? Add 'than' plus the thing being compared against.

## `expected-number`

- **Severity**: error
- **Variant**: `ExpectedNumber`

**Example**:

```text
expected a number
```

> This measure phrase needs a number — '2', '3.14' — and none appeared. A measure without a quantity measures nothing. How much, exactly? Put the number in front of the unit.

## `empty-restriction`

- **Severity**: error
- **Variant**: `EmptyRestriction`

**Example**:

```text
empty relative clause
```

> This relative clause is empty — 'the X that ...' with nothing after 'that'. A restriction narrows the noun down, so an empty one narrows nothing. Which ones did you mean? State the property that picks them out, or drop the 'that' entirely.

## `gapping-resolution-failed`

- **Severity**: error
- **Variant**: `GappingResolutionFailed`

**Example**:

```text
no verb to borrow for the gapped clause
```

> This reads like a gapped construction — '... and Mary, a pear' — where the verb is borrowed from the previous clause, but I found no verb there to borrow. What is the second subject doing? Either repeat the verb outright, or make sure the first clause states it plainly.

## `stative-progressive-conflict`

- **Severity**: error
- **Variant**: `StativeProgressiveConflict`

**Example**:

```text
stative verb used in the progressive
```

> A stative verb like 'know' or 'love' is in the progressive here, and states do not run in progress — they simply hold or they don't. Is this a state or an ongoing activity? States take the simple form ('knows'); only activities take '-ing'.

## `undefined-variable`

- **Severity**: error
- **Variant**: `UndefinedVariable`
- **Quickfix**: Did you mean '<nearest definition>'?
- **Guide**: <https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LOGOS_QUICKGUIDE.md#2-variables--mutation>

**Example**:

```text
'x' is not declared
```

> I found 'x', but nothing has declared it. Every variable starts life in a Let — reading a name before it exists gives me no value to read. Where should 'x' get its first value? Add 'Let x be ...' above this line, or check the spelling against the name you declared.

## `use-after-move`

- **Severity**: error
- **Variant**: `UseAfterMove`
- **Quickfix**: Use 'a copy of …' instead
- **Guide**: <https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LOGOS_QUICKGUIDE.md#13-output>

**Example**:

```text
cannot use 'x' after giving it away
```

> Cannot use 'x' after giving it away. 'Give x to ...' transferred ownership — 'x' belongs to the receiver now, and this later use has nothing left to hold. Who really needs to own 'x' here? To lend it and keep it, 'Show x'; to hand over a duplicate, give 'a copy of x'.

## `is-value-equality`

- **Severity**: warning
- **Variant**: `IsValueEquality`
- **Quickfix**: Use 'equals' for value comparison
- **Guide**: <https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LOGOS_QUICKGUIDE.md#3-arithmetic-comparison-logic-bitwise>

**Example**:

```text
'x is 5' compares values; use 'equals'
```

> 'x is 5' reads as a type or predicate claim — 'is' asks what something IS, not which value it holds. Value comparison is spelled 'equals'. Are you asking what 'x' is, or whether it holds 5? For the value question, write 'x equals 5'.

## `zero-index`

- **Severity**: info
- **Variant**: `ZeroIndex`
- **Quickfix**: Use 1-based indexing
- **Guide**: <https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LOGOS_QUICKGUIDE.md#5-collections>

**Example**:

```text
indices start at 1; there is no item 0
```

> This asks for item 0, but LOGOS indices start at 1 — in English, 'the 1st item' is the first one; there is no item 0. Which element did you want? The first is 'item 1 of xs' — the zero-based habit is the only thing to unlearn.

## `expected-statement`

- **Severity**: error
- **Variant**: `ExpectedStatement`

**Example**:

```text
expected a statement
```

> I expected a statement here — a sentence that does something, like 'Let', 'Set', 'Show', or 'Return'. What should happen at this point in the program? Start the line with the verb that does it, and end with a period.

## `expected-keyword`

- **Severity**: error
- **Variant**: `ExpectedKeyword`

**Example**:

```text
expected the word 'to'
```

> This construction needs the word 'to' here to complete its shape. Statement forms have fixed connecting words — they are the joints of the sentence. Which form did you start? Insert 'to' where I stopped, or compare your line against that form's example.

## `expected-expression`

- **Severity**: error
- **Variant**: `ExpectedExpression`

**Example**:

```text
expected an expression
```

> I expected an expression here — a value: a number, a variable, a call, or a computation. Something just before this (a 'be', a 'to', an operator) promised a value that never arrived. What value should flow into this spot? Write it, or remove the connector that promised it.

## `expected-identifier`

- **Severity**: error
- **Variant**: `ExpectedIdentifier`

**Example**:

```text
expected a name
```

> I expected a name here — an identifier for a variable, function, or field. This position says which thing the statement acts on, so a keyword or symbol cannot stand in. What is the thing called? Use its declared name, or declare it first with Let.

## `respectively-length-mismatch`

- **Severity**: error
- **Variant**: `RespectivelyLengthMismatch`

**Example**:

```text
'respectively' lists differ: 2 subject(s) vs 3 object(s)
```

> 'Respectively' pairs the two lists one-to-one, but the subject side has 2 element(s) and the object side has 3. With uneven lists, something ends up with no partner. Which pairing did you intend? Even the lists out, or split the sentence so each pairing is explicit.

## `type-mismatch`

- **Severity**: error
- **Variant**: `TypeMismatch`, `TypeMismatchDetailed`
- **Guide**: <https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LOGOS_QUICKGUIDE.md#2-variables--mutation>

**Example**:

```text
expected 'Int', found 'Text'
```

> This slot is typed 'Int', but the value here is 'Text'. LOGOS holds every value to its declared type — that agreement is what the later guarantees stand on. Which side is right, the annotation or the value? Change the one that is lying.

**Example**:

```text
expected 'Int', found 'Text' (in argument 2 of 'compute')
```

> I expected 'Int' here but found 'Text' (in argument 2 of 'compute'). The two sides of this position must agree on one type. Which side has it right? Adjust the annotation or the value so they tell the same story.

## `infinite-type`

- **Severity**: error
- **Variant**: `InfiniteType`

**Example**:

```text
infinite type: T would contain itself
```

> This would make an infinite type: T would have to equal Seq of T, which contains it — a type inside itself, forever. This usually means a value is being folded into its own container. Did you mean to push into the collection rather than rebuild it? A named struct or one level of indirection breaks the cycle.

## `arity-mismatch`

- **Severity**: error
- **Variant**: `ArityMismatch`
- **Guide**: <https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LOGOS_QUICKGUIDE.md#7-functions--closures>

**Example**:

```text
'greet' takes 2 argument(s), but this call passes 1
```

> 'greet' takes 2 argument(s), but this call passes 1. A call must fill every parameter — each one is a promise the function body relies on. Which parameter went missing, or which extra snuck in? Line the call up against the function's '## To' header.

## `field-not-found`

- **Severity**: error
- **Variant**: `FieldNotFound`
- **Guide**: <https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LOGOS_QUICKGUIDE.md#8-structs-enums--field-access>

**Example**:

```text
'Point' has no field named 'z'
```

> 'Point' has no field named 'z'. A struct's fields are fixed at its definition — reads cannot invent new ones. Available fields: x, y. Did you mean one of those, or should the '## A Point has:' definition grow a 'z'?

## `not-a-function`

- **Severity**: error
- **Variant**: `NotAFunction`
- **Guide**: <https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LOGOS_QUICKGUIDE.md#7-functions--closures>

**Example**:

```text
cannot call a value of type 'Int'
```

> This tries to call a value of type 'Int', but only functions and closures can be called. Parentheses after a name mean 'invoke this'. Did you want a function with a similar name, or is this variable shadowing the function you meant?

## `invalid-refinement-predicate`

- **Severity**: error
- **Variant**: `InvalidRefinementPredicate`

**Example**:

```text
refinement predicate is not checkable
```

> This refinement predicate is not one I can check — refinements are comparisons over the declared name, like 'x > 0' or 'n < 100'. What must always be true of this value? State it as a simple comparison; anything richer belongs in an Assert.

## `grammar-error`

- **Severity**: warning
- **Variant**: `GrammarError`

**Example**:

```text
grammar: its vs it's
```

> Grammar: its vs it's. Small grammar slips change the logical reading, so I stop rather than guess. Which meaning did you intend? Adjust the wording — the right form usually reads aloud correctly.

## `scope-violation`

- **Severity**: error
- **Variant**: `ScopeViolation`

**Example**:

```text
scope: pronoun trapped in negation
```

> Scope problem: pronoun trapped in negation. A pronoun can only reach referents its clause can see — things introduced under a negation or inside an 'or' are walled off from later sentences. Who is the pronoun meant to point at? Name the referent outright, or introduce it outside the wall.

## `unresolved-pronoun`

- **Severity**: error
- **Variant**: `UnresolvedPronoun`

**Example**:

```text
pronoun 'she' has no antecedent
```

> This pronoun ('she') has nothing to refer to — no earlier sentence introduced a matching referent this clause can reach. Pronouns only look backward through accessible discourse. Who is 'she' here? Introduce that referent in an earlier sentence, or use the name directly.

## `trailing-tokens`

- **Severity**: error
- **Variant**: `TrailingTokens`

**Example**:

```text
input left over, beginning with a comma (',')
```

> I understood the sentence up to here, but the rest — beginning with a comma (',') — does not fit the structure I built. Accepting it would silently drop that meaning, and LOGOS never drops meaning. Is a connective missing, or are two sentences sharing one period? End the first thought with '.' and start fresh.

## `ast-too-deep`

- **Severity**: error
- **Variant**: `AstTooDeep`

**Example**:

```text
AST nests 3000 levels deep; the limit is 2048
```

> This program nests expressions or blocks 3000 levels deep, past the current 2048-level limit — a tower this tall usually comes from generated code, and every downstream walker would overflow on it. Could each layer land in its own 'Let'? Intermediate bindings reset the depth to one; or raise the gate with LOGOS_MAX_AST_DEPTH=4096 if your stacks are deep.

## `custom`

- **Severity**: error
- **Variant**: `Custom`

**Example**:

```text
zone escape
```

> zone escape
