/**
 * LOGOS on one page, line for line the matrix the site's agent guide carries
 * (`apps/logicaffeine_web/src/ui/pages/landing/content.rs`, `SYNTAX_MATRIX`; the landing lock holds
 * the two equal). The model that mends a program reads it first, so it writes the language as it is.
 */
export const PRIMER_LINES = [
  "The language on one page. Every statement is one English sentence.",
  "Blocks: ## Main is the program. ## To add (a: Int, b: Int) -> Int: is a function.",
  "## A Point has: with An x: Int. below is a struct. ## A Shape is one of: is an enum.",
  "## Page /path is a web page. ## Route GET /api/x -> Seq of Todo: is an API route.",
  "## Test: name is a test of a page. ## Theorem: name is a theorem for the prover.",
  "Bind: Let x be 5. Let mutable n be 0. Let x: Int be 5. Change: Set n to n + 1.",
  "Also: Increase n by 1. Decrease n by 1. Set p's x to 5. Set item i of xs to v.",
  "Types: Int, Float, Text, Bool, Seq of Int, Map of Text to Int, Set of Int, some 30, none.",
  "Make: a new Seq of Int, [1, 2, 3], a new Map of Text to Int, {1, 2, 3}, a new Point.",
  "Read: item 1 of xs is the first item, length of xs, xs contains v, p's x, p.x.",
  "Slice: items 2 through 4 of xs. Change a list or set: Push v to xs. Pop from xs into y.",
  "Add v to s. Remove v from s. Arithmetic: x + y, x - y, x * y, x / y, x % y.",
  "Text: \"Hi, {name}!\" fills the hole with name, a combined with b joins two Texts.",
  "Compare: is equal to or ==, is not or !=, is less than or <, is greater than or >.",
  "Also: is at most or <=, is at least or >=, is between lo and hi, is divisible by n.",
  "Logic in a test: and, or, not. Choose: If c: then Otherwise If d: then Otherwise:.",
  "A block is the lines indented four spaces under the colon.",
  "Repeat: While c:, Repeat for x in xs:, Repeat for i from 1 to n:, Break.",
  "Match: Inspect t: then When V (a): and Otherwise:. Options: some 30, none.",
  "Filter: Let small be every n in xs where n is less than 4.",
  "In place: Remove every t from todos where t's done.",
  "Call: add(3, 7), or xs.f(a) for f(xs, a). A closure: (x: Int) -> x + 1. Return x.",
  "Show x. prints x. Contracts: Let x: Int where x > 0 be 5. Assert that x is equal to 42.",
  "Check that b is at least a. stops the program when it is not.",
  "Time: 500ms, 2s, 5min, 1h, 2026-05-20, 9am, 2 weeks, 2026-05-20 at 4pm.",
  "A page: State: holds Let draft be \"\". Actions: holds To add: and its steps.",
  "View: is HTML. {draft} is a hole, on-click={add}, bind={draft}, {if c:}...{else:}...{end}.",
  "Lists in a View: {for each t in todos, keyed by t's id:}...{end}. Style: is CSS, scoped.",
  "Derived state: Let left always be length of todos. It recomputes after every turn.",
  "A page test: Click \"#add\". Expect that the text of \"#n\" is equal to \"1\".",
  "Logic mode: one sentence per line. Every dog barks. Some cat sleeps. No cat barks.",
  "A theorem: Given: Every man is mortal. Given: Socrates is a man. Prove: He is mortal.",
  "Then Proof: Auto. and the kernel certifies it.",
];

export const PRIMER = PRIMER_LINES.join('\n');
